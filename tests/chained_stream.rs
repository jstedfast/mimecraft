// Chaining and seek cases adapted from MimeKit's UnitTests/IO/ChainedStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    cell::{Cell, RefCell},
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

use mimecraft::{
    ContentEncoding, HeaderList, MimeContent,
    io::{BoundedStream, ChainedStream},
};

#[test]
fn construction_and_append_are_lazy_and_ignore_initial_child_cursors() {
    let mut first = Probe::new(b"one");
    first.cursor.set_position(3);
    let mut second = Probe::new(b"two");
    second.cursor.set_position(2);
    let mut chain = ChainedStream::new();
    chain.add(first);
    chain.add(second);
    assert_eq!(chain.position(), 0);
    assert_eq!(chain.stream_position().unwrap(), 0);
    for child in chain.get_ref() {
        assert!(child.seeks.is_empty());
        assert_eq!(child.reads, 0);
    }
    let mut bytes = Vec::new();
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"onetwo");
    assert_eq!(chain.position(), 6);
}

#[test]
fn empty_chains_and_zero_length_operations_have_no_io() {
    let mut chain = ChainedStream::<Probe>::default();
    assert!(chain.is_empty().unwrap());
    assert_eq!(chain.read(&mut [0; 4]).unwrap(), 0);
    assert_eq!(
        chain.write(b"x").unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    chain.flush().unwrap();
    assert_eq!(chain.seek(SeekFrom::End(0)).unwrap(), 0);
    assert_eq!(
        chain.seek(SeekFrom::Start(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    chain.add(Probe::new(b"abc"));
    assert_eq!(chain.read(&mut []).unwrap(), 0);
    assert_eq!(chain.write(b"").unwrap(), 0);
    assert!(chain.get_ref()[0].seeks.is_empty());
    assert_eq!(chain.get_ref()[0].reads, 0);
    assert_eq!(chain.get_ref()[0].writes, 0);
    assert_eq!(chain.get_ref()[0].flushes, 0);
}

#[test]
fn reads_skip_empty_children_and_append_resumes_after_eof() {
    let mut chain: ChainedStream<_> = [b"".as_slice(), b"a", b"", b"bc", b""]
        .into_iter()
        .map(Cursor::new)
        .collect();
    let mut bytes = Vec::new();
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abc");
    assert_eq!(chain.position(), 3);
    assert_eq!(chain.read(&mut [0; 8]).unwrap(), 0);
    chain.add(Cursor::new(b"def".as_slice()));
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abcdef");
    assert_eq!(chain.position(), 6);
    chain.rewind().unwrap();
    let mut reread = Vec::new();
    chain.read_to_end(&mut reread).unwrap();
    assert_eq!(reread, bytes);
}

#[test]
fn every_seek_boundary_matches_a_contiguous_master() {
    let parts = [b"".as_slice(), b"abc", b"", b"de", b"fghi", b""];
    let mut chain: ChainedStream<_> = parts.into_iter().map(Cursor::new).collect();
    let master = b"abcdefghi";
    for target in (0..=master.len()).chain((0..=master.len()).rev()) {
        for kind in 0..3 {
            let seek = match kind {
                0 => SeekFrom::Start(target as u64),
                1 => SeekFrom::Current(target as i64 - chain.position() as i64),
                _ => SeekFrom::End(target as i64 - master.len() as i64),
            };
            assert_eq!(chain.seek(seek).unwrap(), target as u64);
            let mut bytes = Vec::new();
            chain.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, master[target..]);
            assert_eq!(chain.position(), master.len() as u64);
        }
    }
    for seek in [
        SeekFrom::Start(10),
        SeekFrom::End(1),
        SeekFrom::Current(-10),
        SeekFrom::End(i64::MIN),
    ] {
        assert_eq!(
            chain.seek(seek).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(chain.position(), 9);
    }
}

#[test]
fn deterministic_random_seeks_and_writes_match_the_master() {
    let bytes: Vec<u8> = (0..10 * 1024).map(|index| (index % 251) as u8).collect();
    let sizes = [0, 17, 0, 3000, 1, 4096, 3126, 0];
    let mut start = 0;
    let children: Vec<_> = sizes
        .into_iter()
        .map(|size| {
            let mut child = Probe::new(&bytes[start..start + size]);
            child.limit = 7;
            start += size;
            child
        })
        .collect();
    assert_eq!(start, bytes.len());
    let mut chain = ChainedStream::from(children);
    let mut master = Cursor::new(bytes);
    let mut seed = 0x1234_5678u32;
    // As in MimeKit's tests, operations on the chain should match one master.
    for attempt in 0..300 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let target = u64::from(seed) % (master.get_ref().len() as u64 + 1);
        let seek = match attempt % 3 {
            0 => SeekFrom::Start(target),
            1 => SeekFrom::Current(target as i64 - chain.position() as i64),
            _ => SeekFrom::End(target as i64 - master.get_ref().len() as i64),
        };
        assert_eq!(chain.seek(seek).unwrap(), master.seek(seek).unwrap());
        if attempt % 4 == 0 {
            let data = vec![(attempt % 256) as u8; 35];
            chain.write_all(&data).unwrap();
            master.write_all(&data).unwrap();
        } else {
            let count = (master.get_ref().len() as u64 - target).min(256) as usize;
            let mut actual = vec![0; count];
            let mut expected = vec![0; count];
            chain.read_exact(&mut actual).unwrap();
            master.read_exact(&mut expected).unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(chain.position(), master.position());
        assert_eq!(chain.len().unwrap(), master.get_ref().len() as u64);
    }
    chain.rewind().unwrap();
    let mut actual = Vec::new();
    chain.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, *master.get_ref());
}

#[test]
fn combined_length_queries_preserve_child_cursors_and_chain_position() {
    let mut first = Probe::new(b"abc");
    let mut second = Probe::new(b"defgh");
    first.cursor.set_position(1);
    second.cursor.set_position(4);
    let mut chain = ChainedStream::from(vec![first, second]);
    assert_eq!(chain.len().unwrap(), 8);
    assert_eq!(chain.get_ref()[0].cursor.position(), 1);
    assert_eq!(chain.get_ref()[1].cursor.position(), 4);
    assert_eq!(chain.position(), 0);
    assert!(!chain.is_empty().unwrap());
    let mut empty = ChainedStream::from(vec![Cursor::new(b""), Cursor::new(b"")]);
    assert!(empty.is_empty().unwrap());
    assert_eq!(empty.seek(SeekFrom::Start(0)).unwrap(), 0);
}

#[test]
fn later_read_errors_never_hide_a_successful_prefix() {
    let first = Probe::new(b"abc");
    let second = Probe::new(b"def");
    second.read_error.set(Some(io::ErrorKind::WouldBlock));
    let mut chain = ChainedStream::from(vec![first, second]);
    let mut buffer = [0; 8];
    assert_eq!(chain.read(&mut buffer).unwrap(), 3);
    assert_eq!(&buffer[..3], b"abc");
    assert_eq!(chain.position(), 3);
    assert_eq!(chain.get_ref()[1].reads, 0);
    assert_eq!(
        chain.read(&mut buffer).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(chain.position(), 3);
    assert_eq!(chain.read(&mut buffer).unwrap(), 3);
    assert_eq!(&buffer[..3], b"def");
    assert_eq!(chain.position(), 6);
}

#[test]
fn interrupted_reads_and_writes_follow_standard_exact_io_retries() {
    let first = Probe::new(b"ab");
    let second = Probe::new(b"cd");
    first.read_error.set(Some(io::ErrorKind::Interrupted));
    second.read_error.set(Some(io::ErrorKind::Interrupted));
    let mut chain = ChainedStream::from(vec![first, second]);
    let mut bytes = [0; 4];
    chain.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"abcd");
    chain.rewind().unwrap();
    chain.get_ref()[0]
        .write_error
        .set(Some(io::ErrorKind::Interrupted));
    chain.get_ref()[1]
        .write_error
        .set(Some(io::ErrorKind::Interrupted));
    chain.write_all(b"ABCD").unwrap();
    assert_eq!(chain.get_ref()[0].cursor.get_ref(), b"AB");
    assert_eq!(chain.get_ref()[1].cursor.get_ref(), b"CD");
    assert_eq!(chain.position(), 4);
}

#[test]
fn short_writes_fill_nonfinal_children_and_grow_only_the_last() {
    let mut first = Probe::new(b"...");
    first.limit = 1;
    let mut last = Probe::new(b"..");
    last.limit = 2;
    let mut chain = ChainedStream::from(vec![first, Probe::new(b""), last]);
    chain.write_all(b"abcdefghi").unwrap();
    assert_eq!(chain.get_ref()[0].cursor.get_ref(), b"abc");
    assert_eq!(chain.get_ref()[1].cursor.get_ref(), b"");
    assert_eq!(chain.get_ref()[2].cursor.get_ref(), b"defghi");
    assert_eq!(chain.get_ref()[0].flushes, 1);
    assert_eq!(chain.get_ref()[1].flushes, 1);
    assert_eq!(chain.get_ref()[2].flushes, 0);
    assert_eq!(chain.position(), 9);
    chain.flush().unwrap();
    assert_eq!(chain.get_ref()[2].flushes, 1);
    assert_eq!(chain.len().unwrap(), 9);
}

#[test]
fn failed_boundary_flush_does_not_duplicate_the_previous_write() {
    let first = Probe::new(b"..");
    first.flush_error.set(Some(io::ErrorKind::WouldBlock));
    let mut chain = ChainedStream::from(vec![first, Probe::new(b"..")]);
    assert_eq!(chain.write(b"abcd").unwrap(), 2);
    assert_eq!(chain.position(), 2);
    assert_eq!(
        chain.write(b"cd").unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(chain.position(), 2);
    assert_eq!(chain.get_ref()[0].cursor.get_ref(), b"ab");
    assert_eq!(chain.get_ref()[1].writes, 0);
    chain.write_all(b"cd").unwrap();
    assert_eq!(chain.position(), 4);
    assert_eq!(chain.get_ref()[0].cursor.get_ref(), b"ab");
    assert_eq!(chain.get_ref()[1].cursor.get_ref(), b"cd");
}

#[test]
fn explicit_flush_affects_only_the_current_child_and_can_be_retried() {
    let mut chain = ChainedStream::from(vec![Probe::new(b"ab"), Probe::new(b"cd")]);
    chain.seek(SeekFrom::Start(2)).unwrap();
    chain.get_ref()[1]
        .flush_error
        .set(Some(io::ErrorKind::Other));
    assert_eq!(chain.flush().unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(chain.position(), 2);
    assert_eq!(chain.get_ref()[0].flushes, 0);
    chain.flush().unwrap();
    assert_eq!(chain.get_ref()[1].flushes, 2);
}

#[test]
fn zero_writes_and_later_write_errors_do_not_advance_past_capacity() {
    let mut first = Probe::new(b"..");
    first.limit = 0;
    let mut chain = ChainedStream::from(vec![first, Probe::new(b"..")]);
    assert_eq!(
        chain.write_all(b"abcd").unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(chain.position(), 0);
    assert_eq!(chain.get_ref()[1].writes, 0);
    let second = Probe::new(b"..");
    second
        .write_error
        .set(Some(io::ErrorKind::PermissionDenied));
    let mut chain = ChainedStream::from(vec![Probe::new(b".."), second]);
    assert_eq!(chain.write(b"abcd").unwrap(), 2);
    assert_eq!(
        chain.write(b"cd").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(chain.position(), 2);
    chain.write_all(b"cd").unwrap();
    assert_eq!(chain.position(), 4);
}

#[test]
fn append_after_eof_also_redirects_writes_into_the_new_child() {
    let mut chain = ChainedStream::from(vec![Cursor::new(b"ab".to_vec())]);
    chain.read_to_end(&mut Vec::new()).unwrap();
    chain.add(Cursor::new(Vec::new()));
    chain.write_all(b"cd").unwrap();
    assert_eq!(chain.position(), 4);
    assert_eq!(chain.get_ref()[0].get_ref(), b"ab");
    assert_eq!(chain.get_ref()[1].get_ref(), b"cd");
}

#[test]
fn failed_seek_keeps_logical_position_and_reads_can_recover() {
    for failed_call in 1..=4 {
        let mut chain = ChainedStream::from(vec![Probe::new(b"abc"), Probe::new(b"def")]);
        let mut byte = [0];
        chain.read_exact(&mut byte).unwrap();
        // The second child is queried, restored, then selected by this seek.
        chain.get_ref()[1].fail_seek.set(Some(failed_call));
        assert_eq!(
            chain.seek(SeekFrom::Start(4)).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(chain.position(), 1);
        let mut rest = Vec::new();
        chain.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"bcdef");
    }
}

#[test]
fn length_restore_errors_and_clamped_seeks_are_explicit() {
    let source = Probe::new(b"abc");
    source.fail_seek.set(Some(3));
    let mut chain = ChainedStream::from(vec![source]);
    assert_eq!(
        chain.len().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(chain.position(), 0);
    assert_eq!(chain.len().unwrap(), 3);
    let mut bytes = Vec::new();
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abc");

    let source = Probe::new(b"abc");
    source.clamp_start.set(true);
    let mut chain = ChainedStream::from(vec![source]);
    assert_eq!(
        chain.seek(SeekFrom::Start(2)).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(chain.position(), 0);
}

#[test]
fn truncated_bounded_children_do_not_silently_shift_logical_offsets() {
    let first = BoundedStream::new(Cursor::new(b"a".as_slice()), 0, Some(3)).unwrap();
    let second = BoundedStream::new(Cursor::new(b"b".as_slice()), 0, Some(1)).unwrap();
    let mut chain = ChainedStream::from(vec![first, second]);
    let mut bytes = [0; 4];
    assert_eq!(chain.read(&mut bytes).unwrap(), 1);
    assert_eq!(
        chain.read(&mut bytes).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(chain.position(), 1);
    assert_eq!(chain.get_ref()[1].position(), 0);
    chain.seek(SeekFrom::Start(3)).unwrap();
    chain.read_exact(&mut bytes[..1]).unwrap();
    assert_eq!(bytes[0], b'b');
}

#[test]
fn shared_backing_ranges_read_and_write_without_copying_or_cursor_conflicts() {
    let data: Vec<u8> = (0..64).collect();
    let shared = Shared(Rc::new(RefCell::new(Cursor::new(data.clone()))));
    let mut chain: ChainedStream<_> = [(0, 0), (0, 17), (17, 17), (17, 40), (40, 64)]
        .into_iter()
        .map(|(start, end)| BoundedStream::new(shared.clone(), start, Some(end)).unwrap())
        .collect();
    let mut bytes = Vec::new();
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, data);
    chain.seek(SeekFrom::Start(15)).unwrap();
    chain.write_all(b"XYZW").unwrap();
    assert_eq!(&shared.0.borrow().get_ref()[15..19], b"XYZW");
    chain.seek(SeekFrom::Start(15)).unwrap();
    let mut read = [0; 4];
    chain.read_exact(&mut read).unwrap();
    assert_eq!(&read, b"XYZW");
    chain.seek(SeekFrom::End(0)).unwrap();
    assert!(chain.write(b"x").is_err());
    assert_eq!(chain.position(), 64);
}

#[test]
fn fixed_final_range_overflow_keeps_previously_reported_prefix() {
    let first = BoundedStream::new(Cursor::new(vec![0; 2]), 0, Some(2)).unwrap();
    let second = BoundedStream::new(Cursor::new(vec![0; 2]), 0, Some(2)).unwrap();
    let mut chain = ChainedStream::from(vec![first, second]);
    assert_eq!(chain.write(b"abcde").unwrap(), 2);
    assert_eq!(
        chain.write(b"cde").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(chain.position(), 2);
    chain.write_all(b"cd").unwrap();
    assert!(chain.write(b"e").is_err());
    chain.rewind().unwrap();
    let mut bytes = Vec::new();
    chain.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abcd");
}

#[test]
fn headers_and_body_can_be_parsed_across_child_boundaries() {
    let parts = [
        b"Content-T".as_slice(),
        b"ype: text/plain\r",
        b"\n\r",
        b"\nHello, world!\r\n",
    ];
    let chain: ChainedStream<_> = parts.into_iter().map(Cursor::new).collect();
    let mut reader = BufReader::with_capacity(3, chain);
    let headers = HeaderList::load(&mut reader).unwrap();
    assert_eq!(
        headers.get("content-type").unwrap().decode_value().text,
        "text/plain"
    );
    let mut body = String::new();
    reader.read_to_string(&mut body).unwrap();
    assert_eq!(body, "Hello, world!\r\n");
}

#[test]
fn mime_content_decodes_across_chained_encoded_payload_fragments() {
    let chain: ChainedStream<_> = [b"Q".as_slice(), b"2F", b"", b"0\r\n"]
        .into_iter()
        .map(Cursor::new)
        .collect();
    let mut content = MimeContent::with_encoding(chain, ContentEncoding::Base64);
    let mut decoded = Vec::new();
    assert_eq!(content.decode_to(&mut decoded).unwrap(), 3);
    assert_eq!(decoded, b"Cat");
    assert_eq!(content.get_ref().position(), 0);
    let mut raw = Vec::new();
    assert_eq!(content.write_to(&mut raw).unwrap(), 6);
    assert_eq!(raw, b"Q2F0\r\n");
}

#[test]
fn borrowed_children_are_not_flushed_or_rewound_on_drop() {
    let mut first = Probe::new(b"abc");
    let mut second = Probe::new(b"def");
    {
        let mut chain = ChainedStream::from(vec![&mut first, &mut second]);
        chain.write_all(b"x").unwrap();
    }
    assert_eq!(first.cursor.position(), 1);
    assert_eq!(first.cursor.get_ref(), b"xbc");
    assert_eq!(first.flushes, 0);
    assert_eq!(second.cursor.position(), 0);
    let chain = ChainedStream::from(vec![first, second]);
    let children = chain.into_inner();
    assert_eq!(children[0].cursor.position(), 1);
    assert_eq!(children[0].flushes, 0);
}

#[test]
fn combined_length_overflow_and_extreme_seeks_are_checked() {
    let mut chain = ChainedStream::from(vec![Huge::new(u64::MAX), Huge::new(1)]);
    assert_eq!(chain.len().unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        chain.seek(SeekFrom::Start(0)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(chain.position(), 0);
    let mut chain = ChainedStream::from(vec![Huge::new(u64::MAX)]);
    assert_eq!(chain.seek(SeekFrom::End(0)).unwrap(), u64::MAX);
    assert_eq!(chain.read(&mut [0; 1]).unwrap(), 0);
    assert_eq!(
        chain.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        chain.seek(SeekFrom::Current(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(chain.seek(SeekFrom::Start(1u64 << 63)).unwrap(), 1u64 << 63);
    assert_eq!(chain.seek(SeekFrom::Current(i64::MIN)).unwrap(), 0);
    chain.seek(SeekFrom::End(-1)).unwrap();
    assert_eq!(chain.read(&mut [0; 2]).unwrap(), 1);
    assert_eq!(chain.position(), u64::MAX);
    assert_eq!(chain.read(&mut [0; 2]).unwrap(), 0);
    chain.seek(SeekFrom::End(-1)).unwrap();
    assert_eq!(chain.write(b"xy").unwrap(), 1);
    assert_eq!(chain.position(), u64::MAX);
}

#[derive(Debug)]
struct Probe {
    cursor: Cursor<Vec<u8>>,
    limit: usize,
    reads: usize,
    writes: usize,
    flushes: usize,
    seeks: Vec<SeekFrom>,
    read_error: Cell<Option<io::ErrorKind>>,
    write_error: Cell<Option<io::ErrorKind>>,
    flush_error: Cell<Option<io::ErrorKind>>,
    fail_seek: Cell<Option<usize>>,
    clamp_start: Cell<bool>,
}

impl Probe {
    fn new(bytes: &[u8]) -> Self {
        Self {
            cursor: Cursor::new(bytes.to_vec()),
            limit: usize::MAX,
            reads: 0,
            writes: 0,
            flushes: 0,
            seeks: Vec::new(),
            read_error: Cell::new(None),
            write_error: Cell::new(None),
            flush_error: Cell::new(None),
            fail_seek: Cell::new(None),
            clamp_start: Cell::new(false),
        }
    }
}

impl Read for Probe {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if let Some(kind) = self.read_error.take() {
            return Err(kind.into());
        }
        let count = output.len().min(self.limit);
        self.cursor.read(&mut output[..count])
    }
}

impl Write for Probe {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if let Some(kind) = self.write_error.take() {
            return Err(kind.into());
        }
        self.cursor.write(&input[..input.len().min(self.limit)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        match self.flush_error.take() {
            Some(kind) => Err(kind.into()),
            None => Ok(()),
        }
    }
}

impl Seek for Probe {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.seeks.push(from);
        if self.fail_seek.get() == Some(self.seeks.len()) {
            self.cursor.set_position(0);
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if self.clamp_start.get() && matches!(from, SeekFrom::Start(_)) {
            return self.cursor.seek(SeekFrom::Start(0));
        }
        self.cursor.seek(from)
    }
}

#[derive(Clone)]
struct Shared(Rc<RefCell<Cursor<Vec<u8>>>>);
impl Read for Shared {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(output)
    }
}
impl Write for Shared {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(input)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.borrow_mut().flush()
    }
}
impl Seek for Shared {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(from)
    }
}

struct Huge {
    length: u64,
    position: u64,
}
impl Huge {
    fn new(length: u64) -> Self {
        Self {
            length,
            position: 0,
        }
    }
}
impl Seek for Huge {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.length.checked_add_signed(offset),
        }
        .ok_or(io::ErrorKind::InvalidInput)?;
        Ok(self.position)
    }
}
impl Read for Huge {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output
            .len()
            .min(usize::try_from(self.length - self.position).unwrap_or(usize::MAX));
        output[..count].fill(0);
        self.position += count as u64;
        Ok(count)
    }
}
impl Write for Huge {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.position = self
            .position
            .checked_add(input.len() as u64)
            .ok_or(io::ErrorKind::InvalidInput)?;
        self.length = self.length.max(self.position);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

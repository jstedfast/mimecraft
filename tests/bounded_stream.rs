// Range and seek cases adapted from MimeKit's UnitTests/IO/BoundedStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{ContentEncoding, MimeContent, io::BoundedStream};

#[test]
fn construction_is_lazy_and_boundaries_are_absolute() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.cursor.set_position(14);
    let mut stream = BoundedStream::new(&mut source, 6, Some(10)).unwrap();
    assert_eq!(stream.start_boundary(), 6);
    assert_eq!(stream.end_boundary(), Some(10));
    assert_eq!(stream.position(), 0);
    assert_eq!(stream.stream_position().unwrap(), 0);
    assert_eq!(stream.len().unwrap(), 4);
    assert!(!stream.is_empty().unwrap());
    assert_eq!(stream.get_ref().cursor.position(), 14);
    assert!(stream.get_ref().seeks.is_empty());
    let mut body = String::new();
    stream.read_to_string(&mut body).unwrap();
    assert_eq!(body, "BODY");
    assert_eq!(stream.position(), 4);
    assert_eq!(stream.get_ref().cursor.position(), 10);
    let source = stream.into_inner();
    assert_eq!(source.cursor.position(), 10);
    assert_eq!(source.flushes, 0);
}

#[test]
fn invalid_and_empty_ranges_do_not_access_the_source() {
    let mut source = Probe::new(b"data");
    assert_eq!(
        BoundedStream::new(&mut source, 5, Some(4))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(source.seeks.is_empty());
    let mut stream = BoundedStream::new(source, 5, Some(5)).unwrap();
    assert!(stream.is_empty().unwrap());
    assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
    assert_eq!(stream.read(&mut []).unwrap(), 0);
    assert_eq!(stream.write(b"").unwrap(), 0);
    assert_eq!(
        stream.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(stream.get_ref().seeks.is_empty());
    assert_eq!(stream.get_ref().reads, 0);
    assert_eq!(stream.get_ref().writes, 0);
}

#[test]
fn source_seek_vector_supports_all_relative_origins() {
    let input = b"This is some text...";
    for end in [None, Some(input.len() as u64)] {
        let mut source = Cursor::new(input);
        source.set_position(input.len() as u64);
        let mut stream = BoundedStream::new(source, 5, end).unwrap();
        for seek in [
            None,
            Some(SeekFrom::End(-15)),
            Some(SeekFrom::Start(0)),
            Some(SeekFrom::Current(-15)),
        ] {
            if let Some(seek) = seek {
                assert_eq!(stream.seek(seek).unwrap(), 0);
            }
            let mut text = String::new();
            stream.read_to_string(&mut text).unwrap();
            assert_eq!(text, "is some text...");
            assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
        }
        assert_eq!(
            stream.seek(SeekFrom::Current(-16)).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            stream.seek(SeekFrom::End(-20)).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        if end.is_some() {
            assert_eq!(
                stream.seek(SeekFrom::End(5)).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}

#[test]
fn end_relative_seeks_use_the_view_end_not_the_source_end() {
    let mut stream = BoundedStream::new(Probe::new(b"prefixBODYsuffix"), 6, Some(10)).unwrap();
    assert_eq!(stream.seek(SeekFrom::End(0)).unwrap(), 4);
    assert_eq!(stream.get_ref().cursor.position(), 10);
    assert_eq!(stream.seek(SeekFrom::End(-2)).unwrap(), 2);
    assert_eq!(stream.get_ref().cursor.position(), 8);
    let calls = stream.get_ref().seeks.len();
    for seek in [SeekFrom::Start(5), SeekFrom::End(1), SeekFrom::Current(-3)] {
        assert_eq!(
            stream.seek(seek).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(stream.position(), 2);
        assert_eq!(stream.get_ref().cursor.position(), 8);
        assert_eq!(stream.get_ref().seeks.len(), calls);
    }
}

#[test]
fn generated_ranges_read_exactly_the_requested_slice() {
    let input: Vec<u8> = (0..32).collect();
    for start in 0..=input.len() {
        for end in start..=input.len() {
            for chunk in [1, 3, 32, 64] {
                let mut stream =
                    BoundedStream::new(Cursor::new(&input), start as u64, Some(end as u64))
                        .unwrap();
                let mut result = Vec::new();
                let mut buffer = vec![0; chunk];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    result.extend_from_slice(&buffer[..count]);
                }
                assert_eq!(result, input[start..end]);
                assert_eq!(stream.position(), (end - start) as u64);
                stream.rewind().unwrap();
                assert_eq!(stream.position(), 0);
                assert_eq!(stream.get_ref().position(), start as u64);
            }
        }
    }
}

#[test]
fn short_and_interrupted_reads_update_only_successful_progress() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.limit = 1;
    source.read_error = Some(io::ErrorKind::Interrupted);
    let mut stream = BoundedStream::new(source, 6, Some(10)).unwrap();
    assert_eq!(
        stream.read(&mut [0; 4]).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(stream.position(), 0);
    let mut body = [0; 4];
    stream.read_exact(&mut body).unwrap();
    assert_eq!(&body, b"BODY");
    assert_eq!(stream.position(), 4);
    let reads = stream.get_ref().reads;
    let seeks = stream.get_ref().seeks.len();
    assert_eq!(stream.read(&mut [0; 4]).unwrap(), 0);
    assert_eq!(stream.get_ref().reads, reads);
    assert_eq!(stream.get_ref().seeks.len(), seeks);
}

#[test]
#[allow(
    clippy::seek_from_current,
    reason = "A zero seek must resynchronize the source; querying position must not."
)]
fn source_cursor_changes_are_repaired_for_reads_writes_and_same_position_seeks() {
    let mut stream = BoundedStream::new(Probe::new(b"prefixBODYsuffix"), 6, Some(10)).unwrap();
    let mut byte = [0];
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"B");
    stream.get_mut().cursor.set_position(0);
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"O");
    stream.get_mut().cursor.set_position(15);
    stream.write_all(b"d").unwrap();
    assert_eq!(stream.get_ref().cursor.get_ref(), b"prefixBOdYsuffix");
    stream.get_mut().cursor.set_position(0);
    assert_eq!(stream.seek(SeekFrom::Current(0)).unwrap(), 3);
    assert_eq!(stream.get_ref().cursor.position(), 9);
    stream.get_mut().cursor.set_position(0);
    assert_eq!(stream.stream_position().unwrap(), 3);
    assert_eq!(stream.get_ref().cursor.position(), 0);
}

#[test]
fn unbounded_length_queries_restore_the_real_source_cursor() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.cursor.set_position(2);
    let mut stream = BoundedStream::new(source, 6, None).unwrap();
    assert_eq!(stream.len().unwrap(), 10);
    assert_eq!(stream.get_ref().cursor.position(), 2);
    assert_eq!(stream.position(), 0);
    assert_eq!(stream.seek(SeekFrom::End(-6)).unwrap(), 4);
    assert_eq!(stream.get_ref().cursor.position(), 10);
    assert_eq!(stream.seek(SeekFrom::End(3)).unwrap(), 13);
    assert_eq!(stream.get_ref().cursor.position(), 19);
    assert_eq!(stream.len().unwrap(), 10);
    assert_eq!(stream.get_ref().cursor.position(), 19);
    assert_eq!(stream.position(), 13);
}

#[test]
fn early_eof_does_not_shorten_fixed_ranges_or_hide_later_growth() {
    for end in [None, Some(10)] {
        let mut stream = BoundedStream::new(Cursor::new(b"prefixB".to_vec()), 6, end).unwrap();
        let mut body = Vec::new();
        stream.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"B");
        assert_eq!(stream.position(), 1);
        assert_eq!(stream.len().unwrap(), if end.is_some() { 4 } else { 1 });
        assert_eq!(stream.read(&mut [0; 4]).unwrap(), 0);
        stream.get_mut().get_mut().extend_from_slice(b"ODY");
        assert_eq!(stream.len().unwrap(), 4);
        stream.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"BODY");
        assert_eq!(stream.position(), 4);
    }
}

#[test]
fn source_ending_before_start_has_explicit_length_error() {
    let mut stream = BoundedStream::new(Cursor::new(b"short"), 10, None).unwrap();
    stream.get_mut().set_position(2);
    assert_eq!(
        stream.len().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(stream.get_ref().position(), 2);
    assert_eq!(stream.position(), 0);
    assert_eq!(
        stream.seek(SeekFrom::End(0)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(stream.read(&mut [0; 4]).unwrap(), 0);
    let mut fixed = BoundedStream::new(Cursor::new(b"short"), 10, Some(20)).unwrap();
    assert_eq!(fixed.len().unwrap(), 10);
    assert_eq!(fixed.read(&mut [0; 4]).unwrap(), 0);
}

#[test]
fn writes_reject_an_overlong_request_before_any_io() {
    let mut source = Probe::new(b"A");
    source.cursor.set_position(1);
    let mut stream = BoundedStream::new(source, 0, Some(2)).unwrap();
    assert_eq!(
        stream.write(b"bcd").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(stream.position(), 0);
    assert_eq!(stream.get_ref().cursor.get_ref(), b"A");
    assert_eq!(stream.get_ref().cursor.position(), 1);
    assert!(stream.get_ref().seeks.is_empty());
    assert_eq!(stream.get_ref().writes, 0);
    assert_eq!(
        stream.write_all(b"bcd").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(stream.get_ref().writes, 0);
}

#[test]
fn short_writes_preserve_neighbors_and_advance_by_actual_count() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.limit = 2;
    let mut stream = BoundedStream::new(source, 6, Some(10)).unwrap();
    assert_eq!(stream.write(b"body").unwrap(), 2);
    assert_eq!(stream.position(), 2);
    stream.write_all(b"dy").unwrap();
    assert_eq!(stream.get_ref().cursor.get_ref(), b"prefixbodysuffix");
    assert_eq!(stream.position(), 4);
    assert_eq!(
        stream.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    stream.flush().unwrap();
    assert_eq!(stream.get_ref().flushes, 1);
    assert_eq!(stream.position(), 4);
}

#[test]
fn writing_can_grow_a_source_within_the_configured_range() {
    for end in [None, Some(8)] {
        let mut stream = BoundedStream::new(Cursor::new(b"pre".to_vec()), 3, end).unwrap();
        stream.write_all(b"BODY!").unwrap();
        assert_eq!(stream.get_ref().get_ref(), b"preBODY!");
        assert_eq!(stream.len().unwrap(), 5);
        assert_eq!(stream.position(), 5);
    }
}

#[test]
fn write_zero_interruption_and_flush_errors_are_propagated() {
    let mut source = Probe::new(b"....");
    source.write_error = Some(io::ErrorKind::Interrupted);
    let mut stream = BoundedStream::new(source, 1, Some(3)).unwrap();
    stream.write_all(b"hi").unwrap();
    assert_eq!(stream.get_ref().cursor.get_ref(), b".hi.");
    stream.rewind().unwrap();
    stream.get_mut().write_error = Some(io::ErrorKind::PermissionDenied);
    assert_eq!(
        stream.write(b"x").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(stream.position(), 0);
    stream.get_mut().limit = 0;
    assert_eq!(
        stream.write_all(b"x").unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(stream.position(), 0);
    stream.get_mut().flush_error = true;
    assert_eq!(stream.flush().unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(stream.position(), 0);
}

#[test]
fn seek_and_position_query_failures_do_not_advance_the_view() {
    for fail_seek in [1, 2] {
        let mut source = Probe::new(b"prefixBODYsuffix");
        source.fail_seek = Some(fail_seek);
        let mut stream = BoundedStream::new(source, 6, Some(10)).unwrap();
        assert_eq!(
            stream.read(&mut [0; 4]).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(stream.position(), 0);
        assert_eq!(stream.get_ref().reads, 0);
        let mut body = [0; 4];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"BODY");
    }
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.fail_seek = Some(1);
    let mut stream = BoundedStream::new(source, 6, Some(10)).unwrap();
    assert!(stream.seek(SeekFrom::Start(2)).is_err());
    assert_eq!(stream.position(), 0);
    assert_eq!(stream.seek(SeekFrom::Start(2)).unwrap(), 2);
}

#[test]
fn length_query_and_restore_failures_are_reported_and_recoverable() {
    for fail_seek in 1..=3 {
        let mut source = Probe::new(b"prefixBODYsuffix");
        source.cursor.set_position(2);
        source.fail_seek = Some(fail_seek);
        let mut stream = BoundedStream::new(source, 6, None).unwrap();
        assert_eq!(
            stream.len().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(stream.position(), 0);
        let mut body = [0; 4];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"BODY");
    }
}

#[test]
fn clamped_source_seeks_cannot_escape_the_range() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    source.clamp_start = true;
    let mut stream = BoundedStream::new(source, 6, Some(10)).unwrap();
    assert_eq!(
        stream.read(&mut [0; 4]).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        stream.write(b"bad").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        stream.seek(SeekFrom::Start(2)).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(stream.position(), 0);
    assert_eq!(stream.get_ref().reads, 0);
    assert_eq!(stream.get_ref().writes, 0);
}

#[test]
fn arithmetic_checks_cover_large_offsets_and_signed_extremes() {
    let mut stream =
        BoundedStream::new(Cursor::new(Vec::<u8>::new()), u64::MAX - 2, Some(u64::MAX)).unwrap();
    assert_eq!(stream.len().unwrap(), 2);
    assert_eq!(stream.seek(SeekFrom::End(0)).unwrap(), 2);
    assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
    for seek in [
        SeekFrom::Start(3),
        SeekFrom::Current(i64::MAX),
        SeekFrom::End(i64::MIN),
    ] {
        assert_eq!(
            stream.seek(seek).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(stream.position(), 2);
    }
    assert_eq!(
        stream.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut stream = BoundedStream::new(Cursor::new(Vec::<u8>::new()), u64::MAX, None).unwrap();
    assert_eq!(stream.seek(SeekFrom::Start(0)).unwrap(), 0);
    assert_eq!(
        stream.read(&mut [0; 1]).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        stream.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        stream.seek(SeekFrom::Start(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(stream.read(&mut []).unwrap(), 0);
    assert_eq!(stream.write(b"").unwrap(), 0);
    let mut stream = BoundedStream::new(Cursor::new(b""), 0, None).unwrap();
    stream.seek(SeekFrom::Start(1u64 << 63)).unwrap();
    assert_eq!(stream.seek(SeekFrom::Current(i64::MIN)).unwrap(), 0);
}

#[test]
fn nested_ranges_use_their_immediate_source_coordinates() {
    let outer = BoundedStream::new(Cursor::new(b"prefixBODYsuffix"), 6, Some(10)).unwrap();
    let mut inner = BoundedStream::new(outer, 1, Some(3)).unwrap();
    let mut bytes = Vec::new();
    inner.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"OD");
    assert_eq!(inner.position(), 2);
    assert_eq!(inner.get_ref().position(), 3);
    assert_eq!(inner.get_ref().get_ref().position(), 9);
    inner.rewind().unwrap();
    assert_eq!(inner.get_ref().position(), 1);
}

#[test]
fn mime_content_copies_and_decodes_only_its_bounded_body() {
    let mut source = Cursor::new(b"header\r\nQ2F0\r\nsuffix");
    let bounded = BoundedStream::new(&mut source, 8, Some(14)).unwrap();
    let mut content = MimeContent::with_encoding(bounded, ContentEncoding::Base64);
    let mut decoded = Vec::new();
    assert_eq!(content.decode_to(&mut decoded).unwrap(), 3);
    assert_eq!(decoded, b"Cat");
    assert_eq!(content.get_ref().position(), 0);
    let mut raw = Vec::new();
    assert_eq!(content.write_to(&mut raw).unwrap(), 6);
    assert_eq!(raw, b"Q2F0\r\n");
    let mut opened = content.open().unwrap();
    let mut decoded = Vec::new();
    opened.read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, b"Cat");
    drop(opened);
    let bounded = content.into_inner();
    let source = bounded.into_inner();
    assert_eq!(source.position(), 14);
    let mut suffix = Vec::new();
    source.read_to_end(&mut suffix).unwrap();
    assert_eq!(suffix, b"suffix");
}

#[test]
fn dropping_a_borrowed_view_neither_flushes_nor_restores_position() {
    let mut source = Probe::new(b"prefixBODYsuffix");
    {
        let mut stream = BoundedStream::new(&mut source, 6, Some(10)).unwrap();
        stream.write_all(b"b").unwrap();
    }
    assert_eq!(source.cursor.position(), 7);
    assert_eq!(source.cursor.get_ref(), b"prefixbODYsuffix");
    assert_eq!(source.flushes, 0);
}

#[derive(Debug)]
struct Probe {
    cursor: Cursor<Vec<u8>>,
    limit: usize,
    reads: usize,
    writes: usize,
    flushes: usize,
    seeks: Vec<SeekFrom>,
    read_error: Option<io::ErrorKind>,
    write_error: Option<io::ErrorKind>,
    fail_seek: Option<usize>,
    flush_error: bool,
    clamp_start: bool,
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
            read_error: None,
            write_error: None,
            fail_seek: None,
            flush_error: false,
            clamp_start: false,
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
        if self.flush_error {
            Err(io::Error::other("flush failed"))
        } else {
            Ok(())
        }
    }
}

impl Seek for Probe {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.seeks.push(from);
        if self.fail_seek == Some(self.seeks.len()) {
            self.cursor.set_position(0);
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        if self.clamp_start && matches!(from, SeekFrom::Start(_)) {
            return self.cursor.seek(SeekFrom::Start(0));
        }
        self.cursor.seek(from)
    }
}

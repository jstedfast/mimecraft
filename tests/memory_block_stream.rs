// Includes cases adapted from MimeKit's UnitTests/IO/MemoryBlockStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, HeaderList, MimeContent,
    encodings::Base64Encoder,
    filters::EncoderFilter,
    io::{BoundedStream, ChainedStream, FilteredWriter, MemoryBlockStream},
};

const BLOCK: usize = 2048;

fn bytes(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i * 31 + i / 251) as u8).collect()
}

fn stream(bytes: &[u8]) -> MemoryBlockStream {
    let mut stream = MemoryBlockStream::new();
    stream.write_all(bytes).unwrap();
    stream.rewind().unwrap();
    stream
}

#[test]
fn empty_stream_and_empty_io_do_not_move_the_cursor() {
    for mut stream in [MemoryBlockStream::new(), MemoryBlockStream::default()] {
        assert!(stream.is_empty());
        assert_eq!(stream.len(), 0);
        assert_eq!(stream.position(), 0);
        assert_eq!(stream.stream_position().unwrap(), 0);
        assert_eq!(stream.read(&mut [1; 16]).unwrap(), 0);
        assert_eq!(stream.read(&mut []).unwrap(), 0);
        assert_eq!(stream.write(b"").unwrap(), 0);
        stream.flush().unwrap();
        stream.set_len(0).unwrap();
        assert!(stream.to_vec().unwrap().is_empty());
        for from in [SeekFrom::Start(0), SeekFrom::Current(0), SeekFrom::End(0)] {
            assert_eq!(stream.seek(from).unwrap(), 0);
        }
    }

    let mut stream = stream(b"abc");
    stream.seek(SeekFrom::Start(1)).unwrap();
    assert_eq!(stream.read(&mut []).unwrap(), 0);
    assert_eq!(stream.write(b"").unwrap(), 0);
    stream.flush().unwrap();
    assert_eq!(stream.position(), 1);
    assert_eq!(stream.to_vec().unwrap(), b"abc");
}

#[test]
fn canonical_read_write_and_array_cases_cross_block_boundaries() {
    let data = bytes(9 * 1024);
    for write_size in [1, 7, BLOCK - 1, BLOCK, BLOCK + 1, 4096, data.len()] {
        let mut stream = MemoryBlockStream::new();
        // Write the content into the memory block stream in varying chunks.
        for chunk in data.chunks(write_size) {
            assert_eq!(stream.write(chunk).unwrap(), chunk.len());
        }
        assert_eq!(stream.len(), data.len() as u64);
        assert_eq!(stream.position(), stream.len());
        assert_eq!(stream.to_vec().unwrap(), data);
        for read_size in [1, 19, BLOCK - 1, BLOCK, BLOCK + 1, 4096, data.len() + 1] {
            stream.rewind().unwrap();
            // This is our master stream; all operations should match its results.
            let mut master = Cursor::new(&data);
            let mut output = vec![0xaa; read_size];
            let mut expected = output.clone();
            loop {
                let actual = stream.read(&mut output).unwrap();
                let count = master.read(&mut expected).unwrap();
                assert_eq!(actual, count);
                assert_eq!(output, expected);
                assert_eq!(stream.position(), master.position());
                if actual == 0 {
                    break;
                }
            }
        }
    }
}

#[test]
fn all_seek_origins_map_block_edges_to_the_correct_bytes() {
    let data = bytes(9 * 1024);
    let mut stream = stream(&data);
    for target in [0, 1, 2047, 2048, 2049, 4095, 4096, 4097, 9215, 9216] {
        for origin in 0..3 {
            let from = match origin {
                0 => SeekFrom::Start(target),
                1 => SeekFrom::Current(target as i64 - stream.position() as i64),
                _ => SeekFrom::End(target as i64 - data.len() as i64),
            };
            assert_eq!(stream.seek(from).unwrap(), target);
            let mut output = [0; 31];
            let count = stream.read(&mut output).unwrap();
            let start = target as usize;
            assert_eq!(count, 31.min(data.len() - start));
            assert_eq!(&output[..count], &data[start..start + count]);
            assert_eq!(stream.position(), target + count as u64);
        }
    }
}

#[test]
fn invalid_seeks_leave_contents_length_and_position_unchanged() {
    let mut stream = stream(b"abcdef");
    stream.seek(SeekFrom::Start(3)).unwrap();
    for from in [
        SeekFrom::Start(7),
        SeekFrom::Start(MemoryBlockStream::MAX_LEN),
        SeekFrom::Start(u64::MAX),
        SeekFrom::Current(-4),
        SeekFrom::Current(4),
        SeekFrom::Current(i64::MIN),
        SeekFrom::Current(i64::MAX),
        SeekFrom::End(-7),
        SeekFrom::End(1),
        SeekFrom::End(i64::MIN),
        SeekFrom::End(i64::MAX),
    ] {
        assert_eq!(
            stream.seek(from).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(stream.position(), 3);
        assert_eq!(stream.len(), 6);
        assert_eq!(stream.to_vec().unwrap(), b"abcdef");
    }
}

#[test]
fn shrinking_and_regrowing_never_exposes_truncated_bytes() {
    let data = vec![0xa5; BLOCK * 5 + 17];
    for length in [0, 1, BLOCK - 1, BLOCK, BLOCK + 1, BLOCK * 2, data.len()] {
        let mut stream = stream(&data);
        stream.seek(SeekFrom::End(0)).unwrap();
        stream.set_len(length as u64).unwrap();
        assert_eq!(stream.len(), length as u64);
        assert_eq!(stream.position(), length as u64);
        assert_eq!(stream.to_vec().unwrap(), data[..length]);
        stream.set_len((data.len() + BLOCK) as u64).unwrap();
        assert_eq!(stream.position(), length as u64);
        let grown = stream.to_vec().unwrap();
        assert_eq!(&grown[..length], &data[..length]);
        assert!(grown[length..].iter().all(|&byte| byte == 0));
        let mut tail = Vec::new();
        stream.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, grown[length..]);
    }
}

#[test]
fn resize_zero_fills_growth_and_clamps_only_positions_beyond_the_new_end() {
    let mut stream = MemoryBlockStream::new();
    stream.set_len((BLOCK * 3 + 7) as u64).unwrap();
    assert_eq!(stream.position(), 0);
    assert!(stream.to_vec().unwrap().iter().all(|&byte| byte == 0));
    stream.seek(SeekFrom::Start(10)).unwrap();
    stream.set_len((BLOCK * 4) as u64).unwrap();
    assert_eq!(stream.position(), 10);
    stream.set_len(11).unwrap();
    assert_eq!(stream.position(), 10);
    stream.set_len(11).unwrap();
    assert_eq!(stream.position(), 10);
    stream.set_len(7).unwrap();
    assert_eq!(stream.position(), 7);
    stream.write_all(b"tail").unwrap();
    assert_eq!(stream.to_vec().unwrap(), b"\0\0\0\0\0\0\0tail");
}

#[test]
fn oversized_resize_is_rejected_before_mutating_the_stream() {
    let mut stream = stream(b"body");
    stream.seek(SeekFrom::Start(2)).unwrap();
    let limit = MemoryBlockStream::MAX_LEN.min(usize::MAX as u64);
    for length in [limit + 1, u64::MAX] {
        assert_eq!(
            stream.set_len(length).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(stream.len(), 4);
        assert_eq!(stream.position(), 2);
        assert_eq!(stream.to_vec().unwrap(), b"body");
    }
}

#[test]
fn array_copy_is_complete_independent_and_does_not_seek() {
    let data = bytes(BLOCK * 3 + 1);
    let mut stream = stream(&data);
    stream.seek(SeekFrom::Start(BLOCK as u64)).unwrap();
    let mut copy = stream.to_vec().unwrap();
    assert_eq!(copy, data);
    copy.fill(0);
    assert_eq!(stream.to_vec().unwrap(), data);
    assert_eq!(stream.position(), BLOCK as u64);
}

#[test]
fn overwriting_spans_blocks_without_truncating_the_suffix() {
    let mut expected = bytes(BLOCK * 4 + 1);
    let mut stream = stream(&expected);
    let patch = vec![0xff; BLOCK + 7];
    stream.seek(SeekFrom::Start((BLOCK - 2) as u64)).unwrap();
    stream.write_all(&patch).unwrap();
    expected[BLOCK - 2..BLOCK - 2 + patch.len()].copy_from_slice(&patch);
    assert_eq!(stream.to_vec().unwrap(), expected);
    assert_eq!(stream.len(), expected.len() as u64);
    stream.seek(SeekFrom::End(-1)).unwrap();
    stream.write_all(b"XYZ").unwrap();
    expected.pop();
    expected.extend_from_slice(b"XYZ");
    assert_eq!(stream.to_vec().unwrap(), expected);
    assert_eq!(stream.position(), expected.len() as u64);
}

#[test]
fn deterministic_mixed_operations_match_a_contiguous_model() {
    let mut stream = MemoryBlockStream::new();
    let mut master = Cursor::new(Vec::new());
    let mut seed = 0x59b2_0e38_u32;
    let mut coverage = [0; 6];
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as usize
    };
    for step in 0..700 {
        let operation = next() % coverage.len();
        coverage[operation] += 1;
        match operation {
            0 => {
                let input = bytes(next() % 4097);
                assert_eq!(stream.write(&input).unwrap(), master.write(&input).unwrap());
            }
            1 => {
                let mut actual = vec![0x7f; next() % 5000];
                let mut expected = actual.clone();
                assert_eq!(
                    stream.read(&mut actual).unwrap(),
                    master.read(&mut expected).unwrap()
                );
                assert_eq!(actual, expected);
            }
            2 => {
                let length = next() % (BLOCK * 12);
                stream.set_len(length as u64).unwrap();
                master.get_mut().resize(length, 0);
                master.set_position(master.position().min(length as u64));
            }
            3 => {
                let target = (next() % (master.get_ref().len() + 1)) as u64;
                let from = match next() % 3 {
                    0 => SeekFrom::Start(target),
                    1 => SeekFrom::Current(target as i64 - master.position() as i64),
                    _ => SeekFrom::End(target as i64 - master.get_ref().len() as i64),
                };
                assert_eq!(stream.seek(from).unwrap(), master.seek(from).unwrap());
            }
            4 => {
                assert_eq!(
                    stream.seek(SeekFrom::End(1)).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
            _ => stream.flush().unwrap(),
        }
        assert_eq!(stream.position(), master.position(), "step {step}");
        assert_eq!(stream.len(), master.get_ref().len() as u64, "step {step}");
        assert_eq!(&stream.to_vec().unwrap(), master.get_ref(), "step {step}");
    }
    assert!(coverage.into_iter().all(|count| count > 30));
}

#[test]
fn filtered_writing_and_mime_content_decode_across_many_blocks() {
    let payload = bytes(BLOCK * 5 + 43);
    let mut encoded = MemoryBlockStream::new();
    {
        let mut writer =
            FilteredWriter::new(&mut encoded, EncoderFilter::new(Base64Encoder::new()));
        for chunk in payload.chunks(137) {
            writer.write_all(chunk).unwrap();
        }
        writer.finish().unwrap();
    }
    let wire = encoded.to_vec().unwrap();
    let mut content = MimeContent::with_encoding(encoded, ContentEncoding::Base64);
    let mut raw = MemoryBlockStream::new();
    assert_eq!(content.write_to(&mut raw).unwrap(), wire.len() as u64);
    assert_eq!(raw.to_vec().unwrap(), wire);
    let mut decoded = MemoryBlockStream::new();
    assert_eq!(
        content.decode_to(&mut decoded).unwrap(),
        payload.len() as u64
    );
    assert_eq!(decoded.to_vec().unwrap(), payload);
    let mut again = Vec::new();
    content.open().unwrap().read_to_end(&mut again).unwrap();
    assert_eq!(again, payload);
}

#[test]
fn buffered_header_loading_preserves_body_read_ahead() {
    let mut wire = b"Subject: ".to_vec();
    wire.extend(vec![b'x'; BLOCK + 7]);
    wire.extend_from_slice(b"\r\nContent-Type: text/plain\r\n\r\n");
    let body = bytes(BLOCK * 2 + 1);
    wire.extend_from_slice(&body);
    let mut reader = BufReader::with_capacity(37, stream(&wire));
    let headers = HeaderList::load(&mut reader).unwrap();
    assert_eq!(
        headers.get("subject").unwrap().decode_value().text,
        "x".repeat(BLOCK + 7)
    );
    assert_eq!(
        headers.get("content-type").unwrap().decode_value().text,
        "text/plain"
    );
    let mut output = Vec::new();
    reader.read_to_end(&mut output).unwrap();
    assert_eq!(output, body);
}

#[test]
fn bounded_ranges_read_and_overwrite_the_same_block_storage() {
    let mut expected = bytes(BLOCK * 4);
    let mut source = stream(&expected);
    {
        let start = BLOCK - 5;
        let end = BLOCK * 3 + 3;
        let mut range = BoundedStream::new(&mut source, start as u64, Some(end as u64)).unwrap();
        let mut output = Vec::new();
        range.read_to_end(&mut output).unwrap();
        assert_eq!(output, expected[start..end]);
        range.rewind().unwrap();
        range.write_all(b"replacement").unwrap();
        expected[start..start + 11].copy_from_slice(b"replacement");
    }
    assert_eq!(source.to_vec().unwrap(), expected);
}

#[test]
fn chaining_block_streams_preserves_nonfinal_lengths_and_grows_the_last() {
    let first = bytes(BLOCK + 3);
    let second = bytes(BLOCK - 1);
    let mut expected = [first.as_slice(), second.as_slice()].concat();
    let mut chain = ChainedStream::from(vec![stream(&first), stream(&second)]);
    chain.seek(SeekFrom::Start(BLOCK as u64)).unwrap();
    let patch = vec![0xa5; BLOCK * 2];
    chain.write_all(&patch).unwrap();
    expected.resize(BLOCK + patch.len(), 0);
    expected[BLOCK..].copy_from_slice(&patch);
    assert_eq!(chain.get_ref()[0].len(), first.len() as u64);
    assert_eq!(chain.len().unwrap(), expected.len() as u64);
    chain.rewind().unwrap();
    let mut actual = Vec::new();
    chain.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, expected);
}

// Includes cases adapted from MimeKit's UnitTests/IO/MeasuringStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, HeaderList, MimeContent,
    encodings::{Base64Encoder, NewLineFormat},
    filters::{EncoderFilter, encoder_filter},
    io::{BoundedStream, ChainedStream, FilteredWriter, MeasuringStream},
};

#[test]
fn construction_and_empty_io_are_noops() {
    for mut sink in [MeasuringStream::new(), MeasuringStream::default()] {
        assert!(sink.is_empty());
        assert_eq!(sink.len(), 0);
        assert_eq!(sink.position(), 0);
        assert_eq!(sink.stream_position().unwrap(), 0);
        assert_eq!(sink.write(b"").unwrap(), 0);
        sink.flush().unwrap();
        for from in [SeekFrom::Start(0), SeekFrom::Current(0), SeekFrom::End(0)] {
            assert_eq!(sink.seek(from).unwrap(), 0);
        }
        assert!(sink.is_empty());
    }
}

#[test]
fn canonical_writes_measure_length_without_retaining_contents() {
    let mut sink = MeasuringStream::new();
    let bytes: Vec<u8> = (0..1099).map(|i| (i * 37) as u8).collect();
    assert_eq!(sink.write(&bytes).unwrap(), 1099);
    sink.flush().unwrap();
    assert_eq!(sink.len(), 1099);
    assert_eq!(sink.position(), 1099);
    assert!(!sink.is_empty());
    sink.write_all(&bytes).unwrap();
    assert_eq!(sink.len(), 2198);
    assert_eq!(sink.position(), 2198);
}

#[test]
fn overwrites_measure_extent_rather_than_cumulative_writes() {
    let mut sink = MeasuringStream::new();
    sink.write_all(b"abcdef").unwrap();
    sink.rewind().unwrap();
    sink.write_all(b"xy").unwrap();
    assert_eq!(sink.len(), 6);
    assert_eq!(sink.position(), 2);
    sink.seek(SeekFrom::End(-1)).unwrap();
    sink.write_all(b"xyz").unwrap();
    assert_eq!(sink.len(), 8);
    assert_eq!(sink.position(), 8);
    sink.rewind().unwrap();
    sink.write_all(b"12345678").unwrap();
    assert_eq!(sink.len(), 8);
}

#[test]
fn canonical_seeks_use_all_origins_without_changing_length() {
    let mut sink = MeasuringStream::new();
    sink.set_len(1024);
    for offset in [0, 1, 127, 512, 1023, 1024] {
        assert_eq!(sink.seek(SeekFrom::Start(offset)).unwrap(), offset);
        // Seek backwards from the current position.
        assert_eq!(sink.seek(SeekFrom::Current(-(offset as i64))).unwrap(), 0);
        // Seek forwards from the current position.
        assert_eq!(sink.seek(SeekFrom::Current(offset as i64)).unwrap(), offset);
        // Seek backwards from the end of the stream.
        assert_eq!(
            sink.seek(SeekFrom::End(offset as i64 - 1024)).unwrap(),
            offset
        );
        assert_eq!(sink.position(), offset);
        assert_eq!(sink.len(), 1024);
    }
}

#[test]
fn resizing_clamps_position_and_can_reset_measurement() {
    let mut sink = MeasuringStream::new();
    sink.set_len(1024);
    assert_eq!(sink.len(), 1024);
    assert_eq!(sink.position(), 0);
    sink.seek(SeekFrom::Start(512)).unwrap();
    sink.set_len(2048);
    assert_eq!(sink.position(), 512);
    sink.set_len(1024);
    assert_eq!(sink.position(), 512);
    sink.set_len(100);
    assert_eq!(sink.position(), 100);
    assert_eq!(sink.len(), 100);
    sink.set_len(100);
    assert_eq!(sink.position(), 100);
    sink.set_len(0);
    assert!(sink.is_empty());
    assert_eq!(sink.position(), 0);
    sink.write_all(b"new").unwrap();
    assert_eq!(sink.len(), 3);
}

#[test]
fn invalid_seeks_preserve_position_and_length() {
    let mut sink = MeasuringStream::new();
    sink.set_len(10);
    sink.seek(SeekFrom::Start(5)).unwrap();
    for from in [
        SeekFrom::Start(11),
        SeekFrom::Start(u64::MAX),
        SeekFrom::Current(-6),
        SeekFrom::Current(6),
        SeekFrom::Current(i64::MIN),
        SeekFrom::Current(i64::MAX),
        SeekFrom::End(-11),
        SeekFrom::End(1),
        SeekFrom::End(i64::MIN),
        SeekFrom::End(i64::MAX),
    ] {
        assert_eq!(
            sink.seek(from).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.position(), 5);
        assert_eq!(sink.len(), 10);
    }
}

#[test]
fn maximum_lengths_and_write_overflow_are_checked_without_allocation() {
    let mut sink = MeasuringStream::new();
    sink.set_len(u64::MAX);
    assert_eq!(sink.len(), u64::MAX);
    assert_eq!(sink.position(), 0);
    sink.set_len(u64::MAX - 1);
    assert_eq!(sink.seek(SeekFrom::End(0)).unwrap(), u64::MAX - 1);
    assert_eq!(
        sink.write(b"xy").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(sink.position(), u64::MAX - 1);
    assert_eq!(sink.len(), u64::MAX - 1);
    assert_eq!(sink.write(b"x").unwrap(), 1);
    assert_eq!(sink.position(), u64::MAX);
    assert_eq!(sink.len(), u64::MAX);
    assert_eq!(
        sink.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(sink.write(b"").unwrap(), 0);
    sink.flush().unwrap();
    for from in [SeekFrom::Current(1), SeekFrom::End(1)] {
        assert_eq!(
            sink.seek(from).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.position(), u64::MAX);
        assert_eq!(sink.len(), u64::MAX);
    }
    sink.seek(SeekFrom::Start(1u64 << 63)).unwrap();
    assert_eq!(sink.seek(SeekFrom::Current(i64::MIN)).unwrap(), 0);
    assert_eq!(sink.seek(SeekFrom::End(i64::MIN)).unwrap(), i64::MAX as u64);
    sink.set_len(0);
    sink.write_all(b"reusable").unwrap();
    assert_eq!(sink.len(), 8);
}

#[test]
fn deterministic_operations_match_a_contiguous_stream_model() {
    let mut sink = MeasuringStream::new();
    let mut master = Cursor::new(Vec::new());
    let mut seed = 0xa7e3_532d_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as usize
    };
    let data = [0x7f; 2048];
    for step in 0..600 {
        match step % 4 {
            0 => {
                let bytes = &data[..next() % (data.len() + 1)];
                assert_eq!(sink.write(bytes).unwrap(), master.write(bytes).unwrap());
            }
            1 => {
                let target = (next() % (master.get_ref().len() + 1)) as u64;
                let from = match next() % 3 {
                    0 => SeekFrom::Start(target),
                    1 => SeekFrom::Current(target as i64 - master.position() as i64),
                    _ => SeekFrom::End(target as i64 - master.get_ref().len() as i64),
                };
                assert_eq!(sink.seek(from).unwrap(), master.seek(from).unwrap());
            }
            2 => {
                let length = next() % 16384;
                sink.set_len(length as u64);
                master.get_mut().resize(length, 0);
                master.set_position(master.position().min(length as u64));
            }
            _ => sink.flush().unwrap(),
        }
        assert_eq!(sink.position(), master.position(), "step {step}");
        assert_eq!(sink.len(), master.get_ref().len() as u64, "step {step}");
    }
}

#[test]
fn header_measurement_matches_verbatim_and_normalized_serialization() {
    let input = b"Subject: example\n\tcontinued\nX-Test: value\r\n\r\nbody";
    let headers = HeaderList::load(&mut Cursor::new(input)).unwrap();
    let mut expected = Vec::new();
    let mut sink = MeasuringStream::new();
    headers.write_raw_to(&mut expected).unwrap();
    headers.write_raw_to(&mut sink).unwrap();
    assert_eq!(sink.len(), expected.len() as u64);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        expected.clear();
        sink.set_len(0);
        headers
            .write_to_with_new_line_format(&mut expected, format)
            .unwrap();
        headers
            .write_to_with_new_line_format(&mut sink, format)
            .unwrap();
        assert_eq!(sink.len(), expected.len() as u64);
        assert_eq!(sink.position(), sink.len());
    }
}

#[test]
fn finalizing_filters_counts_padding_and_terminators_exactly_once() {
    let mut writer = FilteredWriter::new(
        MeasuringStream::new(),
        EncoderFilter::new(Base64Encoder::new()),
    );
    writer.write_all(b"C").unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.get_ref().len(), 0);
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().len(), 6);
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().len(), 6);
}

#[test]
fn transfer_encoding_measurements_match_real_output_for_varied_chunks() {
    let data: Vec<u8> = (0..8195).map(|i| (i * 31 + i / 251) as u8).collect();
    for encoding in [
        ContentEncoding::Unspecified,
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut expected = FilteredWriter::new(Vec::new(), encoder_filter(encoding));
        expected.write_all(&data).unwrap();
        expected.finish().unwrap();
        for size in [1, 3, 76, 4096] {
            let mut writer = FilteredWriter::new(MeasuringStream::new(), encoder_filter(encoding));
            for chunk in data.chunks(size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap();
            assert_eq!(
                writer.get_ref().len(),
                expected.get_ref().len() as u64,
                "{encoding:?}, {size}"
            );
        }
    }
}

#[test]
fn mime_content_counts_raw_and_decoded_lengths_on_repeated_copies() {
    let mut content = MimeContent::with_encoding(Cursor::new(b"Q2F0\r\n"), ContentEncoding::Base64);
    let mut sink = MeasuringStream::new();
    assert_eq!(content.write_to(&mut sink).unwrap(), 6);
    assert_eq!(sink.len(), 6);
    sink.set_len(0);
    assert_eq!(content.decode_to(&mut sink).unwrap(), 3);
    assert_eq!(sink.len(), 3);
    assert_eq!(content.decode_to(&mut sink).unwrap(), 3);
    assert_eq!(sink.len(), 6);
}

#[test]
fn bounded_and_chained_sinks_keep_their_normal_seek_and_write_rules() {
    let mut first = MeasuringStream::new();
    first.set_len(3);
    let mut chain = ChainedStream::from(vec![first, MeasuringStream::new()]);
    chain.write_all(b"abcdef").unwrap();
    assert_eq!(chain.get_ref()[0].len(), 3);
    assert_eq!(chain.get_ref()[1].len(), 3);
    assert_eq!(chain.len().unwrap(), 6);
    chain.seek(SeekFrom::Start(2)).unwrap();
    chain.write_all(b"12345").unwrap();
    assert_eq!(chain.len().unwrap(), 7);
    assert_eq!(chain.position(), 7);

    let mut inner = MeasuringStream::new();
    inner.set_len(10);
    let mut bounded = BoundedStream::new(inner, 2, Some(6)).unwrap();
    bounded.write_all(b"abcd").unwrap();
    assert_eq!(bounded.position(), 4);
    assert_eq!(bounded.get_ref().len(), 10);
    assert_eq!(bounded.get_ref().position(), 6);
    assert_eq!(
        bounded.write(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(bounded.position(), 4);
}

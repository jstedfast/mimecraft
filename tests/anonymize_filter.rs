// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Write};

use mimecraft::{
    encodings::{Base64Decoder, Base64Encoder},
    filters::{AnonymizeFilter, DecoderFilter, EncoderFilter, MimeFilter},
    io::{FilteredReader, FilteredWriter},
};

#[test]
fn all_bytes_preserve_only_the_four_mime_whitespace_bytes() {
    let input: Vec<_> = (0..=255).collect();
    let before = input.clone();
    let mut expected = vec![b'x'; 256];
    for byte in *b" \t\r\n" {
        expected[usize::from(byte)] = byte;
    }
    for size in 1..=input.len() {
        let mut filter = AnonymizeFilter::new();
        let mut output = b"\0\xffprefix".to_vec();
        for chunk in input.chunks(size) {
            let length = output.len();
            filter.filter(chunk, &mut output);
            assert_eq!(output.len() - length, chunk.len());
            filter.filter(b"", &mut output);
        }
        filter.finish(&mut output);
        assert_eq!(&output[..8], b"\0\xffprefix");
        assert_eq!(&output[8..], expected);
        assert_eq!(input, before);
    }
}

#[test]
fn every_partition_preserves_bare_and_split_line_endings() {
    let input = b"A \t\r\n\x0b\x0c\xff\rZ";
    let expected = b"x \t\r\nxxx\rx";
    for cuts in 0..1 << (input.len() - 1) {
        let mut filter = AnonymizeFilter::new();
        let mut output = Vec::new();
        let mut start = 0;
        for end in 1..=input.len() {
            if end == input.len() || cuts & (1 << (end - 1)) != 0 {
                filter.filter(&input[start..end], &mut output);
                filter.filter(b"", &mut output);
                start = end;
            }
        }
        filter.finish(&mut output);
        filter.finish(&mut output);
        assert_eq!(output, expected, "cuts {cuts}");
    }
}

#[test]
fn masking_is_per_byte_even_for_unicode_whitespace_and_invalid_utf8() {
    for (input, expected) in [
        (
            "A\u{a0}\u{2003}\u{1f408}\t\u{e9}\r\n".as_bytes(),
            b"xxxxxxxxxx\txx\r\n".as_slice(),
        ),
        (b"\xff\xc0\x80\xfe\0".as_slice(), b"xxxxx".as_slice()),
        (b"\r\n \t\n\r".as_slice(), b"\r\n \t\n\r".as_slice()),
        (b"".as_slice(), b"".as_slice()),
    ] {
        let mut filter = AnonymizeFilter::new();
        let mut output = Vec::new();
        for byte in input {
            filter.filter(std::slice::from_ref(byte), &mut output);
        }
        filter.finish(&mut output);
        assert_eq!(output, expected);
        assert_eq!(output.len(), input.len());
    }
}

#[test]
fn empty_input_finish_reset_and_output_buffer_changes_need_no_state() {
    let mut filter = AnonymizeFilter::new();
    let mut output = b"untouched".to_vec();
    filter.filter(b"", &mut output);
    filter.finish(&mut output);
    filter.reset();
    filter.finish(&mut output);
    assert_eq!(output, b"untouched");

    filter.filter(b"First\r", &mut output);
    assert_eq!(output, b"untouchedxxxxx\r");
    let mut copy = filter;
    output.clear();
    filter.filter(b"\nSecond", &mut output);
    filter.finish(&mut output);
    assert_eq!(output, b"\nxxxxxx");

    let mut other = b"other:".to_vec();
    copy.filter(b"\nThird", &mut other);
    assert_eq!(other, b"other:\nxxxxx");
    filter.reset();
    filter.filter(b" reused ", &mut other);
    assert_eq!(other, b"other:\nxxxxx xxxxxx ");
}

#[test]
fn generic_boxed_and_borrowed_filters_use_the_same_masking() {
    fn generic<F: MimeFilter + Default + Clone + Copy + std::fmt::Debug>() {
        let mut filter = F::default();
        let mut output = Vec::new();
        filter.filter(b"abc\r\n", &mut output);
        filter.finish(&mut output);
        filter.reset();
        assert_eq!(output, b"xxx\r\n");
    }
    generic::<AnonymizeFilter>();

    let mut filter = AnonymizeFilter::new();
    let mut output = Vec::new();
    {
        let borrowed: &mut dyn MimeFilter = &mut filter;
        let mut boxed: Box<dyn MimeFilter + '_> = Box::new(borrowed);
        boxed.filter(b"A\tB", &mut output);
        boxed.finish(&mut output);
        boxed.reset();
    }
    assert_eq!(output, b"x\tx");
    let mut boxed: Box<dyn MimeFilter> = Box::new(filter);
    boxed.filter(b"\nC", &mut output);
    boxed.finish(&mut output);
    assert_eq!(output, b"x\tx\nx");
}

struct ChunkedReader<R> {
    inner: R,
    limit: usize,
}

impl<R: Read> Read for ChunkedReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.limit);
        self.inner.read(&mut output[..length])
    }
}

#[test]
fn nested_reader_decodes_before_masking_and_retains_decoded_layout() {
    let encoded = b"SGVsbG8NCldvcmxkIQ==";
    for limit in 1..=encoded.len() {
        let source = ChunkedReader {
            inner: Cursor::new(encoded),
            limit,
        };
        let decoded = FilteredReader::new(source, DecoderFilter::new(Base64Decoder::new()));
        let mut reader = FilteredReader::new(decoded, AnonymizeFilter::new());
        let mut output = Vec::new();
        let mut byte = [0; 1];
        while reader.read(&mut byte).unwrap() != 0 {
            output.extend_from_slice(&byte);
        }
        assert_eq!(output, b"xxxxx\r\nxxxxxx");
        assert_eq!(reader.read(&mut byte).unwrap(), 0);
    }
}

#[test]
fn nested_writer_masks_before_encoding_and_finishes_in_pipeline_order() {
    let input = b"Hello\r\nWorld!";
    for size in 1..=input.len() {
        let encoder = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
        let mut writer = FilteredWriter::new(encoder, AnonymizeFilter::new());
        for chunk in input.chunks(size) {
            writer.write_all(chunk).unwrap();
            writer.flush().unwrap();
        }
        // Finish the masking layer before the encoder so any pending encoder
        // tail is emitted after all masked bytes, not before them.
        writer.finish().unwrap();
        writer.get_mut().finish().unwrap();
        let (encoder, _, pending) = writer.into_parts();
        assert!(pending.is_empty());
        let (encoded, _, pending) = encoder.into_parts();
        assert!(pending.is_empty());
        assert_eq!(encoded, b"eHh4eHgNCnh4eHh4eA==\r\n");
        let mut decoder = Base64Decoder::new();
        let mut decoded = Vec::new();
        decoder.decode(&encoded, &mut decoded);
        assert_eq!(decoded, b"xxxxx\r\nxxxxxx");
    }
}

struct RetryWriter {
    fail: bool,
    bytes: Vec<u8>,
}

impl Write for RetryWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if self.fail {
            self.fail = false;
            return Err(io::Error::other("one simulated sink failure"));
        }
        let length = input.len().min(2);
        self.bytes.extend_from_slice(&input[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writer_failure_retains_only_masked_output_and_retry_does_not_duplicate_it() {
    let sink = RetryWriter {
        fail: true,
        bytes: Vec::new(),
    };
    let mut writer = FilteredWriter::new(sink, AnonymizeFilter::new());
    writer.write_all(b"secret \ttext\r\n").unwrap();
    assert_eq!(writer.finish().unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(writer.buffer(), b"xxxxxx \txxxx\r\n");
    assert!(writer.get_ref().bytes.is_empty());
    writer.finish().unwrap();
    writer.finish().unwrap();
    assert!(writer.buffer().is_empty());
    assert_eq!(writer.get_ref().bytes, b"xxxxxx \txxxx\r\n");
}

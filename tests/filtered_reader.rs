// Includes read-side cases adapted from MimeKit's IO/FilteredStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    io::{self, BufRead, Cursor, ErrorKind, Read},
};

use mimecraft::{
    ContentEncoding, EncodingConstraint,
    encodings::{Base64Decoder, Base64Encoder, Rfc2231Decoder, UuEncoder},
    filters::{
        BestEncodingFilter, DecoderFilter, Dos2UnixFilter, EncoderFilter, MboxFromFilter,
        MimeFilter, PassThroughFilter, TrailingWhitespaceFilter, Unix2DosFilter, decoder_filter,
    },
    io::FilteredReader,
};

#[derive(Debug)]
struct ChunkedReader<'a> {
    remaining: &'a [u8],
    chunk_size: usize,
    calls: usize,
}

impl<'a> ChunkedReader<'a> {
    fn new(input: &'a [u8], chunk_size: usize) -> Self {
        assert!(chunk_size != 0);
        Self {
            remaining: input,
            chunk_size,
            calls: 0,
        }
    }
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        let count = self.remaining.len().min(self.chunk_size).min(output.len());
        output[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        Ok(count)
    }
}

enum Step {
    Data(&'static [u8]),
    Error(ErrorKind),
    Eof,
}

struct ScriptedReader {
    steps: VecDeque<Step>,
    calls: usize,
}

impl ScriptedReader {
    fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            calls: 0,
        }
    }
}

impl Read for ScriptedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        self.calls += 1;
        match self.steps.pop_front().unwrap_or(Step::Eof) {
            Step::Data(data) => {
                let count = data.len().min(output.len());
                output[..count].copy_from_slice(&data[..count]);
                if count < data.len() {
                    self.steps.push_front(Step::Data(&data[count..]));
                }
                Ok(count)
            }
            Step::Error(kind) => Err(io::Error::new(kind, "scripted read failure")),
            Step::Eof => Ok(0),
        }
    }
}

#[derive(Debug, Default)]
struct CountingFilter {
    calls: usize,
    finishes: usize,
    resets: usize,
    tail: Vec<u8>,
}

impl MimeFilter for CountingFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.calls += 1;
        output.extend_from_slice(input);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        self.finishes += 1;
        output.extend_from_slice(&self.tail);
    }

    fn reset(&mut self) {
        self.resets += 1;
    }
}

fn collect(reader: &mut impl Read, chunk_size: usize) -> Vec<u8> {
    let mut buffer = vec![0xaa; chunk_size + 4];
    let mut output = Vec::new();
    loop {
        buffer.fill(0xaa);
        let count = reader.read(&mut buffer[2..2 + chunk_size]).unwrap();
        assert!(count <= chunk_size);
        assert_eq!(&buffer[..2], &[0xaa; 2]);
        assert!(buffer[2 + count..].iter().all(|&byte| byte == 0xaa));
        if count == 0 {
            return output;
        }
        output.extend_from_slice(&buffer[2..2 + count]);
    }
}

#[test]
fn mimekit_read_flushes_trailing_cr_at_eof() {
    for source_size in [1, 2, 4096] {
        for output_size in [1, 2, 7, 4096] {
            let source = ChunkedReader::new(b"a\r\n\r\nb\r", source_size);
            let mut reader =
                FilteredReader::new(source, Dos2UnixFilter::with_ensure_new_line(true));
            assert_eq!(collect(&mut reader, output_size), b"a\n\nb\n");
            let calls = reader.get_ref().calls;
            assert_eq!(reader.read(&mut [0; 16]).unwrap(), 0);
            assert!(reader.fill_buf().unwrap().is_empty());
            assert_eq!(reader.get_ref().calls, calls);
        }
    }
}

#[test]
fn base64_read_decodes_binary_data_across_source_and_output_boundaries() {
    let original: Vec<u8> = (0..=255).cycle().take(16384).collect();
    let mut encoded = Vec::new();
    let mut encoder = Base64Encoder::new();
    encoder.encode(&original, &mut encoded);
    encoder.finish(&mut encoded);
    for source_size in [1, 2, 3, 4096] {
        for output_size in [1, 7, 4096] {
            let source = ChunkedReader::new(&encoded, source_size);
            let filter = decoder_filter(ContentEncoding::Base64);
            let mut reader = FilteredReader::new(source, filter);
            assert_eq!(collect(&mut reader, output_size), original);
        }
    }
}

#[test]
fn input_without_output_is_not_a_false_eof() {
    let mut input = vec![b' '; 12288];
    input.extend_from_slice(b"Q2F0");
    let source = ChunkedReader::new(&input, 4096);
    let mut reader = FilteredReader::new(source, DecoderFilter::new(Base64Decoder::new()));
    let mut output = [0; 3];
    reader.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"Cat");
    assert_eq!(reader.get_ref().calls, 4);
    assert_eq!(reader.read(&mut output).unwrap(), 0);
    assert_eq!(reader.get_ref().calls, 5);

    let blanks = b" \t".repeat(8192);
    let source = ChunkedReader::new(&blanks, 4096);
    let mut reader = FilteredReader::new(source, TrailingWhitespaceFilter::new());
    assert!(collect(&mut reader, 1).is_empty());
    assert_eq!(reader.get_ref().calls, 5);

    let source = ChunkedReader::new(b"From", 1);
    let mut reader = FilteredReader::new(source, MboxFromFilter::new());
    assert_eq!(collect(&mut reader, 1), b"From");
    assert_eq!(reader.get_ref().calls, 5);
}

#[test]
fn expanding_filters_retain_output_larger_than_the_source_buffer() {
    let input = vec![b'\n'; 10000];
    let mut reader = FilteredReader::new(input.as_slice(), Unix2DosFilter::new());
    assert_eq!(collect(&mut reader, 3), b"\r\n".repeat(input.len()));
}

#[test]
fn eof_finishes_once_and_delivers_a_large_tail_before_returning_zero() {
    let filter = CountingFilter {
        tail: vec![b'x'; 8193],
        ..CountingFilter::default()
    };
    let mut reader = FilteredReader::new(ChunkedReader::new(b"", 1), filter);
    assert_eq!(reader.read(&mut []).unwrap(), 0);
    assert_eq!(reader.get_ref().calls, 0);
    assert_eq!(reader.filter().finishes, 0);
    assert_eq!(reader.fill_buf().unwrap().len(), 8193);
    assert_eq!(reader.filter().finishes, 1);
    assert_eq!(reader.fill_buf().unwrap().len(), 8193);
    assert_eq!(collect(&mut reader, 1), vec![b'x'; 8193]);
    for _ in 0..3 {
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.read(&mut [0; 16]).unwrap(), 0);
        assert!(reader.fill_buf().unwrap().is_empty());
    }
    assert_eq!(reader.filter().calls, 0);
    assert_eq!(reader.filter().finishes, 1);
    assert_eq!(reader.filter().resets, 0);
    assert_eq!(reader.get_ref().calls, 1);
}

#[test]
fn uu_terminator_is_not_repeated_on_reads_after_eof() {
    let mut reader = FilteredReader::new(&b""[..], EncoderFilter::new(UuEncoder::new()));
    assert_eq!(collect(&mut reader, 1), b"`\r\n");
    assert!(reader.filter().has_output());
    assert!(collect(&mut reader, 1).is_empty());
    assert!(reader.fill_buf().unwrap().is_empty());
}

#[test]
fn fill_buf_consume_and_read_share_one_output_cursor() {
    let source = ChunkedReader::new(b"abcd", 4);
    let mut reader = FilteredReader::new(source, CountingFilter::default());
    assert!(reader.buffer().is_empty());
    assert_eq!(reader.read(&mut []).unwrap(), 0);
    assert_eq!(reader.get_ref().calls, 0);
    assert_eq!(reader.fill_buf().unwrap(), b"abcd");
    assert_eq!(reader.fill_buf().unwrap(), b"abcd");
    assert_eq!(reader.get_ref().calls, 1);
    assert_eq!(reader.filter().calls, 1);
    reader.consume(1);
    assert_eq!(reader.buffer(), b"bcd");
    assert_eq!(reader.read(&mut []).unwrap(), 0);
    assert_eq!(reader.buffer(), b"bcd");
    let mut buffer = [0xaa; 2];
    assert_eq!(reader.read(&mut buffer).unwrap(), 2);
    assert_eq!(&buffer, b"bc");
    assert_eq!(reader.fill_buf().unwrap(), b"d");
    reader.consume(0);
    assert_eq!(reader.buffer(), b"d");
    reader.consume(1);
    assert!(reader.buffer().is_empty());
    assert!(reader.fill_buf().unwrap().is_empty());
    assert_eq!(reader.get_ref().calls, 2);
    assert_eq!(reader.filter().finishes, 1);
}

#[test]
fn source_errors_preserve_partial_filter_state_and_do_not_touch_destination() {
    for kind in [
        ErrorKind::WouldBlock,
        ErrorKind::Interrupted,
        ErrorKind::Other,
    ] {
        let source = ScriptedReader::new([
            Step::Data(b"Fr"),
            Step::Error(kind),
            Step::Data(b"om sender\n"),
            Step::Eof,
        ]);
        let mut reader = FilteredReader::new(source, MboxFromFilter::new());
        let mut output = [0xaa; 2];
        let error = reader.read(&mut output).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "scripted read failure");
        assert_eq!(output, [0xaa; 2]);
        assert!(reader.buffer().is_empty());
        assert_eq!(collect(&mut reader, 2), b">From sender\n");
    }
}

#[test]
fn buffered_output_is_delivered_before_reading_a_source_error() {
    let source = ScriptedReader::new([
        Step::Data(b"abc"),
        Step::Error(ErrorKind::WouldBlock),
        Step::Data(b"d"),
        Step::Eof,
    ]);
    let mut reader = FilteredReader::new(source, CountingFilter::default());
    assert_eq!(reader.read(&mut [0; 1]).unwrap(), 1);
    assert_eq!(reader.get_ref().calls, 1);
    assert_eq!(reader.fill_buf().unwrap(), b"bc");
    reader.consume(2);
    assert_eq!(reader.fill_buf().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(reader.filter().finishes, 0);
    assert_eq!(reader.filter().resets, 0);
    assert_eq!(collect(&mut reader, 2), b"d");
    assert_eq!(reader.filter().calls, 2);
    assert_eq!(reader.filter().finishes, 1);
}

#[test]
fn read_to_end_retries_interrupted_and_keeps_progress_on_other_errors() {
    let source = ScriptedReader::new([
        Step::Error(ErrorKind::Interrupted),
        Step::Data(b"ab"),
        Step::Error(ErrorKind::Interrupted),
        Step::Data(b"cd"),
        Step::Error(ErrorKind::WouldBlock),
        Step::Data(b"ef"),
        Step::Eof,
    ]);
    let mut reader = FilteredReader::new(source, PassThroughFilter::new());
    let mut output = b"prefix".to_vec();
    assert_eq!(
        reader.read_to_end(&mut output).unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    assert_eq!(output, b"prefixabcd");
    assert_eq!(reader.read_to_end(&mut output).unwrap(), 2);
    assert_eq!(output, b"prefixabcdef");
}

#[test]
fn source_zero_is_latched_even_if_the_source_later_has_more_bytes() {
    let source = ScriptedReader::new([Step::Data(b"a"), Step::Eof, Step::Data(b"late")]);
    let mut reader = FilteredReader::new(source, CountingFilter::default());
    assert_eq!(collect(&mut reader, 1), b"a");
    assert_eq!(reader.get_ref().calls, 2);
    assert!(collect(&mut reader, 1).is_empty());
    assert_eq!(reader.filter().finishes, 1);
    let mut late = Vec::new();
    reader.get_mut().read_to_end(&mut late).unwrap();
    assert_eq!(late, b"late");
    assert!(collect(&mut reader, 1).is_empty());
}

#[test]
fn nested_readers_deliver_upstream_final_bytes_before_finishing_downstream() {
    for (input, expected) in [
        (b"x\r".as_slice(), b"eAo=\n".as_slice()),
        (b"\r".as_slice(), b"Cg==\n".as_slice()),
        (b"".as_slice(), b"Cg==\n".as_slice()),
    ] {
        for size in [1, 2, 4096] {
            let source = ChunkedReader::new(input, size);
            let normalized =
                FilteredReader::new(source, Dos2UnixFilter::with_ensure_new_line(true));
            let encoded = FilteredReader::new(normalized, EncoderFilter::new(Base64Encoder::new()));
            let mut reader = FilteredReader::new(encoded, Dos2UnixFilter::new());
            assert_eq!(collect(&mut reader, 1), expected);
            assert!(reader.fill_buf().unwrap().is_empty());
            assert!(reader.get_ref().filter().has_output());
        }
    }
}

#[test]
fn into_parts_recovers_unread_output_and_pending_filter_state() {
    for input in [b"ab\rcd".as_slice(), b"ab\r"] {
        let source = ChunkedReader::new(input, 3);
        let mut reader = FilteredReader::new(source, Dos2UnixFilter::new());
        let mut first = [0];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"a");
        assert_eq!(reader.buffer(), b"b");
        let (source, filter, mut remaining) = reader.into_parts();
        assert_eq!(source.calls, 1);
        assert_eq!(remaining, b"b");
        let mut resumed = FilteredReader::new(source, filter);
        resumed.read_to_end(&mut remaining).unwrap();
        assert_eq!([first.to_vec(), remaining].concat(), input);
    }

    let mut reader = FilteredReader::new(
        &b""[..],
        CountingFilter {
            tail: b"end".to_vec(),
            ..CountingFilter::default()
        },
    );
    reader.read_exact(&mut [0; 1]).unwrap();
    let (_, filter, remaining) = reader.into_parts();
    assert_eq!(filter.finishes, 1);
    assert_eq!(remaining, b"nd");
}

#[test]
fn new_preserves_existing_filter_state_and_decoder_eof_does_not_reset_it() {
    let mut decoder = Base64Decoder::new();
    decoder.decode(b"Q", &mut Vec::new());
    let mut reader = FilteredReader::new(&b"2F0"[..], DecoderFilter::new(decoder));
    assert_eq!(collect(&mut reader, 1), b"Cat");

    let mut reader = FilteredReader::new(&b"name%2"[..], DecoderFilter::new(Rfc2231Decoder::new()));
    assert_eq!(collect(&mut reader, 1), b"name");
    let (_, mut filter, remaining) = reader.into_parts();
    assert!(remaining.is_empty());
    let mut output = Vec::new();
    filter.filter(b"0", &mut output);
    assert_eq!(output, b" ");
}

#[test]
fn borrowed_sources_and_filters_are_not_finished_or_reset_on_early_drop() {
    let mut source = ChunkedReader::new(b"abc", 3);
    let mut filter = CountingFilter {
        tail: b"tail".to_vec(),
        ..CountingFilter::default()
    };
    {
        let borrowed: &mut dyn MimeFilter = &mut filter;
        let mut reader = FilteredReader::new(&mut source, borrowed);
        reader.read_exact(&mut [0; 1]).unwrap();
        assert_eq!(reader.buffer(), b"bc");
    }
    assert_eq!(source.calls, 1);
    assert_eq!(filter.calls, 1);
    assert_eq!(filter.finishes, 0);
    assert_eq!(filter.resets, 0);
    let mut borrowed: &mut dyn MimeFilter = &mut filter;
    MimeFilter::reset(&mut borrowed);
    assert_eq!(filter.resets, 1);
}

#[test]
fn filter_inspection_includes_read_ahead_not_only_delivered_bytes() {
    let source = Cursor::new(b"a\0");
    let mut reader = FilteredReader::new(source, BestEncodingFilter::new());
    assert_eq!(
        reader.filter().best_encoding(EncodingConstraint::None),
        ContentEncoding::SevenBit
    );
    reader.read_exact(&mut [0; 1]).unwrap();
    assert_eq!(reader.get_ref().position(), 2);
    assert_eq!(reader.buffer(), b"\0");
    assert_eq!(
        reader.filter().best_encoding(EncodingConstraint::None),
        ContentEncoding::Binary
    );
}

#[test]
fn standard_copy_and_buffered_line_reads_work_with_filtered_content() {
    let source = ChunkedReader::new(b"first\r\nsecond\r", 1);
    let mut reader = FilteredReader::new(source, Dos2UnixFilter::with_ensure_new_line(true));
    let mut output = Vec::new();
    assert_eq!(io::copy(&mut reader, &mut output).unwrap(), 13);
    assert_eq!(output, b"first\nsecond\n");

    let mut reader = FilteredReader::new(&b"a\xff\r\nlast"[..], PassThroughFilter::new());
    let mut line = Vec::new();
    assert_eq!(reader.read_until(b'\n', &mut line).unwrap(), 4);
    assert_eq!(line, b"a\xff\r\n");
    line.clear();
    assert_eq!(reader.read_until(b'\n', &mut line).unwrap(), 4);
    assert_eq!(line, b"last");
    assert_eq!(reader.read_until(b'\n', &mut line).unwrap(), 0);
}

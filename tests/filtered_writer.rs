// Includes write-side behavior adapted from MimeKit's IO/FilteredStreamTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    io::{self, Cursor, ErrorKind, IoSlice, Write},
};

use mimecraft::{
    ContentEncoding, EncodingConstraint,
    encodings::{
        Base64Decoder, Base64Encoder, QuotedPrintableEncoder, Rfc2231Decoder, UuEncoder, YEncoder,
    },
    filters::{
        BestEncodingFilter, DecoderFilter, Dos2UnixFilter, EncoderFilter, MboxFromFilter,
        MimeFilter, PassThroughFilter, TrailingWhitespaceFilter, Unix2DosFilter, decoder_filter,
    },
    io::FilteredWriter,
};

enum Step {
    Limit(usize),
    Error(ErrorKind),
}

struct ScriptedWriter {
    bytes: Vec<u8>,
    steps: VecDeque<Step>,
    flush_errors: VecDeque<ErrorKind>,
    max_write: usize,
    writes: usize,
    flushes: usize,
}

impl ScriptedWriter {
    fn new(max_write: usize, steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            bytes: Vec::new(),
            steps: steps.into_iter().collect(),
            flush_errors: VecDeque::new(),
            max_write,
            writes: 0,
            flushes: 0,
        }
    }
}

impl Write for ScriptedWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        let limit = match self.steps.pop_front() {
            Some(Step::Limit(limit)) => limit,
            Some(Step::Error(kind)) => return Err(io::Error::new(kind, "scripted write failure")),
            None => self.max_write,
        };
        let count = input.len().min(limit);
        self.bytes.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        match self.flush_errors.pop_front() {
            Some(kind) => Err(io::Error::new(kind, "scripted flush failure")),
            None => Ok(()),
        }
    }
}

#[derive(Default)]
struct CountingFilter {
    calls: usize,
    finishes: usize,
    resets: usize,
    seen: Vec<u8>,
    max_chunk: usize,
    tail: Vec<u8>,
}

impl MimeFilter for CountingFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.calls += 1;
        self.seen.extend_from_slice(input);
        self.max_chunk = self.max_chunk.max(input.len());
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

fn check_chunks<F: MimeFilter + Clone>(filter: F, input: &[u8]) {
    let mut direct = filter.clone();
    let mut expected = Vec::new();
    direct.filter(input, &mut expected);
    direct.finish(&mut expected);
    for input_size in [1, 2, 3, 61, 4096, 8193] {
        for sink_size in [1, 7, usize::MAX] {
            let sink = ScriptedWriter::new(sink_size, []);
            let mut writer = FilteredWriter::new(sink, filter.clone());
            for chunk in input.chunks(input_size) {
                writer.write_all(chunk).unwrap();
            }
            writer.finish().unwrap();
            assert_eq!(writer.get_ref().bytes, expected);
            assert!(writer.buffer().is_empty());
            writer.finish().unwrap();
            assert_eq!(writer.get_ref().bytes, expected);
        }
    }
}

#[test]
fn bounded_writes_report_input_accepted_and_buffer_before_sink_io() {
    let input = vec![b'x'; 10000];
    let sink = ScriptedWriter::new(3, []);
    let mut writer = FilteredWriter::new(sink, CountingFilter::default());
    assert_eq!(writer.write(&input).unwrap(), 4096);
    assert_eq!(writer.get_ref().writes, 0);
    assert_eq!(writer.buffer(), &input[..4096]);
    assert_eq!(writer.filter().seen.len(), 4096);
    assert_eq!(writer.write(&input[4096..]).unwrap(), 4096);
    assert_eq!(writer.get_ref().bytes.len(), 4096);
    assert_eq!(writer.buffer().len(), 4096);
    writer.write_all(&input[8192..]).unwrap();
    assert_eq!(writer.filter().seen, input);
    assert_eq!(writer.filter().max_chunk, 4096);
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, input);
}

#[test]
fn codecs_and_expanding_filters_match_direct_filtering_under_short_writes() {
    let input: Vec<u8> = (0..=255).cycle().take(8448).collect();
    check_chunks(PassThroughFilter::new(), &input);
    check_chunks(EncoderFilter::new(Base64Encoder::new()), &input);
    check_chunks(EncoderFilter::new(QuotedPrintableEncoder::new()), &input);
    check_chunks(EncoderFilter::new(UuEncoder::new()), &input);
    check_chunks(EncoderFilter::new(YEncoder::new()), &input);
    check_chunks(Unix2DosFilter::new(), &vec![b'\n'; 8448]);
    check_chunks(Dos2UnixFilter::with_ensure_new_line(true), b"a\r\n\r\nb\r");
    check_chunks(MboxFromFilter::new(), b"From one\nFr");
    check_chunks(TrailingWhitespaceFilter::new(), b" \ttext \t\r\nlast \t");

    let mut encoded = Vec::new();
    let mut encoder = Base64Encoder::new();
    encoder.encode(&input, &mut encoded);
    encoder.finish(&mut encoded);
    check_chunks(DecoderFilter::new(Base64Decoder::new()), &encoded);
    check_chunks(DecoderFilter::new(Rfc2231Decoder::new()), b"file%20name%2");
}

#[test]
fn flush_does_not_finalize_or_reset_an_incomplete_base64_group() {
    let mut writer = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
    writer.write_all(b"C").unwrap();
    assert!(writer.buffer().is_empty());
    writer.flush().unwrap();
    writer.flush().unwrap();
    assert!(writer.get_ref().is_empty());
    writer.write_all(b"at").unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.get_ref(), b"Q2F0");
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"Q2F0\r\n");
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"Q2F0\r\n");
}

#[test]
fn flush_preserves_pending_cr_from_prefix_and_whitespace() {
    let mut writer = FilteredWriter::new(Vec::new(), Dos2UnixFilter::new());
    writer.write_all(b"a\r").unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.get_ref(), b"a");
    writer.write_all(b"\nb").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"a\nb");

    let mut writer = FilteredWriter::new(Vec::new(), MboxFromFilter::new());
    writer.write_all(b"Fr").unwrap();
    writer.flush().unwrap();
    assert!(writer.get_ref().is_empty());
    writer.write_all(b"om ").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b">From ");

    let mut writer = FilteredWriter::new(Vec::new(), TrailingWhitespaceFilter::new());
    writer.write_all(b"a ").unwrap();
    writer.flush().unwrap();
    writer.write_all(b"b ").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"a b");
}

#[test]
fn errors_draining_old_output_do_not_accept_or_refilter_new_input() {
    for kind in [
        ErrorKind::WouldBlock,
        ErrorKind::BrokenPipe,
        ErrorKind::Other,
    ] {
        let sink = ScriptedWriter::new(2, [Step::Limit(2), Step::Error(kind)]);
        let mut writer = FilteredWriter::new(sink, CountingFilter::default());
        assert_eq!(writer.write(b"first").unwrap(), 5);
        let error = writer.write(b"second").unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "scripted write failure");
        assert_eq!(writer.get_ref().bytes, b"fi");
        assert_eq!(writer.buffer(), b"rst");
        assert_eq!(writer.filter().seen, b"first");
        assert_eq!(writer.filter().calls, 1);
        assert_eq!(writer.filter().finishes, 0);
        assert_eq!(writer.write(b"second").unwrap(), 6);
        writer.finish().unwrap();
        assert_eq!(writer.get_ref().bytes, b"firstsecond");
        assert_eq!(writer.filter().seen, b"firstsecond");
        assert_eq!(writer.filter().calls, 2);
        assert_eq!(writer.filter().finishes, 1);
    }
}

#[test]
fn zero_writes_fail_explicitly_and_preserve_the_unwritten_suffix() {
    let sink = ScriptedWriter::new(2, [Step::Limit(1), Step::Limit(0)]);
    let mut writer = FilteredWriter::new(sink, PassThroughFilter::new());
    writer.write_all(b"abcd").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), ErrorKind::WriteZero);
    assert_eq!(writer.get_ref().bytes, b"a");
    assert_eq!(writer.buffer(), b"bcd");
    assert_eq!(writer.get_ref().flushes, 0);
    writer.flush().unwrap();
    assert_eq!(writer.get_ref().bytes, b"abcd");
    assert!(writer.buffer().is_empty());
    writer.write_all(b"e").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, b"abcde");
}

#[test]
fn failed_writes_can_retry_input_without_reencoding_partial_base64_output() {
    for delivered in 0..4 {
        let mut steps = Vec::new();
        if delivered != 0 {
            steps.push(Step::Limit(delivered));
        }
        steps.push(Step::Error(ErrorKind::WouldBlock));
        let sink = ScriptedWriter::new(1, steps);
        let mut writer = FilteredWriter::new(sink, EncoderFilter::new(Base64Encoder::new()));
        assert_eq!(writer.write(b"Cat").unwrap(), 3);
        assert_eq!(writer.buffer(), b"Q2F0");
        assert_eq!(
            writer.write(b"Dog").unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        assert_eq!(writer.get_ref().bytes, &b"Q2F0"[..delivered]);
        assert_eq!(writer.buffer(), &b"Q2F0"[delivered..]);
        assert_eq!(writer.write(b"Dog").unwrap(), 3);
        writer.finish().unwrap();
        assert_eq!(writer.get_ref().bytes, b"Q2F0RG9n\r\n");
    }
}

#[test]
fn large_final_output_survives_a_failure_across_the_input_chunk_size() {
    let sink = ScriptedWriter::new(7, [Step::Limit(4095), Step::Error(ErrorKind::WouldBlock)]);
    let filter = CountingFilter {
        tail: vec![b'x'; 8193],
        ..CountingFilter::default()
    };
    let mut writer = FilteredWriter::new(sink, filter);
    writer.write_all(b"abc").unwrap();
    assert_eq!(writer.finish().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.get_ref().bytes.len(), 4095);
    assert_eq!(writer.buffer().len(), 8196 - 4095);
    writer.finish().unwrap();
    assert_eq!(
        writer.get_ref().bytes,
        [b"abc".to_vec(), vec![b'x'; 8193]].concat()
    );
    assert_eq!(writer.filter().finishes, 1);
}

#[test]
fn interrupted_sink_writes_are_retried_without_duplicate_bytes() {
    let sink = ScriptedWriter::new(
        1,
        [
            Step::Error(ErrorKind::Interrupted),
            Step::Limit(2),
            Step::Error(ErrorKind::Interrupted),
            Step::Limit(1),
        ],
    );
    let mut writer = FilteredWriter::new(sink, PassThroughFilter::new());
    writer.write_all(b"abcde").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, b"abcde");
    assert_eq!(writer.get_ref().writes, 6);
}

#[test]
fn finish_retries_do_not_repeat_final_output_or_allow_new_input() {
    let sink = ScriptedWriter::new(2, [Step::Limit(2), Step::Error(ErrorKind::WouldBlock)]);
    let filter = CountingFilter {
        tail: b"!end".to_vec(),
        ..CountingFilter::default()
    };
    let mut writer = FilteredWriter::new(sink, filter);
    writer.write_all(b"abc").unwrap();
    assert_eq!(writer.finish().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.get_ref().bytes, b"ab");
    assert_eq!(writer.buffer(), b"c!end");
    assert_eq!(writer.filter().finishes, 1);
    assert_eq!(
        writer.write(b"later").unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
    assert_eq!(writer.filter().seen, b"abc");
    assert_eq!(writer.write(b"").unwrap(), 0);
    writer.finish().unwrap();
    writer.finish().unwrap();
    writer.flush().unwrap();
    assert_eq!(writer.get_ref().bytes, b"abc!end");
    assert_eq!(writer.filter().finishes, 1);
    assert_eq!(writer.filter().resets, 0);
}

#[test]
fn finishing_after_a_partially_failed_flush_appends_the_tail_in_order() {
    let sink = ScriptedWriter::new(1, [Step::Limit(2), Step::Error(ErrorKind::Other)]);
    let filter = CountingFilter {
        tail: b"!".to_vec(),
        ..CountingFilter::default()
    };
    let mut writer = FilteredWriter::new(sink, filter);
    writer.write_all(b"abcd").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), ErrorKind::Other);
    assert_eq!(writer.buffer(), b"cd");
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, b"abcd!");
    assert_eq!(writer.filter().finishes, 1);
}

#[test]
fn failed_sink_flush_is_retryable_without_reencoding_or_repeating_tail() {
    for kind in [
        ErrorKind::Interrupted,
        ErrorKind::WouldBlock,
        ErrorKind::Other,
    ] {
        let mut sink = ScriptedWriter::new(1, []);
        sink.flush_errors.push_back(kind);
        let filter = CountingFilter {
            tail: b"!".to_vec(),
            ..CountingFilter::default()
        };
        let mut writer = FilteredWriter::new(sink, filter);
        writer.write_all(b"abc").unwrap();
        let error = writer.finish().unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "scripted flush failure");
        assert!(writer.buffer().is_empty());
        assert_eq!(writer.get_ref().bytes, b"abc!");
        writer.finish().unwrap();
        assert_eq!(writer.get_ref().bytes, b"abc!");
        assert_eq!(writer.filter().finishes, 1);
        assert_eq!(writer.get_ref().flushes, 2);
    }
}

#[test]
fn failed_nonfinalizing_flush_does_not_close_the_filter() {
    let mut sink = ScriptedWriter::new(1, []);
    sink.flush_errors.push_back(ErrorKind::WouldBlock);
    let mut writer = FilteredWriter::new(sink, CountingFilter::default());
    writer.write_all(b"a").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(writer.filter().finishes, 0);
    writer.write_all(b"b").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, b"ab");
}

#[test]
fn empty_writes_do_not_perform_io_even_with_output_pending_or_after_finish() {
    let sink = ScriptedWriter::new(1, []);
    let mut writer = FilteredWriter::new(sink, CountingFilter::default());
    assert_eq!(writer.write(b"").unwrap(), 0);
    assert_eq!(writer.filter().calls, 0);
    writer.write_all(b"x").unwrap();
    assert_eq!(writer.write(b"").unwrap(), 0);
    assert_eq!(writer.get_ref().writes, 0);
    assert_eq!(writer.buffer(), b"x");
    writer.finish().unwrap();
    let writes = writer.get_ref().writes;
    let flushes = writer.get_ref().flushes;
    assert_eq!(writer.write(b"").unwrap(), 0);
    assert_eq!(writer.get_ref().writes, writes);
    assert_eq!(writer.get_ref().flushes, flushes);
}

#[test]
fn empty_uu_stream_emits_one_terminator_even_after_a_zero_write() {
    let sink = ScriptedWriter::new(1, [Step::Limit(1), Step::Limit(0)]);
    let mut writer = FilteredWriter::new(sink, EncoderFilter::new(UuEncoder::new()));
    writer.flush().unwrap();
    assert!(writer.get_ref().bytes.is_empty());
    assert_eq!(writer.finish().unwrap_err().kind(), ErrorKind::WriteZero);
    assert_eq!(writer.get_ref().bytes, b"`");
    assert_eq!(writer.buffer(), b"\r\n");
    writer.finish().unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref().bytes, b"`\r\n");
    assert!(writer.filter().has_output());
}

#[test]
fn into_parts_recovers_only_unwritten_bytes_and_the_pending_filter_state() {
    let sink = ScriptedWriter::new(1, [Step::Limit(1), Step::Error(ErrorKind::Other)]);
    let mut writer = FilteredWriter::new(sink, Dos2UnixFilter::new());
    writer.write_all(b"ab\r").unwrap();
    assert_eq!(writer.flush().unwrap_err().kind(), ErrorKind::Other);
    let (mut sink, filter, pending) = writer.into_parts();
    assert_eq!(sink.bytes, b"a");
    assert_eq!(pending, b"b");
    sink.write_all(&pending).unwrap();
    let mut resumed = FilteredWriter::new(sink, filter);
    resumed.write_all(b"\nc").unwrap();
    resumed.finish().unwrap();
    assert_eq!(resumed.get_ref().bytes, b"ab\nc");
}

#[test]
fn borrowed_components_survive_drop_without_implicit_io_or_finalization() {
    let mut sink = ScriptedWriter::new(1, []);
    let mut filter = CountingFilter::default();
    {
        let sink: &mut dyn Write = &mut sink;
        let filter: &mut dyn MimeFilter = &mut filter;
        let mut writer = FilteredWriter::new(sink, filter);
        writer.write_all(b"pending").unwrap();
        assert_eq!(writer.buffer(), b"pending");
    }
    assert_eq!(sink.writes, 0);
    assert_eq!(sink.flushes, 0);
    assert_eq!(filter.finishes, 0);
    assert_eq!(filter.resets, 0);
    assert_eq!(filter.seen, b"pending");
}

#[test]
fn new_retains_codec_state_and_decoder_finish_keeps_incomplete_escapes() {
    let mut encoder = Base64Encoder::new();
    encoder.encode(b"C", &mut Vec::new());
    let mut writer = FilteredWriter::new(Vec::new(), EncoderFilter::new(encoder));
    writer.write_all(b"at").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"Q2F0\r\n");

    let mut writer = FilteredWriter::new(Vec::new(), DecoderFilter::new(Rfc2231Decoder::new()));
    writer.write_all(b"name%2").unwrap();
    writer.finish().unwrap();
    let (mut output, mut filter, pending) = writer.into_parts();
    assert_eq!(output, b"name");
    assert!(pending.is_empty());
    filter.filter(b"0", &mut output);
    assert_eq!(output, b"name ");
}

#[test]
fn nested_writers_finish_from_outermost_to_innermost() {
    for input in [b"x\r".as_slice(), b""] {
        let encoded = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
        let mut normalized =
            FilteredWriter::new(encoded, Dos2UnixFilter::with_ensure_new_line(true));
        normalized.write_all(input).unwrap();
        normalized.finish().unwrap();
        // Outer finish flushes the inner writer, but must not pad its codec.
        assert!(normalized.get_ref().get_ref().is_empty());
        normalized.get_mut().finish().unwrap();
        let expected = if input.is_empty() {
            b"Cg==\r\n"
        } else {
            b"eAo=\r\n"
        };
        assert_eq!(normalized.get_ref().get_ref(), expected);
    }
}

#[test]
fn nested_writer_errors_retry_without_replaying_transformed_input() {
    let sink = ScriptedWriter::new(1, [Step::Limit(1), Step::Error(ErrorKind::WouldBlock)]);
    let inner_filter = CountingFilter {
        tail: b"]".to_vec(),
        ..CountingFilter::default()
    };
    let outer_filter = CountingFilter {
        tail: b"!".to_vec(),
        ..CountingFilter::default()
    };
    let inner = FilteredWriter::new(sink, inner_filter);
    let mut outer = FilteredWriter::new(inner, outer_filter);
    outer.write_all(b"abc").unwrap();
    assert_eq!(outer.finish().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(outer.get_ref().filter().seen, b"abc!");
    assert_eq!(outer.get_ref().filter().finishes, 0);
    outer.finish().unwrap();
    outer.get_mut().finish().unwrap();
    assert_eq!(outer.get_ref().get_ref().bytes, b"abc!]");
    assert_eq!(outer.filter().finishes, 1);
    assert_eq!(outer.get_ref().filter().finishes, 1);
    assert_eq!(outer.get_ref().filter().seen, b"abc!");
}

#[test]
fn copy_into_boxed_decoder_and_standard_write_helpers_work() {
    let original: Vec<u8> = (0..=255).cycle().take(10000).collect();
    let mut encoded = Vec::new();
    let mut encoder = Base64Encoder::new();
    encoder.encode(&original, &mut encoded);
    encoder.finish(&mut encoded);
    let mut source = Cursor::new(&encoded);
    let mut writer = FilteredWriter::new(Vec::new(), decoder_filter(ContentEncoding::Base64));
    assert_eq!(
        io::copy(&mut source, &mut writer).unwrap(),
        encoded.len() as u64
    );
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), &original);

    let mut writer = FilteredWriter::new(Vec::new(), PassThroughFilter::new());
    write!(writer, "value: {}", 42).unwrap();
    let buffers = [
        IoSlice::new(b""),
        IoSlice::new(b"\n"),
        IoSlice::new(b"unused"),
    ];
    assert_eq!(writer.write_vectored(&buffers).unwrap(), 1);
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"value: 42\n");
}

#[test]
fn analysis_counts_accepted_input_before_delivery() {
    let mut writer = FilteredWriter::new(Vec::new(), BestEncodingFilter::new());
    writer.write_all(b"hello\0world").unwrap();
    assert_eq!(
        writer.filter().best_encoding(EncodingConstraint::None),
        ContentEncoding::Binary
    );
    assert!(writer.get_ref().is_empty());
    writer.finish().unwrap();
    assert_eq!(writer.get_ref(), b"hello\0world");
}

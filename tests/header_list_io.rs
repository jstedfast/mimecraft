// Header-list round-trip vectors adapted from MimeKit's HeaderListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read, Write},
    num::NonZeroUsize,
};

use mimecraft::{HeaderList, HeaderParser, HeaderTooLong, encodings::NewLineFormat};

fn load(input: &[u8]) -> HeaderList {
    HeaderList::load(&mut &input[..]).unwrap()
}

#[test]
fn buffered_loading_preserves_body_and_index_at_every_buffer_size() {
    let raw = b"Received: one\r\nsUbJeCt \t: folded\r\n\tvalue\nreceived: two\nBad Name: \xff\n";
    let input = [raw.as_slice(), b"\r\nbody\0\xff\r\n"].concat();
    for capacity in 1..=input.len() + 1 {
        let mut reader = BufReader::with_capacity(capacity, &input[..]);
        let headers = HeaderList::load(&mut reader).unwrap();
        assert_eq!(headers.len(), 4);
        assert_eq!(headers.index_of("RECEIVED"), Some(0));
        assert_eq!(headers.get_all("received").count(), 2);
        assert_eq!(
            headers.get("subject").unwrap().decode_value().text,
            "folded\tvalue"
        );
        assert!(headers.as_slice()[3].is_invalid());
        let mut output = Vec::new();
        headers.write_raw_to(&mut output).unwrap();
        assert_eq!(output, raw);
        let mut body = Vec::new();
        reader.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"body\0\xff\r\n", "capacity {capacity}");
    }
}

#[test]
fn loading_handles_empty_truncated_and_invalid_first_records() {
    for (input, raw, count) in [
        (&b""[..], &b""[..], 0),
        (b"\nbody", b"", 0),
        (b"\r\nbody", b"", 0),
        (b"\r", b"", 0),
        (b"Subject: unterminated", b"Subject: unterminated", 1),
        (b"Subject: bare CR\r", b"Subject: bare CR\r", 1),
        (b"Broken\nX: valid", b"Broken\nX: valid", 2),
        (b"X: v\n\r", b"X: v\n", 1),
        (b" \tcontinuation\n", b" \tcontinuation\n", 1),
    ] {
        for capacity in 1..=input.len().max(1) {
            let mut reader = BufReader::with_capacity(capacity, input);
            let headers = HeaderList::load(&mut reader).unwrap();
            assert_eq!(headers.len(), count);
            let mut output = Vec::new();
            headers.write_raw_to(&mut output).unwrap();
            assert_eq!(output, raw);
        }
    }
    assert!(load(b"Broken\nX: valid").as_slice()[0].is_invalid());
}

#[test]
fn parser_buffered_reads_keep_separator_metadata_and_finish_without_more_io() {
    for separator in [&b"\n"[..], &b"\r\n"[..], &b"\r"[..], &b""[..]] {
        let input = [b"X: v\n".as_slice(), separator].concat();
        let mut reader = ScriptedReader::new(&input, 1);
        let mut parser = HeaderParser::new();
        let mut headers = vec!["Before: retained".parse().unwrap()];
        parser.read_from(&mut reader, &mut headers).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].raw(), b"X: v\n");
        assert_eq!(
            parser.body_separator(),
            (!separator.is_empty()).then_some(separator)
        );
        let calls = reader.calls;
        parser.read_from(&mut reader, &mut headers).unwrap();
        assert_eq!(reader.calls, calls);
        assert_eq!(headers.len(), 2);
    }
}

#[test]
fn parser_can_switch_from_slice_input_to_buffered_input() {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    parser
        .parse(b"Subject: folded\r\n\t", &mut headers)
        .unwrap();
    let mut reader = &b"continuation\r\n\r\nbody"[..];
    parser.read_from(&mut reader, &mut headers).unwrap();
    assert_eq!(reader, b"body");
    assert_eq!(headers[0].raw(), b"Subject: folded\r\n\tcontinuation\r\n");
    assert_eq!(parser.body_separator(), Some(&b"\r\n"[..]));
}

#[test]
fn record_limits_include_folding_but_exclude_the_body_separator() {
    let record = b"X: v\r\n\tmore\r\n";
    let input = [record.as_slice(), b"\r\nbody"].concat();
    for capacity in 1..=input.len() {
        let mut reader = BufReader::with_capacity(capacity, &input[..]);
        let headers = HeaderList::load_with_max_header_length(
            &mut reader,
            NonZeroUsize::new(record.len()).unwrap(),
        )
        .unwrap();
        assert_eq!(headers.as_slice()[0].raw(), record);

        let mut reader = BufReader::with_capacity(capacity, &input[..]);
        let error = HeaderList::load_with_max_header_length(
            &mut reader,
            NonZeroUsize::new(record.len() - 1).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let inner = error
            .get_ref()
            .unwrap()
            .downcast_ref::<HeaderTooLong>()
            .unwrap();
        assert_eq!(inner.limit(), record.len() - 1);
    }
}

#[test]
fn terminal_parser_failure_does_not_attempt_another_read() {
    let mut reader = ScriptedReader::new(b"A: b\nB: too long\n\n", 1);
    let mut parser = HeaderParser::with_max_header_length(NonZeroUsize::new(5).unwrap());
    let mut headers = Vec::new();
    let error = parser.read_from(&mut reader, &mut headers).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].raw(), b"A: b\n");
    let calls = reader.calls;
    assert_eq!(
        parser
            .read_from(&mut reader, &mut headers)
            .unwrap_err()
            .kind(),
        error.kind()
    );
    assert_eq!(reader.calls, calls);
    assert!(!parser.is_finished());
}

#[test]
fn transient_read_failures_keep_partial_parser_state_and_emitted_records() {
    let input = b"A: one\nSubject: two\r\n\tcontinued\r\n\r\nbody";
    let mut reader = ScriptedReader::new(input, 1);
    reader.failures = VecDeque::from([
        (0, io::ErrorKind::Interrupted),
        (13, io::ErrorKind::WouldBlock),
        (18, io::ErrorKind::Interrupted),
    ]);
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    assert_eq!(
        parser
            .read_from(&mut reader, &mut headers)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].raw(), b"A: one\n");
    parser.read_from(&mut reader, &mut headers).unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[1].raw(), b"Subject: two\r\n\tcontinued\r\n");
    assert_eq!(&input[reader.position..], b"body");
}

#[test]
fn collection_load_retries_interrupted_reads_and_propagates_other_errors() {
    let mut reader = ScriptedReader::new(b"X: v\n\nbody", 2);
    reader.failures.push_back((0, io::ErrorKind::Interrupted));
    assert_eq!(HeaderList::load(&mut reader).unwrap().len(), 1);
    assert_eq!(reader.position, 6);
    let mut reader = ScriptedReader::new(b"X: v\nY: value", 1);
    reader
        .failures
        .push_back((8, io::ErrorKind::PermissionDenied));
    assert_eq!(
        HeaderList::load(&mut reader).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(reader.position, 8);
}

#[test]
fn source_header_list_load_vector_round_trips_with_default_crlf() {
    let input = concat!(
        "From: Joe Schmoe <joe.schmoe@example.com>\r\n",
        "To: Jane Doe <jane@example.com>\r\n",
        "Subject: Hello, World!\r\n",
        "Date: Wed, 17 Jul 2019 16:00:00 -0400\r\n",
    );
    let headers = load(input.as_bytes());
    let mut output = Vec::new();
    headers.write_to(&mut output).unwrap();
    assert_eq!(output, input.as_bytes());
    assert_eq!(load(&output), headers);
}

#[test]
fn writing_preserves_values_and_records_while_normalizing_only_newlines() {
    let input = b"sUbJeCt \t: =?utf-8?q?caf=C3=A9?=\n\tfolded\r\nX-Raw: \xff\0a\rb\nBroken\r\nLast: no newline\r";
    let headers = load(input);
    let original = headers.clone();
    for (format, expected) in [
        (NewLineFormat::Dos, &b"sUbJeCt \t: =?utf-8?q?caf=C3=A9?=\r\n\tfolded\r\nX-Raw: \xff\0a\rb\r\nBroken\r\nLast: no newline\r"[..]),
        (NewLineFormat::Unix, &b"sUbJeCt \t: =?utf-8?q?caf=C3=A9?=\n\tfolded\nX-Raw: \xff\0a\rb\nBroken\nLast: no newline\r"[..]),
    ] {
        let mut output = Vec::new();
        headers.write_to_with_new_line_format(&mut output, format).unwrap();
        assert_eq!(output, expected);
        assert_eq!(headers, original);
    }
    let mut output = Vec::new();
    headers.write_raw_to(&mut output).unwrap();
    assert_eq!(output, input);
}

#[test]
fn carriage_return_only_invalid_lines_survive_repeated_source_round_trips() {
    for (input, format, expected) in [
        (
            "A: b\n\r\r\nC: d\n\n",
            NewLineFormat::Unix,
            "A: b\n\r\r\nC: d\n\n",
        ),
        (
            "A: b\n\r\r\nC: d\n\n",
            NewLineFormat::Dos,
            "A: b\r\n\r\r\nC: d\r\n\r\n",
        ),
        (
            "A: b\r\n\r\r\r\nC: d\r\n\r\n",
            NewLineFormat::Unix,
            "A: b\n\r\r\r\nC: d\n\n",
        ),
        (
            "A: b\r\n\r\r\r\nC: d\r\n\r\n",
            NewLineFormat::Dos,
            "A: b\r\n\r\r\r\nC: d\r\n\r\n",
        ),
        (
            "A: b\n\r\r\n x\r\nC: d\n\n",
            NewLineFormat::Unix,
            "A: b\n\r\r\n x\nC: d\n\n",
        ),
        (
            "A: b\n\r\r\n x\nC: d\n\n",
            NewLineFormat::Dos,
            "A: b\r\n\r\r\n x\r\nC: d\r\n\r\n",
        ),
    ] {
        let mut input = input.as_bytes().to_vec();
        for _ in 0..5 {
            let headers = load(&input);
            assert_eq!(headers.len(), 3);
            assert!(headers.as_slice()[1].is_invalid());
            let mut output = Vec::new();
            headers
                .write_to_with_new_line_format(&mut output, format)
                .unwrap();
            // Separator ownership remains outside HeaderList in the Rust API.
            output.extend_from_slice(if format == NewLineFormat::Dos {
                b"\r\n"
            } else {
                b"\n"
            });
            assert_eq!(output, expected.as_bytes());
            input = output;
        }
    }
}

#[test]
fn conversion_state_spans_chunks_and_records_and_drains_before_bypass() {
    let mut bytes = b"X: ".to_vec();
    bytes.extend(std::iter::repeat_n(b'a', 4092));
    bytes.extend_from_slice(b"\r\n\tfolded\r\nLast: tail\r");
    let headers = load(&bytes);
    let mut output = Vec::new();
    headers
        .write_to_with_new_line_format(&mut output, NewLineFormat::Unix)
        .unwrap();
    let mut expected = b"X: ".to_vec();
    expected.extend(std::iter::repeat_n(b'a', 4092));
    expected.extend_from_slice(b"\n\tfolded\nLast: tail\r");
    assert_eq!(output, expected);
    let mut output = Vec::new();
    headers.write_to(&mut output).unwrap();
    assert_eq!(output, bytes);

    let mut headers = load(b"X: tail\r");
    headers.extend(load(b"\r\r\n continuation\r\n"));
    let mut output = Vec::new();
    headers
        .write_to_with_new_line_format(&mut output, NewLineFormat::Unix)
        .unwrap();
    assert_eq!(output, b"X: tail\r\r\r\n continuation\n");

    let mut records = load(b"X: tail\r").into_vec();
    records.extend(load(b"Y: next\n").into_vec());
    let headers = HeaderList::from(records);
    let mut output = Vec::new();
    headers.write_raw_to(&mut output).unwrap();
    assert_eq!(output, b"X: tail\rY: next\n");
    let mut output = Vec::new();
    headers
        .write_to_with_new_line_format(&mut output, NewLineFormat::Unix)
        .unwrap();
    assert_eq!(output, b"X: tail\rY: next\n");
}

#[test]
fn empty_lists_and_invalid_newline_options_have_explicit_flush_behavior() {
    let headers = HeaderList::new();
    let mut sink = TestWriter::new();
    headers.write_raw_to(&mut sink).unwrap();
    assert_eq!(sink.flushes, 0);
    headers.write_to(&mut sink).unwrap();
    assert_eq!(sink.flushes, 1);
    let error = headers
        .write_to_with_new_line_format(&mut sink, NewLineFormat::Mixed)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(sink.flushes, 1);
    assert!(sink.bytes.is_empty());
    assert_eq!(sink.writes, 0);
    assert_eq!(
        load(b"X: v\n")
            .write_to_with_new_line_format(&mut sink, NewLineFormat::Mixed)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(sink.writes, 0);
    assert_eq!(sink.flushes, 1);
}

#[test]
fn short_interrupted_and_failed_writes_report_exact_output_prefixes() {
    let headers = load(b"X: v\r\n\r\r\n folded\r\nY: tail\r");
    for format in [None, Some(NewLineFormat::Dos), Some(NewLineFormat::Unix)] {
        let mut expected = Vec::new();
        match format {
            None => headers.write_raw_to(&mut expected),
            Some(format) => headers.write_to_with_new_line_format(&mut expected, format),
        }
        .unwrap();
        for fail_after in 0..=expected.len() {
            let mut sink = TestWriter::new();
            sink.interrupt_once = true;
            sink.fail_after = Some(fail_after);
            let result = match format {
                None => headers.write_raw_to(&mut sink),
                Some(format) => headers.write_to_with_new_line_format(&mut sink, format),
            };
            if fail_after < expected.len() {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            } else {
                result.unwrap();
            }
            assert_eq!(sink.bytes, expected[..fail_after]);
        }
    }
}

#[test]
fn zero_write_and_flush_failures_are_not_hidden() {
    let headers = load(b"X: v\n");
    let mut sink = TestWriter::new();
    sink.zero_write = true;
    assert_eq!(
        headers.write_to(&mut sink).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    assert_eq!(
        headers.write_raw_to(&mut sink).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    let mut sink = TestWriter::new();
    sink.fail_flush = true;
    assert_eq!(
        headers.write_to(&mut sink).unwrap_err().kind(),
        io::ErrorKind::Other
    );
    assert_eq!(sink.bytes, b"X: v\r\n");
    let mut sink = TestWriter::new();
    sink.fail_flush = true;
    headers.write_raw_to(&mut sink).unwrap();
    assert_eq!(sink.flushes, 0);

    let mut sink = TestWriter::new();
    sink.fail_flush = true;
    let headers = load(b"X: v\n\r\r\nY: not reached\n");
    assert_eq!(
        headers.write_to(&mut sink).unwrap_err().kind(),
        io::ErrorKind::Other
    );
    assert_eq!(sink.bytes, b"X: v\r\n");
}

#[test]
fn buffered_io_accepts_trait_objects_without_owning_or_closing_them() {
    let mut input = &b"X: v\n\nbody"[..];
    let reader: &mut dyn BufRead = &mut input;
    let headers = HeaderList::load(reader).unwrap();
    assert_eq!(input, b"body");
    let mut output = Vec::new();
    let writer: &mut dyn Write = &mut output;
    headers.write_to(writer).unwrap();
    assert_eq!(output, b"X: v\r\n");
}

struct ScriptedReader<'a> {
    input: &'a [u8],
    position: usize,
    chunk: usize,
    calls: usize,
    failures: VecDeque<(usize, io::ErrorKind)>,
}

impl<'a> ScriptedReader<'a> {
    fn new(input: &'a [u8], chunk: usize) -> Self {
        Self {
            input,
            position: 0,
            chunk,
            calls: 0,
            failures: VecDeque::new(),
        }
    }
}

impl BufRead for ScriptedReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.calls += 1;
        if self
            .failures
            .front()
            .is_some_and(|&(at, _)| self.position >= at)
        {
            return Err(self.failures.pop_front().unwrap().1.into());
        }
        Ok(&self.input[self.position..(self.position + self.chunk).min(self.input.len())])
    }

    fn consume(&mut self, amount: usize) {
        self.position += amount;
    }
}

impl Read for ScriptedReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let count = input.len().min(output.len());
        output[..count].copy_from_slice(&input[..count]);
        self.consume(count);
        Ok(count)
    }
}

struct TestWriter {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
    interrupt_once: bool,
    fail_after: Option<usize>,
    zero_write: bool,
    fail_flush: bool,
}

impl TestWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            writes: 0,
            flushes: 0,
            interrupt_once: false,
            fail_after: None,
            zero_write: false,
            fail_flush: false,
        }
    }
}

impl Write for TestWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if std::mem::take(&mut self.interrupt_once) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.zero_write {
            return Ok(0);
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        if remaining == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = input.len().min(3).min(remaining);
        self.bytes.extend_from_slice(&input[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            Err(io::Error::other("flush failed"))
        } else {
            Ok(())
        }
    }
}

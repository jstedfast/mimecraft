// Header-list round-trip vectors adapted from MimeKit's HeaderListTests.cs.
// Header masking vectors adapted from MimeKit's MimeAnonymizerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Cursor, Read, Write},
    num::NonZeroUsize,
};

use mimecraft::{
    ContentEncoding, DeliveryStatusError, DeliveryStatusParseOptions, DispositionNotificationError,
    DispositionNotificationParseOptions, Header, HeaderList, HeaderParser, HeaderTooLong,
    MessageDeliveryStatus, MessageDispositionNotification, MimeContent,
    anonymize::{self, HeaderOptions},
    encodings::NewLineFormat,
};

fn load(input: &[u8]) -> HeaderList {
    HeaderList::load(&mut &input[..]).unwrap()
}

fn delivery_status(bytes: &[u8]) -> MessageDeliveryStatus {
    let mut report = MessageDeliveryStatus::new();
    report
        .part_mut()
        .set_content(MimeContent::new(Cursor::new(bytes.to_vec())));
    report
}

fn disposition_notification(bytes: &[u8]) -> MessageDispositionNotification {
    let mut report = MessageDispositionNotification::new();
    report
        .part_mut()
        .set_content(MimeContent::new(Cursor::new(bytes.to_vec())));
    report
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

#[test]
fn source_anonymizer_signature_vector_writes_only_supplied_headers() {
    // MimeAnonymizerTests includes a message separator; the header-only API
    // leaves separator ownership and all body processing to the caller.
    let mut input = &b"DKIM-Signature: v=1; a=rsa-sha256; d=example.com; s=selector;\n\tbh=abc123==; b=def456==\n\nbody"[..];
    let headers = HeaderList::load(&mut input).unwrap();
    let expected = "DKIM-Signature: x=xx x=xxxxxxxxxxx x=xxxxxxxxxxxx x=xxxxxxxxx\n\txx=xxxxxx==x x=xxxxxx==\n";
    let mut output = Vec::new();
    anonymize::write_headers(&headers, &mut output).unwrap();
    assert_eq!(output, expected.replace('\n', "\r\n").as_bytes());
    output.clear();
    anonymize::write_headers_with_options(
        headers.as_slice(),
        &mut output,
        &HeaderOptions::default(),
        NewLineFormat::Unix,
    )
    .unwrap();
    assert_eq!(output, expected.as_bytes());
    assert_eq!(input, b"body");
}

#[test]
fn masked_writing_normalizes_preserved_fields_and_keeps_order_and_duplicates() {
    let headers = load(b"sUbJeCt \t: top\r\n\tline\nTo: Alice <a@b.c>\nSubject: again\nDate: private\r\nBad Name: \xff\0\nX-Last: tail\r");
    let original = headers.clone();
    let options = HeaderOptions {
        preserve_headers: vec!["subject".into(), "Bad Name".into()],
    };
    let original_options = options.clone();
    let expected = "sUbJeCt \t: top\n\tline\nTo: xxxxx <x@x.x>\nSubject: again\nDate: private\nxxx xxxxx xx\nX-Last: xxxx\r";
    for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
        let mut output = Vec::new();
        anonymize::write_headers_with_options(&headers, &mut output, &options, format).unwrap();
        let expected = if format == NewLineFormat::Dos {
            expected.replace('\n', "\r\n")
        } else {
            expected.into()
        };
        assert_eq!(output, expected.as_bytes());
        assert_eq!(headers, original);
        assert_eq!(options, original_options);
    }
}

#[test]
fn masked_writing_converts_cr_only_invalid_lines_without_reordering() {
    let headers = load(b"A: b\r\n\r\r\n more\r\nC: d\r\n");
    assert!(headers.as_slice()[1].is_invalid());
    for (format, expected) in [
        (NewLineFormat::Unix, b"A: x\n\r\n xxxx\nC: x\n".as_slice()),
        (NewLineFormat::Dos, b"A: x\r\n\r\r\n xxxx\r\nC: x\r\n"),
    ] {
        let mut output = Vec::new();
        anonymize::write_headers_with_options(
            &headers,
            &mut output,
            &HeaderOptions::default(),
            format,
        )
        .unwrap();
        assert_eq!(output, expected);
    }
}

#[test]
fn masked_writing_handles_buffer_boundaries_and_finalizes_trailing_cr() {
    // Put CRLF pairs and encoded-word transitions across the 4096-byte chunks.
    for padding in [4068, 4069, 4070, 4084, 4085, 4086, 4087, 4088, 4089, 8182] {
        let mut raw = b"Subject: ".to_vec();
        raw.extend(std::iter::repeat_n(b'a', padding));
        raw.extend_from_slice(b"=?utf-8?q?hello?=\r\n\tend\r");
        let headers = load(&raw);
        for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
            let mut expected = b"Subject: ".to_vec();
            expected.extend(std::iter::repeat_n(b'x', padding));
            expected.extend_from_slice(if format == NewLineFormat::Unix {
                b"=?utf-8?q?xxxxx?=\n\txxx\r"
            } else {
                b"=?utf-8?q?xxxxx?=\r\n\txxx\r"
            });
            let mut output = TestWriter::new();
            output.interrupt_once = true;
            anonymize::write_headers_with_options(
                &headers,
                &mut output,
                &HeaderOptions::default(),
                format,
            )
            .unwrap();
            assert_eq!(output.bytes, expected, "padding {padding}, {format:?}");
            assert_eq!(output.flushes, 1);
        }
    }
}

#[test]
fn masked_iterables_do_not_repair_unterminated_records_or_add_separators() {
    let first = load(b"X: =?utf-8");
    let second = load(b"Y: private\r");
    let mut output = Vec::new();
    let sink: &mut dyn Write = &mut output;
    anonymize::write_headers_with_options(
        first.iter().chain(second.iter()).chain(first.iter()),
        sink,
        &HeaderOptions::default(),
        NewLineFormat::Unix,
    )
    .unwrap();
    // The first value's incomplete charset state must not leak into Y's value.
    assert_eq!(output, b"X: =?utf-8Y: xxxxxxx\rX: =?utf-8");
    output.write_all(b"caller tail").unwrap();
    assert!(output.ends_with(b"caller tail"));
}

#[test]
fn masked_preserved_values_are_not_reparsed_or_sanitized_by_the_writer() {
    let record = Header::parse(b"X-Keep: \xff\0\r\nHeader-Like: visible\n").unwrap();
    let options = HeaderOptions {
        preserve_headers: vec!["x-keep".into()],
    };
    let mut output = Vec::new();
    anonymize::write_headers_with_options([&record], &mut output, &options, NewLineFormat::Unix)
        .unwrap();
    assert_eq!(output, b"X-Keep: \xff\0\nHeader-Like: visible\n");
    assert_eq!(record.raw(), b"X-Keep: \xff\0\r\nHeader-Like: visible\n");
}

#[test]
fn masked_writer_rejects_mixed_before_iteration_and_flushes_empty_input() {
    struct UnreachableHeaders;
    impl IntoIterator for UnreachableHeaders {
        type Item = &'static Header;
        type IntoIter = std::iter::Empty<Self::Item>;

        fn into_iter(self) -> Self::IntoIter {
            panic!("invalid options must be checked before obtaining an iterator");
        }
    }
    let mut output = TestWriter::new();
    assert_eq!(
        anonymize::write_headers_with_options(
            UnreachableHeaders,
            &mut output,
            &HeaderOptions::default(),
            NewLineFormat::Mixed,
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(output.writes, 0);
    assert_eq!(output.flushes, 0);
    let headers = HeaderList::default();
    for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
        anonymize::write_headers_with_options(
            &headers,
            &mut output,
            &HeaderOptions::default(),
            format,
        )
        .unwrap();
    }
    assert_eq!(output.writes, 0);
    assert_eq!(output.flushes, 2);
    assert!(output.bytes.is_empty());
    output.fail_flush = true;
    assert_eq!(
        anonymize::write_headers(&headers, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Other,
    );
}

#[test]
fn masked_writer_propagates_errors_after_exact_prefixes_and_retry_starts_over() {
    let headers = load(b"Subject: hello\r\nBad Name: private\nY: tail\r");
    let original = headers.clone();
    let expected = "Subject: xxxxx\nxxx xxxxx xxxxxxx\nY: xxxx\r";
    for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
        let expected = if format == NewLineFormat::Dos {
            expected.replace('\n', "\r\n")
        } else {
            expected.into()
        };
        for fail_after in 0..=expected.len() {
            let mut output = TestWriter::new();
            output.interrupt_once = true;
            output.fail_after = Some(fail_after);
            let result = anonymize::write_headers_with_options(
                &headers,
                &mut output,
                &HeaderOptions::default(),
                format,
            );
            if fail_after < expected.len() {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
                assert_eq!(output.flushes, 0);
            } else {
                result.unwrap();
                assert_eq!(output.flushes, 1);
            }
            assert_eq!(output.bytes, expected.as_bytes()[..fail_after]);
            assert_eq!(headers, original);
        }
    }
    let mut output = TestWriter::new();
    output.fail_after = Some(3);
    assert!(anonymize::write_headers(&headers, &mut output).is_err());
    output.fail_after = None;
    anonymize::write_headers(&headers, &mut output).unwrap();
    assert_eq!(
        output.bytes,
        format!("Sub{}", expected.replace('\n', "\r\n")).as_bytes()
    );
}

#[test]
fn masked_writer_reports_zero_writes_and_final_flush_errors() {
    let headers = load(b"X: value\r");
    let mut output = TestWriter::new();
    output.zero_write = true;
    assert_eq!(
        anonymize::write_headers(&headers, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero,
    );
    assert!(output.bytes.is_empty());
    assert_eq!(output.flushes, 0);
    output.zero_write = false;
    output.fail_flush = true;
    assert_eq!(
        anonymize::write_headers_with_options(
            &headers,
            &mut output,
            &HeaderOptions::default(),
            NewLineFormat::Unix,
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::Other,
    );
    assert_eq!(output.bytes, b"X: xxxxx\r");
    assert_eq!(output.flushes, 1);
}

#[test]
fn masked_writer_does_not_finalize_the_callers_filter() {
    use mimecraft::{encodings::Base64Encoder, filters::EncoderFilter, io::FilteredWriter};

    let headers = load(b"X: a");
    let mut destination = FilteredWriter::new(Vec::new(), EncoderFilter::new(Base64Encoder::new()));
    anonymize::write_headers(&headers, &mut destination).unwrap();
    assert_eq!(destination.get_ref(), b"WDog");
    destination.finish().unwrap();
    assert_eq!(destination.get_ref(), b"WDogeA==\r\n");
}

#[test]
fn delivery_status_masking_preserves_selected_fields_in_every_group() {
    let raw = b"\r\nX: first\nAction: failed\r\nAction: kept\nBad Name: secret\n\r\n\nTo: Bob <b@x>\nAction: next\nStatus: 5.1.1";
    let expected = "X: xxxxx\nAction: failed\nAction: kept\nxxx xxxxx xxxxxx\n\nTo: xxx <x@x>\nAction: next\nStatus: xxxxx";
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut report = delivery_status(raw);
        report
            .part_mut()
            .headers_mut()
            .replace_all(Header::parse(b"Content-Type: MESSAGE/GLOBAL-DELIVERY-STATUS").unwrap())
            .unwrap();
        report
            .part_mut()
            .set_content_transfer_encoding(ContentEncoding::Base64);
        report
            .part_mut()
            .content_mut()
            .set_new_line_format(Some(NewLineFormat::Mixed));
        report.part_mut().content_mut().get_mut().set_position(3);
        let before = report.part().headers().clone();
        let options = HeaderOptions {
            preserve_headers: vec!["aCtIoN".into(), "Bad Name".into()],
        };
        let mut output = Vec::new();
        anonymize::write_delivery_status_content_with_options(
            &mut report,
            &mut output,
            &options,
            format,
            &DeliveryStatusParseOptions::default(),
        )
        .unwrap();
        assert_eq!(
            output,
            if format == NewLineFormat::Dos {
                expected.replace('\n', "\r\n").into_bytes()
            } else {
                expected.as_bytes().to_vec()
            }
        );
        assert_eq!(report.part().headers(), &before);
        assert_eq!(report.part().content().get_ref().get_ref(), raw);
        assert_eq!(report.part().content().get_ref().position(), 0);
        assert_eq!(
            report.part().content().encoding(),
            ContentEncoding::Unspecified
        );
        assert_eq!(
            report.part().content().new_line_format(),
            Some(NewLineFormat::Mixed)
        );
    }
}

#[test]
fn notification_masking_emits_only_the_first_decoded_field_block() {
    let raw = b"Content-Transfer-Encoding: unknown\r\nOriginal-Message-ID: <ab@cd>\nStatus: private\nBad Name: private\n\nX-Trailing: secret";
    let expected = "Content-Transfer-Encoding: unknown\nOriginal-Message-ID: <xx@xx>\nStatus: private\nxxx xxxxx xxxxxxx\n";
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut report = disposition_notification(raw);
        report
            .part_mut()
            .headers_mut()
            .replace_all(
                Header::parse(b"Content-Type: MESSAGE/GLOBAL-DISPOSITION-NOTIFICATION").unwrap(),
            )
            .unwrap();
        report
            .part_mut()
            .set_content_transfer_encoding(ContentEncoding::Base64);
        let before = report.part().headers().clone();
        let options = HeaderOptions {
            preserve_headers: vec!["status".into()],
        };
        let mut output = Vec::new();
        anonymize::write_disposition_notification_content_with_options(
            &mut report,
            &mut output,
            &options,
            format,
            &DispositionNotificationParseOptions::default(),
        )
        .unwrap();
        assert_eq!(
            output,
            if format == NewLineFormat::Dos {
                expected.replace('\n', "\r\n").into_bytes()
            } else {
                expected.as_bytes().to_vec()
            }
        );
        assert_eq!(report.part().headers(), &before);
        assert_eq!(report.part().content().get_ref().get_ref(), raw);
        assert_eq!(report.part().content().get_ref().position(), 0);
        assert_eq!(
            report.part().content().encoding(),
            ContentEncoding::Unspecified
        );
    }
}

#[test]
fn report_masking_decodes_stored_and_office365_encodings_without_reencoding() {
    let encoded = b"QWN0aW9uOiBmYWlsZWQNCg0K";
    let mut delivery = delivery_status(b"");
    delivery.part_mut().set_content(MimeContent::with_encoding(
        Cursor::new(encoded.to_vec()),
        ContentEncoding::Base64,
    ));
    let mut output = Vec::new();
    anonymize::write_delivery_status_content(&mut delivery, &mut output).unwrap();
    assert_eq!(output, b"Action: xxxxxx\r\n");
    assert_eq!(delivery.part().content().get_ref().get_ref(), encoded);

    let mut notification = disposition_notification(b"");
    notification
        .part_mut()
        .set_content(MimeContent::with_encoding(
            Cursor::new(encoded.to_vec()),
            ContentEncoding::Base64,
        ));
    output.clear();
    anonymize::write_disposition_notification_content(&mut notification, &mut output).unwrap();
    assert_eq!(output, b"Action: xxxxxx\r\n");
    assert_eq!(notification.part().content().get_ref().get_ref(), encoded);

    let raw = b"Content-Transfer-Encoding: base64\r\n\r\nQWN0aW9uOiBmYWlsZWQNCg0K";
    let mut delivery = delivery_status(raw);
    let mut parse = DeliveryStatusParseOptions {
        max_decoded_bytes: raw.len() + b"Action: failed\r\n\r\n".len(),
        ..Default::default()
    };
    output.clear();
    anonymize::write_delivery_status_content_with_options(
        &mut delivery,
        &mut output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
        &parse,
    )
    .unwrap();
    // The encoding field survives even though the recipient group is decoded.
    assert_eq!(
        output,
        b"Content-Transfer-Encoding: base64\r\n\r\nAction: xxxxxx\r\n"
    );
    parse.max_decoded_bytes -= 1;
    let mut untouched = TestWriter::new();
    assert!(matches!(
        anonymize::write_delivery_status_content_with_options(
            &mut delivery, &mut untouched, &HeaderOptions::default(), NewLineFormat::Dos, &parse,
        ),
        Err(DeliveryStatusError::Io(error)) if error.kind() == io::ErrorKind::InvalidData
    ));
    assert_eq!(untouched.writes + untouched.flushes, 0);
    assert_eq!(delivery.part().content().get_ref().position(), 0);
}

#[test]
fn report_masking_empty_and_truncated_bodies_have_no_added_separator() {
    for (raw, expected) in [
        (b"".as_slice(), b"".as_slice()),
        (b"\r", b""),
        (b"\r\n", b""),
        (b"X: a", b"X: x"),
        (b"X: a\r", b"X: x\r"),
    ] {
        let mut output = TestWriter::new();
        anonymize::write_delivery_status_content(&mut delivery_status(raw), &mut output).unwrap();
        assert_eq!(output.bytes, expected);
        assert_eq!(output.flushes, 1);
        let mut output = TestWriter::new();
        anonymize::write_disposition_notification_content(
            &mut disposition_notification(raw),
            &mut output,
        )
        .unwrap();
        assert_eq!(output.bytes, expected);
        assert_eq!(output.flushes, 1);
    }
    let mut output = Vec::new();
    anonymize::write_disposition_notification_content(
        &mut disposition_notification(b"\nX: ignored"),
        &mut output,
    )
    .unwrap();
    assert!(output.is_empty());
}

#[test]
fn report_masking_write_failures_include_group_separators_and_support_retry() {
    let raw = b"X: first\n\nY: second\r";
    let expected = b"X: xxxxx\r\n\r\nY: xxxxxx\r";
    for limit in 0..expected.len() {
        let mut report = delivery_status(raw);
        let mut output = TestWriter::new();
        output.fail_after = Some(limit);
        output.interrupt_once = true;
        assert!(matches!(
            anonymize::write_delivery_status_content(&mut report, &mut output),
            Err(DeliveryStatusError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
        ));
        assert_eq!(output.bytes, expected[..limit]);
        assert_eq!(report.part().content().get_ref().position(), 0);
        let mut retry = Vec::new();
        anonymize::write_delivery_status_content(&mut report, &mut retry as &mut dyn Write)
            .unwrap();
        assert_eq!(retry, expected);
    }
    let expected = b"X: xxxxx\r\n";
    for limit in 0..expected.len() {
        let mut report = disposition_notification(raw);
        let mut output = TestWriter::new();
        output.fail_after = Some(limit);
        output.interrupt_once = true;
        assert!(matches!(
            anonymize::write_disposition_notification_content(&mut report, &mut output),
            Err(DispositionNotificationError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
        ));
        assert_eq!(output.bytes, expected[..limit]);
        assert_eq!(report.part().content().get_ref().position(), 0);
        let mut retry = Vec::new();
        anonymize::write_disposition_notification_content(
            &mut report,
            &mut retry as &mut dyn Write,
        )
        .unwrap();
        assert_eq!(retry, expected);
    }
}

#[test]
fn report_masking_zero_writes_and_flush_errors_are_not_hidden() {
    let mut delivery = delivery_status(b"X: a\n\nY: b\n");
    let mut notification = disposition_notification(b"X: a\n");
    for zero_write in [false, true] {
        let mut output = TestWriter::new();
        output.zero_write = zero_write;
        output.fail_flush = !zero_write;
        let kind = if zero_write {
            io::ErrorKind::WriteZero
        } else {
            io::ErrorKind::Other
        };
        assert!(matches!(
            anonymize::write_delivery_status_content(&mut delivery, &mut output),
            Err(DeliveryStatusError::Io(error)) if error.kind() == kind
        ));
        assert_eq!(
            output.bytes,
            if zero_write {
                b"".as_slice()
            } else {
                b"X: x\r\n"
            }
        );
        let mut output = TestWriter::new();
        output.zero_write = zero_write;
        output.fail_flush = !zero_write;
        assert!(matches!(
            anonymize::write_disposition_notification_content(&mut notification, &mut output),
            Err(DispositionNotificationError::Io(error)) if error.kind() == kind
        ));
    }
    let mut output = TestWriter::new();
    output.fail_flush = true;
    assert!(matches!(
        anonymize::write_delivery_status_content(&mut delivery_status(b""), &mut output),
        Err(DeliveryStatusError::Io(error)) if error.kind() == io::ErrorKind::Other
    ));
    assert!(matches!(
        anonymize::write_disposition_notification_content(&mut disposition_notification(b""), &mut output),
        Err(DispositionNotificationError::Io(error)) if error.kind() == io::ErrorKind::Other
    ));
}

#[test]
fn report_masking_validates_the_entire_parsed_report_before_destination_io() {
    for (raw, limit) in [
        (
            b"X: a\n\nContent-Transfer-Encoding: bogus\n".as_slice(),
            16384,
        ),
        (b"X: a\n\nY: too long\n", 5),
    ] {
        let mut report = delivery_status(raw);
        let mut output = TestWriter::new();
        assert!(matches!(
            anonymize::write_delivery_status_content_with_options(
                &mut report, &mut output, &HeaderOptions::default(), NewLineFormat::Unix,
                &DeliveryStatusParseOptions { max_header_length: limit, ..Default::default() },
            ),
            Err(DeliveryStatusError::Io(error)) if error.kind() == io::ErrorKind::InvalidData
        ));
        assert_eq!(output.writes + output.flushes, 0);
        assert_eq!(report.part().content().get_ref().position(), 0);
    }
    let raw = b"X: a\r\n b\r\n\r\nignored tail";
    for parse in [
        DispositionNotificationParseOptions {
            max_decoded_bytes: raw.len() - 1,
            ..Default::default()
        },
        DispositionNotificationParseOptions {
            max_header_length: 9,
            ..Default::default()
        },
    ] {
        let mut report = disposition_notification(raw);
        let mut output = TestWriter::new();
        assert!(matches!(
            anonymize::write_disposition_notification_content_with_options(
                &mut report, &mut output, &HeaderOptions::default(), NewLineFormat::Unix, &parse,
            ),
            Err(DispositionNotificationError::Io(error)) if error.kind() == io::ErrorKind::InvalidData
        ));
        assert_eq!(output.writes + output.flushes, 0);
    }
    let mut report = disposition_notification(raw);
    let mut output = Vec::new();
    anonymize::write_disposition_notification_content_with_options(
        &mut report,
        &mut output,
        &HeaderOptions::default(),
        NewLineFormat::Unix,
        &DispositionNotificationParseOptions {
            max_decoded_bytes: raw.len(),
            max_header_length: b"X: a\r\n b\r\n".len(),
        },
    )
    .unwrap();
    assert_eq!(output, b"X: x\n x\n");
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

// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    AddressList, ContentEncoding, Header, HeaderList, HeaderParser, HeaderTooLong, MimeContent,
    MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, MimePartWriteOptions, Multipart,
    TextPart, encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

fn leaf(body: &[u8]) -> MimeEntity {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(body.to_vec())));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.into()
}

fn serialize(message: &mut MimeMessage, format: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    message
        .write_to_with_options(
            &mut output,
            &MimePartWriteOptions {
                new_line_format: format,
                ..Default::default()
            },
        )
        .unwrap();
    output
}

#[test]
fn constructors_are_deterministic_and_do_not_require_a_readable_source_type() {
    struct NoIo;
    let empty: MimeMessage<NoIo> = MimeMessage::default();
    assert!(empty.headers().is_empty());
    assert!(empty.body().is_none());
    assert_eq!(empty.leaf_parts().count(), 0);
    let container = Multipart::<NoIo>::new("mixed", "boundary").unwrap();
    let mut message = MimeMessage::with_body(container.into());
    assert!(message.headers().is_empty());
    assert!(message.body_mut().is_some());
    assert_eq!(message.leaf_parts().count(), 0);
    assert!(message.set_body(None).is_some());
    assert!(message.body().is_none());
}

#[test]
fn construction_replacement_and_extraction_preserve_headers_sources_and_ownership() {
    let raw = headers(b"Subject: first\nSubject: duplicate\nContent-X: misplaced\n");
    let header_pointer = raw.get("subject").unwrap().raw().as_ptr();
    let mut source = Cursor::new(b"original".to_vec());
    source.set_position(5);
    {
        let body = MimePart::new(MimeContent::new(&mut source));
        let mut message = MimeMessage::from_parts(raw.clone(), Some(body.into()));
        assert_eq!(message.headers(), &raw);
        let old = message.set_body(None).unwrap();
        assert!(message.body().is_none());
        assert_eq!(
            old.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            5
        );
        assert!(message.set_body(Some(old)).is_none());
        let (actual_headers, body) = message.into_parts();
        assert_eq!(actual_headers, raw);
        assert_eq!(body.unwrap().leaf_parts().count(), 1);
    }
    assert_eq!(source.position(), 5);
    let body = leaf(b"moved");
    let body_pointer = body
        .leaf_parts()
        .next()
        .unwrap()
        .content()
        .get_ref()
        .get_ref()
        .as_ptr();
    let message = MimeMessage::from_parts(raw, Some(body));
    assert_eq!(
        message.headers().get("subject").unwrap().raw().as_ptr(),
        header_pointer
    );
    let (_, body) = message.into_parts();
    assert_eq!(
        body.unwrap()
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref()
            .as_ptr(),
        body_pointer
    );
}

#[test]
fn header_edits_are_independent_and_typed_factories_work_without_caches() {
    let mut message = MimeMessage::with_body(leaf(b"body"));
    let addresses = AddressList::parse(b"Alice <alice@example.com>").unwrap();
    message
        .headers_mut()
        .replace_all(addresses.to_header("From").unwrap())
        .unwrap();
    assert_eq!(
        message
            .headers()
            .get("from")
            .unwrap()
            .parse_addresses()
            .unwrap()
            .mailboxes()
            .next()
            .unwrap()
            .display_name(),
        "Alice"
    );
    message
        .headers_mut()
        .add(Header::parse(b"Content-Type: text/html").unwrap());
    assert_eq!(
        message
            .body()
            .unwrap()
            .headers()
            .get("Content-Type")
            .unwrap()
            .raw_value(),
        b" application/octet-stream\r\n"
    );
    message
        .body_mut()
        .unwrap()
        .headers_mut()
        .add(Header::parse(b"X-Body: kept").unwrap());
    assert!(!message.headers().contains("X-Body"));
    let from = message.headers().get("from").unwrap().raw().to_vec();
    message.set_body(Some(leaf(b"replacement")));
    assert_eq!(message.headers().get("from").unwrap().raw(), from);
    assert!(message.headers().contains("Content-Type"));
}

#[test]
fn message_leaf_traversal_preserves_depth_first_order() {
    let mut inner: Multipart = Multipart::new("alternative", "inner").unwrap();
    inner.children_mut().extend([leaf(b"one"), leaf(b"two")]);
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    outer.children_mut().extend([
        Multipart::new("mixed", "empty").unwrap().into(),
        inner.into(),
        leaf(b"three"),
    ]);
    let message = MimeMessage::with_body(outer.into());
    let actual: Vec<&[u8]> = message
        .leaf_parts()
        .map(|part| part.content().get_ref().get_ref().as_slice())
        .collect();
    assert_eq!(actual, [b"one".as_slice(), b"two", b"three"]);
}

#[test]
fn content_header_classification_uses_parsed_case_insensitive_prefixes() {
    for field in [
        "Content-Type",
        "cOnTeNt-Transfer-Encoding",
        "CONTENT-Extension",
        "Content-",
    ] {
        assert!(
            Header::parse(format!("{field}: value").as_bytes())
                .unwrap()
                .is_content_header()
        );
    }
    for field in [
        "Content",
        "ContentType",
        "X-Content-Type",
        "MIME-Version",
        "",
    ] {
        assert!(
            !Header::parse(format!("{field}: value").as_bytes())
                .unwrap()
                .is_content_header()
        );
    }
    let invalid = headers(b"Content-no-colon\n");
    assert!(!invalid.iter().next().unwrap().is_content_header());
    assert!(
        Header::parse(b"Content-Type \t: text/plain")
            .unwrap()
            .is_content_header()
    );
}

#[test]
fn loading_partitions_only_root_headers_and_preserves_raw_records_within_each_list() {
    let raw = concat!(
        "Received: first\nContent-Type: text/plain;\n charset=utf-8\n",
        "Subject: =?utf-8?q?hello?=\nCONTENT-X-Custom: \tvalue\n",
        "MIME-Version: 1.0\nReceived: second\nContent-: extension\n",
        "Content-no-colon\nContent-Transfer-Encoding: base64\n\n",
        "aGVsbG8=\n"
    );
    let message = MimeMessage::load(&mut raw.as_bytes()).unwrap();
    let expected_message = headers(b"Received: first\nSubject: =?utf-8?q?hello?=\nMIME-Version: 1.0\nReceived: second\nContent-no-colon\n");
    let expected_body = headers(b"Content-Type: text/plain;\n charset=utf-8\nCONTENT-X-Custom: \tvalue\nContent-: extension\nContent-Transfer-Encoding: base64\n");
    assert_eq!(message.headers(), &expected_message);
    assert_eq!(message.body().unwrap().headers(), &expected_body);
    let body = message.leaf_parts().next().unwrap();
    assert_eq!(body.content().encoding(), ContentEncoding::Base64);
    assert_eq!(body.content().get_ref().get_ref(), b"aGVsbG8=\n");
    assert_eq!(body.content().get_ref().position(), 0);
    assert_eq!(body.content().new_line_format(), Some(NewLineFormat::Unix));
    assert_eq!(
        message
            .headers()
            .get("subject")
            .unwrap()
            .decode_value()
            .text,
        "hello"
    );
}

#[test]
fn opaque_loading_does_not_repartition_descendant_headers_or_parse_embedded_messages() {
    let raw = b"Subject: outer\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nX-Child: kept\r\nContent-Type: message/rfc822\r\n\r\nSubject: nested\r\nContent-Type: text/plain\r\n\r\nhello\r\n--b--\r\n";
    let message = MimeMessage::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            parse_message_parts: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(message.headers().len(), 1);
    let MimeEntity::Multipart(body) = message.body().unwrap() else {
        panic!("container expected")
    };
    assert_eq!(body.headers().len(), 1);
    assert!(body.children()[0].headers().contains("X-Child"));
    let child = message.leaf_parts().next().unwrap();
    assert_eq!(
        child.content().get_ref().get_ref(),
        b"Subject: nested\r\nContent-Type: text/plain\r\n\r\nhello"
    );
}

#[test]
fn empty_loaded_content_has_a_body_but_no_generated_headers() {
    for raw in [
        &b"\r\n"[..],
        b"Subject: empty\r\n\r\n",
        b"Date: invalid\r\n\r\n",
    ] {
        let message = MimeMessage::load(&mut &raw[..]).unwrap();
        assert!(message.body().is_some());
        assert!(message.body().unwrap().headers().is_empty());
        assert_eq!(message.leaf_parts().count(), 1);
        assert!(
            message
                .leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref()
                .is_empty()
        );
        assert!(!message.headers().contains("From"));
        assert!(!message.headers().contains("Message-Id"));
        assert!(!message.headers().contains("MIME-Version"));
    }
}

#[test]
fn message_and_body_headers_share_one_separator_without_synthetic_fields() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    message
        .headers_mut()
        .add(Header::parse(b"Subject: greeting").unwrap());
    let before_headers = message.headers().clone();
    let before_body_headers = message.body().unwrap().headers().clone();
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let nl = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let raw = serialize(&mut message, format);
        assert_eq!(
            raw,
            format!("Subject: greeting{nl}Content-Type: text/plain; charset=utf-8{nl}{nl}hello")
                .as_bytes()
        );
        assert_eq!(message.headers(), &before_headers);
        assert_eq!(message.body().unwrap().headers(), &before_body_headers);
        assert_eq!(serialize(&mut message, format), raw);
    }
}

#[test]
fn absent_body_empty_message_and_truncated_headers_write_complete_separators() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let nl = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let mut empty: MimeMessage = MimeMessage::new();
        assert_eq!(serialize(&mut empty, format), nl.as_bytes());
        let raw = headers(b"Subject: truncated");
        let mut no_body: MimeMessage = MimeMessage::from_parts(raw.clone(), None);
        assert_eq!(
            serialize(&mut no_body, format),
            format!("Subject: truncated{nl}{nl}").as_bytes()
        );
        assert_eq!(no_body.headers(), &raw);
        no_body.set_body(Some(leaf(b"body")));
        assert_eq!(serialize(&mut no_body, format), format!("Subject: truncated{nl}Content-Type: application/octet-stream{nl}Content-Transfer-Encoding: binary{nl}{nl}body").as_bytes());
    }
}

#[test]
fn interleaved_input_retains_raw_values_order_and_duplicate_fields() {
    let input = b"Subject: first\r\nContent-Type: text/plain\r\nReceived: x\r\nContent-X: value\r\nSubject: second\r\n\r\nbody";
    let mut message = MimeMessage::load(&mut &input[..]).unwrap();
    assert_eq!(serialize(&mut message, NewLineFormat::Dos), input);
    assert_eq!(message.headers().get_all("subject").count(), 2);
}

#[test]
fn interleaved_headers_round_trip_each_root_kind_and_nested_messages() {
    for raw in [
        "Content-Type: text/plain\nX-Message: value\nContent-Language: en\nSubject: end\n\nbody",
        "Content-Type: message/rfc822\nX-Message: wrapper\nContent-X: end\n\nContent-Type: text/plain\nSubject: inner\nContent-Language: en\n\nbody",
        "Content-Type: multipart/mixed; boundary=outer\nX-Message: value\nContent-X: root\nSubject: end\n\npreamble\n--outer\nContent-Type: text/plain\n\nchild\n--outer\nContent-Type: message/rfc822\n\nContent-Type: multipart/mixed; boundary=inner\nSubject: inner\nContent-Language: en\n\n--inner\nContent-Type: application/octet-stream\nContent-Transfer-Encoding: binary\n\nbinary\n--inner--\n\n--outer--\nepilogue",
    ] {
        for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
            let input = raw.replace('\n', newline);
            let mut message = MimeMessage::load(&mut input.as_bytes()).unwrap();
            let options = MimePartWriteOptions {
                new_line_format: format,
                ..Default::default()
            };
            let separator = format!("{newline}{newline}");
            let prefix_len = input.find(&separator).unwrap() + separator.len();
            let mut headers = Vec::new();
            message
                .write_headers_to_with_options(&mut headers, &options)
                .unwrap();
            assert_eq!(headers, input.as_bytes()[..prefix_len]);
            assert_eq!(serialize(&mut message, format), input.as_bytes());
            assert_eq!(serialize(&mut message, format), input.as_bytes());
            let mut content = Vec::new();
            message
                .body_mut()
                .unwrap()
                .write_content_to_with_options(&mut content, &options)
                .unwrap();
            assert_eq!(content, input.as_bytes()[prefix_len..]);
        }
    }
}

#[test]
fn interleaved_malformed_and_truncated_replacements_keep_header_framing() {
    let raw = b"Content-Type: text/plain\n\r\r\nSubject: folded\n\tvalue\nContent-X: end\n\nbody";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    message
        .headers_mut()
        .replace_all(headers(b"Subject: truncated\r").remove("Subject").unwrap())
        .unwrap();
    let expected = b"Content-Type: text/plain\n\r\r\nContent-X: end\nSubject: truncated\n\nbody";
    assert_eq!(serialize(&mut message, NewLineFormat::Unix), expected);
    let mut loaded = MimeMessage::load(&mut &expected[..]).unwrap();
    assert_eq!(serialize(&mut loaded, NewLineFormat::Unix), expected);
}

#[test]
fn message_options_reach_body_encoders_and_root_newline_handling() {
    let options = MimePartWriteOptions {
        new_line_format: NewLineFormat::Unix,
        max_line_length: 60,
        ensure_new_line: true,
    };
    let mut part = MimePart::new(MimeContent::new(Cursor::new(vec![b'a'; 80])));
    part.set_content_transfer_encoding(ContentEncoding::Base64);
    let mut message = MimeMessage::with_body(part.into());
    let mut output = Vec::new();
    message
        .write_to_with_options(&mut output, &options)
        .unwrap();
    let body_start = output
        .windows(2)
        .position(|bytes| bytes == b"\n\n")
        .unwrap()
        + 2;
    let lines: Vec<_> = output[body_start..].split(|&byte| byte == b'\n').collect();
    assert_eq!(
        lines.iter().map(|line| line.len()).collect::<Vec<_>>(),
        [60, 48, 0]
    );
    let mut parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
    let MimeEntity::Part(part) = parsed.body_mut().unwrap() else {
        panic!("leaf expected")
    };
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, vec![b'a'; 80]);

    let mut multipart: Multipart = Multipart::new("mixed", "boundary").unwrap();
    multipart.set_epilogue(b"epilogue".to_vec());
    let mut messages = [
        MimeMessage::new(),
        MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into()),
        MimeMessage::with_body(leaf(b"binary")),
        MimeMessage::with_body(multipart.into()),
    ];
    let endings = [
        b"\n".as_slice(),
        b"\n\nhello\n",
        b"\n\nbinary",
        b"--boundary--\nepilogue\n",
    ];
    for (message, ending) in messages.iter_mut().zip(endings) {
        let mut output = Vec::new();
        message
            .write_to_with_options(&mut output, &options)
            .unwrap();
        assert!(output.ends_with(ending));
    }
}

#[test]
fn supplied_bcc_and_misplaced_headers_are_not_silently_removed_or_moved() {
    let mut message = MimeMessage::from_parts(headers(b"Bcc: private@example.com\r\nResent-Bcc: other@example.com\r\nContent-X: placed-here\r\n"), Some(leaf(b"body")));
    let raw = serialize(&mut message, NewLineFormat::Dos);
    assert!(raw.starts_with(
        b"Bcc: private@example.com\r\nResent-Bcc: other@example.com\r\nContent-X: placed-here\r\n"
    ));
    assert!(message.headers().contains("Content-X"));
    assert!(!message.body().unwrap().headers().contains("Content-X"));
}

#[test]
fn nested_load_edit_write_preserves_binary_bytes_and_root_epilogue() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut inner: Multipart = Multipart::new("alternative", "inner").unwrap();
        inner
            .children_mut()
            .extend([leaf(b"\0\xff\r"), leaf(b"\r\n")]);
        inner.set_epilogue(b"tail\r".to_vec());
        let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
        outer.children_mut().push(inner.into());
        outer.set_preamble(b"preamble".to_vec());
        outer.set_epilogue(b"final".to_vec());
        let mut message = MimeMessage::from_parts(
            headers(b"Subject: before\r\nMIME-Version: 1.0\r\n"),
            Some(outer.into()),
        );
        let raw = serialize(&mut message, format);
        let mut parsed = MimeMessage::load(&mut &raw[..]).unwrap();
        assert_eq!(serialize(&mut parsed, format), raw);
        parsed
            .headers_mut()
            .replace_all(Header::parse(b"Subject: after").unwrap())
            .unwrap();
        let edited = serialize(&mut parsed, format);
        let parsed = MimeMessage::load(&mut &edited[..]).unwrap();
        assert_eq!(
            parsed.headers().get("subject").unwrap().decode_value().text,
            "after"
        );
        let bodies: Vec<&[u8]> = parsed
            .leaf_parts()
            .map(|part| part.content().get_ref().get_ref().as_slice())
            .collect();
        assert_eq!(bodies, [b"\0\xff\r".as_slice(), b"\r\n"]);
    }
}

#[test]
fn load_limits_include_message_headers_and_preserve_entity_error_kinds() {
    let raw = b"Subject: hi\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody\r\n--b--\r\n";
    let options = MimeEntityParseOptions {
        max_input_bytes: raw.len(),
        max_depth: 2,
        ..Default::default()
    };
    assert!(MimeMessage::load_with_options(&mut &raw[..], &options).is_ok());
    assert_eq!(
        MimeMessage::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                max_input_bytes: raw.len() - 1,
                ..options
            }
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        MimeMessage::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                max_depth: 1,
                ..options
            }
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidData
    );
    let error = MimeMessage::load_with_options(
        &mut &b"Subject: too long\r\n\r\n"[..],
        &MimeEntityParseOptions {
            max_header_length: 10,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<HeaderTooLong>()
            .is_some()
    );
    for input in [
        &b""[..],
        b"Subject: incomplete",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\nbody",
    ] {
        assert_eq!(
            MimeMessage::load(&mut &input[..]).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    assert_eq!(
        MimeMessage::load(&mut &b"Content-Transfer-Encoding: unknown\r\n\r\nbody"[..])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn short_and_interrupted_reads_and_source_failures_follow_loader_rules() {
    let raw = b"Subject: hi\r\nContent-Type: text/plain\r\n\r\nbody";
    for capacity in 1..=raw.len() {
        let mut source = BufReader::with_capacity(capacity, Source::new(raw));
        let mut message = MimeMessage::load(&mut source).unwrap();
        assert_eq!(serialize(&mut message, NewLineFormat::Dos), raw);
    }
    for limit in 0..=raw.len() {
        let mut source = Source::new(raw);
        source.fail_after = Some(limit);
        let mut reader = BufReader::with_capacity(7, source);
        assert_eq!(
            MimeMessage::load(&mut reader).unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }
    let mut reader = BufReader::new(Source::new(raw));
    assert_eq!(
        MimeMessage::load_with_options(
            &mut reader,
            &MimeEntityParseOptions {
                max_input_bytes: 0,
                ..Default::default()
            }
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.get_ref().calls, 0);
}

#[test]
fn output_failures_are_partial_retryable_from_start_and_leave_headers_unchanged() {
    let mut message = MimeMessage::load(
        &mut &b"Content-Type: text/plain\r\nSubject: hi\r\nContent-X: end\r\n\r\nbody"[..],
    )
    .unwrap();
    let expected = serialize(&mut message, NewLineFormat::Dos);
    for limit in 0..expected.len() {
        let mut sink = Sink {
            fail_after: Some(limit),
            ..Default::default()
        };
        assert_eq!(
            message.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        assert_eq!(serialize(&mut message, NewLineFormat::Dos), expected);
    }
    let mut sink = Sink::default();
    let destination: &mut dyn Write = &mut sink;
    message.write_to(destination).unwrap();
    assert_eq!(sink.bytes, expected);
    for flush in 1..=sink.flushes {
        let mut sink = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            message.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.flushes, flush);
    }
    assert_eq!(
        message.headers().get("subject").unwrap().raw(),
        b"Subject: hi\r\n"
    );
}

#[test]
fn invalid_write_options_fail_before_output_and_body_errors_keep_header_section() {
    let mut source = Cursor::new(b"body");
    source.set_position(2);
    let mut body = MimePart::new(MimeContent::new(&mut source));
    body.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap());
    let mut message = MimeMessage::from_parts(headers(b"Subject: hi\r\n"), Some(body.into()));
    for options in [
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 59,
            ..Default::default()
        },
    ] {
        let mut sink = Sink::default();
        assert_eq!(
            message
                .write_to_with_options(&mut sink, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
    let mut output = Vec::new();
    assert_eq!(
        message.write_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(output, b"Subject: hi\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: unknown\r\n\r\n");
    drop(message);
    assert_eq!(source.position(), 2);
}

#[test]
fn borrowed_body_read_and_seek_errors_propagate_after_message_headers() {
    for seek_error in [false, true] {
        let mut source = Source::new(b"body");
        source.fail_seek = seek_error;
        source.fail_after = Some(0);
        {
            let body = MimePart::new(MimeContent::new(&mut source));
            let mut message =
                MimeMessage::from_parts(headers(b"Subject: hi\r\n"), Some(body.into()));
            let mut output = Vec::new();
            assert_eq!(
                message.write_to(&mut output).unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert!(
                output
                    .starts_with(b"Subject: hi\r\nContent-Type: application/octet-stream\r\n\r\n")
            );
        }
        assert_eq!(source.position, 0);
    }
}

struct Source<'a> {
    bytes: &'a [u8],
    position: usize,
    calls: usize,
    fail_after: Option<usize>,
    fail_seek: bool,
}

impl<'a> Source<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            calls: 0,
            fail_after: None,
            fail_seek: false,
        }
    }
}

impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls % 5 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.position) {
            return Err(io::Error::other("read failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.position);
        let count = output
            .len()
            .min(3)
            .min(remaining)
            .min(self.bytes.len() - self.position);
        output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

impl Seek for Source<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.fail_seek {
            return Err(io::Error::other("seek failed"));
        }
        assert_eq!(position, SeekFrom::Start(0));
        self.position = 0;
        Ok(0)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    calls: usize,
    flushes: usize,
    fail_after: Option<usize>,
    fail_flush: Option<usize>,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls % 5 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.bytes.len()) {
            return Err(io::Error::other("write failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        let count = bytes.len().min(3).min(remaining);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush == Some(self.flushes) {
            return Err(io::Error::other("flush failed"));
        }
        Ok(())
    }
}

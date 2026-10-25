// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, Header, HeaderList, HeaderParser, MessagePart, MimeContent, MimeEntity,
    MimeMessage, MimePart, MimePartWriteOptions, Multipart, TextPart, encodings::NewLineFormat,
};

fn container(boundary: &str) -> Multipart {
    Multipart::with_boundary("mixed", boundary).unwrap()
}

fn leaf(bytes: &[u8], encoding: ContentEncoding) -> MimeEntity {
    let mut source = Cursor::new(bytes.to_vec());
    source.set_position(2);
    let mut part = MimePart::new(MimeContent::new(source));
    part.set_content_transfer_encoding(encoding);
    part.into()
}

fn wire(part: &mut Multipart, options: &MimePartWriteOptions) -> Vec<u8> {
    let mut output = Vec::new();
    part.write_to_with_options(&mut output, options).unwrap();
    output
}

fn content_bytes(output: &[u8], format: NewLineFormat) -> &[u8] {
    let separator = if format == NewLineFormat::Dos {
        b"\r\n\r\n".as_slice()
    } else {
        b"\n\n"
    };
    let start = output
        .windows(separator.len())
        .position(|value| value == separator)
        .unwrap();
    &output[start + separator.len()..]
}

fn nested() -> Multipart {
    let mut inner = container("inner");
    inner
        .children_mut()
        .push(TextPart::new("plain", "hello").unwrap().into());
    inner.set_epilogue(b"tail".to_vec());
    let mut outer = container("outer");
    outer.set_preamble(b"pre".to_vec());
    outer.children_mut().push(inner.into());
    outer.set_epilogue(b"done".to_vec());
    outer
}

#[test]
fn nested_tree_has_exact_framing_and_root_only_newline_option() {
    let expected = concat!(
        "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
        "pre\r\n--outer\r\n",
        "Content-Type: multipart/mixed; boundary=inner\r\n\r\n",
        "--inner\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n\r\n",
        "hello\r\n--inner--\r\n",
        "tail\r\n--outer--\r\n",
        "done",
    );
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for ensure in [false, true] {
            let mut part = nested();
            let options = MimePartWriteOptions {
                new_line_format: format,
                ensure_new_line: ensure,
                ..Default::default()
            };
            let output = wire(&mut part, &options);
            let mut expected = expected.to_owned();
            if ensure {
                expected.push_str("\r\n");
            }
            if format == NewLineFormat::Unix {
                expected = expected.replace("\r\n", "\n");
            }
            assert_eq!(output, expected.as_bytes());
            assert_eq!(part.preamble(), b"pre");
            assert_eq!(part.epilogue(), b"done");
            assert_eq!(
                part.leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .position(),
                0
            );
            assert_eq!(wire(&mut part, &options), output);
            let mut content_only = Vec::new();
            part.write_content_to_with_options(&mut content_only, &options)
                .unwrap();
            assert_eq!(content_only, content_bytes(&output, format));
        }
    }
}

#[test]
fn delimiter_newline_never_consumes_binary_suffix_bytes_or_empty_bodies() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let newline = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        for bytes in [
            &b""[..],
            b"\0\xff",
            b"body\r",
            b"body\n",
            b"body\r\n",
            b"body\r\n\r\n",
        ] {
            let mut part = container("outer");
            part.children_mut()
                .push(leaf(bytes, ContentEncoding::Binary));
            part.children_mut()
                .push(leaf(b"second", ContentEncoding::Binary));
            let output = wire(
                &mut part,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line: true,
                    ..Default::default()
                },
            );
            let child_headers = format!(
                "Content-Type: application/octet-stream{newline}Content-Transfer-Encoding: binary{newline}{newline}"
            );
            let prefix = format!(
                "Content-Type: multipart/mixed; boundary=outer{newline}{newline}--outer{newline}{child_headers}"
            );
            let delimiter_newline = if format == NewLineFormat::Unix && bytes.ends_with(b"\r") {
                "\r\n"
            } else {
                newline
            };
            let middle = format!("{delimiter_newline}--outer{newline}{child_headers}");
            let ending = format!("second{newline}--outer--{newline}");
            let expected = [
                prefix.as_bytes(),
                bytes,
                middle.as_bytes(),
                ending.as_bytes(),
            ]
            .concat();
            assert_eq!(output, expected);
            let content_start = prefix.len();
            let delimiter_start = content_start + bytes.len();
            assert_eq!(&output[content_start..delimiter_start], bytes);
            assert!(output[delimiter_start..].starts_with(middle.as_bytes()));
            let mut content_only = Vec::new();
            part.write_content_to_with_options(
                &mut content_only,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(content_only, content_bytes(&output, format));
        }
    }
}

#[test]
fn empty_containers_and_nonempty_preambles_are_properly_terminated() {
    for preamble in [&b""[..], b"intro", b"intro\n", b"intro\r", b"intro\r\n"] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let newline = if format == NewLineFormat::Dos {
                "\r\n"
            } else {
                "\n"
            };
            let mut part = container("empty");
            part.set_preamble(preamble.to_vec());
            let output = wire(
                &mut part,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ..Default::default()
                },
            );
            let text = if preamble.is_empty() {
                String::new()
            } else {
                format!("intro{newline}")
            };
            assert_eq!(output, format!(
                "Content-Type: multipart/mixed; boundary=empty{newline}{newline}{text}--empty--{newline}"
            ).as_bytes());
        }
    }
    let mut outer = container("outer");
    outer.children_mut().push(container("empty").into());
    assert_eq!(
        wire(&mut outer, &MimePartWriteOptions::default()),
        concat!(
            "Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\n",
            "Content-Type: multipart/mixed; boundary=empty\r\n\r\n--empty--\r\n",
            "\r\n--outer--\r\n"
        )
        .as_bytes()
    );
}

#[test]
fn raw_preamble_and_epilogue_preserve_non_newline_bytes() {
    let mut part = container("b");
    part.set_preamble(b"\xffa\r\nb\nc\rd".to_vec());
    part.set_epilogue(b"\0a\r\nb\nc\rd".to_vec());
    let before = part.headers().clone();
    let output = wire(
        &mut part,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    );
    assert_eq!(
        output,
        b"Content-Type: multipart/mixed; boundary=b\n\n\xffa\nb\nc\rd\n--b--\n\0a\nb\nc\rd"
    );
    assert_eq!(part.headers(), &before);
    assert_eq!(part.preamble(), b"\xffa\r\nb\nc\rd");
    assert_eq!(part.epilogue(), b"\0a\r\nb\nc\rd");
}

#[test]
fn unix_delimiter_guard_handles_short_writes_and_nested_epilogues() {
    let mut inner = container("inner");
    inner
        .children_mut()
        .push(leaf(b"binary\r", ContentEncoding::Binary));
    inner.set_epilogue(b"tail\r".to_vec());
    let mut outer = container("outer");
    outer.children_mut().push(inner.into());
    let options = MimePartWriteOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    let expected = wire(&mut outer, &options);
    assert!(
        expected
            .windows(b"binary\r\r\n--inner--\n".len())
            .any(|bytes| bytes == b"binary\r\r\n--inner--\n")
    );
    assert!(expected.ends_with(b"tail\r\r\n--outer--\n"));
    let mut sink = Sink {
        chunk: 1,
        interrupt: true,
        ..Default::default()
    };
    outer.write_to_with_options(&mut sink, &options).unwrap();
    assert_eq!(sink.bytes, expected);
}

#[test]
fn leaf_serialization_is_reused_for_every_transfer_encoding() {
    for encoding in [
        ContentEncoding::Unspecified,
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut single = leaf(b"Cat\n", encoding);
        let mut leaf_output = Vec::new();
        single.write_to(&mut leaf_output).unwrap();
        let mut part = container("b");
        part.children_mut().push(leaf(b"Cat\n", encoding));
        let before = part.children()[0].headers().clone();
        let output = wire(&mut part, &MimePartWriteOptions::default());
        assert_eq!(
            output,
            [
                b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n".as_slice(),
                &leaf_output,
                b"\r\n--b--\r\n"
            ]
            .concat()
        );
        assert_eq!(part.children()[0].headers(), &before);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            b"Cat\n"
        );
    }
    let mut source = MimePart::new(MimeContent::with_encoding(
        Cursor::new(b"Q2F0\r\n".to_vec()),
        ContentEncoding::Base64,
    ));
    source.set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    let mut part = container("b");
    part.children_mut().push(source.into());
    assert!(
        wire(&mut part, &MimePartWriteOptions::default())
            .ends_with(b"\r\n\r\nCat=\r\n\r\n--b--\r\n")
    );
}

#[test]
fn leaf_roots_match_direct_writer_and_multipart_entity_roots_match_containers() {
    let mut entity = leaf(b"hello", ContentEncoding::Base64);
    let mut actual = Vec::new();
    entity.write_to(&mut actual).unwrap();
    let MimeEntity::Part(mut part) = entity else {
        panic!("leaf expected")
    };
    let mut expected = Vec::new();
    part.write_to(&mut expected).unwrap();
    assert_eq!(actual, expected);

    let expected = wire(&mut nested(), &MimePartWriteOptions::default());
    let mut root: MimeEntity = nested().into();
    let mut output = Vec::new();
    let destination: &mut dyn Write = &mut output;
    root.write_to(destination).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn invalid_root_metadata_and_options_fail_before_output_or_flush() {
    for raw in [
        None,
        Some("Content-Type: invalid"),
        Some("Content-Type: text/plain; boundary=b"),
        Some("Content-Type: multipart/mixed"),
        Some("Content-Type: multipart/mixed; boundary=\"bad \""),
    ] {
        let mut part = container("b");
        part.headers_mut().clear();
        if let Some(raw) = raw {
            part.headers_mut()
                .add(Header::parse(raw.as_bytes()).unwrap());
        }
        part.children_mut()
            .push(leaf(b"untouched", ContentEncoding::Binary));
        let mut sink = Sink::default();
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            part.write_content_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(sink.calls + sink.flushes, 0);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            2
        );
    }
    for options in [
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 59,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 999,
            ..Default::default()
        },
    ] {
        let mut sink = Sink::default();
        assert_eq!(
            nested()
                .write_to_with_options(&mut sink, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            nested()
                .write_content_to_with_options(&mut sink, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
}

#[test]
fn multipart_transfer_encodings_are_checked_using_the_first_header() {
    for encoding in ["7bit", "8bit", "binary", "=?us-ascii?q?7bit?="] {
        let mut part = container("b");
        part.headers_mut().add(
            Header::parse(format!("Content-Transfer-Encoding: {encoding}").as_bytes()).unwrap(),
        );
        assert!(part.write_to(&mut Vec::new()).is_ok());
    }
    for encoding in ["base64", "quoted-printable", "x-uuencode", "unknown", ""] {
        let mut part = container("b");
        part.headers_mut().add(
            Header::parse(format!("Content-Transfer-Encoding: {encoding}").as_bytes()).unwrap(),
        );
        part.headers_mut()
            .add(Header::parse(b"Content-Transfer-Encoding: 7bit").unwrap());
        let mut sink = Sink::default();
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            part.write_content_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
}

#[test]
fn ancestor_boundary_overlap_is_rejected_but_sibling_reuse_is_allowed() {
    for (outer, inner) in [
        ("same", "same"),
        ("outer", "outer-long"),
        ("outer-long", "outer"),
    ] {
        let mut part = container(outer);
        let mut nested = container("distinct");
        nested.children_mut().push(container(inner).into());
        part.children_mut().push(nested.into());
        let mut output = Vec::new();
        let error = part.write_to(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("ancestor"));
        assert!(!output.ends_with(format!("--{outer}--\r\n").as_bytes()));
    }
    let mut part = container("outer");
    part.children_mut()
        .extend([container("shared").into(), container("shared").into()]);
    let output = wire(&mut part, &MimePartWriteOptions::default());
    assert_eq!(
        output
            .windows(b"--shared--".len())
            .filter(|bytes| *bytes == b"--shared--")
            .count(),
        2
    );
}

#[test]
fn container_headers_repair_truncation_without_mutating_raw_records() {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser
        .parse(
            b"Content-Type: multipart/mixed; boundary=b\nX: unterminated",
            &mut records,
        )
        .unwrap();
    parser.finish(&mut records).unwrap();
    let raw: HeaderList = records.into();
    let mut part: Multipart =
        Multipart::from_parts(raw.clone(), Vec::new(), Vec::new(), Vec::new());
    let output = wire(&mut part, &MimePartWriteOptions::default());
    assert_eq!(
        output,
        b"Content-Type: multipart/mixed; boundary=b\r\nX: unterminated\r\n\r\n--b--\r\n"
    );
    assert_eq!(part.headers(), &raw);
}

#[test]
fn deep_trees_stream_without_recursive_serialization() {
    let mut root = leaf(b"deep", ContentEncoding::Binary);
    for depth in 0..512 {
        let mut part = container(&format!("boundary-{depth:04}-end"));
        part.children_mut().push(root);
        root = part.into();
    }
    let mut sink = Sink {
        chunk: 3,
        interrupt: true,
        ..Default::default()
    };
    root.write_to(&mut sink).unwrap();
    assert!(sink.bytes.ends_with(b"\r\n--boundary-0511-end--\r\n"));
    assert_eq!(
        sink.bytes
            .windows(4)
            .filter(|bytes| *bytes == b"deep")
            .count(),
        1
    );
    assert_eq!(
        root.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        0
    );
}

#[test]
fn parsed_raw_marker_write_failures_preserve_prefix_and_allow_retry() {
    let expected = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b \t\nContent-Type: text/plain\r\n\r\nbody\r\n--b-- \t";
    let mut entity = MimeEntity::load(&mut expected.as_slice()).unwrap();
    for limit in 0..expected.len() {
        let mut sink = Sink {
            fail_after: Some(limit),
            chunk: 1,
            interrupt: true,
            ..Default::default()
        };
        assert_eq!(
            entity.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        let mut output = Vec::new();
        entity.write_to(&mut output).unwrap();
        assert_eq!(output, expected);
    }
}

#[test]
fn write_zero_short_interrupted_and_flush_failures_propagate_without_continuing() {
    let expected = wire(&mut nested(), &MimePartWriteOptions::default());
    for limit in 0..expected.len() {
        let mut part = nested();
        let mut sink = Sink {
            fail_after: Some(limit),
            chunk: 3,
            interrupt: true,
            ..Default::default()
        };
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        assert_eq!(wire(&mut part, &MimePartWriteOptions::default()), expected);
    }
    assert_eq!(
        nested()
            .write_to(&mut Sink {
                chunk: 0,
                ..Default::default()
            })
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero
    );
    let mut sink = Sink::default();
    nested().write_to(&mut sink).unwrap();
    for flush in 1..=sink.flushes {
        let mut failing = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            nested().write_to(&mut failing).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(failing.flushes, flush);
    }
}

#[test]
fn child_io_failure_stops_before_following_siblings_and_preserves_borrows() {
    for fail_seek in [None, Some(1), Some(2)] {
        let mut first = Source::new(false);
        let mut failing = Source::new(fail_seek.is_none());
        failing.fail_seek = fail_seek;
        let mut last = Source::new(false);
        {
            let mut root = Multipart::with_boundary("mixed", "outer").unwrap();
            for source in [&mut first, &mut failing, &mut last] {
                root.children_mut()
                    .push(MimePart::new(MimeContent::new(source)).into());
            }
            let mut output = Vec::new();
            assert_eq!(
                root.write_to(&mut output).unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert!(!output.ends_with(b"--outer--\r\n"));
        }
        assert_eq!(first.inner.position(), 0);
        assert_eq!(first.seeks, 2);
        assert_eq!(failing.seeks, fail_seek.unwrap_or(1));
        assert_eq!(last.seeks, 0);
        assert_eq!(last.inner.position(), 2);
    }
}

#[test]
fn descendant_metadata_errors_leave_earlier_siblings_written_and_later_sources_untouched() {
    let mut invalid_container = container("inner");
    invalid_container.headers_mut().clear();
    invalid_container
        .children_mut()
        .push(leaf(b"untouched", ContentEncoding::Binary));
    let mut invalid_leaf = leaf(b"untouched", ContentEncoding::Binary);
    invalid_leaf
        .headers_mut()
        .replace_all(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap())
        .unwrap();
    for invalid in [invalid_container.into(), invalid_leaf] {
        let mut root = container("outer");
        root.children_mut().extend([
            leaf(b"first", ContentEncoding::Binary),
            invalid,
            leaf(b"last", ContentEncoding::Binary),
        ]);
        let mut sink = Sink::default();
        assert_eq!(
            root.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(sink.bytes.ends_with(b"\r\n\r\nfirst\r\n--outer\r\n"));
        assert_eq!(
            root.leaf_parts()
                .map(|part| part.content().get_ref().position())
                .collect::<Vec<_>>(),
            [0, 2, 2]
        );
    }
}

#[test]
fn large_raw_sections_and_chunked_binary_sources_cross_filter_buffers() {
    let payload = b"binary\0\r\n\xff".repeat(4096);
    let mut source = Source::new(false);
    source.inner = Cursor::new(payload.clone());
    let mut body = MimePart::new(MimeContent::new(source));
    body.set_content_transfer_encoding(ContentEncoding::Binary);
    let mut root = Multipart::with_boundary("mixed", "outer").unwrap();
    root.children_mut().push(body.into());
    root.set_preamble(b"a\r\nb\n".repeat(4096));
    root.set_epilogue(b"a\r\nb\n".repeat(4096));
    let mut sink = Sink {
        chunk: 7,
        interrupt: true,
        ..Default::default()
    };
    root.write_to_with_options(
        &mut sink,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    )
    .unwrap();
    let normalized = b"a\nb\n".repeat(4096);
    assert_eq!(sink.bytes, [
        b"Content-Type: multipart/mixed; boundary=outer\n\n".as_slice(), &normalized,
        b"--outer\nContent-Type: application/octet-stream\nContent-Transfer-Encoding: binary\n\n",
        &payload, b"\n--outer--\n", &normalized
    ].concat());
    let source = root.leaf_parts().next().unwrap().content().get_ref();
    assert_eq!(source.inner.position(), 0);
    assert!(source.reads > payload.len() / 3);
}

#[test]
fn content_only_entity_dispatch_omits_only_the_root_header_section() {
    let message = MimeMessage::from_parts(
        vec![
            Header::parse(b"Subject: attached").unwrap(),
            Header::parse(b"Bcc: kept@example.com").unwrap(),
        ]
        .into(),
        Some(nested().into()),
    );
    let mut entities = [
        leaf(b"binary\0\r", ContentEncoding::Binary),
        nested().into(),
        MessagePart::new(message).into(),
    ];
    for entity in &mut entities {
        let headers = entity.headers().clone();
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            for ensure_new_line in [false, true] {
                let options = MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line,
                    ..Default::default()
                };
                let mut complete = Vec::new();
                entity
                    .write_to_with_options(&mut complete, &options)
                    .unwrap();
                let mut content_only = Vec::new();
                entity
                    .write_content_to_with_options(&mut content_only as &mut dyn Write, &options)
                    .unwrap();
                assert_eq!(content_only, content_bytes(&complete, format));
                assert_eq!(entity.headers(), &headers);
                for part in entity.leaf_parts() {
                    assert_eq!(part.content().get_ref().position(), 0);
                }
            }
        }
        let mut complete = Vec::new();
        entity.write_to(&mut complete).unwrap();
        let mut content_only = Vec::new();
        entity.write_content_to(&mut content_only).unwrap();
        assert_eq!(content_only, content_bytes(&complete, NewLineFormat::Dos));
    }
}

#[test]
fn content_only_empty_containers_and_absent_or_empty_messages_have_exact_framing() {
    for (format, newline) in [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")] {
        let options = MimePartWriteOptions {
            new_line_format: format,
            ensure_new_line: true,
            ..Default::default()
        };
        let mut empty = container("empty");
        let mut output = Vec::new();
        empty
            .write_content_to_with_options(&mut output, &options)
            .unwrap();
        assert_eq!(output, format!("--empty--{newline}").as_bytes());
        empty.set_preamble(b"intro".to_vec());
        empty.set_epilogue(b"tail".to_vec());
        output.clear();
        empty
            .write_content_to_with_options(&mut output, &options)
            .unwrap();
        assert_eq!(
            output,
            format!("intro{newline}--empty--{newline}tail{newline}").as_bytes()
        );

        let mut wrapper: MessagePart = MessagePart::new(MimeMessage::new());
        let mut sink = Sink::default();
        wrapper
            .write_content_to_with_options(&mut sink, &options)
            .unwrap();
        assert_eq!(sink.bytes, newline.as_bytes());
        assert!(sink.flushes > 0);
        wrapper.set_message(None);
        let mut sink = Sink::default();
        wrapper
            .write_content_to_with_options(&mut sink, &options)
            .unwrap();
        assert!(sink.bytes.is_empty());
        assert!(sink.flushes > 0);
    }
    let mut absent: MessagePart = MessagePart::new(MimeMessage::new());
    absent.set_message(None);
    let mut sink = Sink::default();
    absent.write_content_to(&mut sink).unwrap();
    assert!(sink.bytes.is_empty());
    assert!(sink.flushes > 0);
}

#[test]
fn content_only_wrappers_keep_enclosed_message_headers_and_do_not_filter_bcc() {
    for content_type in ["message/rfc822", "message/global", "text/rfc822-headers"] {
        let message = MimeMessage::from_parts(
            vec![
                Header::parse(b"Subject: inside").unwrap(),
                Header::parse(b"Bcc: kept@example.com").unwrap(),
            ]
            .into(),
            Some(leaf(b"binary", ContentEncoding::Binary)),
        );
        let mut wrapper = MessagePart::from_parts(
            vec![
                Header::parse(format!("Content-Type: {content_type}").as_bytes()).unwrap(),
                Header::parse(b"X-Wrapper: omit").unwrap(),
            ]
            .into(),
            Some(message),
        );
        let before = wrapper.headers().clone();
        let mut output = Vec::new();
        wrapper.write_content_to(&mut output).unwrap();
        assert_eq!(
            output,
            b"Subject: inside\r\nBcc: kept@example.com\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: binary\r\n\r\nbinary"
        );
        assert_eq!(wrapper.headers(), &before);
    }
}

#[test]
fn content_only_preserves_digest_child_defaults_but_not_for_standalone_wrappers() {
    let wrapper = MessagePart::from_parts(HeaderList::new(), Some(MimeMessage::new()));
    let mut digest: Multipart = Multipart::with_boundary("digest", "digest").unwrap();
    digest.children_mut().push(wrapper.into());
    let mut output = Vec::new();
    digest.write_content_to(&mut output).unwrap();
    assert_eq!(output, b"--digest\r\n\r\n\r\n\r\n--digest--\r\n");
    let MimeEntity::Message(wrapper) = &mut digest.children_mut()[0] else {
        panic!("wrapper expected")
    };
    let mut sink = Sink::default();
    assert_eq!(
        wrapper.write_content_to(&mut sink).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(sink.calls + sink.flushes, 0);
}

#[test]
fn content_only_still_validates_root_wrappers_and_all_options_before_io() {
    for raw in [
        b"".as_slice(),
        b"Content-Type: text/plain",
        b"Content-Type: invalid",
        b"Content-Type: message/global\r\nContent-Transfer-Encoding: base64",
        b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding: unknown",
    ] {
        let mut parser = HeaderParser::new();
        let mut records = Vec::new();
        parser.parse(raw, &mut records).unwrap();
        parser.finish(&mut records).unwrap();
        let mut part = MessagePart::from_parts(
            records.into(),
            Some(MimeMessage::with_body(leaf(
                b"untouched",
                ContentEncoding::Binary,
            ))),
        );
        let mut sink = Sink::default();
        assert_eq!(
            part.write_content_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(sink.calls + sink.flushes, 0);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            2
        );
    }
    let mut roots = [
        leaf(b"untouched", ContentEncoding::Base64),
        nested().into(),
        MessagePart::new(MimeMessage::with_body(leaf(
            b"untouched",
            ContentEncoding::Binary,
        )))
        .into(),
    ];
    for options in [
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 59,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 999,
            ..Default::default()
        },
    ] {
        for root in &mut roots {
            let positions: Vec<_> = root
                .leaf_parts()
                .map(|part| part.content().get_ref().position())
                .collect();
            let mut sink = Sink::default();
            assert_eq!(
                root.write_content_to_with_options(&mut sink, &options)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(sink.calls + sink.flushes, 0);
            assert_eq!(
                root.leaf_parts()
                    .map(|part| part.content().get_ref().position())
                    .collect::<Vec<_>>(),
                positions
            );
        }
    }
}

#[test]
fn content_only_root_boundaries_remain_active_inside_attached_messages() {
    for (outer, inner) in [
        ("same", "same"),
        ("outer", "outer-long"),
        ("outer-long", "outer"),
    ] {
        let mut root = container(outer);
        let attached = MessagePart::new(MimeMessage::with_body(container(inner).into()));
        root.children_mut().push(attached.into());
        let mut output = Vec::new();
        let error = root.write_content_to(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("ancestor"));
        assert_eq!(
            output,
            format!("--{outer}\r\nContent-Type: message/rfc822\r\n\r\nContent-Type: multipart/mixed; boundary={inner}\r\n\r\n").as_bytes()
        );
    }
}

#[test]
fn content_only_sink_errors_stop_at_the_failure_and_retries_restart() {
    let mut successful = Sink {
        chunk: 3,
        interrupt: true,
        ..Default::default()
    };
    nested().write_content_to(&mut successful).unwrap();
    let expected = &successful.bytes;
    for limit in 0..expected.len() {
        let mut root = nested();
        let mut sink = Sink {
            chunk: 3,
            interrupt: true,
            fail_after: Some(limit),
            ..Default::default()
        };
        assert_eq!(
            root.write_content_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        let mut retry = Vec::new();
        root.write_content_to(&mut retry).unwrap();
        assert_eq!(&retry, expected);
    }
    assert_eq!(
        nested()
            .write_content_to(&mut Sink {
                chunk: 0,
                ..Default::default()
            })
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero
    );
    for flush in 1..=successful.flushes {
        let mut failing = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            nested().write_content_to(&mut failing).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(failing.flushes, flush);
    }
}

#[test]
fn content_only_child_io_failure_leaves_later_borrowed_sources_untouched() {
    let mut first = Source::new(false);
    let mut failing = Source::new(true);
    let mut last = Source::new(false);
    {
        let mut root = Multipart::with_boundary("mixed", "outer").unwrap();
        for source in [&mut first, &mut failing, &mut last] {
            root.children_mut()
                .push(MimePart::new(MimeContent::new(source)).into());
        }
        let mut output = Vec::new();
        assert_eq!(
            root.write_content_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert!(output.starts_with(b"--outer\r\n"));
        assert!(!output.ends_with(b"--outer--\r\n"));
    }
    assert_eq!(first.seeks, 2);
    assert_eq!(first.inner.position(), 0);
    assert_eq!(failing.seeks, 1);
    assert_eq!(last.seeks, 0);
    assert_eq!(last.inner.position(), 2);
}

#[test]
fn content_only_deep_wrappers_remain_iterative_and_omit_exactly_one_header_section() {
    const DEPTH: usize = 10_000;
    let mut root = leaf(b"deep", ContentEncoding::Binary);
    for _ in 0..DEPTH {
        root = MessagePart::new(MimeMessage::with_body(root)).into();
    }
    let mut output = Vec::new();
    root.write_content_to(&mut output).unwrap();
    let wrapper_header = b"Content-Type: message/rfc822\r\n\r\n";
    let count = output
        .windows(wrapper_header.len())
        .filter(|value| *value == wrapper_header)
        .count();
    assert_eq!(count, DEPTH - 1);
    assert!(output.ends_with(b"\r\n\r\ndeep"));
    assert_eq!(
        root.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        0
    );
    // Writing is iterative; the model's ordinary Drop still recurses.
    while let MimeEntity::Message(wrapper) = root {
        root = wrapper.into_parts().1.unwrap().into_parts().1.unwrap();
    }
}

struct Source {
    inner: Cursor<Vec<u8>>,
    fail: bool,
    seeks: usize,
    reads: usize,
    fail_seek: Option<usize>,
}

impl Source {
    fn new(fail: bool) -> Self {
        let mut inner = Cursor::new(b"body".to_vec());
        inner.set_position(2);
        Self {
            inner,
            fail,
            seeks: 0,
            reads: 0,
            fail_seek: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail {
            return Err(io::Error::other("read failed"));
        }
        let count = bytes.len().min(3);
        self.inner.read(&mut bytes[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.inner.seek(pos)
    }
}

struct Sink {
    bytes: Vec<u8>,
    calls: usize,
    flushes: usize,
    chunk: usize,
    interrupt: bool,
    fail_after: Option<usize>,
    fail_flush: Option<usize>,
}

impl Default for Sink {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            calls: 0,
            flushes: 0,
            chunk: usize::MAX,
            interrupt: false,
            fail_after: None,
            fail_flush: None,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.bytes.len()) {
            return Err(io::Error::other("write failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        let count = bytes.len().min(self.chunk).min(remaining);
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

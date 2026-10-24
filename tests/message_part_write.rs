// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, Header, HeaderList, HeaderParser, MessagePart, MessagePartError, MimeContent,
    MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, MimePartWriteOptions, Multipart,
    TextPart, UnknownContentEncoding, encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

fn leaf(body: &[u8]) -> MimeEntity {
    let mut source = Cursor::new(body.to_vec());
    source.set_position(2);
    let mut part = MimePart::new(MimeContent::new(source));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.into()
}

fn attachment(body: MimeEntity) -> MessagePart {
    MessagePart::new(MimeMessage::from_parts(
        headers(b"Subject: attached\r\n"),
        Some(body),
    ))
}

fn wire(part: &mut MessagePart, format: NewLineFormat, ensure: bool) -> Vec<u8> {
    let mut output = Vec::new();
    part.write_to_with_options(
        &mut output,
        &MimePartWriteOptions {
            new_line_format: format,
            ensure_new_line: ensure,
            ..Default::default()
        },
    )
    .unwrap();
    output
}

#[test]
fn wrapper_message_and_body_each_have_the_correct_header_separator() {
    let expected = concat!(
        "Content-Type: message/rfc822\r\n\r\nSubject: attached\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n\r\nhello",
    );
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for ensure in [false, true] {
            let mut part = attachment(TextPart::new("plain", "hello").unwrap().into());
            let before = part.headers().clone();
            let mut expected = expected.to_owned();
            if ensure {
                expected.push_str("\r\n");
            }
            if format == NewLineFormat::Unix {
                expected = expected.replace("\r\n", "\n");
            }
            assert_eq!(wire(&mut part, format, ensure), expected.as_bytes());
            assert_eq!(wire(&mut part, format, ensure), expected.as_bytes());
            assert_eq!(part.headers(), &before);
            assert!(!part.message().unwrap().headers().contains("MIME-Version"));
            assert_eq!(
                part.leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .position(),
                0
            );
        }
    }
}

#[test]
fn absent_message_and_present_bodyless_message_remain_distinct() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let nl = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let mut part: MessagePart = MessagePart::new(MimeMessage::new());
        part.set_message(None);
        assert_eq!(
            wire(&mut part, format, true),
            format!("Content-Type: message/rfc822{nl}{nl}").as_bytes()
        );
        part.set_message(Some(MimeMessage::new()));
        assert_eq!(
            wire(&mut part, format, true),
            format!("Content-Type: message/rfc822{nl}{nl}{nl}").as_bytes()
        );
        part.message_mut()
            .unwrap()
            .headers_mut()
            .add(Header::parse(b"Subject: empty").unwrap());
        assert_eq!(
            wire(&mut part, format, false),
            format!("Content-Type: message/rfc822{nl}{nl}Subject: empty{nl}{nl}").as_bytes()
        );
    }
}

#[test]
fn all_entry_points_serialize_the_same_wrapped_message_and_keep_bcc() {
    let mut part = attachment(leaf(b"binary\0\xff\r"));
    part.message_mut()
        .unwrap()
        .headers_mut()
        .add(Header::parse(b"Bcc: private@example.com").unwrap());
    let expected = wire(&mut part, NewLineFormat::Dos, true);
    assert!(expected.ends_with(b"binary\0\xff\r"));
    assert!(
        expected
            .windows(b"Bcc: private@example.com\r\n".len())
            .any(|bytes| bytes == b"Bcc: private@example.com\r\n")
    );
    let mut entity: MimeEntity = part.into();
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert_eq!(output, expected);
    let mut message = MimeMessage::with_body(entity);
    message
        .headers_mut()
        .add(Header::parse(b"Subject: outer").unwrap());
    output.clear();
    message.write_to(&mut output).unwrap();
    assert_eq!(
        output,
        [b"Subject: outer\r\n".as_slice(), &expected].concat()
    );
}

#[test]
fn nested_wrappers_forward_encoder_line_limits_and_root_epilogue_options() {
    let mut leaf = MimePart::new(MimeContent::new(Cursor::new(vec![b'a'; 80])));
    leaf.set_content_transfer_encoding(ContentEncoding::Base64);
    let inner = attachment(leaf.into());
    let mut outer = attachment(inner.into());
    let options = MimePartWriteOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ensure_new_line: true,
    };
    let mut output = Vec::new();
    outer.write_to_with_options(&mut output, &options).unwrap();
    let start = output
        .windows(2)
        .rposition(|bytes| bytes == b"\n\n")
        .unwrap()
        + 2;
    assert_eq!(
        output[start..]
            .split(|&byte| byte == b'\n')
            .map(|line| line.len())
            .collect::<Vec<_>>(),
        [60, 48, 0]
    );
    let mut multipart: Multipart = Multipart::new("mixed", "inner").unwrap();
    multipart.set_epilogue(b"tail".to_vec());
    let mut part = attachment(attachment(multipart.into()).into());
    assert!(wire(&mut part, NewLineFormat::Unix, true).ends_with(b"--inner--\ntail\n"));
    assert!(wire(&mut part, NewLineFormat::Unix, false).ends_with(b"--inner--\ntail"));
}

#[test]
fn multipart_children_do_not_force_newlines_inside_attached_messages() {
    let child = attachment(TextPart::new("plain", "hello").unwrap().into());
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    outer.children_mut().push(child.into());
    outer.set_epilogue(b"tail".to_vec());
    let mut output = Vec::new();
    outer
        .write_to_with_options(
            &mut output,
            &MimePartWriteOptions {
                new_line_format: NewLineFormat::Unix,
                ensure_new_line: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        output,
        concat!(
            "Content-Type: multipart/mixed; boundary=outer\n\n--outer\n",
            "Content-Type: message/rfc822\n\nSubject: attached\n",
            "Content-Type: text/plain; charset=utf-8\n\nhello\n--outer--\ntail\n",
        )
        .as_bytes()
    );
}

#[test]
fn binary_suffixes_survive_parent_framing_across_message_wrappers() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let separator = if format == NewLineFormat::Dos {
            b"\r\n\r\n".as_slice()
        } else {
            b"\n\n"
        };
        for suffix in [
            b"".as_slice(),
            b"\r",
            b"\n",
            b"\r\n",
            b"\r\n\r\n",
            b"\0\xff",
        ] {
            let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
            let mut child = attachment(attachment(leaf(suffix)).into());
            let expected_child_body = wire(&mut child, format, false);
            let body_start = expected_child_body
                .windows(separator.len())
                .position(|bytes| bytes == separator)
                .unwrap()
                + separator.len();
            outer.children_mut().push(child.into());
            let mut sink = Sink::default();
            outer
                .write_to_with_options(
                    &mut sink,
                    &MimePartWriteOptions {
                        new_line_format: format,
                        ensure_new_line: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            let parsed = MimeEntity::load_with_options(
                &mut sink.bytes.as_slice(),
                &MimeEntityParseOptions {
                    parse_message_parts: false,
                    ..Default::default()
                },
            )
            .unwrap();
            let loaded = parsed.leaf_parts().next().unwrap();
            assert_eq!(
                loaded.content().get_ref().get_ref(),
                &expected_child_body[body_start..]
            );
            assert!(loaded.content().get_ref().get_ref().ends_with(suffix));
        }
    }
}

#[test]
fn ancestor_boundaries_remain_active_across_multiple_message_wrappers() {
    for (outer_boundary, inner_boundary) in [
        ("same", "same"),
        ("prefix", "prefix-long"),
        ("prefix-long", "prefix"),
    ] {
        let inner: Multipart = Multipart::new("mixed", inner_boundary).unwrap();
        let child = attachment(attachment(inner.into()).into());
        let mut outer: Multipart = Multipart::new("mixed", outer_boundary).unwrap();
        outer.children_mut().push(child.into());
        let mut output = Vec::new();
        assert_eq!(
            outer.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.ends_with(format!(
            "Subject: attached\r\nContent-Type: multipart/mixed; boundary={inner_boundary}\r\n\r\n"
        ).as_bytes()));
        assert_eq!(
            output
                .windows(b"Content-Type: multipart/".len())
                .filter(|bytes| *bytes == b"Content-Type: multipart/")
                .count(),
            2
        );
        // The merged header section is already written, but validation still
        // rejects overlapping boundaries before any inner delimiter is emitted.
        assert!(
            !output
                .windows(format!("--{inner_boundary}--").len())
                .any(|bytes| bytes == format!("--{inner_boundary}--").as_bytes())
        );
    }
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    for _ in 0..2 {
        let sibling: Multipart = Multipart::new("mixed", "reused").unwrap();
        outer.children_mut().push(attachment(sibling.into()).into());
    }
    let mut output = Vec::new();
    outer.write_to(&mut output).unwrap();
    assert_eq!(
        output
            .windows(b"--reused--".len())
            .filter(|bytes| *bytes == b"--reused--")
            .count(),
        2
    );
}

#[test]
fn wrapper_metadata_is_checked_before_its_output_even_without_a_message() {
    for (raw, typed) in [
        (b"".as_slice(), true),
        (b"Content-Type: invalid\r\n", true),
        (b"Content-Type: text/plain\r\n", true),
        (
            b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding: base64\r\n",
            false,
        ),
        (
            b"Content-Type: message/global\r\nContent-Transfer-Encoding: quoted-printable\r\n",
            false,
        ),
        (
            b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding: x-uuencode\r\n",
            false,
        ),
    ] {
        let mut part: MessagePart = MessagePart::from_parts(headers(raw), None);
        let mut sink = Sink::default();
        let error = part.write_to(&mut sink).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        if typed {
            assert!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<MessagePartError>()
                    .is_some()
            );
        }
        assert_eq!(sink.calls + sink.flushes, 0);
    }
    let mut part: MessagePart = MessagePart::new(MimeMessage::new());
    part.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap());
    part.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: 7bit").unwrap());
    let error = part.write_to(&mut Vec::new()).unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<UnknownContentEncoding>()
            .is_some()
    );
}

#[test]
fn transparent_wrapper_encodings_and_truncated_records_are_preserved() {
    for encoding in ["7bit", "8bit", "binary"] {
        let raw = format!("Content-Type: message/global\nContent-Transfer-Encoding: {encoding}");
        let message =
            MimeMessage::from_parts(headers(b"Subject: truncated"), Some(leaf(b"\xff\0\r")));
        let mut part = MessagePart::from_parts(headers(raw.as_bytes()), Some(message));
        let output = wire(&mut part, NewLineFormat::Unix, true);
        assert!(output.starts_with(format!("Content-Type: message/global\nContent-Transfer-Encoding: {encoding}\n\nSubject: truncated\nContent-Type: application/octet-stream\n").as_bytes()));
        assert!(output.ends_with(b"\n\n\xff\0\r"));
        assert!(!part.headers().as_slice()[1].raw().ends_with(b"\n"));
    }
}

#[test]
fn options_and_descendant_errors_have_explicit_output_boundaries() {
    let mut source = Cursor::new(b"body");
    source.set_position(2);
    let mut leaf = MimePart::new(MimeContent::new(&mut source));
    leaf.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap());
    let message = MimeMessage::from_parts(headers(b"Subject: attached\r\n"), Some(leaf.into()));
    let mut part = MessagePart::new(message);
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
            part.write_to_with_options(&mut sink, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
    let mut output = Vec::new();
    assert_eq!(
        part.write_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        output,
        b"Content-Type: message/rfc822\r\n\r\nSubject: attached\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: unknown\r\n\r\n"
    );
    drop(part);
    assert_eq!(source.position(), 2);
}

#[test]
fn short_writes_interrupts_partial_failures_and_flush_errors_propagate() {
    let mut part = attachment(leaf(b"body\0\r"));
    let expected = wire(&mut part, NewLineFormat::Dos, false);
    for limit in 0..expected.len() {
        let mut sink = Sink {
            fail_after: Some(limit),
            ..Default::default()
        };
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        assert_eq!(wire(&mut part, NewLineFormat::Dos, false), expected);
    }
    let mut sink = Sink::default();
    let output: &mut dyn Write = &mut sink;
    part.write_to(output).unwrap();
    assert_eq!(sink.bytes, expected);
    for flush in 1..=sink.flushes {
        let mut sink = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.flushes, flush);
    }
}

#[test]
fn borrowed_source_errors_propagate_through_message_and_multipart_frames() {
    struct FailingSource {
        fail_seek: bool,
    }
    impl Read for FailingSource {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("read"))
        }
    }
    impl Seek for FailingSource {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            if self.fail_seek {
                Err(io::Error::other("seek"))
            } else {
                Ok(0)
            }
        }
    }
    for fail_seek in [false, true] {
        let mut source = FailingSource { fail_seek };
        let body = MimePart::new(MimeContent::new(&mut source));
        let part = MessagePart::new(MimeMessage::with_body(body.into()));
        let mut outer = Multipart::new("mixed", "outer").unwrap();
        outer.children_mut().push(part.into());
        let mut output = Vec::new();
        assert_eq!(
            outer.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert!(output.ends_with(b"Content-Type: application/octet-stream\r\n\r\n"));
    }
}

#[test]
fn deep_message_chains_are_written_and_traversed_without_recursion() {
    let depth = 4000;
    let mut entity = leaf(b"body");
    for _ in 0..depth {
        entity = MessagePart::new(MimeMessage::with_body(entity)).into();
    }
    let leaves = entity.leaf_parts().count();
    let mut output = Vec::new();
    let result = entity.write_to(&mut output);
    // Drop has no explicit stack; dismantle the intentionally deep test tree.
    while let MimeEntity::Message(part) = entity {
        entity = part.into_parts().1.unwrap().into_parts().1.unwrap();
    }
    result.unwrap();
    assert_eq!(leaves, 1);
    assert_eq!(
        output
            .windows(b"Content-Type: message/rfc822\r\n\r\n".len())
            .filter(|bytes| *bytes == b"Content-Type: message/rfc822\r\n\r\n")
            .count(),
        depth
    );
    assert!(output.ends_with(b"\r\n\r\nbody"));
}

#[test]
fn opaque_option_leaves_wrappers_available_for_explicit_decoding_and_loading() {
    let mut part = attachment(leaf(b"body\0\r"));
    let raw = wire(&mut part, NewLineFormat::Dos, false);
    let MimeEntity::Part(mut loaded) = MimeEntity::load_with_options(
        &mut raw.as_slice(),
        &MimeEntityParseOptions {
            parse_message_parts: false,
            ..Default::default()
        },
    )
    .unwrap() else {
        panic!("opaque parsing was requested");
    };
    let mut decoded = loaded.content_mut().open().unwrap();
    let inner = MimeMessage::load(&mut decoded).unwrap();
    assert!(inner.headers().contains("Subject"));
    assert_eq!(
        inner
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref(),
        b"body\0\r"
    );
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

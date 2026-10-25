// Adapted from MimeKit's MimeAnonymizer.cs traversal and framing rules.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, HeaderList, MessagePart, MimeContent, MimeEntity, MimeMessage, MimePart,
    Multipart, TextPart,
    anonymize::{self, WriteOptions},
    encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn leaf(raw: &[u8]) -> MimeEntity {
    MimePart::from_parts(
        headers(b"Content-Type: text/plain\n"),
        MimeContent::new(Cursor::new(raw.to_vec())),
    )
    .into()
}

fn fixture() -> MimeMessage {
    MimeMessage::load(
        &mut concat!(
            "Subject: outer\nContent-Type: multipart/mixed; boundary=outer\nX-Keep: keep\n\n",
            "Private pre\n--outer\n",
            "Content-Type: message/rfc822\nX-Keep: wrapper\n\n",
            "From: Alice <a@b>\nContent-Type: text/plain\nSubject: inner\n\n",
            "Cat\n--outer\n",
            "Content-Type: message/global-delivery-status\nContent-Transfer-Encoding: base64\n\n",
            "QWN0aW9uOiBmYWlsZWQNCg0K\n--outer\n",
            "Content-Type: message/global-disposition-notification\n\n",
            "Original-Message-ID: <abc@x>\nX-Keep: report\n\nignored\n",
            "--outer--\nPrivate tail",
        )
        .as_bytes(),
    )
    .unwrap()
}

fn expected_fixture(format: NewLineFormat, ensure: bool) -> Vec<u8> {
    let mut expected = concat!(
        "Subject: xxxxx\nContent-Type: multipart/mixed; boundary=outer\nX-Keep: keep\n\n",
        "xxxxxxx xxx\n--outer\n",
        "Content-Type: message/rfc822\nX-Keep: wrapper\n\n",
        "From: xxxxx <x@x>\nContent-Type: text/plain\nSubject: xxxxx\n\n",
        "xxx\n--outer\n",
        "Content-Type: message/global-delivery-status\nContent-Transfer-Encoding: base64\n\n",
        "Action: failed\n\n--outer\n",
        "Content-Type: message/global-disposition-notification\n\n",
        "Original-Message-ID: <xxx@x>\nX-Keep: report\n\n",
        "--outer--\nxxxxxxx xxxx",
    )
    .to_owned();
    if ensure {
        expected.push('\n');
    }
    if format == NewLineFormat::Dos {
        expected = expected.replace('\n', "\r\n");
    }
    expected.into_bytes()
}

fn fixture_options(format: NewLineFormat, ensure: bool) -> WriteOptions {
    let mut options = WriteOptions::default();
    options.part.new_line_format = format;
    options.part.ensure_new_line = ensure;
    options.headers.preserve_headers = vec!["x-keep".into(), "action".into()];
    options
}

#[test]
fn complete_messages_interleave_nested_headers_and_dispatch_reports() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for ensure in [false, true] {
            let mut message = fixture();
            let mut before = Vec::new();
            message.write_to(&mut before).unwrap();
            let expected = expected_fixture(format, ensure);
            let options = fixture_options(format, ensure);
            for _ in 0..2 {
                let mut output = Vec::new();
                anonymize::write_message_with_options(
                    &mut message,
                    &mut output as &mut dyn Write,
                    &options,
                )
                .unwrap();
                assert_eq!(output, expected);
                assert!(
                    message.body().unwrap().leaf_parts().all(|part| part
                        .content()
                        .get_ref()
                        .position()
                        == 0)
                );
            }
            let mut after = Vec::new();
            message.write_to(&mut after).unwrap();
            assert_eq!(after, before);
        }
    }
}

#[test]
fn entity_content_omits_only_the_root_headers() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut entity = fixture().into_parts().1.unwrap();
        let options = fixture_options(format, false);
        let mut complete = Vec::new();
        anonymize::write_entity_with_options(&mut entity, &mut complete, &options).unwrap();
        let newline = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let header = format!("Content-Type: multipart/mixed; boundary=outer{newline}{newline}");
        let message_prefix = format!(
            "Subject: xxxxx{newline}Content-Type: multipart/mixed; boundary=outer{newline}X-Keep: keep{newline}{newline}"
        );
        let expected = expected_fixture(format, false);
        assert_eq!(
            complete,
            [header.as_bytes(), &expected[message_prefix.len()..]].concat()
        );
        let mut content = Vec::new();
        anonymize::write_entity_content_with_options(&mut entity, &mut content, &options).unwrap();
        assert_eq!(content, complete[header.len()..]);
    }
}

#[test]
fn empty_messages_wrappers_and_multiparts_keep_distinct_framing() {
    let mut message: MimeMessage = MimeMessage::new();
    let mut output = Vec::new();
    anonymize::write_message(&mut message, &mut output).unwrap();
    assert_eq!(output, b"\r\n");

    for present in [false, true] {
        let mut wrapper: MessagePart = MessagePart::new(MimeMessage::new());
        if !present {
            wrapper.set_message(None);
        }
        let mut entity = wrapper.into();
        output.clear();
        anonymize::write_entity_content(&mut entity, &mut output).unwrap();
        assert_eq!(output, if present { b"\r\n".as_slice() } else { b"" });
        output.clear();
        anonymize::write_entity(&mut entity, &mut output).unwrap();
        assert_eq!(
            output,
            if present {
                b"Content-Type: message/rfc822\r\n\r\n\r\n".as_slice()
            } else {
                b"Content-Type: message/rfc822\r\n\r\n"
            }
        );
    }
    for preamble in [b"".as_slice(), b"private"] {
        let mut multipart: Multipart = Multipart::with_boundary("mixed", "b").unwrap();
        multipart.set_preamble(preamble.to_vec());
        let mut entity = multipart.into();
        output.clear();
        anonymize::write_entity_content(&mut entity, &mut output).unwrap();
        assert_eq!(
            output,
            if preamble.is_empty() {
                b"--b--\r\n".as_slice()
            } else {
                b"xxxxxxx\r\n--b--\r\n"
            }
        );
    }
}

#[test]
fn report_dispatch_uses_stored_encoding_and_checks_exact_per_report_limits() {
    for subtype in [
        "delivery-status",
        "global-delivery-status",
        "disposition-notification",
        "global-disposition-notification",
    ] {
        let mut entity: MimeEntity = MimePart::from_parts(
            headers(
                format!("Content-Type: message/{subtype}\nContent-Transfer-Encoding: unknown\n")
                    .as_bytes(),
            ),
            MimeContent::with_encoding(
                Cursor::new(b"QWN0aW9uOiBmYWlsZWQNCg0K".to_vec()),
                ContentEncoding::Base64,
            ),
        )
        .into();
        let mut options = WriteOptions::default();
        options.part.ensure_new_line = true;
        options.delivery_status.max_decoded_bytes = b"Action: failed\r\n\r\n".len();
        options.disposition_notification.max_decoded_bytes = b"Action: failed\r\n\r\n".len();
        let mut output = Vec::new();
        anonymize::write_entity_with_options(&mut entity, &mut output, &options).unwrap();
        let prefix = format!(
            "Content-Type: message/{subtype}\r\nContent-Transfer-Encoding: unknown\r\n\r\n"
        );
        assert_eq!(output, [prefix.as_bytes(), b"Action: xxxxxx\r\n"].concat());
        options.delivery_status.max_decoded_bytes -= 1;
        options.disposition_notification.max_decoded_bytes -= 1;
        output.clear();
        assert_eq!(
            anonymize::write_entity_with_options(&mut entity, &mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(output, prefix.as_bytes());
    }
}

#[test]
fn office365_remainder_and_notification_trailing_blocks_keep_specialized_rules() {
    let mut status = MimePart::from_parts(
        headers(b"Content-Type: message/delivery-status\n"),
        MimeContent::new(Cursor::new(
            b"Content-Transfer-Encoding: base64\r\n\r\nQWN0aW9uOiBmYWlsZWQNCg0K".to_vec(),
        )),
    )
    .into();
    let mut output = Vec::new();
    anonymize::write_entity_content(&mut status, &mut output).unwrap();
    assert_eq!(
        output,
        b"Content-Transfer-Encoding: base64\r\n\r\nAction: xxxxxx\r\n"
    );

    for raw in [b"\nX: ignored".as_slice(), b"X: a\n\nX: ignored", b"X: a"] {
        let mut notification = MimePart::from_parts(
            headers(b"Content-Type: message/disposition-notification\n"),
            MimeContent::new(Cursor::new(raw.to_vec())),
        )
        .into();
        output.clear();
        let mut options = WriteOptions::default();
        options.part.ensure_new_line = true;
        anonymize::write_entity_content_with_options(&mut notification, &mut output, &options)
            .unwrap();
        assert_eq!(
            output,
            if raw.starts_with(b"\n") {
                b"".as_slice()
            } else if raw.ends_with(b"a") {
                b"X: x".as_slice()
            } else {
                b"X: x\r\n".as_slice()
            }
        );
    }
}

#[test]
fn ordinary_and_opaque_leaves_mask_after_serialization_without_reparsing() {
    for (encoding, expected) in [
        (ContentEncoding::Base64, b"xxxx\r\n".as_slice()),
        // QP appends a soft break; masking also replaces its '=' marker.
        (ContentEncoding::QuotedPrintable, b"xxxx\r\n".as_slice()),
        (ContentEncoding::Binary, b"xxx".as_slice()),
    ] {
        let mut part = MimePart::from_parts(
            headers(b"Content-Type: text/plain\n"),
            MimeContent::new(Cursor::new(b"Cat".to_vec())),
        );
        part.set_content_transfer_encoding(encoding);
        let mut entity = part.into();
        let mut output = Vec::new();
        anonymize::write_entity_content(&mut entity, &mut output).unwrap();
        assert_eq!(output, expected, "{encoding:?}");
    }
    let mut opaque: MimeEntity = MimePart::from_parts(
        headers(b"Content-Type: message/rfc822\n"),
        MimeContent::new(Cursor::new(b"Subject: secret\n\nCat".to_vec())),
    )
    .into();
    let mut output = Vec::new();
    anonymize::write_entity_content(&mut opaque, &mut output).unwrap();
    assert_eq!(output, b"xxxxxxxx xxxxxx\r\n\r\nxxx");
}

#[test]
fn missing_invalid_duplicate_and_unterminated_content_types_are_not_silently_repaired() {
    let mut entity: MimeEntity = MimePart::from_parts(
        HeaderList::new(),
        MimeContent::new(Cursor::new(b"Cat".to_vec())),
    )
    .into();
    let mut output = Vec::new();
    anonymize::write_entity(&mut entity, &mut output).unwrap();
    assert_eq!(output, b"\r\nxxx");
    *entity.headers_mut() = headers(b"Content-Type: text/plain");
    output.clear();
    anonymize::write_entity(&mut entity, &mut output).unwrap();
    assert_eq!(output, b"Content-Type: text/plain\r\nxxx");
    *entity.headers_mut() = headers(b"Content-Type: invalid\nContent-Type: text/plain\n");
    output.clear();
    assert_eq!(
        anonymize::write_entity(&mut entity, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        output,
        b"Content-Type: invalid\r\nContent-Type: text/plain\r\n\r\n"
    );
    output.clear();
    assert_eq!(
        anonymize::write_entity_content(&mut entity, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(output.is_empty());
}

#[test]
fn configured_encoder_width_and_newline_format_reach_ordinary_leaves() {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(vec![b'a'; 80])));
    part.set_content_transfer_encoding(ContentEncoding::Base64);
    let mut entity = part.into();
    let mut options = WriteOptions::default();
    options.part.new_line_format = NewLineFormat::Unix;
    options.part.max_line_length = 60;
    let mut output = Vec::new();
    anonymize::write_entity_content_with_options(&mut entity, &mut output, &options).unwrap();
    assert_eq!(
        output,
        format!("{}\n{}\n", "x".repeat(60), "x".repeat(48)).as_bytes()
    );
}

#[test]
fn invalid_container_metadata_is_checked_after_masked_headers() {
    let mut multipart: Multipart = Multipart::with_boundary("mixed", "b").unwrap();
    multipart
        .headers_mut()
        .replace_all(mimecraft::Header::parse(b"Content-Type: multipart/mixed\n").unwrap())
        .unwrap();
    let mut entity = multipart.into();
    let mut output = Vec::new();
    assert_eq!(
        anonymize::write_entity(&mut entity, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(output, b"Content-Type: multipart/mixed\r\n\r\n");
    output.clear();
    assert_eq!(
        anonymize::write_entity_content(&mut entity, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(output.is_empty());

    let mut wrapper: MessagePart = MessagePart::new(MimeMessage::with_body(leaf(b"private")));
    wrapper
        .headers_mut()
        .add(mimecraft::Header::parse(b"Content-Transfer-Encoding: base64\n").unwrap());
    let mut message = MimeMessage::with_body(wrapper.into());
    output.clear();
    assert_eq!(
        anonymize::write_message(&mut message, &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        output,
        b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding: base64\r\n\r\n"
    );
}

#[test]
fn binary_bare_cr_survives_before_lf_delimiters() {
    let mut part = MimePart::from_parts(
        headers(b"Content-Type: application/octet-stream\nContent-Transfer-Encoding: binary\n"),
        MimeContent::with_encoding(Cursor::new(b"secret\r".to_vec()), ContentEncoding::Binary),
    );
    let mut direct = Vec::new();
    anonymize::write_part_content(&mut part, &mut direct).unwrap();
    assert_eq!(direct, b"xxxxxx\r");
    let mut multipart = Multipart::with_boundary("mixed", "b").unwrap();
    multipart.children_mut().push(part.into());
    let mut entity = multipart.into();
    let mut options = WriteOptions::default();
    options.part.new_line_format = NewLineFormat::Unix;
    let mut output = Vec::new();
    anonymize::write_entity_content_with_options(&mut entity, &mut output, &options).unwrap();
    assert_eq!(output, b"--b\nContent-Type: application/octet-stream\nContent-Transfer-Encoding: binary\n\nxxxxxx\r\r\n--b--\n");
}

#[test]
fn final_newlines_follow_wrappers_but_not_multipart_children() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let nl = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let mut wrapper: MimeEntity = MessagePart::new(MimeMessage::with_body(leaf(b"Cat"))).into();
        let mut options = WriteOptions::default();
        options.part.new_line_format = format;
        options.part.ensure_new_line = true;
        let mut output = Vec::new();
        anonymize::write_entity_content_with_options(&mut wrapper, &mut output, &options).unwrap();
        assert_eq!(
            output,
            format!("Content-Type: text/plain{nl}{nl}xxx{nl}").as_bytes()
        );
        let mut multipart = Multipart::with_boundary("mixed", "b").unwrap();
        multipart.children_mut().push(wrapper);
        multipart.set_epilogue(b"tail".to_vec());
        let mut entity = multipart.into();
        output.clear();
        anonymize::write_entity_content_with_options(&mut entity, &mut output, &options).unwrap();
        assert_eq!(output, format!("--b{nl}Content-Type: message/rfc822{nl}{nl}Content-Type: text/plain{nl}{nl}xxx{nl}--b--{nl}xxxx{nl}").as_bytes());
    }
}

#[test]
fn digest_defaults_and_ancestor_boundaries_survive_message_wrappers() {
    let mut wrapper = MessagePart::new(MimeMessage::with_body(leaf(b"Cat")));
    wrapper.headers_mut().remove_all("Content-Type");
    let mut digest = Multipart::with_boundary("digest", "outer").unwrap();
    digest.children_mut().push(wrapper.into());
    let mut entity = digest.into();
    let mut output = Vec::new();
    anonymize::write_entity(&mut entity, &mut output).unwrap();
    assert_eq!(output, b"Content-Type: multipart/digest; boundary=outer\r\n\r\n--outer\r\n\r\nContent-Type: text/plain\r\n\r\nxxx\r\n--outer--\r\n");

    let inner: MimeEntity = Multipart::with_boundary("mixed", "outer-child")
        .unwrap()
        .into();
    let wrapper = MessagePart::new(MimeMessage::with_body(inner));
    let mut outer = Multipart::with_boundary("mixed", "outer").unwrap();
    outer.children_mut().push(wrapper.into());
    let mut entity = outer.into();
    output.clear();
    let error = anonymize::write_entity_content(&mut entity, &mut output).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(output, b"--outer\r\nContent-Type: message/rfc822\r\n\r\nContent-Type: multipart/mixed; boundary=outer-child\r\n\r\n");
}

#[test]
fn parsed_overlaps_survive_masking_but_structural_edits_still_fail() {
    let raw = concat!(
        "Content-Type: multipart/mixed; boundary=b\n\n--b \t\n",
        "Content-Type: message/rfc822\n\nSubject: private\n",
        "Content-Type: multipart/mixed; boundary=b-long\n\n--b-long\t \n",
        "Content-Type: text/plain\n\nHello\n--b-long--\n\n--b--\n",
    );
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut entity = MimeEntity::load(&mut raw.as_bytes()).unwrap();
        let mut options = WriteOptions::default();
        options.part.new_line_format = format;
        let expected = raw.replace("private", "xxxxxxx").replace("Hello", "xxxxx");
        let expected = if format == NewLineFormat::Dos {
            expected.replace('\n', "\r\n")
        } else {
            expected
        };
        let mut output = Vec::new();
        anonymize::write_entity_with_options(&mut entity, &mut output, &options).unwrap();
        assert_eq!(output, expected.as_bytes());
        output.clear();
        anonymize::write_entity_content_with_options(&mut entity, &mut output, &options).unwrap();
        let separator = if format == NewLineFormat::Dos {
            "\r\n\r\n"
        } else {
            "\n\n"
        };
        let start = expected.find(separator).unwrap() + separator.len();
        assert_eq!(output, &expected.as_bytes()[start..]);
        let MimeEntity::Multipart(root) = &mut entity else {
            unreachable!()
        };
        let _ = root.children_mut();
        assert_eq!(
            anonymize::write_entity_with_options(&mut entity, &mut Vec::new(), &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData,
        );
    }
}

#[test]
fn malformed_report_fields_fail_after_headers_without_opaque_fallback() {
    let mut entity: MimeEntity = MimePart::from_parts(
        headers(b"Content-Type: message/delivery-status\n"),
        MimeContent::new(Cursor::new(
            b"Action: failed\n\nContent-Transfer-Encoding: bogus\n".to_vec(),
        )),
    )
    .into();
    let mut output = Vec::new();
    let error = anonymize::write_entity(&mut entity, &mut output).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(output, b"Content-Type: message/delivery-status\r\n\r\n");
    assert_eq!(
        entity
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        0
    );
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    fail_after: Option<usize>,
    interrupt: bool,
    zero_write: bool,
    flushes: usize,
    fail_flush: Option<usize>,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
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

#[test]
fn tree_write_and_flush_failures_stop_at_exact_prefixes_and_retry() {
    let expected = expected_fixture(NewLineFormat::Dos, false);
    let options = fixture_options(NewLineFormat::Dos, false);
    for limit in 0..expected.len() {
        let mut message = fixture();
        let mut output = Sink {
            fail_after: Some(limit),
            interrupt: true,
            ..Default::default()
        };
        assert_eq!(
            anonymize::write_message_with_options(&mut message, &mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(output.bytes, expected[..limit]);
        let mut retry = Vec::new();
        anonymize::write_message_with_options(&mut message, &mut retry, &options).unwrap();
        assert_eq!(retry, expected);
    }
    let mut success = Sink::default();
    anonymize::write_message_with_options(&mut fixture(), &mut success, &options).unwrap();
    for index in 1..=success.flushes {
        let mut output = Sink {
            fail_flush: Some(index),
            ..Default::default()
        };
        assert_eq!(
            anonymize::write_message_with_options(&mut fixture(), &mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );
        assert!(expected.starts_with(&output.bytes));
        assert_eq!(output.flushes, index);
    }
    let mut output = Sink {
        zero_write: true,
        ..Default::default()
    };
    assert_eq!(
        anonymize::write_message(&mut fixture(), &mut output)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero
    );
    assert!(output.bytes.is_empty());
    assert_eq!(output.flushes, 0);
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_read: bool,
    fail_seek: Option<usize>,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(b"X: a\n".to_vec()),
            reads: 0,
            seeks: 0,
            fail_read: false,
            fail_seek: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        self.bytes.read(bytes)
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn invalid_options_fail_before_any_source_or_destination_io() {
    let mut invalid = Vec::new();
    let mut options = WriteOptions::default();
    options.part.new_line_format = NewLineFormat::Mixed;
    invalid.push(options);
    for length in [59, 999] {
        let mut options = WriteOptions::default();
        options.part.max_line_length = length;
        invalid.push(options);
    }
    for index in 0..4 {
        let mut options = WriteOptions::default();
        match index {
            0 => options.delivery_status.max_decoded_bytes = 0,
            1 => options.delivery_status.max_header_length = 0,
            2 => options.disposition_notification.max_decoded_bytes = 0,
            _ => options.disposition_notification.max_header_length = 0,
        }
        invalid.push(options);
    }
    for options in invalid {
        let mut source = Source::new();
        let mut message = MimeMessage::with_body(
            MimePart::from_parts(
                headers(b"Content-Type: text/plain\n"),
                MimeContent::new(&mut source),
            )
            .into(),
        );
        let mut output = Sink::default();
        assert_eq!(
            anonymize::write_entity_with_options(
                message.body_mut().unwrap(),
                &mut output,
                &options
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            anonymize::write_entity_content_with_options(
                message.body_mut().unwrap(),
                &mut output,
                &options
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            anonymize::write_message_with_options(&mut message, &mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(output.bytes.is_empty());
        assert_eq!(output.flushes, 0);
        assert_eq!(source.reads + source.seeks, 0);
    }
}

#[test]
fn source_errors_preserve_io_kinds_without_finishing_the_tree() {
    for subtype in [
        "text/plain",
        "message/delivery-status",
        "message/disposition-notification",
    ] {
        for (fail_read, fail_seek) in [(true, None), (false, Some(1)), (false, Some(2))] {
            let mut source = Source {
                fail_read,
                fail_seek,
                ..Source::new()
            };
            let mut entity: MimeEntity<_> = MimePart::from_parts(
                headers(format!("Content-Type: {subtype}\n").as_bytes()),
                MimeContent::new(&mut source),
            )
            .into();
            let mut output = Vec::new();
            assert_eq!(
                anonymize::write_entity(&mut entity, &mut output)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Other
            );
            let prefix = format!("Content-Type: {subtype}\r\n\r\n");
            if subtype != "text/plain" || fail_seek != Some(2) {
                assert_eq!(output, prefix.as_bytes());
            } else {
                assert!(output.starts_with(prefix.as_bytes()));
            }
        }
    }
}

#[test]
fn destination_filters_are_not_finalized_by_tree_writers() {
    use mimecraft::{filters::Unix2DosFilter, io::FilteredWriter};
    let mut entity = leaf(b"Cat");
    let mut output = FilteredWriter::new(Vec::new(), Unix2DosFilter::with_ensure_new_line(true));
    anonymize::write_entity(&mut entity, &mut output).unwrap();
    assert_eq!(output.get_ref(), b"Content-Type: text/plain\r\n\r\nxxx");
    output.finish().unwrap();
    assert_eq!(output.get_ref(), b"Content-Type: text/plain\r\n\r\nxxx\r\n");
}

#[test]
fn deep_messages_and_multiparts_remain_iterative() {
    let mut root = leaf(b"Cat");
    for _ in 0..10_000 {
        root = MessagePart::new(MimeMessage::with_body(root)).into();
    }
    let mut output = Vec::new();
    let result = anonymize::write_entity_content(&mut root, &mut output);
    // Serialization is iterative; dismantle the tree before its recursive Drop.
    while let MimeEntity::Message(wrapper) = root {
        root = wrapper.into_parts().1.unwrap().into_parts().1.unwrap();
    }
    result.unwrap();
    let wrapper = b"Content-Type: message/rfc822\r\n\r\n";
    assert_eq!(
        output
            .windows(wrapper.len())
            .filter(|bytes| *bytes == wrapper)
            .count(),
        9_999
    );
    assert!(output.ends_with(b"Content-Type: text/plain\r\n\r\nxxx"));

    let mut root: MimeEntity = TextPart::new("plain", "Cat").unwrap().into();
    for depth in 0..512 {
        let mut multipart = Multipart::with_boundary("mixed", &format!("b{depth:04}-end")).unwrap();
        multipart.children_mut().push(root);
        root = multipart.into();
    }
    output.clear();
    anonymize::write_entity(&mut root, &mut output).unwrap();
    assert!(output.ends_with(b"--b0511-end--\r\n"));
    assert_eq!(
        output.windows(3).filter(|bytes| *bytes == b"xxx").count(),
        1
    );
}

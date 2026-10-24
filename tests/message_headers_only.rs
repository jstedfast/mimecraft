// Adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    Header, HeaderList, HeaderParser, MessagePart, MimeContent, MimeEntity, MimeMessage, MimePart,
    MimePartWriteOptions, Multipart, TextPart, encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("headers-only serialization must not read content")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("headers-only serialization must not seek content")
    }
}

fn leaf(raw: &[u8]) -> MimeEntity<NoIo> {
    MimePart::from_parts(headers(raw), MimeContent::new(NoIo)).into()
}

fn serialize<R>(message: &MimeMessage<R>, format: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    message
        .write_headers_to_with_options(
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
fn writes_message_then_root_body_headers_without_reading_or_mutation() {
    let message = MimeMessage::from_parts(
        headers(b"sUbJeCt: =?utf-8?q?hello?=\n\tworld\nReceived: first\r\nReceived: second\n"),
        Some(leaf(b"Content-Type: text/plain\nX-Body: \xff\n")),
    );
    let before = message.headers().clone();
    let body_before = message.body().unwrap().headers().clone();
    for (format, expected) in [
        (
            NewLineFormat::Dos,
            b"sUbJeCt: =?utf-8?q?hello?=\r\n\tworld\r\nReceived: first\r\nReceived: second\r\nContent-Type: text/plain\r\nX-Body: \xff\r\n\r\n".as_slice(),
        ),
        (
            NewLineFormat::Unix,
            b"sUbJeCt: =?utf-8?q?hello?=\n\tworld\nReceived: first\nReceived: second\nContent-Type: text/plain\nX-Body: \xff\n\n",
        ),
    ] {
        assert_eq!(serialize(&message, format), expected);
        assert_eq!(message.headers(), &before);
        assert_eq!(message.body().unwrap().headers(), &body_before);
    }
    let mut output = Vec::new();
    message.write_headers_to(&mut output).unwrap();
    assert_eq!(output, serialize(&message, NewLineFormat::Dos));
}

#[test]
fn empty_and_bodyless_messages_have_complete_separators_without_source_traits() {
    for (format, newline) in [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")] {
        let message: MimeMessage<()> = MimeMessage::new();
        assert_eq!(serialize(&message, format), newline.as_bytes());
        let message = MimeMessage::with_body(leaf(b""));
        assert_eq!(serialize(&message, format), newline.as_bytes());
        let message: MimeMessage<()> =
            MimeMessage::from_parts(headers(b"Subject: truncated"), None);
        assert_eq!(
            serialize(&message, format),
            format!("Subject: truncated{newline}{newline}").as_bytes()
        );
        let body = Multipart::<()>::from_parts(
            headers(b"Content-Type: multipart/invalid"),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let message = MimeMessage::with_body(body.into());
        assert_eq!(
            serialize(&message, format),
            format!("Content-Type: multipart/invalid{newline}{newline}").as_bytes()
        );
        let wrapper = MessagePart::from_parts(
            headers(b"Content-Type: message/rfc822"),
            Some(MimeMessage::<()>::new()),
        );
        let message = MimeMessage::with_body(wrapper.into());
        assert_eq!(
            serialize(&message, format),
            format!("Content-Type: message/rfc822{newline}{newline}").as_bytes()
        );
    }
}

#[test]
fn each_truncated_record_is_terminated_before_the_next_list_or_separator() {
    let message = MimeMessage::from_parts(
        vec![
            headers(b"Subject: first").get("Subject").unwrap().clone(),
            headers(b"X-Message: second")
                .get("X-Message")
                .unwrap()
                .clone(),
        ]
        .into(),
        Some(leaf(b"Content-Type: text/plain")),
    );
    assert_eq!(
        serialize(&message, NewLineFormat::Dos),
        b"Subject: first\r\nX-Message: second\r\nContent-Type: text/plain\r\n\r\n"
    );
    assert_eq!(
        message.headers().get("Subject").unwrap().raw(),
        b"Subject: first"
    );
}

#[test]
fn multipart_headers_do_not_write_descendants_preambles_or_delimiters() {
    let multipart = Multipart::from_parts(
        headers(b"Content-Type: multipart/mixed; boundary=ignored\n"),
        vec![leaf(b"X-Child: must-not-appear\n")],
        b"preamble must not appear".to_vec(),
        b"epilogue must not appear".to_vec(),
    );
    let message = MimeMessage::with_body(multipart.into());
    assert_eq!(
        serialize(&message, NewLineFormat::Unix),
        b"Content-Type: multipart/mixed; boundary=ignored\n\n"
    );
}

#[test]
fn attached_message_headers_are_not_part_of_the_outer_header_section() {
    let inner = MimeMessage::from_parts(
        headers(b"Subject: inner\nX-Inner: private\n"),
        Some(leaf(b"Content-Type: text/plain\n")),
    );
    let wrapper = MessagePart::from_parts(headers(b"Content-Type: message/rfc822\n"), Some(inner));
    let message = MimeMessage::from_parts(headers(b"Subject: outer\n"), Some(wrapper.into()));
    assert_eq!(
        serialize(&message, NewLineFormat::Unix),
        b"Subject: outer\nContent-Type: message/rfc822\n\n"
    );
}

#[test]
fn invalid_body_metadata_does_not_trigger_content_writer_validation() {
    for body in [
        leaf(b"Content-Type: invalid\nContent-Transfer-Encoding: unknown\n"),
        Multipart::from_parts(
            headers(b"Content-Type: multipart/mixed\nContent-Transfer-Encoding: base64\n"),
            vec![leaf(b"Content-Type: invalid\n")],
            Vec::new(),
            Vec::new(),
        )
        .into(),
        MessagePart::from_parts(
            headers(b"Content-Type: text/plain\nContent-Transfer-Encoding: unknown\n"),
            Some(MimeMessage::with_body(leaf(b"X-Inner: skipped\n"))),
        )
        .into(),
        MessagePart::from_parts(headers(b"Content-Type: message/rfc822\n"), None).into(),
    ] {
        let mut expected = Vec::new();
        body.headers().write_to(&mut expected).unwrap();
        expected.extend_from_slice(b"\r\n");
        let message = MimeMessage::with_body(body);
        assert_eq!(serialize(&message, NewLineFormat::Dos), expected);
    }
}

#[test]
fn bcc_duplicates_and_misplaced_headers_are_not_filtered_or_repartitioned() {
    let message = MimeMessage::from_parts(
        headers(b"Bcc: private@example.com\nResent-Bcc: other@example.com\nSubject: one\nSubject: two\nContent-X: message-list\n"),
        Some(leaf(b"X-Body: body-list\n")),
    );
    assert_eq!(
        serialize(&message, NewLineFormat::Unix),
        b"Bcc: private@example.com\nResent-Bcc: other@example.com\nSubject: one\nSubject: two\nContent-X: message-list\nX-Body: body-list\n\n"
    );
    assert!(!message.headers().contains("MIME-Version"));
    assert!(!message.headers().contains("Date"));
    assert!(!message.headers().contains("Message-Id"));
}

#[test]
fn malformed_records_and_carriage_return_only_lines_keep_header_writer_rules() {
    let message: MimeMessage<()> = MimeMessage::from_parts(
        headers(b"invalid\xff\n\r\r\n\tcontinued\nSubject: a\rb\n"),
        None,
    );
    assert_eq!(
        serialize(&message, NewLineFormat::Unix),
        b"invalid\xff\n\r\r\n\tcontinued\nSubject: a\rb\n\n"
    );
    assert_eq!(
        serialize(&message, NewLineFormat::Dos),
        b"invalid\xff\r\n\r\r\n\tcontinued\r\nSubject: a\rb\r\n\r\n"
    );
}

#[test]
fn borrowed_source_positions_remain_unchanged_and_raw_edits_are_visible() {
    let mut source = Cursor::new(b"content".to_vec());
    source.set_position(4);
    {
        let body = MimePart::from_parts(
            headers(b"Content-Type: text/plain\n"),
            MimeContent::new(&mut source),
        );
        let mut message = MimeMessage::with_body(body.into());
        assert_eq!(
            serialize(&message, NewLineFormat::Unix),
            b"Content-Type: text/plain\n\n"
        );
        message
            .headers_mut()
            .add(Header::parse(b"Subject: now").unwrap());
        assert!(serialize(&message, NewLineFormat::Unix).starts_with(b"Subject: now\n"));
    }
    assert_eq!(source.position(), 4);
    assert_eq!(source.into_inner(), b"content");
}

#[test]
fn output_matches_the_full_serialization_header_prefix_for_each_root_kind() {
    let mut multipart: Multipart = Multipart::new("mixed", "boundary").unwrap();
    multipart
        .children_mut()
        .push(TextPart::new("plain", "child").unwrap().into());
    let attached = MessagePart::new(MimeMessage::with_body(
        TextPart::new("plain", "inner").unwrap().into(),
    ));
    let mut messages = [
        MimeMessage::new(),
        MimeMessage::with_body(TextPart::new("plain", "body").unwrap().into()),
        MimeMessage::with_body(multipart.into()),
        MimeMessage::with_body(attached.into()),
    ];
    for message in &mut messages {
        message.set_subject("outer").unwrap();
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let expected = serialize(message, format);
            let mut full = Vec::new();
            message
                .write_to_with_options(
                    &mut full,
                    &MimePartWriteOptions {
                        new_line_format: format,
                        ..Default::default()
                    },
                )
                .unwrap();
            let separator = if format == NewLineFormat::Dos {
                b"\r\n\r\n".as_slice()
            } else {
                b"\n\n"
            };
            let end = full
                .windows(separator.len())
                .position(|part| part == separator)
                .unwrap()
                + separator.len();
            assert_eq!(expected, full[..end]);
        }
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    limit: Option<usize>,
    flushes: usize,
    fail_flush_at: Option<usize>,
    interrupt_once: bool,
    write_zero: bool,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt_once) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.write_zero {
            return Ok(0);
        }
        let remaining = self
            .limit
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        if remaining == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = buf.len().min(remaining).min(3);
        self.bytes.extend_from_slice(&buf[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush_at == Some(self.flushes) {
            Err(io::ErrorKind::Other.into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn invalid_options_fail_before_any_destination_operations() {
    let message = MimeMessage::from_parts(
        headers(b"Subject: header\n"),
        Some(leaf(b"Content-Type: invalid\n")),
    );
    for options in [
        MimePartWriteOptions {
            max_line_length: 59,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 999,
            ..Default::default()
        },
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
    ] {
        let mut output = Sink::default();
        assert_eq!(
            message
                .write_headers_to_with_options(&mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(output.bytes.is_empty());
        assert_eq!(output.flushes, 0);
    }
}

#[test]
fn valid_body_encoding_options_do_not_refold_headers_or_change_output() {
    let message: MimeMessage<()> = MimeMessage::from_parts(
        headers(format!("Subject: {}\n", "x".repeat(200)).as_bytes()),
        None,
    );
    let expected = serialize(&message, NewLineFormat::Dos);
    for width in [60, 78, 998] {
        for ensure_new_line in [false, true] {
            let mut output = Vec::new();
            message
                .write_headers_to_with_options(
                    &mut output,
                    &MimePartWriteOptions {
                        max_line_length: width,
                        ensure_new_line,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(output, expected);
        }
    }
}

#[test]
fn short_interrupted_zero_writes_and_flush_errors_propagate_correctly() {
    let message = MimeMessage::from_parts(
        headers(b"Subject: outer\n"),
        Some(leaf(b"Content-Type: text/plain\n")),
    );
    let expected = serialize(&message, NewLineFormat::Dos);
    let mut output = Sink {
        interrupt_once: true,
        ..Default::default()
    };
    message
        .write_headers_to(&mut output as &mut dyn Write)
        .unwrap();
    assert_eq!(output.bytes, expected);
    assert!(output.flushes > 0);
    let successful_flushes = output.flushes;

    for limit in 0..expected.len() {
        let mut output = Sink {
            limit: Some(limit),
            ..Default::default()
        };
        assert_eq!(
            message.write_headers_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(output.bytes, expected[..limit]);
    }
    let mut output = Sink {
        write_zero: true,
        ..Default::default()
    };
    assert_eq!(
        message.write_headers_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
    for flush in 1..=successful_flushes {
        let mut output = Sink {
            fail_flush_at: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            message.write_headers_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(output.flushes, flush);
        assert!(expected.starts_with(&output.bytes));
        if flush == successful_flushes {
            assert_eq!(output.bytes, expected);
        }
    }
}

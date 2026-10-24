// Adapted from MimeKit's MimeParserTests.cs; excludes LegacyMimeParser.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeEntity, MimeEntityParseOptions, MimeEvent, MimeMessage, MimeParser, MimePartWriteOptions,
    MimePosition, MimeReader, encodings::NewLineFormat,
};
use std::io::{self, BufReader, Read};

#[test]
fn header_parser() {
    let data = b"Header-1: value 1\r\nHeader-2: value 2\r\nHeader-3: value 3\r\n\r\n";
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, data.as_slice()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 3);
        for index in 1..=3 {
            let name = format!("Header-{index}");
            assert_eq!(
                headers.get(&name).unwrap().raw_value(),
                format!(" value {index}\r\n").as_bytes()
            );
        }
    }
}

#[test]
fn truncated_invalid_header_after_valid_header() {
    for tail in ["Junk", "Junk\r", "Junk\r\n"] {
        let text = format!("Header-1: value 1\r\n{tail}");
        for capacity in [1, 7, 4096] {
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
            let headers = parser.parse_headers().unwrap();
            assert_eq!(headers.len(), 2);
            assert_eq!(
                headers.get("Header-1").unwrap().decode_value().text,
                "value 1"
            );
            let invalid = &headers.as_slice()[1];
            assert!(invalid.is_invalid());
            assert_eq!(invalid.raw(), tail.as_bytes());
            assert_eq!(invalid.offset(), Some(19));
        }
    }
}

#[test]
fn truncated_header() {
    assert_single_header("Header-1: value 1");
}

#[test]
fn single_header_no_terminator() {
    assert_single_header("Header-1: value 1\r\n");
}

fn assert_single_header(text: &str) {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers.get("Header-1").unwrap().decode_value().text,
            "value 1"
        );
        assert_eq!(headers.as_slice()[0].raw(), text.as_bytes());
        assert_eq!(parser.position().offset, text.len() as u64);
        assert!(parser.is_end_of_stream());
    }
}

#[test]
fn empty_headers() {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, b"\r\n".as_slice()));
        assert!(parser.parse_headers().unwrap().is_empty());
        assert_eq!(parser.position().offset, 2);
    }
}

const BARE_CR_HEADERS: &str = concat!(
    "From: <mimekit@example.com>\r\nTo: <mimekit@example.com>\r\n",
    "Subject: Test of headers ending with bare carriage-return\r\n",
);

#[test]
fn headers_end_with_bare_carriage_return() {
    assert_bare_carriage_return_headers("\r", false);
}

#[test]
fn headers_with_bare_carriage_return() {
    assert_bare_carriage_return_headers(
        "\rYou might expect this to be a body, but it's really an invalid header.\r\n",
        true,
    );
}

fn assert_bare_carriage_return_headers(tail: &str, invalid: bool) {
    let text = format!("{BARE_CR_HEADERS}{tail}");
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), if invalid { 4 } else { 3 });
        for (header, (id, value)) in headers.iter().zip([
            (mimecraft::HeaderId::From, "<mimekit@example.com>"),
            (mimecraft::HeaderId::To, "<mimekit@example.com>"),
            (
                mimecraft::HeaderId::Subject,
                "Test of headers ending with bare carriage-return",
            ),
        ]) {
            assert_eq!(header.id(), id);
            assert_eq!(header.decode_value().text, value);
        }
        if invalid {
            let header = &headers.as_slice()[3];
            assert!(header.is_invalid());
            assert_eq!(header.raw(), tail.as_bytes());
        }
        assert_eq!(parser.position().offset, text.len() as u64);
    }
}

#[test]
fn status_group_parser_returns_null_at_end_of_stream() {
    let text = b"Reporting-MTA: dns; mm1\r\n\r\n";
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_slice()));
        let group = parser.parse_status_group().unwrap().unwrap();
        assert_eq!(group.len(), 1);
        assert_eq!(
            group.get("Reporting-MTA").unwrap().decode_value().text,
            "dns; mm1"
        );
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        // Calling again continues to return None rather than throwing.
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

#[test]
fn status_group_parser() {
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n",
    );
}

#[test]
fn status_group_parser_trailing_lone_carriage_return() {
    // A trailing lone CR is not a status group and must not create an empty group.
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r",
    );
}

#[test]
fn status_group_parser_trailing_blank_lines() {
    // Trailing blank lines must not create an empty status group.
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r\n\r\n",
    );
}

#[test]
fn status_group_parser_leading_blank_lines() {
    assert_status_groups(
        "\r\n\r\n\r\nReporting-MTA: dns; mm1\r\n\r\n\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n",
    );
}

#[test]
fn status_group_parser_mixed_new_line_formats() {
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\n\r\n\nFinal-Recipient: rfc822; user@example.com\n",
    );
}

#[test]
fn status_group_parser_read_one_byte_at_a_time() {
    assert_status_groups(
        "\r\n\r\nReporting-MTA: dns; mm1\r\n\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r\n",
    );
}

fn assert_status_groups(text: &str) {
    // One-byte reads force CRLF sequences across buffer boundaries.
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        for (field, value) in [
            ("Reporting-MTA", "dns; mm1"),
            ("Final-Recipient", "rfc822; user@example.com"),
        ] {
            let group = parser.parse_status_group().unwrap().unwrap();
            assert_eq!(group.len(), 1);
            assert_eq!(
                std::str::from_utf8(group.get(field).unwrap().raw_value())
                    .unwrap()
                    .trim(),
                value
            );
        }
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        // Repeated EOF calls keep returning None instead of throwing.
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

#[test]
fn status_group_parser_no_content() {
    assert_no_status_groups("");
}

#[test]
fn status_group_parser_single_blank_line() {
    assert_no_status_groups("\r\n");
}

#[test]
fn status_group_parser_only_blank_lines() {
    assert_no_status_groups("\r\n\r\n\r\n");
}

#[test]
fn status_group_parser_only_blank_lines_lf() {
    assert_no_status_groups("\n\n\n");
}

#[test]
fn status_group_parser_only_lone_carriage_return() {
    assert_no_status_groups("\r");
}

fn assert_no_status_groups(text: &str) {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

const MESSAGE_PART_MBOX_MARKER: &str = "From sender Sat Jan  1 00:00:00 2000";

fn assert_message_part_text_body(message: &MimeMessage, expected: &[u8]) {
    assert_eq!(message.subject().unwrap().text, "inner");
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected text body")
    };
    assert!(part.content_type().unwrap().is_none());
    assert_eq!(part.content().get_ref().get_ref(), expected);
}

fn message_part_offsets(
    text: &str,
    capacity: usize,
    mbox: bool,
) -> Vec<(MimePosition, Option<MimePosition>)> {
    let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
    let mut offsets = Vec::new();
    let mut record = |event: MimeEvent<'_>| {
        let positions = match event {
            MimeEvent::MboxMarker { position, end, .. } => (position, Some(end)),
            MimeEvent::EntityBegin {
                position,
                headers_end,
                ..
            } => (position, Some(headers_end)),
            MimeEvent::MessageBegin { position }
            | MimeEvent::MessageEnd { position }
            | MimeEvent::EntityEnd { position, .. } => (position, None),
            MimeEvent::Content { .. } => return Ok(()),
            MimeEvent::Headers { .. } => panic!("expected message events"),
        };
        for position in std::iter::once(positions.0).chain(positions.1) {
            assert!(position.offset <= text.len() as u64);
            assert_eq!(
                position.line,
                1 + text.as_bytes()[..position.offset as usize]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count() as u64
            );
        }
        offsets.push(positions);
        Ok(())
    };
    if mbox {
        while reader.read_mbox_message(&mut record).unwrap() {}
    } else {
        reader.read_message(&mut record).unwrap();
    }
    offsets
}

fn assert_message_part_short_reads(
    text: &str,
    mbox: bool,
    count: usize,
    serializable: bool,
    mut check: impl FnMut(&MimeMessage, usize, MimePosition),
) {
    let mut expected = None;
    // Owned synchronous parsing does not require seeking. Persistent-source
    // and async variants are not available; exercise the same short reads.
    for capacity in [4096, 7, 2, 1] {
        let mut parser = MimeParser::with_options(
            BufReader::with_capacity(capacity, text.as_bytes()),
            MimeEntityParseOptions {
                max_depth: 16,
                ..Default::default()
            },
        )
        .unwrap();
        let mut serialized = Vec::new();
        for index in 0..count {
            assert!(!parser.is_end_of_stream());
            let mut message = if mbox {
                parser.parse_mbox_message().unwrap().unwrap()
            } else {
                parser.parse_message().unwrap()
            };
            check(&message, index, parser.position());
            if serializable {
                let mut output = Vec::new();
                message
                    .write_to_with_options(
                        &mut output,
                        &MimePartWriteOptions {
                            new_line_format: NewLineFormat::Unix,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                serialized.push(output);
            }
        }
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position().offset, text.len() as u64);
        let actual = (serialized, message_part_offsets(text, capacity, mbox));
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected, "capacity {capacity}");
        } else {
            expected = Some(actual);
        }
    }
}

#[test]
fn message_part_immediate_mbox_boundary() {
    for newline in ["\n", "\r\n"] {
        for marker in [
            "From invalid\0preheader",
            "From sender Sat Jan  1 00:00:01 2000",
        ] {
            let prefix = format!(
                "{MESSAGE_PART_MBOX_MARKER}{newline}Content-Type: message/rfc822{newline}{newline}"
            );
            let text =
                format!("{prefix}{marker}{newline}Subject: inner{newline}{newline}body{newline}");
            assert_message_part_short_reads(&text, true, 2, true, |message, index, position| {
                if index == 0 {
                    let MimeEntity::Message(part) = message.body().unwrap() else {
                        panic!("expected message wrapper")
                    };
                    assert!(part.message().is_none());
                    // The logical entity end is at the marker; mimecraft's
                    // consumed position also includes its five-byte lookahead.
                    assert_eq!(position.offset, prefix.len() as u64 + 5);
                } else {
                    assert_message_part_text_body(message, format!("body{newline}").as_bytes());
                }
            });
        }
    }
}

#[test]
fn message_part_immediate_multipart_boundary() {
    // Short reads must not leave the boundary probe with only part of a marker.
    // Preserve the preceding newline so empty-wrapper end offsets stay correct.
    for newline in ["\n", "\r\n"] {
        for closing in [false, true] {
            for length in [1, 140] {
                let boundary = "b".repeat(length);
                let mut text = format!(
                    "Content-Type: multipart/mixed; boundary=\"{boundary}\"{newline}{newline}--{boundary}{newline}Content-Type: message/rfc822{newline}{newline}"
                );
                if !closing {
                    text.push_str(&format!("--{boundary}{newline}Content-Type: text/plain{newline}{newline}sibling{newline}"));
                }
                text.push_str(&format!("--{boundary}--{newline}"));
                // Recovery accepts long boundaries, but mimecraft's writer
                // intentionally rejects them. Only valid boundaries round-trip.
                assert_message_part_short_reads(&text, false, 1, length <= 70, |message, _, _| {
                    let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                        panic!("expected multipart")
                    };
                    assert_eq!(multipart.children().len(), if closing { 1 } else { 2 });
                    let MimeEntity::Message(part) = &multipart.children()[0] else {
                        panic!("expected message wrapper")
                    };
                    assert!(part.message().is_none());
                    if !closing {
                        let MimeEntity::Part(part) = &multipart.children()[1] else {
                            panic!("expected sibling body")
                        };
                        assert_eq!(part.content().get_ref().get_ref(), b"sibling");
                    }
                });
            }
        }
    }
}

#[test]
fn message_part_at_end_of_stream() {
    for newline in ["\n", "\r\n"] {
        for mbox in [false, true] {
            let mut text = format!("Content-Type: message/rfc822{newline}{newline}");
            if mbox {
                text = format!("{MESSAGE_PART_MBOX_MARKER}{newline}{text}");
            }
            assert_message_part_short_reads(&text, mbox, 1, true, |message, _, _| {
                let MimeEntity::Message(part) = message.body().unwrap() else {
                    panic!("expected message wrapper")
                };
                if mbox {
                    assert!(part.message().is_none());
                } else {
                    let inner = part.message().unwrap();
                    assert!(inner.headers().is_empty());
                    let MimeEntity::Part(body) = inner.body().unwrap() else {
                        panic!("expected empty text body")
                    };
                    assert!(body.content().get_ref().get_ref().is_empty());
                }
            });
        }
    }
}

#[test]
fn message_part_embedded_message_short_reads() {
    for newline in ["\n", "\r\n"] {
        for mbox in [false, true] {
            let mut text = format!(
                "Content-Type: message/rfc822{newline}{newline}Subject: inner{newline}{newline}body{newline}"
            );
            if mbox {
                text = format!("{MESSAGE_PART_MBOX_MARKER}{newline}{text}");
            }
            assert_message_part_short_reads(&text, mbox, 1, true, |message, _, _| {
                let MimeEntity::Message(part) = message.body().unwrap() else {
                    panic!("expected message wrapper")
                };
                assert_message_part_text_body(
                    part.message().unwrap(),
                    format!("body{newline}").as_bytes(),
                );
            });
        }
    }
}

struct ThrowingReadStream<'a> {
    bytes: &'a [u8],
    position: usize,
    limit: usize,
}

impl Read for ThrowingReadStream<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.position >= self.limit {
            return Err(io::Error::other("boundary lookahead exceeded limit"));
        }
        if self.position == self.bytes.len() {
            return Ok(0);
        }
        output[0] = self.bytes[self.position];
        self.position += 1;
        Ok(1)
    }
}

#[test]
fn message_part_boundary_lookahead_is_bounded() {
    let prefix = format!("{MESSAGE_PART_MBOX_MARKER}\nContent-Type: message/rfc822\n\n");
    let text = format!(
        "{prefix}From {}\nSubject: inner\n\nbody\n",
        "x".repeat(10000)
    );
    for capacity in [1, 7, 4096] {
        let source = ThrowingReadStream {
            bytes: text.as_bytes(),
            position: 0,
            limit: prefix.len() + 128,
        };
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, source));
        let message = parser.parse_mbox_message().unwrap().unwrap();
        let MimeEntity::Message(part) = message.body().unwrap() else {
            panic!("expected message wrapper")
        };
        assert!(part.message().is_none());
        assert_eq!(parser.position().offset, prefix.len() as u64 + 5);
        let (source, pending) = parser.into_parts();
        assert_eq!(pending, b"From ");
        assert!(source.get_ref().position <= prefix.len() + 128);
    }
}

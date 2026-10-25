// Adapted from MimeKit's MimeReader mbox scanning and boundary handling.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read};

use mimecraft::{
    MimeEntity, MimeEntityParseOptions, MimeEvent, MimeParser, MimePosition, MimeReader,
    encodings::NewLineFormat,
};

fn check_position(wire: &[u8], position: MimePosition) {
    assert!(position.offset <= wire.len() as u64);
    assert_eq!(
        position.line,
        1 + wire[..position.offset as usize]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u64
    );
}

#[test]
fn fused_mbox_counts_preserve_body_formats_without_marker_framing() {
    let cases = [
        (
            b"x\n".repeat(3000),
            Some(NewLineFormat::Unix),
            b"\r\n".as_slice(),
        ),
        (b"x\r\n".repeat(2000), Some(NewLineFormat::Dos), b"\n"),
        (b"x".repeat(9000), None, b"\r\n"),
    ];
    let mut wire = Vec::new();
    for (body, _, framing) in &cases {
        wire.extend_from_slice(b"From sender\nSubject: scan\n\n");
        wire.extend_from_slice(body);
        wire.extend_from_slice(framing);
    }
    wire.extend_from_slice(b"From last\nSubject: eof\n\nlast\r\n");
    for capacity in [1, 2, 3, 15, 16, 17, 31, 32, 33, 4095, 4096, 4097, 8192] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        for (expected, format, _) in &cases {
            let mut body = Vec::new();
            let mut hint = None;
            assert!(
                reader
                    .read_mbox_message(|event| {
                        match event {
                            MimeEvent::Content {
                                bytes, position, ..
                            } => {
                                let offset = position.offset as usize;
                                assert_eq!(&wire[offset..offset + bytes.len()], bytes);
                                body.extend_from_slice(bytes);
                            }
                            MimeEvent::EntityEnd {
                                new_line_format, ..
                            } => hint = new_line_format,
                            _ => {}
                        }
                        Ok(())
                    })
                    .unwrap()
            );
            assert_eq!(&body, expected, "capacity {capacity}");
            assert_eq!(hint, *format, "capacity {capacity}");
            assert_eq!(reader.pending_input(), b"From ");
        }
        let mut last = Vec::new();
        assert!(
            reader
                .read_mbox_message(|event| {
                    match event {
                        MimeEvent::Content { bytes, .. } => last.extend_from_slice(bytes),
                        MimeEvent::EntityEnd {
                            new_line_format, ..
                        } => {
                            assert_eq!(new_line_format, Some(NewLineFormat::Dos));
                        }
                        _ => {}
                    }
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(last, b"last\r\n");
        assert!(reader.is_end_of_stream());
    }
}

#[test]
fn mbox_messages_preserve_markers_offsets_body_bytes_and_next_prefix() {
    let first = b"ignored preface\r\nFrom first\r\nSubject: one\r\n\r\nbody\r\n>From escaped\r\nxFrom midline\r\n";
    let second = b"From second\nSubject: two\n\nlast\n";
    let wire = [first.as_slice(), second.as_slice()].concat();
    for capacity in [1, 2, 3, 4, 5, 7, 31, 4096, 65536] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        for (index, expected) in [
            b"body\r\n>From escaped\r\nxFrom midline".as_slice(),
            b"last\n",
        ]
        .into_iter()
        .enumerate()
        {
            let mut body = Vec::new();
            let mut markers = 0;
            let mut messages = 0;
            assert!(
                reader
                    .read_mbox_message(|event| {
                        match event {
                            MimeEvent::MboxMarker {
                                bytes,
                                position,
                                end,
                            } => {
                                assert_eq!(
                                    bytes,
                                    if index == 0 {
                                        b"From first\r\n".as_slice()
                                    } else {
                                        b"From second\n"
                                    }
                                );
                                assert_eq!(
                                    &wire[position.offset as usize..end.offset as usize],
                                    bytes
                                );
                                check_position(&wire, position);
                                check_position(&wire, end);
                                markers += 1;
                            }
                            MimeEvent::Content {
                                bytes, position, ..
                            } => {
                                check_position(&wire, position);
                                assert_eq!(
                                    &wire[position.offset as usize
                                        ..position.offset as usize + bytes.len()],
                                    bytes
                                );
                                assert!(!bytes.is_empty() && bytes.len() <= 4096);
                                body.extend_from_slice(bytes);
                            }
                            MimeEvent::MessageBegin { position } => {
                                check_position(&wire, position);
                                messages += 1;
                            }
                            MimeEvent::MessageEnd { position } => {
                                check_position(&wire, position);
                                assert_eq!(
                                    position.offset as usize,
                                    if index == 0 {
                                        first.len() - 2
                                    } else {
                                        wire.len()
                                    }
                                );
                            }
                            MimeEvent::EntityBegin {
                                headers,
                                position,
                                headers_end,
                                ..
                            } => {
                                check_position(&wire, position);
                                check_position(&wire, headers_end);
                                assert_eq!(
                                    headers.get("Subject").unwrap().offset(),
                                    Some(position.offset)
                                );
                            }
                            MimeEvent::EntityEnd { position, .. } => {
                                check_position(&wire, position)
                            }
                            MimeEvent::Headers { .. } => panic!("not standalone headers"),
                        }
                        Ok(())
                    })
                    .unwrap()
            );
            assert_eq!(markers, 1);
            assert_eq!(messages, 1);
            assert_eq!(body, expected);
            assert_eq!(
                reader.pending_input(),
                if index == 0 { b"From " } else { b"".as_slice() }
            );
            assert_eq!(reader.is_end_of_stream(), index == 1);
        }
        assert!(
            !reader
                .read_mbox_message(|_| panic!("EOF has no events"))
                .unwrap()
        );
    }
}

#[test]
fn mbox_parser_constructs_independent_nested_models_and_replays_lookahead() {
    let first = b"From first\nSubject: first\nContent-Type: multipart/mixed; boundary=b\n\npre\n--b\nContent-Type: message/rfc822\n\nSubject: inner\n\nbody\n--b--\nepilogue\n";
    let second = b"From second\nSubject: second\n\nlast";
    let wire = [first.as_slice(), second.as_slice()].concat();
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, wire.as_slice()));
        let message = parser.parse_mbox_message().unwrap().unwrap();
        let MimeEntity::Multipart(part) = message.body().unwrap() else {
            panic!("expected multipart");
        };
        assert_eq!(part.preamble(), b"pre\n");
        assert_eq!(part.epilogue(), b"epilogue");
        assert_eq!(
            message
                .leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            b"body"
        );
        let (source, pending) = parser.into_parts();
        assert_eq!(pending, b"From ");
        let mut resumed = MimeParser::new(Cursor::new(pending).chain(source));
        let next = resumed.parse_mbox_message().unwrap().unwrap();
        assert_eq!(
            next.headers().get("Subject").unwrap().decode_value().text,
            "second"
        );
        assert!(resumed.parse_mbox_message().unwrap().is_none());
        assert_eq!(
            message
                .headers()
                .get("Subject")
                .unwrap()
                .decode_value()
                .text,
            "first"
        );
    }
}

#[test]
fn mbox_byte_order_mark_preserves_offsets_content_and_limits() {
    let wire = b"\xef\xbb\xbfFrom a\nX: y\n\n\xef\xbb\xbfbody\n";
    for capacity in [1, 2, 3, 7, 4096] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        let mut markers = 0;
        let mut body = Vec::new();
        assert!(
            reader
                .read_mbox_message(|event| {
                    match event {
                        MimeEvent::MboxMarker {
                            bytes, position, ..
                        } => {
                            assert_eq!(bytes, b"From a\n");
                            assert_eq!(position, MimePosition { offset: 3, line: 1 });
                            markers += 1;
                        }
                        MimeEvent::EntityBegin { headers, .. } => {
                            assert_eq!(headers.get("X").unwrap().offset(), Some(10));
                        }
                        MimeEvent::Content {
                            bytes, position, ..
                        } => {
                            check_position(wire, position);
                            assert_eq!(
                                &wire[position.offset as usize
                                    ..position.offset as usize + bytes.len()],
                                bytes
                            );
                            body.extend_from_slice(bytes);
                        }
                        _ => {}
                    }
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(markers, 1);
        assert_eq!(body, b"\xef\xbb\xbfbody\n");
        assert_eq!(reader.position().offset, wire.len() as u64);
        assert!(!reader.read_mbox_message(|_| panic!("EOF")).unwrap());

        for max_input_bytes in [1, 2, 3, wire.len() - 1, wire.len()] {
            let mut parser = MimeParser::with_options(
                BufReader::with_capacity(capacity, wire.as_slice()),
                MimeEntityParseOptions {
                    max_input_bytes,
                    max_header_length: 7,
                    ..Default::default()
                },
            )
            .unwrap();
            let result = parser.parse_mbox_message();
            if max_input_bytes == wire.len() {
                assert!(result.unwrap().is_some());
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                assert!(!parser.is_end_of_stream());
            }
        }
    }
}

#[test]
fn mbox_byte_order_mark_eof_errors_interruptions_and_replacement() {
    for input in [
        b"".as_slice(),
        b"\xef",
        b"\xef\xbb",
        b"\xef\xbb\xbf",
        b"\xefFrom a\n\nbody",
        b"\xef\xbbFrom a\n\nbody",
        b"\xef\xbb\xbfFrom a\n\nbody",
    ] {
        for fail in [false, true] {
            let mut parser = MimeParser::new(Interrupting {
                input,
                interrupt: false,
                fail,
                eof_seen: false,
            });
            let result = parser.parse_mbox_message();
            if input == b"\xefFrom a\n\nbody" || input == b"\xef\xbbFrom a\n\nbody" {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            } else if fail {
                assert_eq!(result.unwrap_err().to_string(), "source");
                assert!(!parser.is_end_of_stream());
            } else if input == b"\xef" || input == b"\xef\xbb" {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
            } else {
                assert_eq!(result.unwrap().is_some(), input.len() > 3);
                assert!(parser.is_end_of_stream());
                assert!(parser.parse_mbox_message().unwrap().is_none());
            }
            if !parser.is_end_of_stream() {
                assert_eq!(
                    parser.parse_mbox_message().unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
            parser.replace_source(Interrupting {
                input: b"\xef\xbb\xbfFrom b\n\nreplacement",
                interrupt: false,
                fail: false,
                eof_seen: false,
            });
            assert!(parser.parse_mbox_message().unwrap().is_some());
            assert!(parser.parse_mbox_message().unwrap().is_none());
        }
    }
}

#[test]
fn mbox_limits_reset_per_message_and_do_not_charge_the_next_marker() {
    let message = b"From a\nX: y\n\nbody\n";
    let wire = message.repeat(4);
    for capacity in [1, 3, 4096] {
        let options = MimeEntityParseOptions {
            max_input_bytes: message.len(),
            max_header_length: 7,
            ..Default::default()
        };
        let mut parser =
            MimeParser::with_options(BufReader::with_capacity(capacity, wire.as_slice()), options)
                .unwrap();
        for _ in 0..4 {
            assert!(parser.parse_mbox_message().unwrap().is_some());
        }
        assert!(parser.parse_mbox_message().unwrap().is_none());
        for max_input_bytes in [1, 5, 6, message.len() - 1] {
            let options = MimeEntityParseOptions {
                max_input_bytes,
                ..options
            };
            let mut parser = MimeParser::with_options(
                BufReader::with_capacity(capacity, wire.as_slice()),
                options,
            )
            .unwrap();
            assert_eq!(
                parser.parse_mbox_message().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert!(!parser.is_end_of_stream());
        }
        // A partial or mismatching prefix beyond the budget is not EOF or a marker.
        for suffix in [b"F".as_slice(), b"From", b"Frog\n", b"x"] {
            let wire = [message.as_slice(), suffix].concat();
            let mut parser = MimeParser::with_options(
                BufReader::with_capacity(capacity, wire.as_slice()),
                options,
            )
            .unwrap();
            assert_eq!(
                parser.parse_mbox_message().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}

#[test]
fn mbox_recovers_truncated_messages_but_requires_valid_envelope_markers() {
    for wire in [b"no markers\n".as_slice(), b"From ", b"From a\n"] {
        let mut parser = MimeParser::with_options(
            wire,
            MimeEntityParseOptions {
                allow_header_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(parser.parse_mbox_message().is_err(), "{wire:?}");
        assert!(!parser.is_end_of_stream());
        assert_eq!(
            parser.parse_mbox_message().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    for wire in [
        b"From a\nX: y\nFrom b\n\n".as_slice(),
        b"From a\nContent-Type: invalid\n\n",
        b"From a\nContent-Type: multipart/mixed; boundary=b\n\n--b\n\nbody\nFrom b\n\n",
    ] {
        let mut parser = MimeParser::new(wire);
        assert!(parser.parse_mbox_message().unwrap().is_some());
        if wire.ends_with(b"From b\n\n") {
            assert!(parser.parse_mbox_message().unwrap().is_some());
        }
        assert!(parser.parse_mbox_message().unwrap().is_none());
        assert!(parser.is_end_of_stream());
    }
    let wire = b"From a\nX: y";
    let mut parser = MimeParser::with_options(
        wire.as_slice(),
        MimeEntityParseOptions {
            allow_header_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(parser.parse_mbox_message().unwrap().is_some());
    assert!(parser.is_end_of_stream());
    let options = MimeEntityParseOptions {
        max_header_length: 6,
        ..Default::default()
    };
    let mut parser = MimeParser::with_options(b"From a\n\n".as_slice(), options).unwrap();
    assert_eq!(
        parser.parse_mbox_message().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn mbox_selection_and_source_replacement_preserve_failure_guards() {
    let wire = b"From a\n\nbody\nFrom b\n\nlast".as_slice();
    let mut reader = MimeReader::new(wire);
    assert!(reader.read_mbox_message(|_| Ok(())).unwrap());
    assert_eq!(
        reader.read_message(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        reader.read_headers(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        reader.read_status_group(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.pending_input(), b"From ");
    let _old = reader.replace_source(b"");
    assert!(reader.pending_input().is_empty());
    assert_eq!(reader.position(), MimePosition::default());
    assert!(
        !reader
            .read_mbox_message(|_| panic!("empty archive"))
            .unwrap()
    );
    assert!(reader.is_end_of_stream());
    assert!(
        !reader
            .read_mbox_message(|_| panic!("repeated EOF"))
            .unwrap()
    );
    for fail_at in [0, 1, 2, 3, 4, 5] {
        let mut reader = MimeReader::new(b"From a\n\nbody".as_slice());
        let mut count = 0;
        let error = reader
            .read_mbox_message(|_| {
                let current = count;
                count += 1;
                if current == fail_at {
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, "callback"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert!(!reader.is_end_of_stream());
    }
}

#[test]
fn consecutive_markers_recover_empty_messages_with_logging() {
    use mimecraft::MimeComplianceViolation as Violation;
    for newline in ["\n", "\r\n"] {
        let marker = format!("From -{newline}");
        let text = format!(
            "{marker}{marker}From b@example.com{newline}Subject: second{newline}{newline}body{newline}"
        );
        for capacity in [1, 2, 3, 7, 4096] {
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
            for index in 1..=2 {
                let mut issues = Vec::new();
                let message = parser
                    .parse_mbox_message_with_logger(&mut |issue| {
                        issues.push(issue);
                        Ok(())
                    })
                    .unwrap()
                    .unwrap();
                assert!(message.headers().is_empty());
                let MimeEntity::Part(body) = message.body().unwrap() else {
                    panic!("empty message must have a leaf body")
                };
                assert!(body.content().get_ref().get_ref().is_empty());
                assert_eq!(
                    issues
                        .iter()
                        .map(|issue| issue.violation)
                        .collect::<Vec<_>>(),
                    [
                        Violation::MissingBodySeparator,
                        Violation::MissingDate,
                        Violation::MissingFrom,
                        Violation::MissingMessageId,
                    ]
                );
                for issue in issues {
                    assert_eq!(issue.stream_offset, (marker.len() * index) as u64);
                    assert_eq!(issue.line_number, index as u64 + 1);
                    assert_eq!(issue.column_number, 1);
                    assert_eq!(
                        issue.position_kind,
                        mimecraft::MimeCompliancePositionKind::ElementStart
                    );
                }
            }
            let message = parser.parse_mbox_message().unwrap().unwrap();
            assert_eq!(
                message
                    .headers()
                    .get("Subject")
                    .unwrap()
                    .decode_value()
                    .text,
                "second"
            );
            assert!(parser.parse_mbox_message().unwrap().is_none());

            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
            let mut ended = false;
            let error = reader
                .read_mbox_message_with_logger(
                    &mut |issue: mimecraft::MimeComplianceIssue| {
                        assert_eq!(issue.violation, Violation::MissingBodySeparator);
                        Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "empty message rejected",
                        ))
                    },
                    |event| {
                        ended |= matches!(
                            event,
                            MimeEvent::MessageEnd { .. } | MimeEvent::EntityEnd { .. }
                        );
                        Ok(())
                    },
                )
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(error.to_string(), "empty message rejected");
            assert!(!ended);
            assert_eq!(
                reader
                    .read_mbox_message(|_| panic!("failed reader"))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}

struct Interrupting<'a> {
    input: &'a [u8],
    interrupt: bool,
    fail: bool,
    eof_seen: bool,
}
impl Read for Interrupting<'_> {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("use fill_buf");
    }
}
impl BufRead for Interrupting<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.input.is_empty() && self.fail {
            return Err(io::Error::other("source"));
        }
        if self.input.is_empty() {
            assert!(!self.eof_seen, "do not probe again after observing EOF");
            self.eof_seen = true;
        }
        Ok(&self.input[..self.input.len().min(1)])
    }
    fn consume(&mut self, count: usize) {
        self.input = &self.input[count..];
    }
}

#[test]
fn mbox_retries_interruptions_but_preserves_real_source_failures() {
    for fail in [false, true] {
        let source = Interrupting {
            input: b"From a\n\nbody\nFrom b\n\nlast",
            interrupt: false,
            fail,
            eof_seen: false,
        };
        let mut parser = MimeParser::new(source);
        assert!(parser.parse_mbox_message().unwrap().is_some());
        let result = parser.parse_mbox_message();
        if fail {
            assert_eq!(result.unwrap_err().to_string(), "source");
            assert!(!parser.is_end_of_stream());
        } else {
            assert!(result.unwrap().is_some());
            assert!(parser.is_end_of_stream());
        }
    }
}

#[test]
fn bulk_content_preserves_split_crlf_false_markers_and_nested_boundaries() {
    let mut body = b"A\rB\nC\r\n--not-boundary\nFrog\n>From escaped\r\n".repeat(257);
    body.extend_from_slice(&vec![b'x'; 4095]);
    body.extend_from_slice(b"From midline\r");
    let prefix = b"From a\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\n";
    let suffix = b"\r\n--b--\r\nFrom b\n\nlast";
    let wire = [prefix.as_slice(), &body, suffix.as_slice()].concat();
    for capacity in [1, 2, 3, 4, 5, 7, 31, 63, 64, 65, 4095, 4096, 4097, 8192] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, wire.as_slice()));
        let message = parser.parse_mbox_message().unwrap().unwrap();
        assert_eq!(
            message
                .leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            &body,
            "capacity {capacity}"
        );
        assert!(parser.parse_mbox_message().unwrap().is_some());
        assert!(parser.parse_mbox_message().unwrap().is_none());
    }
}

#[test]
fn large_ordinary_content_is_borrowed_from_the_source_not_copied_into_lines() {
    let body =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/\r\n".repeat(16384);
    let wire = [b"Subject: bulk\r\n\r\n".as_slice(), &body].concat();
    let source_start = wire.as_ptr() as usize;
    let source_end = source_start + wire.len();
    let mut borrowed = 0;
    let mut total = 0;
    MimeReader::new(wire.as_slice())
        .read_message(|event| {
            if let MimeEvent::Content { bytes, .. } = event {
                let start = bytes.as_ptr() as usize;
                if start >= source_start && start + bytes.len() <= source_end {
                    borrowed += bytes.len();
                }
                assert!(!bytes.is_empty() && bytes.len() <= 4096);
                total += bytes.len();
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(total, body.len());
    assert!(
        borrowed > body.len() * 99 / 100,
        "{borrowed}/{total} bytes borrowed"
    );
}

#[test]
fn content_length_uses_the_first_usable_outer_value() {
    for (headers, protects) in [
        ("", false),
        ("Content-Length: 0\n", false),
        ("Content-Length: 1\n", true),
        ("Content-Length:1\n", false),
        ("Content-Length:\t0001 trailing text\n", true),
        ("Content-Length:\n \t1\n", true),
        ("Content-Length: \u{b}1\n", false),
        ("Content-Length: -1\n", false),
        ("Content-Length: +1\n", false),
        ("Content-Length: invalid\n", false),
        ("Content-Length: 2147483648\n", false),
        ("Content-Length: 9999999999999999999999999999\n", false),
        ("Content-Length: \nContent-Length: 1\n", true),
        ("Content-Length: bad\nContent-Length: 1\n", true),
        ("Content-Length: 2147483648\nContent-Length: 1\n", true),
        ("Content-Length: 0\nContent-Length: 1\n", false),
        ("Content-Length: 1\nContent-Length: 0\n", true),
    ] {
        for newline in ["\n", "\r\n"] {
            let protected = "From body\nSubject: inside\n\npayload".replace('\n', newline);
            let text = format!("From first\nSubject: first\n{headers}\nFrom body\nSubject: inside\n\npayload\nFrom next\nSubject: next\n\nlast").replace('\n', newline);
            for capacity in [1, 7, 4096] {
                for respect_content_length in [false, true] {
                    for logged in [false, true] {
                        let mut parser = MimeParser::with_options(
                            BufReader::with_capacity(capacity, text.as_bytes()),
                            MimeEntityParseOptions {
                                respect_content_length,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        let first = if logged {
                            parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
                        } else {
                            parser.parse_mbox_message()
                        }
                        .unwrap()
                        .unwrap();
                        let MimeEntity::Part(part) = first.body().unwrap() else {
                            panic!()
                        };
                        assert_eq!(
                            part.content().get_ref().get_ref(),
                            if respect_content_length && protects {
                                protected.as_bytes()
                            } else {
                                b""
                            },
                            "{headers:?}/{newline:?}/{capacity}/{respect_content_length}/{logged}",
                        );
                        if !(respect_content_length && protects) {
                            let inside = parser.parse_mbox_message().unwrap().unwrap();
                            assert_eq!(inside.subject().unwrap().text, "inside");
                        }
                        let next = parser.parse_mbox_message().unwrap().unwrap();
                        assert_eq!(next.subject().unwrap().text, "next");
                        assert!(parser.parse_mbox_message().unwrap().is_none());
                        assert!(parser.is_end_of_stream());
                    }
                }
            }
        }
    }
}

#[test]
fn content_length_threshold_is_a_line_start_floor_not_a_body_limit() {
    for newline in ["\n", "\r\n"] {
        let body = format!(
            "From protected{newline}{}{newline}From next{newline}Subject: next{newline}{newline}last{newline}",
            "plain\nFrog\n>From escaped\nxFrom midline\n"
                .repeat(150)
                .replace('\n', newline),
        );
        let next = body.find("From next").unwrap();
        for length in [
            0,
            1,
            2,
            3,
            4,
            5,
            4095,
            4096,
            4097,
            next - 1,
            next,
            next + 1,
            body.len(),
            i32::MAX as usize,
        ] {
            let headers = format!("From first{newline}Content-Length: {length}{newline}{newline}");
            let text = format!("{headers}{body}");
            let stop = if length == 0 {
                Some(0)
            } else if length <= next {
                Some(next)
            } else {
                None
            };
            let expected = stop.map_or(body.as_str(), |stop| {
                body[..stop].strip_suffix(newline).unwrap_or(&body[..stop])
            });
            for capacity in [1, 2, 3, 4, 5, 7, 31, 4095, 4096, 4097, 16384] {
                let options = MimeEntityParseOptions {
                    respect_content_length: true,
                    max_input_bytes: stop.map_or(text.len(), |stop| headers.len() + stop),
                    ..Default::default()
                };
                let mut reader = MimeReader::with_options(
                    BufReader::with_capacity(capacity, text.as_bytes()),
                    options,
                )
                .unwrap();
                let mut content = Vec::new();
                reader
                    .read_mbox_message(|event| {
                        if let MimeEvent::Content {
                            bytes, position, ..
                        } = event
                        {
                            check_position(text.as_bytes(), position);
                            assert_eq!(
                                bytes,
                                &text.as_bytes()[position.offset as usize
                                    ..position.offset as usize + bytes.len()]
                            );
                            content.extend_from_slice(bytes);
                        }
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(content, expected.as_bytes(), "{length}/{capacity}");
                assert_eq!(
                    reader.position().offset as usize,
                    stop.map_or(text.len(), |stop| headers.len() + stop + 5)
                );
                assert_eq!(
                    reader.pending_input(),
                    if stop.is_some() {
                        b"From ".as_slice()
                    } else {
                        b""
                    }
                );
                assert_eq!(reader.is_end_of_stream(), stop.is_none());
                let mut parser = MimeParser::with_options(
                    BufReader::with_capacity(capacity, text.as_bytes()),
                    options,
                )
                .unwrap();
                let first = parser.parse_mbox_message().unwrap().unwrap();
                let MimeEntity::Part(part) = first.body().unwrap() else {
                    panic!()
                };
                assert_eq!(part.content().get_ref().get_ref(), expected.as_bytes());
            }
        }
    }
}

#[test]
fn content_length_does_not_disable_mime_boundaries_or_use_nested_lengths() {
    for nested_length in [0, i32::MAX] {
        for newline in ["\n", "\r\n"] {
            let body = format!("From preamble\n--b\nContent-Type: message/rfc822\nContent-Length: {nested_length}\n\nFrom embedded\nSubject: embedded\nContent-Length: {nested_length}\n\nFrom body\npayload\n--b--\nFrom epilogue\n").replace('\n', newline);
            let headers = format!(
                "Content-Type: multipart/mixed; boundary=b{newline}Content-Length: {}{newline}{newline}",
                body.len()
            );
            let first = format!("{headers}{body}");
            let text = format!(
                "From first{newline}{first}{newline}From next{newline}Subject: next{newline}{newline}last"
            );
            for capacity in [1, 7, 4096] {
                let options = MimeEntityParseOptions {
                    respect_content_length: true,
                    ..Default::default()
                };
                let mut parser = MimeParser::with_options(
                    BufReader::with_capacity(capacity, text.as_bytes()),
                    options,
                )
                .unwrap();
                let mut message = parser
                    .parse_mbox_message_with_logger(&mut |_| Ok(()))
                    .unwrap()
                    .unwrap();
                let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                    panic!()
                };
                assert_eq!(multipart.children().len(), 1);
                let MimeEntity::Message(wrapper) = &multipart.children()[0] else {
                    panic!()
                };
                let embedded = wrapper.message().unwrap();
                assert_eq!(embedded.subject().unwrap().text, "embedded");
                let MimeEntity::Part(part) = embedded.body().unwrap() else {
                    panic!()
                };
                assert_eq!(
                    part.content().get_ref().get_ref(),
                    format!("From body{newline}payload").as_bytes()
                );
                let mut output = Vec::new();
                message
                    .write_to_with_options(
                        &mut output,
                        &mimecraft::MimePartWriteOptions {
                            new_line_format: if newline == "\n" {
                                NewLineFormat::Unix
                            } else {
                                NewLineFormat::Dos
                            },
                            ..Default::default()
                        },
                    )
                    .unwrap();
                assert_eq!(output, first.as_bytes());
                assert_eq!(
                    parser
                        .parse_mbox_message()
                        .unwrap()
                        .unwrap()
                        .subject()
                        .unwrap()
                        .text,
                    "next"
                );
                assert!(parser.parse_mbox_message().unwrap().is_none());

                // Content-Length has no effect outside mbox mode.
                let mut standalone = MimeParser::with_options(
                    BufReader::with_capacity(capacity, first.as_bytes()),
                    options,
                )
                .unwrap();
                let mut ordinary =
                    MimeParser::new(BufReader::with_capacity(capacity, first.as_bytes()));
                let mut actual = Vec::new();
                let mut expected = Vec::new();
                standalone
                    .parse_message()
                    .unwrap()
                    .write_to(&mut actual)
                    .unwrap();
                ordinary
                    .parse_message()
                    .unwrap()
                    .write_to(&mut expected)
                    .unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn protected_content_respects_input_limits() {
    let wire = b"From a\nContent-Length: 2147483647\n\nFrom body\n\nbody\n";
    for max_input_bytes in [wire.len() - 1, wire.len()] {
        let mut parser = MimeParser::with_options(
            wire.as_slice(),
            MimeEntityParseOptions {
                respect_content_length: true,
                max_input_bytes,
                ..Default::default()
            },
        )
        .unwrap();
        let result = parser.parse_mbox_message();
        if max_input_bytes < wire.len() {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert!(!parser.is_end_of_stream());
        } else {
            assert!(result.unwrap().is_some());
            assert!(parser.is_end_of_stream());
        }
    }
}

#[test]
fn protected_content_errors_and_source_replacement() {
    use mimecraft::{MimeComplianceIssue, MimeComplianceViolation};

    let wire = b"From first\nContent-Length: 1000\n\nFrom body\n\0content\n";
    for capacity in [1, 7, 4096] {
        for logged in [false, true] {
            let mut reader = MimeReader::with_options(
                BufReader::with_capacity(capacity, wire.as_slice()),
                MimeEntityParseOptions {
                    respect_content_length: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut ended = false;
            let mut callback = |event: MimeEvent<'_>| {
                ended |= matches!(
                    event,
                    MimeEvent::EntityEnd { .. } | MimeEvent::MessageEnd { .. }
                );
                if !logged && matches!(event, MimeEvent::Content { .. }) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "content rejected",
                    ));
                }
                Ok(())
            };
            let error = if logged {
                reader.read_mbox_message_with_logger(
                    &mut |issue: MimeComplianceIssue| {
                        if issue.violation == MimeComplianceViolation::UnexpectedNullBytesInBody {
                            assert_eq!(
                                issue.stream_offset as usize,
                                wire.iter().position(|&b| b == 0).unwrap()
                            );
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "content rejected",
                            ))
                        } else {
                            Ok(())
                        }
                    },
                    &mut callback,
                )
            } else {
                reader.read_mbox_message(&mut callback)
            }
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(error.to_string(), "content rejected");
            assert!(!ended);
            assert!(!reader.is_end_of_stream());
            assert_eq!(
                reader
                    .read_mbox_message(|_| panic!("failed reader"))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            reader.replace_source(BufReader::with_capacity(
                capacity,
                b"From a\n\nfirst\nFrom b\n\nsecond".as_slice(),
            ));
            assert!(reader.read_mbox_message(|_| Ok(())).unwrap());
            assert!(reader.read_mbox_message(|_| Ok(())).unwrap());
            assert!(!reader.read_mbox_message(|_| Ok(())).unwrap());
        }
    }
    for fail in [false, true] {
        let mut parser = MimeParser::with_options(
            Interrupting {
                input: wire,
                interrupt: false,
                fail,
                eof_seen: false,
            },
            MimeEntityParseOptions {
                respect_content_length: true,
                ..Default::default()
            },
        )
        .unwrap();
        let result = parser.parse_mbox_message();
        if fail {
            assert_eq!(result.unwrap_err().to_string(), "source");
            assert_eq!(
                parser.parse_mbox_message().unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        } else {
            assert!(result.unwrap().is_some());
            assert!(parser.is_end_of_stream());
        }
        parser.replace_source(Interrupting {
            input: b"From a\n\nfirst\nFrom b\n\nsecond",
            interrupt: false,
            fail: false,
            eof_seen: false,
        });
        assert!(parser.options().respect_content_length);
        assert!(parser.parse_mbox_message().unwrap().is_some());
        assert!(parser.parse_mbox_message().unwrap().is_some());
        assert!(parser.parse_mbox_message().unwrap().is_none());
    }
}

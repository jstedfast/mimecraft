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

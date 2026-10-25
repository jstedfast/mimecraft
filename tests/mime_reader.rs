// Adapted from MimeKit's MimeReader and MimeParser callback contracts.
// Includes tests adapted from MimeKit's MimeReaderTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::{fs, path::Path};

use mimecraft::{
    ContentEncoding, HeaderTooLong, MimeComplianceIssue,
    MimeCompliancePositionKind as PositionKind, MimeComplianceViolation as Violation,
    MimeContentKind, MimeEntity, MimeEntityKind, MimeEntityParseOptions, MimeEvent, MimeParser,
    MimePosition, MimeReader, encodings::NewLineFormat,
};

fn assert_position(wire: &[u8], position: MimePosition) {
    assert!(position.offset <= wire.len() as u64);
    assert_eq!(
        position.line,
        1 + wire[..position.offset as usize]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count() as u64
    );
}

#[test]
fn nested_events_preserve_raw_positions_content_and_message_structure() {
    let wire = b"Subject: root\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
preamble\r\n--b\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n\
aGVsbG8=\r\n--b\r\nContent-Type: message/rfc822\r\n\r\n\
From: nested\r\nContent-Type: text/plain\r\n\r\ninner\nline\r\n\
--b\r\nContent-Type: message/rfc822\r\n\r\n--b--\r\nepilogue";
    for capacity in [1, 2, 3, 7, 4096] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        let mut order = Vec::new();
        let mut bodies: Vec<Vec<u8>> = Vec::new();
        let mut preamble = Vec::new();
        let mut epilogue = Vec::new();
        let mut hints = Vec::new();
        let mut body_end = 0;
        reader
            .read_message(|event| {
                match event {
                    MimeEvent::Headers { .. } => panic!("message reading emits entity headers"),
                    MimeEvent::MboxMarker { .. } => {
                        panic!("standalone reading has no mbox markers")
                    }
                    MimeEvent::MessageBegin { position } => {
                        assert_position(wire, position);
                        order.push("message+");
                    }
                    MimeEvent::MessageEnd { position } => {
                        assert_position(wire, position);
                        order.push("message-");
                    }
                    MimeEvent::EntityBegin {
                        headers,
                        kind,
                        encoding,
                        position,
                        headers_end,
                    } => {
                        assert_position(wire, position);
                        assert_position(wire, headers_end);
                        assert!(headers_end.offset > position.offset);
                        for header in &headers {
                            let offset = header.offset().unwrap() as usize;
                            assert_eq!(&wire[offset..offset + header.raw().len()], header.raw());
                        }
                        order.push(match kind {
                            MimeEntityKind::Part => {
                                if bodies.is_empty() {
                                    assert_eq!(encoding, ContentEncoding::Base64);
                                }
                                bodies.push(Vec::new());
                                body_end = headers_end.offset;
                                "part+"
                            }
                            MimeEntityKind::Multipart => "multipart+",
                            MimeEntityKind::MessagePart => "wrapper+",
                        });
                    }
                    MimeEvent::EntityEnd {
                        kind,
                        position,
                        new_line_format,
                    } => {
                        assert_position(wire, position);
                        order.push(match kind {
                            MimeEntityKind::Part => {
                                assert_eq!(position.offset, body_end);
                                hints.push(new_line_format);
                                "part-"
                            }
                            MimeEntityKind::Multipart => "multipart-",
                            MimeEntityKind::MessagePart => "wrapper-",
                        });
                    }
                    MimeEvent::Content {
                        kind,
                        bytes,
                        position,
                    } => {
                        assert_position(wire, position);
                        assert!(!bytes.is_empty());
                        assert!(bytes.len() <= 4096);
                        let offset = position.offset as usize;
                        assert_eq!(&wire[offset..offset + bytes.len()], bytes);
                        match kind {
                            MimeContentKind::Body => {
                                bodies.last_mut().unwrap().extend_from_slice(bytes);
                                body_end = position.offset + bytes.len() as u64;
                            }
                            MimeContentKind::Preamble => preamble.extend_from_slice(bytes),
                            MimeContentKind::Epilogue => epilogue.extend_from_slice(bytes),
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            order,
            [
                "message+",
                "multipart+",
                "part+",
                "part-",
                "wrapper+",
                "message+",
                "part+",
                "part-",
                "message-",
                "wrapper-",
                "wrapper+",
                "wrapper-",
                "multipart-",
                "message-",
            ]
        );
        assert_eq!(bodies, [b"aGVsbG8=".to_vec(), b"inner\nline".to_vec()]);
        assert_eq!(preamble, b"preamble\r\n");
        assert_eq!(epilogue, b"epilogue");
        assert_eq!(hints, [None, Some(NewLineFormat::Unix)]);
        assert_eq!(reader.position().offset, wire.len() as u64);
        assert_position(wire, reader.position());
    }
}

fn scan_body(wire: &[u8]) -> (Vec<u8>, Option<NewLineFormat>) {
    let mut output = Vec::new();
    let mut format = None;
    MimeReader::new(BufReader::with_capacity(17, wire))
        .read_entity(|event| {
            match event {
                MimeEvent::Content {
                    kind: MimeContentKind::Body,
                    bytes,
                    position,
                } => {
                    assert_position(wire, position);
                    assert_eq!(
                        &wire[position.offset as usize..position.offset as usize + bytes.len()],
                        bytes
                    );
                    assert!(bytes.len() <= 4096);
                    output.extend_from_slice(bytes);
                }
                MimeEvent::EntityEnd {
                    kind: MimeEntityKind::Part,
                    position,
                    new_line_format,
                } => {
                    assert_position(wire, position);
                    format = new_line_format;
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    (output, format)
}

#[test]
fn fused_content_counts_exclude_delimiter_newlines_across_refills() {
    let cases = [
        (b"x\0\xff".repeat(3000), None, b"\r\n".as_slice()),
        (
            [
                b"x\n".repeat(3000),
                b"From ordinary\n-False\n--not-b".to_vec(),
            ]
            .concat(),
            Some(NewLineFormat::Unix),
            b"\r\n",
        ),
        (
            [b"x\r\n".repeat(2000), b"tail".to_vec()].concat(),
            Some(NewLineFormat::Dos),
            b"\n",
        ),
        (
            b"x\r\nY\nZ\r".repeat(1000),
            Some(NewLineFormat::Mixed),
            b"\r\n",
        ),
    ];
    let mut wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
    for (body, _, framing) in &cases {
        wire.extend_from_slice(b"--b\r\nContent-Type: application/octet-stream\r\n\r\n");
        wire.extend_from_slice(body);
        wire.extend_from_slice(framing);
    }
    wire.extend_from_slice(b"--b--\r\n");
    let lines: Vec<_> = std::iter::once(1u64)
        .chain(wire.iter().scan(1u64, |line, &byte| {
            *line += u64::from(byte == b'\n');
            Some(*line)
        }))
        .collect();
    for capacity in [
        1, 2, 3, 15, 16, 17, 31, 32, 33, 63, 64, 65, 4095, 4096, 4097, 8192,
    ] {
        let mut bodies: Vec<Vec<u8>> = Vec::new();
        let mut hints = Vec::new();
        let mut start = 0;
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        reader
            .read_entity(|event| {
                match event {
                    MimeEvent::EntityBegin {
                        kind: MimeEntityKind::Part,
                        headers_end,
                        ..
                    } => {
                        bodies.push(Vec::new());
                        start = headers_end.offset as usize;
                    }
                    MimeEvent::Content {
                        kind: MimeContentKind::Body,
                        bytes,
                        position,
                    } => {
                        let body = bodies.last_mut().unwrap();
                        assert_eq!(position.offset as usize, start + body.len());
                        assert_eq!(position.line, lines[position.offset as usize]);
                        assert!(!bytes.is_empty() && bytes.len() <= 4096);
                        assert_eq!(&wire[start + body.len()..][..bytes.len()], bytes);
                        body.extend_from_slice(bytes);
                    }
                    MimeEvent::EntityEnd {
                        kind: MimeEntityKind::Part,
                        position,
                        new_line_format,
                    } => {
                        assert_eq!(position.offset as usize, start + cases[hints.len()].0.len());
                        assert_eq!(position.line, lines[position.offset as usize]);
                        hints.push(new_line_format);
                    }
                    _ => {}
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(bodies.len(), cases.len());
        for (index, (body, format, _)) in cases.iter().enumerate() {
            assert_eq!(&bodies[index], body, "capacity {capacity}, part {index}");
            assert_eq!(hints[index], *format, "capacity {capacity}, part {index}");
        }
        assert_eq!(reader.position().offset as usize, wire.len());
        assert_eq!(reader.position().line, *lines.last().unwrap());
    }
}

#[test]
fn long_lines_and_chunk_split_crlf_keep_boundary_owned_bytes_out_of_content() {
    for length in [4094, 4095, 4096, 4097, 8191, 8192] {
        for ending in [b"".as_slice(), b"\r", b"\n", b"\r\n", b"\r\r", b"\r\n\n"] {
            let mut body = vec![b'x'; length];
            body.extend_from_slice(ending);
            // Binary serialization preserves LF as well as bare CR; ordinary
            // text serialization would intentionally normalize stored newlines.
            let mut wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Transfer-Encoding: binary\r\n\r\n"
                .to_vec();
            wire.extend_from_slice(&body);
            wire.extend_from_slice(b"\r\n--b--\r\n");
            let (actual, format) = scan_body(&wire);
            assert_eq!(actual, body, "length {length}, ending {ending:?}");
            let expected = match ending {
                b"\n" => Some(NewLineFormat::Unix),
                b"\r\n" => Some(NewLineFormat::Dos),
                b"\r\n\n" => Some(NewLineFormat::Mixed),
                _ => None,
            };
            assert_eq!(format, expected);
            let mut parsed = MimeParser::new(wire.as_slice()).parse_entity().unwrap();
            let mut rewritten = Vec::new();
            parsed.write_to(&mut rewritten).unwrap();
            assert_eq!(
                rewritten
                    .iter()
                    .zip(&wire)
                    .position(|(actual, expected)| actual != expected),
                None,
                "length {length}, ending {ending:?}, output size {}, input size {}",
                rewritten.len(),
                wire.len(),
            );
            assert_eq!(rewritten.len(), wire.len());
        }
    }
}

#[test]
fn midline_boundary_prefixes_and_ambiguous_padding_are_lossless() {
    for suffix in [b"X\r\n".as_slice(), b"\r\n", b"X"] {
        let mut wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\n".to_vec();
        let mut body = vec![b'x'; 4096];
        body.extend_from_slice(b"--b\r\n--b");
        body.extend_from_slice(&vec![b' '; 20_000]);
        body.extend_from_slice(suffix);
        wire.extend_from_slice(&body);
        if suffix == b"\r\n" {
            // This long padded line really is a delimiter, not body content.
            wire.extend_from_slice(b"\r\nsecond\r\n--b--");
            let (actual, _) = scan_body(&wire);
            let mut expected = vec![b'x'; 4096];
            expected.extend_from_slice(b"--bsecond");
            assert_eq!(actual, expected);
        } else {
            wire.extend_from_slice(b"\r\n--b--");
            assert_eq!(scan_body(&wire).0, body);
        }
    }
}

#[test]
fn long_preambles_and_epilogues_are_streamed_and_preserved() {
    let preamble = vec![b'p'; 50_000];
    let epilogue = vec![b'e'; 50_000];
    let mut wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n".to_vec();
    wire.extend_from_slice(&preamble);
    wire.extend_from_slice(b"\n--b--\r\n");
    wire.extend_from_slice(&epilogue);
    let mut before = Vec::new();
    let mut after = Vec::new();
    MimeReader::new(wire.as_slice())
        .read_entity(|event| {
            if let MimeEvent::Content {
                kind,
                bytes,
                position,
            } = event
            {
                assert_position(&wire, position);
                assert!(bytes.len() <= 4096);
                match kind {
                    MimeContentKind::Preamble => before.extend_from_slice(bytes),
                    MimeContentKind::Epilogue => after.extend_from_slice(bytes),
                    _ => panic!("empty multipart has no leaf content"),
                }
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(&before[..before.len() - 1], preamble);
    assert_eq!(before.last(), Some(&b'\n'));
    assert_eq!(after, epilogue);
}

#[test]
fn callbacks_run_before_a_long_unterminated_body_is_read_to_eof() {
    let source = b"\r\n".as_slice().chain(io::repeat(b'x').take(1_000_000));
    let mut reader = MimeReader::new(BufReader::with_capacity(31, source));
    let mut calls = 0;
    let error = reader
        .read_entity(|event| {
            if let MimeEvent::Content { bytes, .. } = event {
                calls += 1;
                assert_eq!(bytes, &[b'x'; 4096]);
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "callback stopped",
                ));
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "callback stopped");
    assert_eq!(calls, 1);
    assert!(reader.position().offset < 20_000);
    assert_eq!(
        reader
            .read_entity(|_| panic!("must not call again"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.into_inner().fill_buf().unwrap()[0], b'x');
}

#[test]
fn unrelated_padded_lines_stream_without_waiting_for_their_ending() {
    for prefix in [
        "\r\nlead\n--unrelated",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\nlead\n--unrelated",
    ] {
        let source = prefix.as_bytes().chain(io::repeat(b' ').take(1_000_000));
        let mut reader = MimeReader::new(BufReader::with_capacity(31, source));
        let error = reader
            .read_entity(|event| {
                if let MimeEvent::Content { bytes, .. } = event
                    && bytes.starts_with(b"--unrelated")
                {
                    assert!(!bytes.is_empty() && bytes.len() <= 4096);
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stop padding"));
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "stop padding");
        assert!(reader.position().offset < 20_000);
    }
}

#[test]
fn callback_failures_at_every_event_stop_without_later_callbacks() {
    let wire =
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\np\r\n--b\r\n\r\nbody\r\n--b--\r\ne";
    let mut total = 0;
    MimeReader::new(wire.as_slice())
        .read_message(|_| {
            total += 1;
            Ok(())
        })
        .unwrap();
    for fail_at in 0..total {
        let mut calls = 0;
        let mut reader = MimeReader::new(wire.as_slice());
        let error = reader
            .read_message(|_| {
                let current = calls;
                calls += 1;
                if current == fail_at {
                    Err(io::Error::new(io::ErrorKind::BrokenPipe, "event failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(calls, fail_at + 1);
        assert_eq!(
            reader
                .read_message(|_| panic!("used reader"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        if fail_at == 0 {
            assert_eq!(reader.position(), MimePosition::default());
        }
    }
}

#[test]
fn parser_uses_owned_content_and_keeps_input_relative_offsets() {
    let wire = b"skipSubject: value\r\nContent-Type: text/plain\r\n\r\nbody";
    let mut source = Cursor::new(wire.as_slice());
    source.set_position(4);
    let mut parser = MimeParser::new(&mut source);
    assert_eq!(parser.position(), MimePosition::default());
    assert_eq!(parser.get_ref().position(), 4);
    let mut message = parser.parse_message().unwrap();
    assert_eq!(message.headers().get("Subject").unwrap().offset(), Some(0));
    assert!(!message.headers().contains("Content-Type"));
    let Some(MimeEntity::Part(body)) = message.body() else {
        panic!("expected a leaf");
    };
    assert_eq!(body.content().get_ref().get_ref(), b"body");
    assert_eq!(body.content().get_ref().position(), 0);
    assert_eq!(parser.position().offset, wire.len() as u64 - 4);
    assert_eq!(
        parser.parse_entity().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(parser.into_inner().position(), wire.len() as u64);
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, &wire[4..]);
}

struct NoIo;
impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("no I/O")
    }
}
impl BufRead for NoIo {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        panic!("no I/O")
    }
    fn consume(&mut self, _: usize) {
        panic!("no I/O")
    }
}

#[test]
fn option_validation_precedes_io_and_limits_remain_strict() {
    for options in [
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
    ] {
        assert!(
            matches!(MimeReader::with_options(NoIo, options), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
        assert!(
            matches!(MimeParser::with_options(NoIo, options), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
    }
    let wire = b"\r\nbody";
    let options = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        ..Default::default()
    };
    let mut reader = MimeReader::with_options(wire.as_slice(), options).unwrap();
    assert_eq!(*reader.options(), options);
    reader.read_entity(|_| Ok(())).unwrap();
    let options = MimeEntityParseOptions {
        max_input_bytes: wire.len() - 1,
        ..options
    };
    let mut parser = MimeParser::with_options(wire.as_slice(), options).unwrap();
    assert_eq!(*parser.options(), options);
    assert_eq!(
        parser.parse_entity().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(parser.position().offset, options.max_input_bytes as u64);

    let source = b"X: ".as_slice().chain(io::repeat(b'x').take(1_000_000));
    let mut reader = MimeReader::new(BufReader::new(source));
    let error = reader.read_entity(|_| Ok(())).unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<HeaderTooLong>()
            .is_some()
    );
    assert!(reader.position().offset < 30_000);
}

#[test]
fn opaque_message_option_and_failed_parser_lifecycle_are_explicit() {
    let wire = b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nbody";
    let options = MimeEntityParseOptions {
        parse_message_parts: false,
        ..Default::default()
    };
    let mut message_events = 0;
    let mut data = Vec::new();
    MimeReader::with_options(wire.as_slice(), options)
        .unwrap()
        .read_entity(|event| {
            match event {
                MimeEvent::MessageBegin { .. } | MimeEvent::MessageEnd { .. } => {
                    message_events += 1
                }
                MimeEvent::Content { bytes, .. } => data.extend_from_slice(bytes),
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(message_events, 0);
    assert_eq!(data, b"Subject: inner\r\n\r\nbody");
    let mut parser = MimeParser::new(b"invalid first header\r\n".as_slice());
    assert_eq!(
        parser.parse_message().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        parser.parse_message().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

const INNERMOST_CONTENT: &str = "This is the innermost part.";

fn generate_extremely_nested_message(multipart: bool, depth: usize) -> Vec<u8> {
    use std::fmt::Write as _;

    let mut wire = String::from("From: mimekit@example.org\r\n");
    for index in 0..depth {
        if multipart {
            write!(
                wire,
                "Content-Type: multipart/mixed; boundary=\"{index}\"\r\n\r\n--{index}\r\n"
            )
            .unwrap();
        } else {
            wire.push_str("Content-Type: message/rfc822\r\n\r\n");
        }
    }
    wire.push_str("Content-Type: text/plain\r\n\r\n");
    wire.push_str(INNERMOST_CONTENT);
    if multipart {
        for index in (0..depth).rev() {
            write!(wire, "\r\n--{index}--").unwrap();
        }
        wire.push_str("\r\n");
    }
    wire.into_bytes()
}

// MimeReader must not recurse through nested entities: extremely deep
// multipart or message/rfc822 trees can overflow the stack and abort the process
// when the caller raises the depth limit. Do not construct recursive models here.
#[test]
fn extremely_nested_entities() {
    for (multipart, nesting) in [(true, 10_000), (false, 100_000)] {
        let wire = generate_extremely_nested_message(multipart, nesting);
        let options = MimeEntityParseOptions {
            max_depth: nesting + 1,
            ..Default::default()
        };
        let mut reader =
            MimeReader::with_options(BufReader::with_capacity(4096, wire.as_slice()), options)
                .unwrap();
        let mut depth = 0;
        let mut maximum = 0;
        let mut leaves = 0;
        let mut messages = 0;
        let mut content = Vec::new();
        reader
            .read_message(|event| {
                match event {
                    MimeEvent::EntityBegin { kind, .. } => {
                        depth += 1;
                        maximum = maximum.max(depth);
                        if kind == MimeEntityKind::Part {
                            leaves += 1;
                        } else {
                            assert_eq!(
                                kind,
                                if multipart {
                                    MimeEntityKind::Multipart
                                } else {
                                    MimeEntityKind::MessagePart
                                }
                            );
                        }
                    }
                    MimeEvent::EntityEnd { .. } => {
                        assert!(depth > 0);
                        depth -= 1;
                    }
                    MimeEvent::MessageBegin { .. } => messages += 1,
                    MimeEvent::MessageEnd { .. } => {
                        assert!(messages > 0);
                        messages -= 1;
                    }
                    MimeEvent::Content { bytes, .. } => content.extend_from_slice(bytes),
                    _ => panic!("unexpected standalone header or mbox event"),
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(maximum, nesting + 1);
        assert_eq!(depth, 0);
        assert_eq!(messages, 0);
        assert_eq!(leaves, 1);
        assert_eq!(content, INNERMOST_CONTENT.as_bytes());
        assert_eq!(reader.position().offset, wire.len() as u64);
        assert!(reader.is_end_of_stream());
    }
}

#[test]
fn entity_frames_are_released_when_parsing_fails() {
    let recovered = b"From: mimekit@example.org\r\nContent-Type: text/plain\r\n\r\nbody\r\n";
    for multipart in [false, true] {
        let wire = generate_extremely_nested_message(multipart, 20);
        for capacity in [1, 7, 4096] {
            for logged in [false, true] {
                let mut reader =
                    MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
                let mut begins = 0;
                let mut callback = |event: MimeEvent<'_>| {
                    if let MimeEvent::EntityBegin { kind, .. } = event {
                        begins += 1;
                        if kind == MimeEntityKind::Part {
                            return Err(io::Error::other("OnMimePartBegin"));
                        }
                    } else {
                        assert!(matches!(event, MimeEvent::MessageBegin { .. }));
                    }
                    Ok(())
                };
                let error = if logged {
                    reader.read_message_with_logger(&mut |_| Ok(()), &mut callback)
                } else {
                    reader.read_message(&mut callback)
                }
                .unwrap_err();
                assert_eq!(error.to_string(), "OnMimePartBegin");
                assert_eq!(begins, 21);
                assert!(!reader.is_end_of_stream());
                let failed_position = reader.position();
                assert_eq!(
                    reader
                        .read_message(|_| panic!("failed source must not emit events"))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidInput
                );
                assert_eq!(reader.position(), failed_position);

                // MimeKit checks its reusable frame cache for retained types.
                // Rust's frames are operation-local and dropped on error; reuse
                // must start a fresh tree without completing any old frames.
                let old =
                    reader.replace_source(BufReader::with_capacity(capacity, recovered.as_slice()));
                drop(old);
                assert_eq!(reader.position(), MimePosition::default());
                assert!(reader.pending_input().is_empty());
                let mut events = Vec::new();
                let mut content = Vec::new();
                reader
                    .read_message(|event| {
                        match event {
                            MimeEvent::MessageBegin { position } => {
                                assert_eq!(position, MimePosition::default());
                                events.push("message+");
                            }
                            MimeEvent::EntityBegin { kind, .. } => {
                                assert_eq!(kind, MimeEntityKind::Part);
                                events.push("part+");
                            }
                            MimeEvent::Content { bytes, .. } => content.extend_from_slice(bytes),
                            MimeEvent::EntityEnd { .. } => events.push("part-"),
                            MimeEvent::MessageEnd { .. } => events.push("message-"),
                            _ => panic!("unexpected standalone header or mbox event"),
                        }
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(events, ["message+", "part+", "part-", "message-"]);
                assert_eq!(content, b"body\r\n");
                assert_eq!(reader.position().offset, recovered.len() as u64);
                assert!(reader.is_end_of_stream());
            }
        }
    }
}

struct FaultSource<'a> {
    bytes: &'a [u8],
    offset: usize,
    fail_at: Option<usize>,
    interrupt: bool,
}

impl Read for FaultSource<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let count = output.len().min(input.len());
        output[..count].copy_from_slice(&input[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for FaultSource<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_at == Some(self.offset) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source failure",
            ));
        }
        Ok(&self.bytes[self.offset..(self.offset + 1).min(self.bytes.len())])
    }

    fn consume(&mut self, amount: usize) {
        assert!(amount <= 1);
        self.offset += amount;
    }
}

#[test]
fn source_failures_and_interruptions_preserve_consumption_and_error_identity() {
    let wire = b"X: a\r\n\r\nbody";
    for fail_at in 0..=wire.len() {
        let source = FaultSource {
            bytes: wire,
            offset: 0,
            fail_at: Some(fail_at),
            interrupt: false,
        };
        let mut parser = MimeParser::new(source);
        let error = parser.parse_message().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "source failure");
        assert_eq!(parser.position().offset, fail_at as u64);
        assert_position(wire, parser.position());
        assert_eq!(parser.into_inner().offset, fail_at);
    }
    let source = FaultSource {
        bytes: wire,
        offset: 0,
        fail_at: None,
        interrupt: false,
    };
    let mut parser = MimeParser::new(source);
    let mut message = parser.parse_message().unwrap();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, wire);
}

#[test]
fn standalone_headers_preserve_raw_blocks_separators_and_unread_bodies() {
    type HeaderCase<'a> = (&'a [u8], &'a [u8], Option<&'a [u8]>, usize);
    let cases: &[HeaderCase<'_>] = &[
        (b"", b"", None, 0),
        (b"\n", b"body", Some(b"\n"), 0),
        (b"\r\n", b"\r\nbody", Some(b"\r\n"), 0),
        (b"\r", b"", Some(b"\r"), 0),
        (b"X: a", b"", None, 1),
        (b"X: a\r", b"", None, 1),
        (b"X: a\r\n", b"", None, 1),
        (b"X: a\r\n\r", b"", Some(b"\r"), 1),
        (b"X: a\r\n\r\n", b"\0\xffbody\n", Some(b"\r\n"), 1),
        (b"X: a\n\tb\r\nX: c\n\n", b"body", Some(b"\n"), 2),
        (b"bad first\nX: a\nbad last", b"", None, 3),
        (b"X: a\n\rnot a separator", b"", None, 2),
    ];
    for &(block, body, separator, count) in cases {
        let wire = [block, body].concat();
        for capacity in 1..=wire.len().max(1) {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
            let mut events = 0;
            let mut parsed = None;
            reader
                .read_headers(|event| {
                    events += 1;
                    let MimeEvent::Headers {
                        headers,
                        position,
                        headers_end,
                        body_separator,
                    } = event
                    else {
                        panic!("standalone reading only emits headers");
                    };
                    assert_eq!(position, MimePosition::default());
                    assert_eq!(headers_end.offset, block.len() as u64);
                    assert_position(&wire, headers_end);
                    assert_eq!(body_separator, separator);
                    assert_eq!(headers.len(), count);
                    let mut offset = 0;
                    for header in &headers {
                        assert_eq!(header.offset(), Some(offset as u64));
                        assert_eq!(&block[offset..offset + header.raw().len()], header.raw());
                        offset += header.raw().len();
                    }
                    assert_eq!(offset, block.len() - separator.map_or(0, <[u8]>::len));
                    parsed = Some(headers);
                    Ok(())
                })
                .unwrap();
            assert_eq!(events, 1);
            assert_eq!(reader.position().offset, block.len() as u64);
            let mut unread = Vec::new();
            reader.into_inner().read_to_end(&mut unread).unwrap();
            assert_eq!(unread, body);

            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, wire.as_slice()));
            let headers = parser.parse_headers().unwrap();
            assert_eq!(headers, parsed.unwrap());
            assert_eq!(parser.position().offset, block.len() as u64);
            let mut unread = Vec::new();
            parser.into_inner().read_to_end(&mut unread).unwrap();
            assert_eq!(unread, body);
        }
    }
}

#[test]
fn standalone_headers_do_not_interpret_metadata_or_underlying_seek_offsets() {
    // Header-only parsing must not require a multipart body or validate the
    // transfer encoding: no entity is being constructed.
    let block = b"Subject: test\r\nContent-Type: multipart/mixed\r\n\
                  Content-Transfer-Encoding: unknown\r\nContent-Type: invalid\r\n\r\n";
    let wire = [b"skip".as_slice(), block, b"body"].concat();
    for parse_message_parts in [false, true] {
        let mut source = Cursor::new(wire.as_slice());
        source.set_position(4);
        let options = MimeEntityParseOptions {
            max_input_bytes: block.len(),
            max_depth: 1,
            parse_message_parts,
            ..Default::default()
        };
        let mut parser = MimeParser::with_options(&mut source, options).unwrap();
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 4);
        assert_eq!(headers.get_all("Content-Type").count(), 2);
        assert_eq!(headers.get("Subject").unwrap().offset(), Some(0));
        assert_eq!(headers.get("Content-Type").unwrap().offset(), Some(15));
        assert_eq!(parser.position().offset, block.len() as u64);
        assert_eq!(parser.into_inner().position(), 4 + block.len() as u64);
        let mut body = Vec::new();
        source.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"body");
    }
}

#[test]
fn standalone_header_input_limits_distinguish_real_eof_from_exhausted_budget() {
    for block in [b"X: a\n\n".as_slice(), b"X: a\n", b"X: a", b"X: a\n\r"] {
        for limit in 1..=block.len() {
            let options = MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            };
            let mut reader = MimeReader::with_options(block, options).unwrap();
            let mut calls = 0;
            let result = reader.read_headers(|_| {
                calls += 1;
                Ok(())
            });
            if limit == block.len() {
                result.unwrap();
                assert_eq!(calls, 1);
            } else {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                assert_eq!(calls, 0);
            }
            assert_eq!(reader.position().offset, limit as u64);
            assert_eq!(reader.into_inner(), &block[limit..]);
        }
    }
    // Finishing a header-only operation must not inspect the body's first byte,
    // even when the separator exactly exhausts the input budget.
    let wire = b"X: a\n\nbody";
    let source = FaultSource {
        bytes: wire,
        offset: 0,
        fail_at: Some(6),
        interrupt: false,
    };
    let options = MimeEntityParseOptions {
        max_input_bytes: 6,
        ..Default::default()
    };
    let mut parser = MimeParser::with_options(source, options).unwrap();
    assert_eq!(parser.parse_headers().unwrap().len(), 1);
    assert_eq!(parser.position(), MimePosition { offset: 6, line: 3 });
    assert_eq!(parser.into_inner().offset, 6);
}

#[test]
fn standalone_header_record_limits_include_folds_and_preserve_error_causes() {
    let record = b"X: a\r\n\tb\r\n";
    let wire = [record.as_slice(), b"\r\nbody"].concat();
    for capacity in [1, 3, wire.len()] {
        for limit in [record.len() - 1, record.len()] {
            let options = MimeEntityParseOptions {
                max_header_length: limit,
                ..Default::default()
            };
            let source = BufReader::with_capacity(capacity, Cursor::new(wire.as_slice()));
            let mut reader = MimeReader::with_options(source, options).unwrap();
            let mut calls = 0;
            let result = reader.read_headers(|_| {
                calls += 1;
                Ok(())
            });
            if limit == record.len() {
                result.unwrap();
                assert_eq!(calls, 1);
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert!(
                    error
                        .get_ref()
                        .unwrap()
                        .downcast_ref::<HeaderTooLong>()
                        .is_some()
                );
                assert_eq!(calls, 0);
                assert_eq!(
                    reader
                        .read_headers(|_| panic!("used reader"))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidInput
                );
            }
            let consumed = reader.position().offset;
            let source = reader.into_inner();
            assert_eq!(
                consumed,
                source.get_ref().position() - source.buffer().len() as u64
            );
        }
    }
}

#[test]
fn standalone_header_errors_stop_events_and_poison_the_reader() {
    for block in [b"X: a\r\n\r\n".as_slice(), b"X: a\r\n\tb"] {
        let has_separator = block.ends_with(b"\r\n\r\n");
        for fail_at in 0..=block.len() {
            let source = FaultSource {
                bytes: block,
                offset: 0,
                fail_at: Some(fail_at),
                interrupt: false,
            };
            let mut reader = MimeReader::new(source);
            let mut calls = 0;
            let result = reader.read_headers(|_| {
                calls += 1;
                Ok(())
            });
            if has_separator && fail_at == block.len() {
                result.unwrap();
                assert_eq!(calls, 1);
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(error.to_string(), "source failure");
                assert_eq!(calls, 0);
            }
            assert_eq!(reader.position().offset, fail_at as u64);
            assert_position(block, reader.position());
            assert_eq!(
                reader
                    .read_message(|_| panic!("used reader"))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(reader.into_inner().offset, fail_at);
        }
    }
    let mut reader = MimeReader::new(b"X: a\n\nbody".as_slice());
    let error = reader
        .read_headers(|_| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "callback failure",
            ))
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(error.to_string(), "callback failure");
    assert_eq!(reader.position().offset, 6);
    assert_eq!(
        reader
            .read_headers(|_| panic!("used reader"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(reader.into_inner(), b"body");
}

#[test]
fn standalone_headers_share_the_single_operation_parser_lifecycle() {
    for first in 0..3 {
        for second in 0..3 {
            let mut parser = MimeParser::new(b"X: a\r\n\r\nbody".as_slice());
            let run = |parser: &mut MimeParser<&[u8]>, operation| match operation {
                0 => parser.parse_headers().map(|_| ()),
                1 => parser.parse_entity().map(|_| ()),
                _ => parser.parse_message().map(|_| ()),
            };
            run(&mut parser, first).unwrap();
            let position = parser.position();
            assert_eq!(
                run(&mut parser, second).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(parser.position(), position);
        }
    }
    let mut parser = MimeParser::new(FaultSource {
        bytes: b"X: a",
        offset: 0,
        fail_at: Some(2),
        interrupt: false,
    });
    assert_eq!(
        parser.parse_headers().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        parser.parse_entity().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn source_replacement_and_completion_queries_perform_no_io() {
    let options = MimeEntityParseOptions {
        max_input_bytes: 512,
        max_header_length: 128,
        max_depth: 4,
        parse_message_parts: false,
        allow_header_only: false,
    };
    let mut reader = MimeReader::with_options(NoIo, options).unwrap();
    assert!(!reader.is_end_of_stream());
    let error = reader
        .read_message(|_| Err(io::Error::other("stop before reading")))
        .unwrap_err();
    assert_eq!(error.to_string(), "stop before reading");
    assert!(!reader.is_end_of_stream());
    let _old = reader.replace_source(NoIo);
    assert_eq!(reader.position(), MimePosition::default());
    assert_eq!(*reader.options(), options);
    assert!(!reader.is_end_of_stream());
    // A new message-begin event demonstrates that replacement cleared the
    // failed-operation guard without accessing either source.
    assert_eq!(
        reader
            .read_message(|_| Err(io::Error::other("new operation")))
            .unwrap_err()
            .to_string(),
        "new operation"
    );

    let mut parser = MimeParser::with_options(NoIo, options).unwrap();
    assert!(!parser.is_end_of_stream());
    let _old = parser.replace_source(NoIo);
    assert_eq!(parser.position(), MimePosition::default());
    assert_eq!(*parser.options(), options);
    assert!(!parser.is_end_of_stream());
}

#[test]
fn replacing_sources_retains_old_read_ahead_and_resets_the_offset_origin() {
    let first = b"Subject: first\r\n\r\nfirst body";
    let second = b"skipSubject: second\r\n\r\nsecond body";
    let mut parser = MimeParser::new(BufReader::with_capacity(128, Cursor::new(first.as_slice())));
    let headers = parser.parse_headers().unwrap();
    assert_eq!(
        parser.position(),
        MimePosition {
            offset: 18,
            line: 3
        }
    );
    assert!(!parser.is_end_of_stream());

    let mut source = Cursor::new(second.as_slice());
    source.set_position(4);
    let mut old = parser.replace_source(BufReader::with_capacity(128, source));
    assert_eq!(old.buffer(), b"first body");
    assert_eq!(old.get_ref().position(), first.len() as u64);
    let mut body = Vec::new();
    old.read_to_end(&mut body).unwrap();
    assert_eq!(body, b"first body");
    assert_eq!(parser.position(), MimePosition::default());
    assert!(!parser.is_end_of_stream());
    assert_eq!(parser.get_ref().get_ref().position(), 4);

    let mut message = parser.parse_message().unwrap();
    assert_eq!(message.headers().get("Subject").unwrap().offset(), Some(0));
    assert_eq!(parser.position().offset, second.len() as u64 - 4);
    assert!(parser.is_end_of_stream());
    assert_eq!(headers.get("Subject").unwrap().raw_value(), b" first\r\n");
    // The returned model must keep owning its content after another source is
    // installed, rather than retaining a view of the parser's current source.
    let _old = parser.replace_source(BufReader::with_capacity(128, Cursor::new(first.as_slice())));
    assert!(!parser.is_end_of_stream());
    parser.parse_message().unwrap();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, &second[4..]);
}

#[test]
fn source_replacement_preserves_limits_and_message_interpretation_after_failures() {
    let wire = b"Content-Type: message/rfc822\r\n\r\nSubject: opaque\r\n\r\nbody";
    let options = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        max_header_length: 32,
        max_depth: 1,
        parse_message_parts: false,
        allow_header_only: false,
    };
    for invalid in [
        b"invalid first header\r\n\r\n".as_slice(),
        b"X: missing separator",
        b"X: 12345678901234567890123456789012345\r\n\r\n",
    ] {
        let mut parser = MimeParser::with_options(invalid, options).unwrap();
        assert!(parser.parse_entity().is_err());
        assert!(!parser.is_end_of_stream());
        let old_remaining = *parser.get_ref();
        let old = parser.replace_source(wire.as_slice());
        assert_eq!(old, old_remaining);
        assert_eq!(*parser.options(), options);
        assert_eq!(parser.position(), MimePosition::default());
        let entity = parser.parse_entity().unwrap();
        let MimeEntity::Part(part) = entity else {
            panic!("opaque-message option must survive source replacement");
        };
        assert_eq!(
            part.headers().get("Content-Type").unwrap().offset(),
            Some(0)
        );
        assert_eq!(
            part.content().get_ref().get_ref(),
            b"Subject: opaque\r\n\r\nbody"
        );
        assert!(parser.is_end_of_stream());

        let oversized = [wire.as_slice(), b"x"].concat();
        let _old = parser.replace_source(oversized.as_slice());
        assert_eq!(
            parser.parse_entity().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(parser.position().offset, options.max_input_bytes as u64);
        assert!(!parser.is_end_of_stream());
        let _old = parser.replace_source(wire.as_slice());
        parser.parse_entity().unwrap();
        assert_eq!(parser.position().offset, wire.len() as u64);
        assert!(parser.is_end_of_stream());
    }
}

#[test]
fn source_replacement_recovers_after_callback_and_source_errors() {
    let wire = b"X: a\r\n\r\nbody";
    let mut reader = MimeReader::new(wire.as_slice());
    reader
        .read_entity(|event| {
            if matches!(event, MimeEvent::Content { .. }) {
                return Err(io::Error::other("content callback failed"));
            }
            Ok(())
        })
        .unwrap_err();
    assert!(!reader.is_end_of_stream());
    let old_remaining = *reader.get_ref();
    assert_eq!(reader.replace_source(wire.as_slice()), old_remaining);
    let mut contents = Vec::new();
    reader
        .read_entity(|event| {
            if let MimeEvent::Content { bytes, .. } = event {
                contents.extend_from_slice(bytes);
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(contents, b"body");
    assert!(reader.is_end_of_stream());

    let source = FaultSource {
        bytes: wire,
        offset: 0,
        fail_at: Some(3),
        interrupt: false,
    };
    let mut parser = MimeParser::new(source);
    assert_eq!(
        parser.parse_message().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!parser.is_end_of_stream());
    let source = FaultSource {
        bytes: wire,
        offset: 0,
        fail_at: None,
        interrupt: false,
    };
    let old = parser.replace_source(source);
    assert_eq!(old.offset, 3);
    assert_eq!(parser.position(), MimePosition::default());
    let message = parser.parse_message().unwrap();
    assert_eq!(message.headers().get("X").unwrap().offset(), Some(0));
    assert!(parser.is_end_of_stream());
}

#[test]
fn header_completion_distinguishes_observed_eof_from_a_separator() {
    for (wire, eof) in [
        (b"".as_slice(), true),
        (b"X: a", true),
        (b"X: a\r\n", true),
        (b"X: a\r\n\r", true),
        (b"\r", true),
        (b"\n", false),
        (b"\r\n", false),
        (b"X: a\r\n\r\n", false),
        (b"X: a\r\n\r\nbody", false),
    ] {
        let mut parser = MimeParser::new(wire);
        assert!(!parser.is_end_of_stream());
        parser.parse_headers().unwrap();
        assert_eq!(parser.is_end_of_stream(), eof);
        assert_eq!(
            parser.parse_headers().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(parser.is_end_of_stream(), eof);
        let _old = parser.replace_source(wire);
        assert!(!parser.is_end_of_stream());
    }

    let source = FaultSource {
        bytes: b"X: a\r\n\r\n",
        offset: 0,
        fail_at: Some(8),
        interrupt: false,
    };
    let mut reader = MimeReader::new(source);
    reader.read_headers(|_| Ok(())).unwrap();
    // Repeated status queries must not probe the next byte to discover EOF.
    assert!(!reader.is_end_of_stream());
    assert!(!reader.is_end_of_stream());
    assert_eq!(reader.into_inner().offset, 8);
}

#[test]
fn completion_is_published_only_after_all_callbacks_succeed() {
    for message in [false, true] {
        let wire = b"\r\nbody".as_slice();
        let mut reader = MimeReader::new(wire);
        let mut callback = |event: MimeEvent<'_>| {
            if matches!(event, MimeEvent::EntityEnd { .. }) && !message
                || matches!(event, MimeEvent::MessageEnd { .. }) && message
            {
                return Err(io::Error::other("final callback failed"));
            }
            Ok(())
        };
        let error = if message {
            reader.read_message(&mut callback)
        } else {
            reader.read_entity(&mut callback)
        }
        .unwrap_err();
        assert_eq!(error.to_string(), "final callback failed");
        assert_eq!(reader.position().offset, wire.len() as u64);
        assert!(!reader.is_end_of_stream());
        let _old = reader.replace_source(wire);
        if message {
            reader.read_message(|_| Ok(())).unwrap();
        } else {
            reader.read_entity(|_| Ok(())).unwrap();
        }
        assert!(reader.is_end_of_stream());
    }
    let mut reader = MimeReader::new(b"X: a".as_slice());
    reader
        .read_headers(|_| Err(io::Error::other("headers callback failed")))
        .unwrap_err();
    assert_eq!(reader.position().offset, 4);
    assert!(!reader.is_end_of_stream());
}

#[test]
fn incremental_status_groups_keep_cumulative_positions_and_raw_headers() {
    let first = b"Reporting-MTA: dns; mx\r\n\tfold\r\nX: one\r\nX: two\r\n\r\n".as_slice();
    let second = b"Action: failed\nbad line\n\n".as_slice();
    let third = b"Status: 5.1.1".as_slice();
    let wire = [
        b"\r\n\n".as_slice(),
        first,
        b"\n\r\n",
        second,
        b"\r\n",
        third,
    ]
    .concat();
    let starts = [
        3,
        3 + first.len() + 3,
        3 + first.len() + 3 + second.len() + 2,
    ];
    for capacity in [1, 2, 3, 7, wire.len() + 1] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        for (index, block) in [first, second, third].into_iter().enumerate() {
            let mut events = 0;
            assert!(
                reader
                    .read_status_group(|event| {
                        events += 1;
                        let MimeEvent::Headers {
                            headers,
                            position,
                            headers_end,
                            body_separator,
                        } = event
                        else {
                            panic!("groups emit only header events");
                        };
                        assert_eq!(position.offset, starts[index] as u64);
                        assert_eq!(headers_end.offset, (starts[index] + block.len()) as u64);
                        assert_position(&wire, position);
                        assert_position(&wire, headers_end);
                        assert_eq!(headers.len(), [3, 2, 1][index]);
                        assert_eq!(
                            body_separator,
                            [Some(b"\r\n".as_slice()), Some(b"\n".as_slice()), None][index]
                        );
                        let mut offset = starts[index];
                        for header in &headers {
                            assert_eq!(header.offset(), Some(offset as u64));
                            assert_eq!(&wire[offset..offset + header.raw().len()], header.raw());
                            offset += header.raw().len();
                        }
                        if index == 0 {
                            assert_eq!(headers.get_all("X").count(), 2);
                        }
                        if index == 1 {
                            assert!(headers.as_slice()[1].is_invalid());
                        }
                        Ok(())
                    })
                    .unwrap()
            );
            assert_eq!(events, 1);
            assert_eq!(
                reader.position().offset,
                (starts[index] + block.len()) as u64
            );
            assert_eq!(reader.is_end_of_stream(), index == 2);
        }
        assert!(
            !reader
                .read_status_group(|_| panic!("no more groups"))
                .unwrap()
        );
        assert!(reader.is_end_of_stream());

        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, wire.as_slice()));
        for (index, start) in starts.into_iter().enumerate() {
            let headers = parser.parse_status_group().unwrap().unwrap();
            assert_eq!(headers.len(), [3, 2, 1][index]);
            assert_eq!(headers.as_slice()[0].offset(), Some(start as u64));
        }
        assert!(parser.parse_status_group().unwrap().is_none());
        assert_eq!(parser.position().offset, wire.len() as u64);
        assert!(parser.is_end_of_stream());
    }
}

#[test]
fn status_group_limits_include_prior_groups_and_skipped_blank_lines() {
    let wire = b"\r\nX: a\r\n\r\n\nY: b\n\n\r";
    for limit in 1..=wire.len() {
        let options = MimeEntityParseOptions {
            max_input_bytes: limit,
            ..Default::default()
        };
        let mut parser = MimeParser::with_options(wire.as_slice(), options).unwrap();
        loop {
            match parser.parse_status_group() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    assert_eq!(limit, wire.len());
                    assert!(parser.is_end_of_stream());
                    break;
                }
                Err(error) => {
                    assert!(limit < wire.len());
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    assert!(!parser.is_end_of_stream());
                    assert_eq!(
                        parser.parse_status_group().unwrap_err().kind(),
                        io::ErrorKind::InvalidInput
                    );
                    break;
                }
            }
        }
        assert_eq!(parser.position().offset, limit as u64);
        assert_eq!(parser.into_inner(), &wire[limit..]);
    }

    let options = MimeEntityParseOptions {
        max_header_length: 6,
        ..Default::default()
    };
    let mut parser =
        MimeParser::with_options(b"X: a\n\nY: b\n\tfold\n\n".as_slice(), options).unwrap();
    let first = parser.parse_status_group().unwrap().unwrap();
    let error = parser.parse_status_group().unwrap_err();
    assert!(error.get_ref().unwrap().is::<HeaderTooLong>());
    assert_eq!(first.as_slice()[0].raw(), b"X: a\n");
    assert_eq!(
        parser.parse_status_group().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn status_group_reading_does_not_touch_the_remainder_or_interpret_encoding_fields() {
    let wire = b"Content-Transfer-Encoding: bogus\r\n\r\nencoded remainder";
    let end = wire.len() - b"encoded remainder".len();
    let mut cursor = Cursor::new([b"skip".as_slice(), wire.as_slice()].concat());
    cursor.set_position(4);
    let mut parser = MimeParser::new(&mut cursor);
    let group = parser.parse_status_group().unwrap().unwrap();
    assert_eq!(group.as_slice()[0].offset(), Some(0));
    assert_eq!(parser.position().offset, end as u64);
    assert_eq!(parser.into_inner().position(), end as u64 + 4);

    let source = FaultSource {
        bytes: wire,
        offset: 0,
        fail_at: Some(end),
        interrupt: false,
    };
    let mut parser = MimeParser::new(source);
    assert_eq!(parser.parse_status_group().unwrap().unwrap(), group);
    assert!(!parser.is_end_of_stream());
    assert_eq!(
        parser.parse_status_group().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(parser.position().offset, end as u64);
    assert_eq!(
        parser.parse_status_group().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn status_group_iteration_is_exclusive_and_source_replacement_resets_it() {
    let wire = b"X: a\n\nY: b\n\n".as_slice();
    let run = |parser: &mut MimeParser<&[u8]>, operation| match operation {
        0 => parser.parse_headers().map(|_| ()),
        1 => parser.parse_entity().map(|_| ()),
        2 => parser.parse_message().map(|_| ()),
        _ => parser.parse_status_group().map(|_| ()),
    };
    for first in 0..4 {
        for second in 0..4 {
            let mut parser = MimeParser::new(wire);
            run(&mut parser, first).unwrap();
            let position = parser.position();
            if first == 3 && second == 3 {
                run(&mut parser, second).unwrap();
                assert!(parser.parse_status_group().unwrap().is_none());
            } else {
                assert_eq!(
                    run(&mut parser, second).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
                assert_eq!(parser.position(), position);
                if first == 3 {
                    assert!(parser.parse_status_group().unwrap().unwrap().contains("Y"));
                }
            }
            let _old = parser.replace_source(wire);
            assert!(!parser.is_end_of_stream());
            assert_eq!(parser.position(), MimePosition::default());
            parser.parse_message().unwrap();
            let _old = parser.replace_source(wire);
            assert!(parser.parse_status_group().unwrap().unwrap().contains("X"));
        }
    }
}

#[test]
fn empty_status_groups_and_eof_do_not_emit_events_or_repeat_io() {
    for wire in [b"".as_slice(), b"\r", b"\r\n\n\r\n\r"] {
        for capacity in [1, 2, 16] {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire));
            assert!(!reader.read_status_group(|_| panic!("empty group")).unwrap());
            assert_eq!(reader.position().offset, wire.len() as u64);
            assert!(reader.is_end_of_stream());
        }
    }
    struct EofOnce(bool);
    impl Read for EofOnce {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("use buffering")
        }
    }
    impl BufRead for EofOnce {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            assert!(!self.0, "must not probe EOF again");
            self.0 = true;
            Ok(&[])
        }
        fn consume(&mut self, amount: usize) {
            assert_eq!(amount, 0);
        }
    }
    let mut parser = MimeParser::new(EofOnce(false));
    for _ in 0..3 {
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
    }
    assert_eq!(
        parser.parse_headers().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn status_group_callback_failure_discards_current_progress_and_requires_a_new_source() {
    let wire = b"X: a\n\nY: b".as_slice();
    let mut reader = MimeReader::new(wire);
    assert!(reader.read_status_group(|_| Ok(())).unwrap());
    let error = reader
        .read_status_group(|_| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "group callback failed",
            ))
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(error.to_string(), "group callback failed");
    assert_eq!(reader.position().offset, wire.len() as u64);
    assert!(!reader.is_end_of_stream());
    assert_eq!(
        reader
            .read_status_group(|_| panic!("failed reader"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    let _old = reader.replace_source(wire);
    assert!(reader.read_status_group(|_| Ok(())).unwrap());
}

#[derive(Debug)]
struct ContentRegion {
    kind: MimeContentKind,
    begin: MimePosition,
    end: MimePosition,
    bytes: Vec<u8>,
}

impl ContentRegion {
    fn new(kind: MimeContentKind, begin: MimePosition) -> Self {
        Self {
            kind,
            begin,
            end: begin,
            bytes: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct EntityOffsets {
    kind: MimeEntityKind,
    begin: MimePosition,
    headers_end: MimePosition,
    end: Option<MimePosition>,
    regions: Vec<ContentRegion>,
}

impl EntityOffsets {
    fn lines(&self, wire: &[u8]) -> u64 {
        let end = self.end.unwrap();
        // Rust exposes physical positions rather than a separate content line
        // count. Count a final unterminated line, but never an empty container.
        end.line - self.headers_end.line
            + u64::from(
                end.offset > self.headers_end.offset && wire[end.offset as usize - 1] != b'\n',
            )
    }
}

// Record content/preamble/epilogue callbacks to compare their bytes against
// source offsets and the corresponding entity-end callbacks.
#[derive(Default)]
struct ContentRecordingReader {
    entities: Vec<EntityOffsets>,
    stack: Vec<usize>,
    line_starts: Vec<usize>,
}

impl ContentRecordingReader {
    fn check_position(&self, wire: &[u8], position: MimePosition) {
        assert!(position.offset <= wire.len() as u64);
        assert_eq!(
            position.line,
            self.line_starts
                .partition_point(|&start| start <= position.offset as usize) as u64
        );
    }

    fn record(&mut self, wire: &[u8], event: MimeEvent<'_>) -> io::Result<()> {
        if self.line_starts.is_empty() {
            self.line_starts.push(0);
            self.line_starts.extend(
                wire.iter()
                    .enumerate()
                    .filter(|&(_, &byte)| byte == b'\n')
                    .map(|(index, _)| index + 1),
            );
        }
        match event {
            MimeEvent::EntityBegin {
                kind,
                position,
                headers_end,
                ..
            } => {
                self.check_position(wire, position);
                self.check_position(wire, headers_end);
                let regions = match kind {
                    MimeEntityKind::Part => {
                        vec![ContentRegion::new(MimeContentKind::Body, headers_end)]
                    }
                    MimeEntityKind::Multipart => {
                        vec![ContentRegion::new(MimeContentKind::Preamble, headers_end)]
                    }
                    MimeEntityKind::MessagePart => Vec::new(),
                };
                self.stack.push(self.entities.len());
                self.entities.push(EntityOffsets {
                    kind,
                    begin: position,
                    headers_end,
                    end: None,
                    regions,
                });
            }
            MimeEvent::Content {
                kind,
                bytes,
                position,
            } => {
                self.check_position(wire, position);
                assert!(!bytes.is_empty() && bytes.len() <= 4096);
                let entity = &mut self.entities[*self.stack.last().unwrap()];
                if entity
                    .regions
                    .last()
                    .is_none_or(|region| region.kind != kind)
                {
                    assert_eq!(kind, MimeContentKind::Epilogue);
                    assert_eq!(entity.kind, MimeEntityKind::Multipart);
                    entity.regions.push(ContentRegion::new(kind, position));
                }
                let region = entity.regions.last_mut().unwrap();
                assert_eq!(position, region.end);
                let offset = position.offset as usize;
                assert_eq!(&wire[offset..offset + bytes.len()], bytes);
                region.bytes.extend_from_slice(bytes);
                region.end = MimePosition {
                    offset: position.offset + bytes.len() as u64,
                    line: position.line
                        + bytes.iter().filter(|&&byte| byte == b'\n').count() as u64,
                };
            }
            MimeEvent::EntityEnd { kind, position, .. } => {
                self.check_position(wire, position);
                let entity = &mut self.entities[self.stack.pop().unwrap()];
                assert_eq!(kind, entity.kind);
                assert!(position.offset >= entity.headers_end.offset);
                entity.end = Some(position);
                if kind == MimeEntityKind::Multipart && entity.regions.len() == 1 {
                    // Unlike MimeKit, empty regions have no explicit callbacks.
                    entity
                        .regions
                        .push(ContentRegion::new(MimeContentKind::Epilogue, position));
                }
                if let Some(region) = entity.regions.last() {
                    assert_eq!(region.end, position);
                }
            }
            MimeEvent::MessageBegin { position } | MimeEvent::MessageEnd { position } => {
                self.check_position(wire, position);
            }
            MimeEvent::MboxMarker {
                bytes,
                position,
                end,
            } => {
                self.check_position(wire, position);
                self.check_position(wire, end);
                assert_eq!(&wire[position.offset as usize..end.offset as usize], bytes);
            }
            MimeEvent::Headers { .. } => panic!("expected entity events"),
        }
        Ok(())
    }

    fn assert_regions(&self, wire: &[u8], expected_count: Option<usize>) {
        assert!(self.stack.is_empty());
        let regions: Vec<_> = self
            .entities
            .iter()
            .flat_map(|entity| &entity.regions)
            .collect();
        if let Some(count) = expected_count {
            assert_eq!(regions.len(), count);
        } else {
            assert!(!regions.is_empty());
        }
        for region in regions {
            assert_eq!(
                region.bytes,
                wire[region.begin.offset as usize..region.end.offset as usize],
                "{:?} at {}",
                region.kind,
                region.begin.offset
            );
        }
    }
}

fn record_message(
    wire: &[u8],
    capacity: usize,
) -> (ContentRecordingReader, Vec<MimeComplianceIssue>) {
    let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire));
    let mut recording = ContentRecordingReader::default();
    let mut issues = Vec::new();
    reader
        .read_message_with_logger(
            &mut |issue| {
                issues.push(issue);
                Ok(())
            },
            |event| recording.record(wire, event),
        )
        .unwrap();
    recording.assert_regions(wire, None);
    assert_eq!(reader.position().offset, wire.len() as u64);
    assert_position(wire, reader.position());
    (recording, issues)
}

#[test]
fn content_callbacks_exclude_boundary_new_line() {
    let long_line = "x".repeat(1500);
    let cases = [
        // Multipart with a preamble, an empty part, a part ending in a blank
        // line, and an epilogue followed by EOF.
        ("From: mimekit@example.org\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\npreamble\r\n--b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nline 1\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nline 2\r\n--b--\r\nepilogue\r\n".to_owned(), 5),
        // The same message with bare linefeeds.
        ("From: mimekit@example.org\nContent-Type: multipart/mixed; boundary=\"b\"\n\npreamble\n--b\n\n--b\nContent-Type: text/plain\n\nline 1\n\n--b\nContent-Type: text/plain\n\nline 2\n--b--\nepilogue\n".to_owned(), 5),
        // Overlong lines consumed mid-line immediately before a boundary.
        (format!("From: mimekit@example.org\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\n\r\n{long_line}\r\n--b\r\n\r\n{long_line}\n--b--\r\n"), 4),
        // The inner multipart's epilogue ends at the outer boundary.
        ("From: mimekit@example.org\r\nContent-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=\"inner\"\r\n\r\n--inner\r\n\r\ninner\r\n--inner--\r\ninner epilogue\r\n--outer--\r\n".to_owned(), 5),
        // Single-part messages ending at EOF, with and without a final newline.
        ("From: mimekit@example.org\r\n\r\nbody\r\n".to_owned(), 1),
        ("From: mimekit@example.org\r\n\r\nbody".to_owned(), 1),
        ("From: mimekit@example.org\r\n\r\nbody\r".to_owned(), 1),
    ];
    for (text, count) in cases {
        for capacity in [1, 2, 3, 7, 4096] {
            let (recording, _) = record_message(text.as_bytes(), capacity);
            recording.assert_regions(text.as_bytes(), Some(count));
        }
    }
}

#[test]
fn content_callbacks_exclude_boundary_new_line_mbox() {
    let wire = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join("mbox")
            .join("jwz.mbox.txt"),
    )
    .unwrap();
    for capacity in [1, 7, 4096] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
        let mut recording = ContentRecordingReader::default();
        while reader
            .read_mbox_message(|event| recording.record(&wire, event))
            .unwrap()
        {}
        recording.assert_regions(&wire, None);
        assert!(reader.is_end_of_stream());
        assert_eq!(reader.position().offset, wire.len() as u64);
        assert!(
            !reader
                .read_mbox_message(|_| panic!("events after EOF"))
                .unwrap()
        );
    }
}

fn single_line_message(content_type: &str, body: &str) -> String {
    format!(
        "From: mimekit@example.org\r\nTo: mimekit@example.org\r\n\
Subject: This is a message with a single line of text\r\n\
Message-Id: <123@example.org>\r\nDate: Tue, 12 Nov 2013 09:12:42 -0500\r\n\
MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}"
    )
}

fn assert_single_line_message(text: &str, entity_index: usize, expected_body: &[u8]) {
    for capacity in [1, 2, 3, 7, 4096] {
        let (recording, issues) = record_message(text.as_bytes(), capacity);
        assert!(issues.is_empty(), "{issues:?}");
        let entity = &recording.entities[entity_index];
        assert_eq!(entity.kind, MimeEntityKind::Part);
        assert_eq!(entity.lines(text.as_bytes()), 1);
        assert_eq!(entity.regions[0].bytes, expected_body);
    }
}

#[test]
fn line_count_single_line() {
    let body = "This is a single line of text";
    assert_single_line_message(
        &single_line_message("text/plain; charset=us-ascii", body),
        0,
        body.as_bytes(),
    );
}

#[test]
fn line_count_single_line_crlf() {
    let body = "This is a single line of text\r\n";
    assert_single_line_message(
        &single_line_message("text/plain; charset=us-ascii", body),
        0,
        body.as_bytes(),
    );
}

#[test]
fn line_count_single_line_in_multipart() {
    let body = concat!(
        "--boundary-marker\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\n",
        "This is a single line of text\r\n--boundary-marker\r\n",
        "Content-Type: application/octet-stream; name=\"attachment.dat\"\r\n",
        "Content-DIsposition: attachment; filename=\"attachment.dat\"\r\n\r\n",
        "ABC\r\n--boundary-marker--\r\n",
    );
    assert_single_line_message(
        &single_line_message("multipart/mixed; boundary=\"boundary-marker\"", body),
        1,
        b"This is a single line of text",
    );
}

#[test]
fn line_count_one_line_of_text_followed_by_blank_line_in_multipart() {
    let body = concat!(
        "--boundary-marker\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\n",
        "This is a single line of text followed by a blank line\r\n\r\n--boundary-marker\r\n",
        "Content-Type: application/octet-stream; name=\"attachment.dat\"\r\n",
        "Content-Disposition: attachment; filename=\"attachment.dat\"\r\n\r\n",
        "ABC\r\n--boundary-marker--\r\n",
    );
    assert_single_line_message(
        &single_line_message("multipart/mixed; boundary=\"boundary-marker\"", body),
        1,
        b"This is a single line of text followed by a blank line\r\n",
    );
}

const EMPTY_CONTAINER_TYPES: [&str; 2] =
    ["multipart/alternative; boundary=inner", "message/rfc822"];

#[test]
fn line_count_empty_container_in_multipart() {
    for content_type in EMPTY_CONTAINER_TYPES {
        for newline in ["\n", "\r\n"] {
            let text = format!(
                "Content-Type: multipart/mixed; boundary=outer{newline}{newline}--outer{newline}Content-Type: {content_type}{newline}{newline}--outer--{newline}"
            );
            for capacity in [1, 2, 3, 7, 4096] {
                let mut recording = ContentRecordingReader::default();
                MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()))
                    .read_entity(|event| recording.record(text.as_bytes(), event))
                    .unwrap();
                assert!(recording.stack.is_empty());
                assert_eq!(recording.entities.len(), 2);
                let child = &recording.entities[1];
                assert_eq!(child.lines(text.as_bytes()), 0);
                assert_eq!(child.headers_end, child.end.unwrap());
            }
        }
    }
}

#[test]
fn line_count_empty_container_in_mbox() {
    for content_type in EMPTY_CONTAINER_TYPES {
        for newline in ["\n", "\r\n"] {
            let text = format!(
                "From a@example.com{newline}Content-Type: {content_type}{newline}{newline}From b@example.com{newline}Subject: second{newline}{newline}body{newline}"
            );
            for capacity in [1, 2, 3, 7, 4096] {
                let mut reader =
                    MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
                let mut recording = ContentRecordingReader::default();
                assert!(
                    reader
                        .read_mbox_message(|event| recording.record(text.as_bytes(), event))
                        .unwrap()
                );
                assert!(recording.stack.is_empty());
                assert_eq!(recording.entities.len(), 1);
                assert_eq!(recording.entities[0].lines(text.as_bytes()), 0);
                assert_eq!(
                    recording.entities[0].headers_end,
                    recording.entities[0].end.unwrap()
                );
                let mut second = ContentRecordingReader::default();
                assert!(
                    reader
                        .read_mbox_message(|event| second.record(text.as_bytes(), event))
                        .unwrap()
                );
                assert_eq!(
                    second.entities[0].regions[0].bytes,
                    format!("body{newline}").as_bytes()
                );
                assert!(
                    !reader
                        .read_mbox_message(|_| panic!("events after EOF"))
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn offsets_empty_message_in_mbox() {
    for newline in ["\n", "\r\n"] {
        let text = format!(
            "From -{newline}From b@example.com{newline}Subject: second{newline}{newline}body{newline}"
        );
        let expected = MimePosition {
            offset: (6 + newline.len()) as u64,
            line: 2,
        };
        for capacity in [1, 2, 3, 7, 4096] {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
            let mut recording = ContentRecordingReader::default();
            let mut message_positions = Vec::new();
            assert!(
                reader
                    .read_mbox_message(|event| {
                        if let MimeEvent::MessageBegin { position }
                        | MimeEvent::MessageEnd { position } = &event
                        {
                            message_positions.push(*position);
                        }
                        recording.record(text.as_bytes(), event)
                    })
                    .unwrap()
            );
            assert_eq!(message_positions, [expected, expected]);
            assert!(recording.stack.is_empty());
            assert_eq!(recording.entities.len(), 1);
            let body = &recording.entities[0];
            assert_eq!(body.begin, expected);
            assert_eq!(body.headers_end, expected);
            assert_eq!(body.end, Some(expected));
            assert_eq!(body.lines(text.as_bytes()), 0);
            assert!(body.regions[0].bytes.is_empty());
            assert!(reader.read_mbox_message(|_| Ok(())).unwrap());
            assert!(
                !reader
                    .read_mbox_message(|_| panic!("events after EOF"))
                    .unwrap()
            );
        }
    }
}

fn assert_single_header_line_count(ending: &str, violation: Violation, line: u64) {
    let text = format!(
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\nMessage-Id: <123@example.org>\r\nFrom: mimekit@example.org{ending}"
    );
    for capacity in [1, 2, 3, 7, 4096] {
        let (recording, issues) = record_message(text.as_bytes(), capacity);
        assert_eq!(recording.entities.len(), 1);
        assert_eq!(recording.entities[0].lines(text.as_bytes()), 0);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, violation);
        assert_eq!(issues[0].stream_offset, text.len() as u64);
        assert_eq!(issues[0].line_number, line);
    }
}

#[test]
fn line_count_non_terminated_single_header() {
    assert_single_header_line_count("", Violation::IncompleteHeader, 3);
}

#[test]
fn line_count_properly_terminated_single_header() {
    assert_single_header_line_count("\r\n", Violation::MissingBodySeparator, 4);
}

#[test]
fn multipart_end_offsets_exclude_shared_boundary_new_line() {
    for newline in ["\n", "\r\n"] {
        for epilogue in ["", "\n", "epilogue\n", "epilogue\n\n"] {
            for ending in ["--outer--\n", "", "From next\n\nnext body\n"] {
                let mbox = ending.starts_with("From ");
                let prefix = format!(
                    "{}Content-Type: multipart/mixed; boundary=outer\n\n\
                     --outer\nContent-Type: message/rfc822\n\n\
                     Content-Type: multipart/mixed; boundary=inner\n\n\
                     --inner\n\nbody\n--inner--\n{epilogue}",
                    if mbox { "From first\n" } else { "" },
                )
                .replace('\n', newline);
                let wire = format!("{prefix}{}", ending.replace('\n', newline)).into_bytes();
                let expected_end = prefix.len() - if ending.is_empty() { 0 } else { newline.len() };
                let expected_epilogue = if ending.is_empty() {
                    epilogue
                } else {
                    epilogue.strip_suffix('\n').unwrap_or(epilogue)
                }
                .replace('\n', newline);
                for capacity in [1, 2, 7, 4096] {
                    let mut reader =
                        MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
                    let mut recorder = ContentRecordingReader::default();
                    let mut message_ends = Vec::new();
                    let mut record = |event: MimeEvent<'_>| {
                        if let MimeEvent::MessageEnd { position } = &event {
                            message_ends.push(*position);
                        }
                        recorder.record(&wire, event)
                    };
                    if mbox {
                        assert!(reader.read_mbox_message(&mut record).unwrap());
                        assert_eq!(reader.position().offset, prefix.len() as u64 + 5);
                    } else {
                        reader.read_message(&mut record).unwrap();
                        assert_eq!(reader.position().offset, wire.len() as u64);
                    }
                    recorder.assert_regions(&wire, None);
                    let wrapper = &recorder.entities[1];
                    let multipart = &recorder.entities[2];
                    assert_eq!(wrapper.kind, MimeEntityKind::MessagePart);
                    assert_eq!(multipart.kind, MimeEntityKind::Multipart);
                    assert_eq!(multipart.end.unwrap().offset, expected_end as u64);
                    assert_eq!(wrapper.end, multipart.end);
                    assert_eq!(message_ends[0], multipart.end.unwrap());
                    assert_eq!(
                        multipart.regions.last().unwrap().bytes,
                        expected_epilogue.as_bytes()
                    );
                    assert_eq!(recorder.entities[3].regions[0].bytes, b"body");
                }
            }
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MimeOffsets {
    mime_type: Option<String>,
    mbox_marker_offset: Option<u64>,
    line_number: u64,
    begin_offset: u64,
    headers_end_offset: u64,
    end_offset: u64,
    message: Option<Box<MimeOffsets>>,
    body: Option<Box<MimeOffsets>>,
    children: Option<Vec<MimeOffsets>>,
    octets: u64,
    lines: Option<u64>,
}

impl MimeOffsets {
    fn finish(
        &mut self,
        wire: &[u8],
        position: MimePosition,
        child_end: Option<(MimePosition, u64)>,
    ) {
        let mut end = position.offset as usize;
        // MimeKit's end callbacks trim a boundary's preceding newline and
        // clamp headersEndOffset to that end, even for an empty body. Rust's
        // begin callback already reported the physical end of the separator.
        // Adapt only this empty-content convention, never nonempty content.
        if position.offset == self.headers_end_offset
            && (wire[end..].starts_with(b"--") || wire[end..].starts_with(b"From "))
        {
            if end > 0 && wire[end - 1] == b'\n' {
                end -= 1;
                if end > 0 && wire[end - 1] == b'\r' {
                    end -= 1;
                }
            }
            end = end.max(self.begin_offset as usize);
        }
        self.end_offset = end as u64;
        if let Some((child_position, child_end)) = child_end
            && child_position == position
        {
            // Empty children can end at the same boundary as their truncated
            // ancestors. Apply the same end convention through that chain.
            self.end_offset = self.end_offset.min(child_end);
        }
        self.headers_end_offset = self.headers_end_offset.min(self.end_offset);
        self.octets = self.end_offset - self.headers_end_offset;
    }
}

struct MboxOffsetFrame {
    kind: Option<MimeEntityKind>,
    offsets: MimeOffsets,
    headers_end_line: u64,
    child_end: Option<(MimePosition, u64)>,
}

#[derive(Default)]
struct MboxOffsetReader {
    messages: Vec<MimeOffsets>,
    stack: Vec<MboxOffsetFrame>,
    marker: Option<u64>,
}

impl MboxOffsetReader {
    fn record(&mut self, wire: &[u8], event: MimeEvent<'_>) -> io::Result<()> {
        match event {
            MimeEvent::MboxMarker { position, .. } => {
                assert!(self.stack.is_empty());
                self.marker = Some(position.offset);
            }
            MimeEvent::MessageBegin { position } => {
                let mut offsets = MimeOffsets {
                    begin_offset: position.offset,
                    line_number: position.line,
                    ..Default::default()
                };
                if let Some(parent) = self.stack.last() {
                    assert_eq!(parent.kind, Some(MimeEntityKind::MessagePart));
                } else {
                    offsets.mbox_marker_offset = self.marker.take();
                }
                self.stack.push(MboxOffsetFrame {
                    kind: None,
                    offsets,
                    headers_end_line: 0,
                    child_end: None,
                });
            }
            MimeEvent::EntityBegin {
                headers,
                kind,
                position,
                headers_end,
                ..
            } => {
                let mime_type = if let Some(header) = headers.get("Content-Type") {
                    match mimecraft::ContentType::parse(header.raw_value()) {
                        Ok(content_type) => content_type.mime_type(),
                        Err(error) => {
                            // Rust retains raw metadata rather than exposing
                            // MimeKit's recovered ContentType in the callback.
                            // These old toolkit headers in jwz are invalid;
                            // the reader defaults them to application/octet-stream.
                            assert!(
                                [b"X-BE2; 12".as_slice(), b"x-sun-attachment"].iter().any(
                                    |value| header
                                        .raw_value()
                                        .trim_ascii()
                                        .eq_ignore_ascii_case(value)
                                ),
                                "Content-Type at {}: {:?}: {error}",
                                position.offset,
                                header.raw_value()
                            );
                            "application/octet-stream".to_owned()
                        }
                    }
                } else if self.stack.last().is_some_and(|frame| {
                    frame.kind == Some(MimeEntityKind::Multipart)
                        && frame
                            .offsets
                            .mime_type
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case("multipart/digest"))
                }) {
                    "message/rfc822".to_owned()
                } else {
                    "text/plain".to_owned()
                };
                if let Some(parent) = self.stack.last_mut()
                    && parent.kind.is_none()
                {
                    parent.offsets.headers_end_offset = headers_end.offset;
                }
                self.stack.push(MboxOffsetFrame {
                    kind: Some(kind),
                    offsets: MimeOffsets {
                        mime_type: Some(mime_type),
                        begin_offset: position.offset,
                        line_number: position.line,
                        headers_end_offset: headers_end.offset,
                        ..Default::default()
                    },
                    headers_end_line: headers_end.line,
                    child_end: None,
                });
            }
            MimeEvent::EntityEnd { kind, position, .. } => {
                let mut frame = self.stack.pop().unwrap();
                assert_eq!(frame.kind, Some(kind));
                frame.offsets.finish(wire, position, frame.child_end);
                // The Rust callbacks expose physical positions. Include a
                // final unterminated content line, but not an empty body.
                let end = frame.offsets.end_offset as usize;
                let end_line = position.line
                    - wire[end..position.offset as usize]
                        .iter()
                        .filter(|&&byte| byte == b'\n')
                        .count() as u64;
                frame.offsets.lines = Some(if frame.offsets.octets == 0 {
                    0
                } else {
                    end_line - frame.headers_end_line + u64::from(wire[end - 1] != b'\n')
                });
                let parent = self.stack.last_mut().unwrap();
                parent.child_end = Some((position, frame.offsets.end_offset));
                match parent.kind {
                    None => {
                        assert!(parent.offsets.body.is_none());
                        parent.offsets.body = Some(Box::new(frame.offsets));
                    }
                    Some(MimeEntityKind::Multipart) => {
                        parent
                            .offsets
                            .children
                            .get_or_insert_with(Vec::new)
                            .push(frame.offsets);
                    }
                    _ => panic!("entities belong to messages or multiparts"),
                }
            }
            MimeEvent::MessageEnd { position } => {
                let mut frame = self.stack.pop().unwrap();
                assert_eq!(frame.kind, None);
                frame.offsets.finish(wire, position, frame.child_end);
                if let Some(parent) = self.stack.last_mut() {
                    assert_eq!(parent.kind, Some(MimeEntityKind::MessagePart));
                    assert!(parent.offsets.message.is_none());
                    parent.child_end = Some((position, frame.offsets.end_offset));
                    parent.offsets.message = Some(Box::new(frame.offsets));
                } else {
                    self.messages.push(frame.offsets);
                }
            }
            MimeEvent::Content { .. } => {}
            MimeEvent::Headers { .. } => panic!("expected MIME tree events"),
        }
        Ok(())
    }
}

fn assert_mime_offsets(expected: &MimeOffsets, actual: &MimeOffsets, path: &str) {
    assert_eq!(actual.mime_type, expected.mime_type, "{path}: MIME type");
    assert_eq!(
        actual.mbox_marker_offset, expected.mbox_marker_offset,
        "{path}: mbox marker"
    );
    assert_eq!(
        actual.line_number, expected.line_number,
        "{path}: begin line"
    );
    assert_eq!(
        actual.begin_offset, expected.begin_offset,
        "{path}: begin offset"
    );
    assert_eq!(
        actual.headers_end_offset, expected.headers_end_offset,
        "{path}: headers end"
    );
    assert_eq!(actual.end_offset, expected.end_offset, "{path}: end offset");
    assert_eq!(actual.octets, expected.octets, "{path}: octets");
    assert_eq!(actual.lines, expected.lines, "{path}: lines");
    match (&expected.message, &actual.message) {
        (Some(expected), Some(actual)) => {
            assert_mime_offsets(expected, actual, &format!("{path}/message"))
        }
        (None, None) => {}
        _ => panic!("{path}: embedded message presence differs"),
    }
    match (&expected.body, &actual.body) {
        (Some(expected), Some(actual)) => {
            assert_mime_offsets(expected, actual, &format!("{path}/0"))
        }
        (None, None) => {}
        _ => panic!("{path}: body presence differs"),
    }
    match (&expected.children, &actual.children) {
        (Some(expected), Some(actual)) => {
            assert_eq!(actual.len(), expected.len(), "{path}: child count");
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                assert_mime_offsets(expected, actual, &format!("{path}.{index}"));
            }
        }
        (None, None) => {}
        _ => panic!("{path}: children presence differs"),
    }
}

fn assert_mbox_offsets(name: &str) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("mbox");
    let original = fs::read(directory.join(format!("{name}.mbox.txt"))).unwrap();
    // Normalize bytes, not text: the archive can contain non-UTF-8 content.
    let unix: Vec<_> = original
        .iter()
        .enumerate()
        .filter(|&(index, &byte)| byte != b'\r' || original.get(index + 1) != Some(&b'\n'))
        .map(|(_, &byte)| byte)
        .collect();
    let mut dos = Vec::with_capacity(original.len());
    for &byte in &unix {
        if byte == b'\n' {
            dos.push(b'\r');
        }
        dos.push(byte);
    }
    for (format, wire) in [("unix", &unix), ("dos", &dos)] {
        // Golden files are upstream data, never generated from the reader
        // under test. A missing or malformed fixture must fail the test.
        let expected: Vec<MimeOffsets> = serde_json::from_slice(
            &fs::read(directory.join(format!("{name}.{format}-offsets.json"))).unwrap(),
        )
        .unwrap();
        for capacity in [1, 7, 4096] {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
            let mut recorder = MboxOffsetReader::default();
            while reader
                .read_mbox_message(|event| recorder.record(wire, event))
                .unwrap()
            {}
            assert!(recorder.stack.is_empty());
            assert_eq!(reader.position().offset, wire.len() as u64);
            assert!(reader.is_end_of_stream());
            assert_eq!(
                recorder.messages.len(),
                expected.len(),
                "{name}/{format}/{capacity}: messages"
            );
            for (index, (expected, actual)) in expected.iter().zip(&recorder.messages).enumerate() {
                assert_mime_offsets(
                    expected,
                    actual,
                    &format!("{name}/{format}/{capacity}/message#{index}"),
                );
            }
        }
    }
}

#[test]
fn issue1189_mbox() {
    assert_mbox_offsets("issue1189");
}

#[test]
fn jwz_mbox() {
    assert_mbox_offsets("jwz");
}

type ExpectedIssue = (Violation, u64, u64, PositionKind);

#[test]
fn mime_compliance_invalid_header_field_name_with_space() {
    assert_mime_compliance_violations(
        "invalid-header-field-with-space.eml",
        &[(Violation::InvalidHeader, 7, 10, PositionKind::Exact)],
    );
}

#[test]
fn mime_compliance_incomplete_header() {
    assert_mime_compliance_violations(
        "incomplete-header.eml",
        &[(Violation::IncompleteHeader, 7, 25, PositionKind::Exact)],
    );
}

#[test]
fn mime_compliance_invalid_content_transfer_encoding_basic() {
    assert_mime_compliance_violations(
        "invalid-content-transfer-encoding-basic.eml",
        &[(
            Violation::InvalidContentTransferEncoding,
            7,
            28,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_invalid_content_transfer_encoding_multipart() {
    assert_mime_compliance_violations(
        "invalid-content-transfer-encoding-multipart.eml",
        &[(
            Violation::IllegalMultipartContentTransferEncoding,
            10,
            28,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_invalid_content_transfer_encoding_rfc822() {
    assert_mime_compliance_violations(
        "invalid-content-transfer-encoding-rfc822.eml",
        &[(
            Violation::IllegalMessageRfc822ContentTransferEncoding,
            7,
            28,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_invalid_content_type() {
    assert_mime_compliance_violations(
        "invalid-content-type.eml",
        &[(
            Violation::InvalidContentType,
            6,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_oversized_line() {
    assert_mime_compliance_violations(
        "invalid-wrapping.eml",
        &[
            (Violation::OversizedLine, 7, 1, PositionKind::LineStart),
            (Violation::OversizedLine, 11, 1, PositionKind::LineStart),
        ],
    );
}

#[test]
fn mime_compliance_missing_body_separator() {
    assert_mime_compliance_violations(
        "missing-body-separator.eml",
        &[(Violation::MissingBodySeparator, 8, 1, PositionKind::Exact)],
    );
}

#[test]
fn mime_compliance_missing_multipart_boundary_parameter() {
    assert_mime_compliance_violations(
        "missing-multipart-boundary-parameter.eml",
        &[(
            Violation::MissingMultipartBoundaryParameter,
            6,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_missing_multipart_boundary() {
    assert_mime_compliance_violations(
        "missing-multipart-boundary.eml",
        &[(
            Violation::MissingMultipartBoundary,
            19,
            1,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_missing_multipart_end_boundary() {
    assert_mime_compliance_violations(
        "missing-multipart-end-boundary.eml",
        &[(
            Violation::MissingMultipartBoundary,
            19,
            1,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_repeated_content_transfer_encoding() {
    assert_mime_compliance_violations(
        "multiple-content-transfer-encodings.eml",
        &[(
            Violation::RepeatedContentTransferEncoding,
            8,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_repeated_content_type() {
    assert_mime_compliance_violations(
        "multiple-content-types.eml",
        &[(
            Violation::RepeatedContentType,
            7,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_unexpected_8bit_bytes_in_header() {
    assert_mime_compliance_violations(
        "raw-koi8r-header.eml",
        &[(
            Violation::Unexpected8BitBytesInHeader,
            3,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_valid_8bit_bytes_in_header() {
    assert_mime_compliance_violations("raw-utf8-header.eml", &[]);
}

#[test]
fn mime_compliance_unexpected_8bit_bytes_in_folded_header() {
    // The invalid 8-bit bytes are on a continuation line. Attribute the
    // violation to the header's beginning: validation needs its full value.
    assert_mime_compliance_violations(
        "8bit-folded-header.eml",
        &[(
            Violation::Unexpected8BitBytesInHeader,
            3,
            1,
            PositionKind::ElementStart,
        )],
    );
}

#[test]
fn mime_compliance_valid_8bit_bytes_in_folded_header() {
    // RFC 6532 permits valid UTF-8, including folded continuation lines.
    assert_mime_compliance_violations("utf8-folded-header.eml", &[]);
}

#[test]
fn mime_compliance_unexpected_8bit_bytes_in_body() {
    assert_mime_compliance_violations(
        "unexpected-8bit-bytes-in-body.eml",
        &[(
            Violation::Unexpected8BitBytesInBody,
            24,
            1,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_unexpected_8bit_bytes_in_preamble() {
    assert_mime_compliance_violations(
        "unexpected-8bit-bytes-in-preamble.eml",
        &[(
            Violation::Unexpected8BitBytesInBody,
            11,
            1,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_unexpected_8bit_bytes_in_epilogue() {
    assert_mime_compliance_violations(
        "unexpected-8bit-bytes-in-epilogue.eml",
        &[(
            Violation::Unexpected8BitBytesInBody,
            28,
            1,
            PositionKind::Exact,
        )],
    );
}

#[test]
fn mime_compliance_unexpected_null_bytes_in_header() {
    assert_mime_compliance_violations(
        "unexpected-null-bytes-in-headers.eml",
        &[(
            Violation::UnexpectedNullBytesInHeader,
            5,
            1,
            PositionKind::LineStart,
        )],
    );
}

#[test]
fn mime_compliance_unexpected_null_bytes_in_body() {
    assert_mime_compliance_violations(
        "unexpected-null-bytes-in-body.eml",
        &[
            (
                Violation::UnexpectedNullBytesInBody,
                18,
                1,
                PositionKind::Exact,
            ),
            // MimeKit reports the scanned line fragment's start (column
            // 1 with its default buffer). mimecraft reports the actual
            // offending byte, independently of buffer boundaries.
            (
                Violation::Unexpected8BitBytesInBody,
                19,
                118,
                PositionKind::Exact,
            ),
        ],
    );
}

fn assert_mime_compliance_violations(name: &str, expected: &[ExpectedIssue]) {
    use Violation::{BareLinefeedInBody, BareLinefeedInHeader};
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("compliance")
        .join(name);
    let original = fs::read(path).unwrap();
    // Normalize bytes, not text: some fixtures deliberately contain
    // non-UTF-8 headers and null bytes.
    let unix: Vec<_> = original
        .iter()
        .enumerate()
        .filter(|&(index, &byte)| byte != b'\r' || original.get(index + 1) != Some(&b'\n'))
        .map(|(_, &byte)| byte)
        .collect();
    let dos: Vec<_> = unix
        .iter()
        .flat_map(|&byte| {
            if byte == b'\n' {
                vec![b'\r', b'\n']
            } else {
                vec![byte]
            }
        })
        .collect();
    for wire in [&unix, &dos] {
        for capacity in [1, 7, 4096] {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_slice()));
            let mut issues = Vec::new();
            reader
                .read_message_with_logger(
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let bare = if wire == &unix {
                wire.iter().filter(|&&byte| byte == b'\n').count()
            } else {
                0
            };
            assert_eq!(
                issues.len(),
                expected.len() + bare,
                "{name} / {capacity}: {issues:?}"
            );
            let actual: Vec<_> = issues
                .into_iter()
                .filter(|issue| {
                    !matches!(issue.violation, BareLinefeedInHeader | BareLinefeedInBody)
                })
                .collect();
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (actual, &(violation, line, column, kind)) in actual.iter().zip(expected) {
                let offset = wire
                    .split_inclusive(|&byte| byte == b'\n')
                    .take(line as usize - 1)
                    .map(<[u8]>::len)
                    .sum::<usize>()
                    + column as usize
                    - 1;
                assert_eq!(
                    (
                        actual.violation,
                        actual.line_number,
                        actual.column_number,
                        actual.position_kind
                    ),
                    (violation, line, column, kind),
                    "{name}"
                );
                assert_eq!(actual.stream_offset, offset as u64, "{name}");
            }
        }
    }
}

#[test]
fn invalid_header_field_name_positions() {
    // Spaces and controls within a field name identify the offending byte,
    // rather than the start of the header.
    let text = concat!(
        "From: mimekit@example.org\r\nTo: mimekit@example.org\r\n",
        "Subject: invalid header field names\r\n",
        "X-Invalid Header: the field name contains a space\r\n",
        "X-Ctl\x01Header: the field name contains a control character\r\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\n",
        "Message-Id: <123@example.org>\r\n\r\nThis is the body.\r\n",
    );
    let expected = [
        (text.find("X-Invalid Header").unwrap() + 9, 4, 10),
        (text.find("X-Ctl\x01Header").unwrap() + 5, 5, 6),
    ];
    for capacity in [1, 7, 4096] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let mut issues = Vec::new();
        reader
            .read_message_with_logger(
                &mut |issue| {
                    issues.push(issue);
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(issues.len(), expected.len());
        for (issue, (offset, line, column)) in issues.iter().zip(expected) {
            assert_eq!(issue.violation, Violation::InvalidHeader);
            assert_eq!(issue.stream_offset, offset as u64);
            assert_eq!(issue.line_number, line);
            assert_eq!(issue.column_number, column);
            assert_eq!(issue.position_kind, PositionKind::Exact);
        }
    }
}

#[test]
fn mime_compliance_blank_header_continuation_line() {
    let cases: &[(&str, &[usize])] = &[
        ("Subject: x\r\n \r\nTo: y\r\n\r\nbody\r\n", &[12]),
        ("Subject: x\n\t\nTo: y\n\nbody\n", &[11]),
        ("Subject: x\n \t \t\nTo: y\n\nbody\n", &[11]),
        // Each blank continuation line is reported, even between valid folds.
        ("Subject: x\n \n \nTo: y\n\nbody\n", &[11, 13]),
        ("A: x\n \nB: y\n\t\n\nbody\n", &[5, 12]),
        ("Subject: x\n \n y\n\nbody\n", &[11]),
        ("Subject: x\n \n\nbody\n", &[11]),
        // RFC 7103 warns this may be mistaken for the header/body separator.
        ("Subject: x\n \nbody\n", &[11]),
        ("Subject: x\r\n \nTo: y\r\n\r\nbody\r\n", &[12]),
        ("Subject: x\n \r \nTo: y\n\nbody\n", &[11]),
        ("Subject: x\n \r\r\nTo: y\n\nbody\n", &[11]),
        ("Subject: x\n  ", &[11]),
        (
            "Content-Type: multipart/mixed; boundary=x\n\n--x\nA: b\n \n\nbody\n--x--\n",
            &[52],
        ),
        // Legitimate folds, blank initial values and body lines are exempt.
        ("Subject: x\n y\n\nbody\n", &[]),
        ("Subject: x\n  y \n\t z\n\nbody\n", &[]),
        ("Subject: \n x\n\nbody\n", &[]),
        ("Subject:\n x\n\nbody\n", &[]),
        ("Subject: x\n\n \nbody\n \n", &[]),
    ];
    for &(text, expected) in cases {
        let data = text.as_bytes();
        for capacity in 1..=data.len() {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, data));
            let mut issues = Vec::new();
            reader
                .read_message_with_logger(
                    &mut |issue: MimeComplianceIssue| {
                        if issue.violation == Violation::BlankHeaderContinuationLine {
                            issues.push(issue);
                        }
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap();
            assert_eq!(
                issues
                    .iter()
                    .map(|issue| issue.stream_offset as usize)
                    .collect::<Vec<_>>(),
                expected
            );
            for issue in issues {
                let offset = issue.stream_offset as usize;
                assert!(offset == 0 || data[offset - 1] == b'\n');
                let line = data[..offset].iter().filter(|&&byte| byte == b'\n').count() + 1;
                let end = data[offset..]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(data.len(), |length| offset + length);
                assert!(end > offset);
                assert!(
                    data[offset..end]
                        .iter()
                        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
                );
                assert_eq!(issue.line_number, line as u64);
                assert_eq!(issue.column_number, 1);
                assert_eq!(issue.position_kind, PositionKind::LineStart);
                assert_eq!(issue.severity(), mimecraft::MimeComplianceSeverity::Major);
            }
        }
    }
}

#[test]
fn mime_compliance_bare_carriage_return_in_header() {
    let cases: &[(&str, &[usize])] = &[
        ("Subject: a\rb\r\n\r\nbody\r\n", &[10]),
        // Report only the first bare CR in each header.
        ("Subject: a\rb\rc\r\n\r\nbody\r\n", &[10]),
        ("A: x\ry\nB: z\rw\n\nbody\n", &[4, 11]),
        ("Subject: a\n\r\r\nTo: b\n\nbody\n", &[11]),
        ("Subject: a\n\r\r\r\nTo: b\n\nbody\n", &[11]),
        ("Subject: a\n b\rc\n\nbody\n", &[13]),
        ("Subject: a\r\r\n\nbody\n", &[10]),
        ("Subject: a\r", &[10]),
        ("Subject: a\rb", &[10]),
        (
            "Content-Type: multipart/mixed; boundary=x\n\n--x\nA: b\rc\n\nbody\n--x--\n",
            &[51],
        ),
        // CRLF terminators and bare CRs in content are not header violations.
        ("Subject: a\r\n\r\nbody\r\n", &[]),
        ("Subject: a\n\nb\rc\n", &[]),
    ];
    for &(text, expected) in cases {
        let data = text.as_bytes();
        // Exercise every CR position relative to the input buffer's end.
        for capacity in 1..=data.len() {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, data));
            let mut issues = Vec::new();
            reader
                .read_message_with_logger(
                    &mut |issue: MimeComplianceIssue| {
                        if issue.violation == Violation::BareCarriageReturnInHeader {
                            issues.push(issue);
                        }
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap();
            assert_eq!(
                issues
                    .iter()
                    .map(|issue| issue.stream_offset as usize)
                    .collect::<Vec<_>>(),
                expected,
                "{text:?} / {capacity}"
            );
            for issue in issues {
                let offset = issue.stream_offset as usize;
                let line_start = data[..offset]
                    .iter()
                    .rposition(|&byte| byte == b'\n')
                    .map_or(0, |index| index + 1);
                let line = data[..offset].iter().filter(|&&byte| byte == b'\n').count() + 1;
                assert_eq!(data[offset], b'\r');
                assert_eq!(issue.line_number, line as u64);
                assert_eq!(issue.column_number, (offset - line_start + 1) as u64);
                assert_eq!(issue.position_kind, PositionKind::Exact);
            }
        }
    }
}

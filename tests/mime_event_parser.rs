// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use mimecraft::{
    MimeComplianceIssue, MimeComplianceOptions, MimeContentKind, MimeEntityParseOptions, MimeEvent,
    MimeEventFormat, MimeEventParser, MimePosition, MimeReader,
};

#[derive(Debug, PartialEq)]
enum Recorded {
    Event(String),
    Content(MimeContentKind, MimePosition, Vec<u8>),
}

fn record(events: &mut Vec<Recorded>, event: MimeEvent<'_>) {
    if let MimeEvent::Content {
        kind,
        bytes,
        position,
    } = event
    {
        assert!(!bytes.is_empty());
        assert!(bytes.len() <= 4096);
        if let Some(Recorded::Content(previous_kind, start, previous)) = events.last_mut()
            && *previous_kind == kind
            && start.offset + previous.len() as u64 == position.offset
        {
            previous.extend_from_slice(bytes);
        } else {
            events.push(Recorded::Content(kind, position, bytes.to_vec()));
        }
    } else if let MimeEvent::EntityBegin {
        headers,
        kind,
        encoding,
        position,
        headers_end,
    } = event
    {
        events.push(Recorded::Event(format!(
            "EntityBegin {:?} {kind:?} {encoding:?} {position:?} {headers_end:?}",
            headers.as_slice()
        )));
    } else {
        events.push(Recorded::Event(format!("{event:?}")));
    }
}

#[derive(Debug, PartialEq)]
struct Capture {
    events: Vec<Recorded>,
    issues: Vec<String>,
    position: MimePosition,
    error: Option<io::ErrorKind>,
}

fn sync(wire: &[u8], format: MimeEventFormat, options: MimeEntityParseOptions) -> Capture {
    let mut reader = MimeReader::with_options(wire, options).unwrap();
    let mut events = Vec::new();
    let mut issues = Vec::new();
    let mut callback = |event: MimeEvent<'_>| {
        record(&mut events, event);
        Ok(())
    };
    let mut logger = |issue: MimeComplianceIssue| {
        issues.push(format!("{issue:?}"));
        Ok(())
    };
    let result = match format {
        MimeEventFormat::Entity => reader.read_entity_with_logger(&mut logger, &mut callback),
        MimeEventFormat::Message => reader.read_message_with_logger(&mut logger, &mut callback),
        MimeEventFormat::Mbox => loop {
            match reader.read_mbox_message_with_logger(&mut logger, &mut callback) {
                Ok(true) => {}
                Ok(false) => break Ok(()),
                Err(error) => break Err(error),
            }
        },
    };
    Capture {
        events,
        issues,
        position: reader.position(),
        error: result.err().map(|e| e.kind()),
    }
}

fn feed(
    wire: &[u8],
    format: MimeEventFormat,
    options: MimeEntityParseOptions,
    chunks: &[usize],
) -> Capture {
    let mut events = Vec::new();
    let mut issues = Vec::new();
    let mut callback = |event: MimeEvent<'_>| {
        record(&mut events, event);
        Ok(())
    };
    let mut logger = |issue: MimeComplianceIssue| {
        issues.push(format!("{issue:?}"));
        Ok(())
    };
    let mut parser = MimeEventParser::with_logger(
        format,
        options,
        MimeComplianceOptions::default(),
        &mut logger,
    )
    .unwrap();
    let mut offset = 0;
    let result = (|| {
        for &length in chunks {
            let end = offset + length;
            assert_eq!(parser.parse(&wire[offset..end], &mut callback)?, length);
            assert!(!parser.is_finished());
            assert_eq!(parser.position().offset, end as u64);
            let position = parser.position();
            assert_eq!(parser.parse(&[], &mut callback)?, 0);
            assert_eq!(parser.position(), position);
            offset = end;
        }
        assert_eq!(offset, wire.len());
        parser.finish(&mut callback)?;
        assert!(parser.is_finished());
        parser.finish(|_| panic!("EOF must be idempotent"))?;
        Ok::<_, io::Error>(())
    })();
    let position = parser.position();
    drop(parser);
    Capture {
        events,
        issues,
        position,
        error: result.err().map(|e| e.kind()),
    }
}

fn assert_splits(wire: &[u8], format: MimeEventFormat, options: MimeEntityParseOptions) {
    let expected = sync(wire, format, options);
    for split in 0..=wire.len() {
        let mut actual = feed(wire, format, options, &[split, wire.len() - split]);
        if expected.error.is_some() {
            // A rejected prefix can be discovered after different amounts of
            // bounded lookahead. Failure does not promise a resumable position.
            actual.position = expected.position;
        }
        assert_eq!(actual, expected, "{format:?}, split {split}, wire {wire:?}");
    }
    let mut actual = feed(wire, format, options, &vec![1; wire.len()]);
    if expected.error.is_some() {
        actual.position = expected.position;
    }
    assert_eq!(actual, expected);
}

#[test]
fn nested_recovery_positions_and_diagnostics_are_independent_of_chunk_boundaries() {
    let wires: &[&[u8]] = &[
        b"Subject: root\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
preamble\r\n--b\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n\
aGVsbG8=\r\n\r\n--b\r\nContent-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\n\
inner\r\n--b\r\nContent-Type: message/rfc822\r\n\r\n--b--\r\nepilogue",
        b"Content-Type: multipart/mixed; boundary=o\n\n--o\n\
Content-Type: multipart/mixed; boundary=i\n\n--i\n\nbody\r\n--o--",
        b"Content-Type: message/rfc822\r\nContent-Transfer-Encoding:\r\n\r\n>From",
        b"Content-Type: multipart/digest; boundary=b\r\n\r\n--b\r\n\r\nSubject: nested",
        b"Content-Type: text/plain\r\n\r\nbody\r",
        b"Subject: folded\r\n\tvalue\r",
        b"Subject: folded\n\nbody\n--not-a-boundary",
        b"",
    ];
    for wire in wires {
        for format in [MimeEventFormat::Entity, MimeEventFormat::Message] {
            assert_splits(wire, format, MimeEntityParseOptions::default());
        }
    }
}

#[test]
fn header_byte_diagnostics_survive_pauses() {
    let wires: &[&[u8]] = &[
        b"Subject: \xc3\xa9 \xf0\x9f\x98\x80\r\n\tvalid UTF-8\r\n\r\n",
        b"Subject: \xc3\xa9\r\n\t\0hello\xffworld\0\xfe\r\n\r\nbody\0\xff",
        b"Subject: \xc3\xa9 \xf0\x9f\x98\x80 \xe2\x82",
        b"Subject: \xc3\r\n \xa9\r\n\r\n",
        b"A: x\0\xff\0\xfe\nB: y\0\xfe\0\xff\n\n",
        b"X-First: ascii\r\nBad\0\xff: value\r\n\r\n",
        b"From: a@\xff.example\r\n\r\n",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\
Content-Type: message/rfc822\r\n\r\n\xff\0bad\r\n--b--\r\n",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\
Content-Type: message/rfc822\r\n\r\nSubject: \xc3\xa9\r\n--b--\r\n",
    ];
    for wire in wires {
        for format in [MimeEventFormat::Message, MimeEventFormat::Entity] {
            assert_splits(wire, format, Default::default());
        }
    }
    assert_splits(
        b"From first\r\nContent-Type: message/rfc822\r\n\r\nSubject: \xc3\xa9\0\xff\r\n\
From next\r\nSubject: next\r\n\r\n",
        MimeEventFormat::Mbox,
        Default::default(),
    );
}

#[test]
fn leading_marker_recovery_survives_pauses() {
    let wires = [
        concat!(
            "Content-Type: message/rfc822\r\n\r\n",
            ">From first preheader\r\n",
            ">From second preheader line is longer\r\n",
            "From: sender@example.org\r\n",
            "Subject: preheader resize\r\n\r\nbody\r\n",
        ),
        "From - \r\n>From munged\r\nSubject: test\r\n\r\nbody\r\n",
        "From sender@example.com Mon Jan  1 00:00:00 2024\r\nSubject: test",
        "From sender@example.com Mon Jan  1 00:00:00 2024\r\n\r\n",
    ];
    for original in wires {
        for wire in [original.to_owned(), original.replace("\r\n", "\n")] {
            assert!(
                sync(
                    wire.as_bytes(),
                    MimeEventFormat::Message,
                    Default::default()
                )
                .error
                .is_none()
            );
            assert_splits(
                wire.as_bytes(),
                MimeEventFormat::Message,
                Default::default(),
            );
        }
    }
}

#[test]
fn message_bom_positions_limits_and_errors_survive_pauses() {
    let wire = b"\xef\xbb\xbfX: y\r\n\r\n\xef\xbb\xbfbody";
    let mut begins = 0;
    let mut body = Vec::new();
    let mut parser = MimeEventParser::new(MimeEventFormat::Message);
    let mut collect = |event: MimeEvent<'_>| {
        match event {
            MimeEvent::MessageBegin { position } => {
                assert_eq!(position, MimePosition::default());
                begins += 1;
            }
            MimeEvent::EntityBegin { position, .. } => {
                assert_eq!(position, MimePosition { offset: 3, line: 1 });
                begins += 1;
            }
            MimeEvent::Content {
                bytes, position, ..
            } => {
                assert_eq!(
                    bytes,
                    &wire[position.offset as usize..position.offset as usize + bytes.len()]
                );
                body.extend_from_slice(bytes);
            }
            _ => {}
        }
        Ok(())
    };
    for byte in wire {
        parser
            .parse(std::slice::from_ref(byte), &mut collect)
            .unwrap();
    }
    parser.finish(&mut collect).unwrap();
    assert_eq!(begins, 2);
    assert_eq!(body, b"\xef\xbb\xbfbody");
    for limit in 1..=wire.len() {
        let options = MimeEntityParseOptions {
            max_input_bytes: limit,
            max_header_length: 6,
            ..Default::default()
        };
        assert_splits(wire, MimeEventFormat::Message, options);
        assert_eq!(
            sync(wire, MimeEventFormat::Message, options).error,
            (limit < wire.len()).then_some(io::ErrorKind::InvalidData)
        );
    }
    for (input, error) in [
        (b"\xef".as_slice(), io::ErrorKind::UnexpectedEof),
        (b"\xef\xbb", io::ErrorKind::UnexpectedEof),
        (b"\xef\xbb\xbf", io::ErrorKind::UnexpectedEof),
        (b"\xefx", io::ErrorKind::InvalidData),
        (b"\xef\xbbX: y\n\nbody", io::ErrorKind::InvalidData),
        (
            b"\xef\xbb\xbf\xef\xbb\xbfX: y\n\nbody",
            io::ErrorKind::InvalidData,
        ),
    ] {
        assert_splits(input, MimeEventFormat::Message, Default::default());
        assert_eq!(
            sync(input, MimeEventFormat::Message, Default::default()).error,
            Some(error)
        );
    }
    // Initialization belongs to the source, not each embedded message.
    let nested = b"\xef\xbb\xbfContent-Type: message/rfc822\n\n\xef\xbb\xbfX: y\n\nbody";
    assert_splits(nested, MimeEventFormat::Message, Default::default());
    assert_eq!(
        sync(nested, MimeEventFormat::Message, Default::default()).error,
        None
    );
    assert_splits(wire, MimeEventFormat::Entity, Default::default());
    assert_eq!(
        sync(wire, MimeEventFormat::Entity, Default::default()).error,
        Some(io::ErrorKind::InvalidData)
    );
}

#[test]
fn mbox_prefix_bom_content_length_and_reporting_budgets_survive_pauses() {
    let body = b"From protected\r\nbody\r\n";
    let wire = [
        b"\xef\xbb\xbfignored\r\nFrom first\r\nContent-Length: ".as_slice(),
        body.len().to_string().as_bytes(),
        b"\r\n\r\n",
        body,
        b"From second\r\nContent-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nlast\r\n",
    ]
    .concat();
    for respect_content_length in [false, true] {
        assert_splits(
            &wire,
            MimeEventFormat::Mbox,
            MimeEntityParseOptions {
                respect_content_length,
                ..Default::default()
            },
        );
    }
    for wire in [
        b"".as_slice(),
        b"\xef\xbb\xbf",
        b"\xef",
        b"\xef\xbb",
        b"\xefx",
        b"not a marker\n",
        b"From incomplete",
        b"From first\nFrom second\n\nbody",
        b"From first\n\nbody\r\nFr",
    ] {
        assert_splits(
            wire,
            MimeEventFormat::Mbox,
            MimeEntityParseOptions::default(),
        );
    }
}

#[test]
fn long_mbox_marker_limits_and_positions_survive_pauses() {
    for newline in ["\n", "\r\n"] {
        let marker = format!("From {}{newline}", "X".repeat(4092));
        let wire = format!("{marker}Subject: test{newline}{newline}body{newline}");
        for max_header_length in [marker.len() - 1, marker.len()] {
            let options = MimeEntityParseOptions {
                max_header_length,
                max_input_bytes: wire.len(),
                ..Default::default()
            };
            let expected = sync(wire.as_bytes(), MimeEventFormat::Mbox, options);
            assert_eq!(
                expected.error,
                (max_header_length < marker.len()).then_some(io::ErrorKind::InvalidData)
            );
            assert_eq!(
                expected.position.offset,
                if expected.error.is_some() {
                    max_header_length
                } else {
                    wire.len()
                } as u64
            );
            for split in [
                0,
                1,
                4,
                5,
                4095,
                4096,
                4097,
                marker.len() - 1,
                marker.len(),
                marker.len() + 1,
                wire.len(),
            ] {
                assert_eq!(
                    feed(
                        wire.as_bytes(),
                        MimeEventFormat::Mbox,
                        options,
                        &[split, wire.len() - split],
                    ),
                    expected,
                    "split {split}, marker length {max_header_length}"
                );
            }
            assert_eq!(
                feed(
                    wire.as_bytes(),
                    MimeEventFormat::Mbox,
                    options,
                    &vec![1; wire.len()],
                ),
                expected
            );
        }
    }
}

#[test]
fn exact_input_limits_distinguish_temporary_exhaustion_from_eof() {
    let wire = b"Subject: test\r\n\r\nbody\r";
    let options = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        ..Default::default()
    };
    assert_splits(wire, MimeEventFormat::Message, options);
    let expected = sync(wire, MimeEventFormat::Message, options);
    assert_eq!(expected.error, None);
    assert_splits(
        wire,
        MimeEventFormat::Message,
        MimeEntityParseOptions {
            max_input_bytes: wire.len() - 1,
            ..options
        },
    );

    let message = b"From sender\r\n\r\nbody\r\n";
    let archive = message.repeat(3);
    let options = MimeEntityParseOptions {
        max_input_bytes: message.len(),
        ..options
    };
    assert_splits(&archive, MimeEventFormat::Mbox, options);
    assert_eq!(sync(&archive, MimeEventFormat::Mbox, options).error, None);
    for suffix in [b"F".as_slice(), b"Fr", b"From", b"Fnope", b"x"] {
        let wire = [message.as_slice(), suffix].concat();
        assert_splits(&wire, MimeEventFormat::Mbox, options);
        assert_eq!(
            sync(&wire, MimeEventFormat::Mbox, options).error,
            Some(io::ErrorKind::InvalidData)
        );
    }
}

#[test]
fn long_lines_and_padded_boundary_candidates_remain_resumable() {
    for suffix in [b"\r\n".as_slice(), b"not-padding\r\n", b""] {
        let wire = [
            b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\n".as_slice(),
            &b"ordinary\r\n".repeat(900),
            &b"x".repeat(9000),
            b"\r\n--b--",
            &b" \t".repeat(5000),
            suffix,
        ]
        .concat();
        let options = MimeEntityParseOptions::default();
        let expected = sync(&wire, MimeEventFormat::Message, options);
        for size in [1, 2, 7, 4095, 4096, 4097, 8192] {
            let chunks: Vec<_> = wire.chunks(size).map(<[u8]>::len).collect();
            assert_eq!(
                feed(&wire, MimeEventFormat::Message, options, &chunks),
                expected,
                "size {size}"
            );
        }
    }
}

#[test]
fn callbacks_and_logger_failures_are_terminal_and_preserve_errors() {
    for wire in [
        b"Subject: test\r\n\r\nbody".as_slice(),
        b"\xef\xbb\xbfSubject: test\r\n\r\nbody",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody\r\n--b--\r\n",
    ] {
        let count = sync(wire, MimeEventFormat::Message, Default::default())
            .events
            .len();
        for fail_at in 0..count {
            let mut parser = MimeEventParser::new(MimeEventFormat::Message);
            let mut calls = 0;
            let mut callback = |_: MimeEvent<'_>| {
                let fail = calls == fail_at;
                calls += 1;
                if fail {
                    Err(io::Error::other("callback sentinel"))
                } else {
                    Ok(())
                }
            };
            let error = parser
                .parse(wire, &mut callback)
                .and_then(|_| parser.finish(&mut callback))
                .unwrap_err();
            assert_eq!(error.to_string(), "callback sentinel");
            assert!(!parser.is_finished());
            assert_eq!(
                parser
                    .parse(b"more", |_| panic!("poisoned"))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                parser.finish(|_| panic!("poisoned")).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
    let mut logger = |_: MimeComplianceIssue| Err(io::Error::other("logger sentinel"));
    let mut parser = MimeEventParser::with_logger(
        MimeEventFormat::Message,
        Default::default(),
        Default::default(),
        &mut logger,
    )
    .unwrap();
    let error = parser
        .parse(b"Subject: test\n\nbody", |_| Ok(()))
        .and_then(|_| parser.finish(|_| Ok(())))
        .unwrap_err();
    assert_eq!(error.to_string(), "logger sentinel");
    assert_eq!(
        parser.finish(|_| panic!("poisoned")).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn ordinary_body_borrows_supplied_input_and_eof_rejects_later_input() {
    let mut parser = MimeEventParser::new(MimeEventFormat::Entity);
    parser.parse(b"\r\n", |_| Ok(())).unwrap();
    let input = b"x".repeat(16000);
    let mut borrowed = 0;
    parser
        .parse(&input, |event| {
            if let MimeEvent::Content { bytes, .. } = event {
                let range = input.as_ptr_range();
                if range.contains(&bytes.as_ptr()) {
                    borrowed += bytes.len();
                }
            }
            Ok(())
        })
        .unwrap();
    assert!(
        borrowed >= input.len() - 4096,
        "ordinary content should not be buffered"
    );
    parser.finish(|_| Ok(())).unwrap();
    assert_eq!(
        parser
            .parse(b"extra", |_| panic!("finished"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn invalid_limits_are_rejected_before_parsing() {
    for options in [
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
    ] {
        assert!(
            matches!(MimeEventParser::with_options(MimeEventFormat::Message, options),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
    }
}

#[test]
fn parser_can_move_between_threads_between_input_chunks() {
    let mut parser = MimeEventParser::new(MimeEventFormat::Message);
    parser.parse(b"Subject: resumable\r", |_| Ok(())).unwrap();
    let mut parser = std::thread::spawn(move || {
        parser.parse(b"\n\r\nbody", |_| Ok(())).unwrap();
        parser
    })
    .join()
    .unwrap();
    parser.finish(|_| Ok(())).unwrap();
    assert!(parser.is_finished());

    fn assert_send<T: Send>(_: &T) {}
    let mut logger = |_: MimeComplianceIssue| Ok(());
    let parser = MimeEventParser::with_logger(
        MimeEventFormat::Message,
        Default::default(),
        Default::default(),
        &mut logger,
    )
    .unwrap();
    assert_send(&parser);
}

#[test]
fn local_logger_budgets_reset_per_message_not_per_feed() {
    use mimecraft::MimeComplianceViolation;
    use std::{cell::RefCell, rc::Rc};

    let issues = Rc::new(RefCell::new(Vec::new()));
    let logged = Rc::clone(&issues);
    let mut logger = move |issue: MimeComplianceIssue| {
        logged.borrow_mut().push(issue);
        Ok(())
    };
    let mut parser = MimeEventParser::with_logger(
        MimeEventFormat::Mbox,
        Default::default(),
        MimeComplianceOptions {
            max_issues_per_violation: 1,
            ..Default::default()
        },
        &mut logger,
    )
    .unwrap();
    let wire = b"From sender\r\nContent-Type: text/plain\r\n\
Content-Type: text/plain\r\nContent-Type: text/plain\r\n\r\nbody\r\n"
        .repeat(3);
    for chunk in wire.chunks(1) {
        parser.parse(chunk, |_| Ok(())).unwrap();
    }
    parser.finish(|_| Ok(())).unwrap();
    for violation in [
        MimeComplianceViolation::RepeatedContentType,
        MimeComplianceViolation::TooManyComplianceIssues,
    ] {
        assert_eq!(
            issues
                .borrow()
                .iter()
                .filter(|issue| issue.violation == violation)
                .count(),
            3
        );
    }
}

#[test]
fn eof_callback_failure_does_not_publish_success() {
    let mut parser = MimeEventParser::new(MimeEventFormat::Message);
    parser.parse(b"Subject: pending", |_| Ok(())).unwrap();
    let error = parser
        .finish(|event| {
            if matches!(event, MimeEvent::MessageEnd { .. }) {
                Err(io::Error::other("end sentinel"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "end sentinel");
    assert!(!parser.is_finished());
    assert_eq!(
        parser.finish(|_| panic!("failed")).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

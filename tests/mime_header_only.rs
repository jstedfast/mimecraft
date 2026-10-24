// Adapted from MimeKit's MimeReader EOF handling and MimeParser construction.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};

use mimecraft::{
    ContentEncoding, HeaderTooLong, MimeEntity, MimeEntityKind, MimeEntityParseOptions, MimeEvent,
    MimeMessage, MimeParser, MimeReader,
};

fn options() -> MimeEntityParseOptions {
    MimeEntityParseOptions {
        allow_header_only: true,
        ..Default::default()
    }
}

#[test]
fn eof_headers_recover_by_default_and_keep_raw_records_without_content() {
    assert!(MimeEntityParseOptions::default().allow_header_only);
    for (wire, raw) in [
        (b"Subject: value".as_slice(), b"Subject: value".as_slice()),
        (b"Subject: value\r\n", b"Subject: value\r\n"),
        (b"Subject: value\n\tfold", b"Subject: value\n\tfold"),
        (b"Subject: value\r", b"Subject: value\r"),
        (b"Subject: value\r\n\r", b"Subject: value\r\n"),
    ] {
        for capacity in [1, 2, 3, 7, 4096] {
            let source = BufReader::with_capacity(capacity, wire);
            assert!(MimeParser::new(source).parse_entity().is_ok());
            let source = BufReader::with_capacity(capacity, wire);
            assert!(MimeParser::new(source).parse_message().is_ok());
            let source = BufReader::with_capacity(capacity, wire);
            let mut parser = MimeParser::with_options(source, options()).unwrap();
            let MimeEntity::Part(part) = parser.parse_entity().unwrap() else {
                panic!("header-only input without structural metadata is a leaf");
            };
            assert_eq!(part.headers().len(), 1);
            assert_eq!(part.headers().as_slice()[0].raw(), raw);
            assert_eq!(part.headers().as_slice()[0].offset(), Some(0));
            assert!(part.content().get_ref().get_ref().is_empty());
            assert_eq!(part.content().get_ref().position(), 0);
            assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
            assert_eq!(part.content().new_line_format(), None);
            assert_eq!(parser.position().offset, wire.len() as u64);
            assert!(parser.is_end_of_stream());
        }
        let mut reader = MimeReader::with_options(wire, options()).unwrap();
        let mut count = 0;
        let end_line = 1 + wire.iter().filter(|&&byte| byte == b'\n').count() as u64;
        reader
            .read_message(|event| {
                match event {
                    MimeEvent::MessageBegin { position } => {
                        assert_eq!(position.offset, 0);
                        assert_eq!(position.line, 1);
                    }
                    MimeEvent::EntityBegin { headers_end, .. } => {
                        assert_eq!(headers_end.offset, wire.len() as u64);
                        assert_eq!(headers_end.line, end_line);
                    }
                    MimeEvent::EntityEnd { position, .. } | MimeEvent::MessageEnd { position } => {
                        assert_eq!(position.offset, wire.len() as u64);
                        assert_eq!(position.line, end_line);
                    }
                    _ => panic!("no content or standalone header event"),
                }
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 4);
        assert!(reader.is_end_of_stream());
    }
}

#[test]
fn header_only_messages_partition_and_interleave_headers_without_synthesizing_metadata() {
    let wire =
        b"Subject: original\r\nContent-Type: text/plain\r\nX-After: kept\r\nSubject: duplicate";
    let mut message = MimeMessage::load_with_options(&mut wire.as_slice(), &options()).unwrap();
    assert_eq!(message.headers().len(), 3);
    assert_eq!(message.headers().get_all("Subject").count(), 2);
    let MimeEntity::Part(body) = message.body().unwrap() else {
        panic!("expected leaf");
    };
    assert_eq!(body.headers().len(), 1);
    assert_eq!(
        body.headers().get("Content-Type").unwrap().offset(),
        Some(b"Subject: original\r\n".len() as u64)
    );
    assert!(body.content().get_ref().get_ref().is_empty());
    assert!(!message.headers().contains("MIME-Version"));
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    // Reading does not invent a separator, but the model's writer emits normal
    // framing so the serialized result can be loaded by the strict parser.
    assert_eq!(output, [wire.as_slice(), b"\r\n\r\n"].concat());
    MimeMessage::load(&mut output.as_slice()).unwrap();
}

#[test]
fn embedded_header_only_messages_emit_nested_events_with_original_offsets() {
    for media_type in ["message/rfc822", "message/global", "text/rfc822-headers"] {
        let prefix = format!("Content-Type: {media_type}\r\n\r\n");
        let wire = format!("{prefix}Subject: inner\r\nContent-Type: text/plain");
        let mut reader =
            MimeReader::with_options(BufReader::with_capacity(1, wire.as_bytes()), options())
                .unwrap();
        let mut events = Vec::new();
        reader
            .read_entity(|event| {
                match event {
                    MimeEvent::EntityBegin {
                        kind,
                        position,
                        headers_end,
                        ..
                    } => match kind {
                        MimeEntityKind::MessagePart => events.push("wrapper+"),
                        MimeEntityKind::Part => {
                            assert_eq!(position.offset, prefix.len() as u64);
                            assert_eq!(headers_end.offset, wire.len() as u64);
                            events.push("part+");
                        }
                        _ => panic!("no multipart"),
                    },
                    MimeEvent::EntityEnd {
                        kind,
                        position,
                        new_line_format,
                    } => {
                        assert_eq!(position.offset, wire.len() as u64);
                        assert_eq!(new_line_format, None);
                        events.push(if kind == MimeEntityKind::Part {
                            "part-"
                        } else {
                            "wrapper-"
                        });
                    }
                    MimeEvent::MessageBegin { position } => {
                        assert_eq!(position.offset, prefix.len() as u64);
                        events.push("message+");
                    }
                    MimeEvent::MessageEnd { position } => {
                        assert_eq!(position.offset, wire.len() as u64);
                        events.push("message-");
                    }
                    _ => panic!("no content or standalone header event"),
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(
            events,
            [
                "wrapper+", "message+", "part+", "part-", "message-", "wrapper-"
            ]
        );
        let MimeEntity::Message(part) =
            MimeEntity::load_with_options(&mut wire.as_bytes(), &options()).unwrap()
        else {
            panic!("expected a message wrapper");
        };
        let message = part.message().unwrap();
        assert_eq!(
            message.headers().get("Subject").unwrap().offset(),
            Some(prefix.len() as u64)
        );
        assert_eq!(
            message
                .leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            b""
        );
    }
}

#[test]
fn header_only_wrapper_classification_and_stored_encoding_are_preserved() {
    let wire = b"Content-Type: message/rfc822";
    let MimeEntity::Message(part) =
        MimeEntity::load_with_options(&mut wire.as_slice(), &options()).unwrap()
    else {
        panic!("transparent wrapper expected");
    };
    assert!(part.message().unwrap().body().is_some());
    let opaque = MimeEntityParseOptions {
        parse_message_parts: false,
        ..options()
    };
    assert!(matches!(
        MimeEntity::load_with_options(&mut wire.as_slice(), &opaque).unwrap(),
        MimeEntity::Part(_)
    ));
    for (label, encoding) in [
        ("base64", ContentEncoding::Base64),
        ("quoted-printable", ContentEncoding::QuotedPrintable),
        ("x-uuencode", ContentEncoding::UuEncode),
    ] {
        let wire = format!("Content-Type: message/rfc822\r\nContent-Transfer-Encoding: {label}");
        let MimeEntity::Part(part) =
            MimeEntity::load_with_options(&mut wire.as_bytes(), &options()).unwrap()
        else {
            panic!("encoded wrappers remain opaque");
        };
        assert_eq!(part.content().encoding(), encoding);
        assert!(part.content().get_ref().get_ref().is_empty());
    }
}

#[test]
fn eof_recovery_retains_metadata_and_framing_but_preserves_limits() {
    assert_eq!(
        MimeEntity::load(&mut b"".as_slice()).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    for mut wire in [
        b"\r".as_slice(),
        b"Content-Type: invalid",
        b"Content-Transfer-Encoding: invalid",
        b"Content-Type: multipart/mixed",
        b"Content-Type: multipart/mixed; boundary=\"\"",
        b"Content-Type: multipart/mixed; boundary=b\r\nContent-Transfer-Encoding: base64",
        b"Content-Type: multipart/mixed; boundary=b",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nSubject: incomplete",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nSubject: incomplete\r\n--b--\r\n",
    ] {
        MimeEntity::load_with_options(&mut wire, &options()).unwrap();
    }
    let wire = b"Content-Type: message/rfc822\r\n\r\nSubject: inner";
    let opts = MimeEntityParseOptions {
        max_depth: 1,
        ..options()
    };
    assert_eq!(
        MimeEntity::load_with_options(&mut wire.as_slice(), &opts)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let opts = MimeEntityParseOptions {
        max_depth: 2,
        ..opts
    };
    MimeEntity::load_with_options(&mut wire.as_slice(), &opts).unwrap();
}

#[test]
fn header_only_loading_does_not_turn_resource_limits_into_eof() {
    let wire = b"Subject: value\r\n\tfold";
    let opts = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        max_header_length: wire.len(),
        ..options()
    };
    MimeEntity::load_with_options(&mut wire.as_slice(), &opts).unwrap();
    let limited = MimeEntityParseOptions {
        max_input_bytes: wire.len() - 1,
        ..opts
    };
    assert_eq!(
        MimeEntity::load_with_options(&mut wire.as_slice(), &limited)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let limited = MimeEntityParseOptions {
        max_header_length: wire.len() - 1,
        ..opts
    };
    let error = MimeEntity::load_with_options(&mut wire.as_slice(), &limited).unwrap_err();
    assert!(error.get_ref().unwrap().is::<HeaderTooLong>());
}

struct FailingEof<'a>(&'a [u8]);
impl Read for FailingEof<'_> {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("use buffered reading");
    }
}
impl BufRead for FailingEof<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.0.is_empty() {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "source failure",
            ))
        } else {
            Ok(self.0)
        }
    }
    fn consume(&mut self, amount: usize) {
        self.0 = &self.0[amount..];
    }
}

#[test]
fn source_and_callback_failures_are_not_recovered_as_header_only_successes() {
    let wire = b"Subject: value".as_slice();
    let mut parser = MimeParser::with_options(FailingEof(wire), options()).unwrap();
    assert_eq!(
        parser.parse_message().unwrap_err().to_string(),
        "source failure"
    );
    assert!(!parser.is_end_of_stream());
    let mut reader = MimeReader::with_options(wire, options()).unwrap();
    let error = reader
        .read_entity(|_| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "callback failure",
            ))
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert!(!reader.is_end_of_stream());
    assert_eq!(
        reader.read_entity(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let _old = reader.replace_source(wire);
    assert!(reader.options().allow_header_only);
    reader.read_entity(|_| Ok(())).unwrap();
    assert!(reader.is_end_of_stream());
}

#[test]
fn complete_bodies_and_standalone_headers_keep_their_existing_behavior() {
    let wire = b"Subject: value\r\n\r\nbody";
    let mut message = MimeMessage::load_with_options(&mut wire.as_slice(), &options()).unwrap();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, wire);
    for allow_header_only in [false, true] {
        let opts = MimeEntityParseOptions {
            allow_header_only,
            ..options()
        };
        let mut parser = MimeParser::with_options(b"Subject: value".as_slice(), opts).unwrap();
        assert_eq!(parser.parse_headers().unwrap().len(), 1);
        let mut parser = MimeParser::with_options(b"\nX: a\n\nY: b".as_slice(), opts).unwrap();
        assert!(parser.parse_status_group().unwrap().unwrap().contains("X"));
        assert!(parser.parse_status_group().unwrap().unwrap().contains("Y"));
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

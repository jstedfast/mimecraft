// Adapted from MimeKit's TextRfc822HeadersTests.cs, ParserOptions.cs, and MimeWriter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, ContentType, Header, HeaderFormatOptions, HeaderList, MessagePart,
    MessagePartError, MimeContent, MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart,
    MimePartWriteOptions, Multipart, TextPart, TextRfc822Headers, TextRfc822HeadersError,
    encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn write(entity: &mut MimeEntity, format: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    entity
        .write_to_with_options(
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
fn construction_and_ownership_do_not_require_source_traits() {
    let mut returned: TextRfc822Headers<()> = TextRfc822Headers::default();
    assert!(returned.message_part().message().is_none());
    assert_eq!(
        returned.message_part().headers(),
        &headers(b"Content-Type: text/rfc822-headers\r\n"),
    );
    let message = MimeMessage::from_parts(headers(b"Subject: original\r\n"), None);
    assert!(
        returned
            .message_part_mut()
            .set_message(Some(message))
            .is_none()
    );
    let old = returned.message_part_mut().set_message(None).unwrap();
    let returned = TextRfc822Headers::with_message(old);
    let MimeEntity::Message(part): MimeEntity<()> = returned.into() else {
        panic!()
    };
    assert_eq!(
        part.message().unwrap().headers(),
        &headers(b"Subject: original\r\n")
    );
    assert!(part.message().unwrap().body().is_none());
    assert_eq!(part.leaf_parts().count(), 0);
}

#[test]
fn wrapping_checks_only_first_media_type_without_repairing_raw_headers() {
    for media_type in ["TeXt/RFC822-HEADERS", "MESSAGE/GLOBAL-HEADERS"] {
        let raw = format!(
            "X-Keep: first\nContent-Type: {media_type}; x=keep\nContent-Type: invalid\nBroken\n"
        );
        let part: MessagePart<()> = MessagePart::from_parts(headers(raw.as_bytes()), None);
        let pointer = part.headers().as_slice()[0].raw().as_ptr();
        let returned = TextRfc822Headers::from_message_part(part).unwrap();
        assert_eq!(returned.message_part().headers(), &headers(raw.as_bytes()));
        assert_eq!(
            returned.into_message_part().headers().as_slice()[0]
                .raw()
                .as_ptr(),
            pointer
        );
    }
    for value in [
        None,
        Some("invalid"),
        Some("text/plain"),
        Some("message/rfc822"),
        Some("multipart/mixed"),
    ] {
        let mut raw = HeaderList::new();
        if let Some(value) = value {
            raw.add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
            raw.add(Header::parse(b"Content-Type: text/rfc822-headers").unwrap());
        }
        let error = TextRfc822Headers::from_message_part(MessagePart::<()>::from_parts(raw, None))
            .unwrap_err();
        match value {
            None => {
                assert!(error.source().is_none());
                assert!(matches!(error, TextRfc822HeadersError::MissingContentType));
            }
            Some("invalid") => {
                assert!(error.source().is_some());
                assert!(matches!(error, TextRfc822HeadersError::ContentType(_)));
            }
            Some(expected) => {
                assert!(error.source().is_none());
                assert!(
                    matches!(error, TextRfc822HeadersError::NotHeaders(value) if value == expected)
                );
            }
        }
    }
}

#[test]
fn canonical_returned_headers_round_trip_with_wrapper_content_id() {
    let original = headers(
        b"From: Sender Name <sender@example.com>\r\n\
To: Recipient Name <recipient@example.com>\r\n\
Subject: Content of a text/rfc822-headers part\r\n",
    );
    let mut returned =
        TextRfc822Headers::with_message(MimeMessage::from_parts(original.clone(), None));
    returned
        .message_part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Id: <id@localhost>").unwrap());
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"From: Postmaster <postmaster@example.com>\r\nSubject: Sorry, but your message bounced\r\n"),
        Some(returned.into()),
    );
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    let parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
    assert!(parsed.headers().contains("Subject"));
    assert!(!parsed.headers().contains("Content-Id"));
    let (_, Some(MimeEntity::Message(part))) = parsed.into_parts() else {
        panic!()
    };
    let returned = TextRfc822Headers::from_message_part(part).unwrap();
    assert_eq!(
        returned
            .message_part()
            .headers()
            .get("Content-Id")
            .unwrap()
            .raw(),
        b"Content-Id: <id@localhost>\r\n"
    );
    assert_eq!(
        returned.message_part().message().unwrap().headers(),
        &original
    );
}

#[test]
fn transparent_header_types_share_message_parsing_and_header_partitioning() {
    for media_type in ["TEXT/RFC822-HEADERS", "MESSAGE/GLOBAL-HEADERS"] {
        for encoding in ["", "7bit", "8bit", "binary"] {
            let cte = if encoding.is_empty() {
                String::new()
            } else {
                format!("Content-Transfer-Encoding: {encoding}\r\n")
            };
            let wire = format!(
                "Content-Type: {media_type}\r\n{cte}\r\nSubject: original\r\nContent-Type: text/plain; charset=utf-8\r\nX: first\r\nX: second\r\n\r\nbody"
            );
            let MimeEntity::Message(part) = MimeEntity::load(&mut wire.as_bytes()).unwrap() else {
                panic!("transparent returned headers must be structured");
            };
            let returned = TextRfc822Headers::from_message_part(part).unwrap();
            let inner = returned.message_part().message().unwrap();
            assert_eq!(inner.headers().get_all("X").count(), 2);
            assert!(!inner.headers().contains("Content-Type"));
            let leaf = inner.leaf_parts().next().unwrap();
            assert!(
                leaf.content_type()
                    .unwrap()
                    .unwrap()
                    .is_mime_type("text", "plain")
            );
            assert_eq!(leaf.content().get_ref().get_ref(), b"body");
            assert_eq!(leaf.content().get_ref().position(), 0);
        }
    }
}

#[test]
fn absent_and_present_empty_messages_round_trip_at_eof_and_report_boundaries() {
    for present in [false, true] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let mut returned: TextRfc822Headers = TextRfc822Headers::new();
            if present {
                returned
                    .message_part_mut()
                    .set_message(Some(MimeMessage::new()));
            }
            let mut entity: MimeEntity = returned.into();
            let wire = write(&mut entity, format);
            let mut parsed = MimeEntity::load(&mut wire.as_slice()).unwrap();
            let MimeEntity::Message(part) = &parsed else {
                panic!()
            };
            assert!(part.message().is_some());
            // Recovery can create a message without inventing its wire separator.
            assert_eq!(write(&mut parsed, format), wire);

            let mut report: Multipart = Multipart::with_boundary("report", "b").unwrap();
            report.children_mut().push(entity);
            report
                .children_mut()
                .push(TextPart::new("plain", "sibling").unwrap().into());
            let mut entity: MimeEntity = report.into();
            let wire = write(&mut entity, format);
            let mut parsed = MimeEntity::load(&mut wire.as_slice()).unwrap();
            let MimeEntity::Multipart(part) = &parsed else {
                panic!()
            };
            let MimeEntity::Message(returned) = &part.children()[0] else {
                panic!()
            };
            assert_eq!(returned.message().is_some(), present);
            assert_eq!(part.children().len(), 2);
            assert_eq!(write(&mut parsed, format), wire);
        }
    }
}

#[test]
fn recovered_inner_headers_can_instead_be_preserved_as_opaque_content() {
    for media_type in ["text/rfc822-headers", "message/global-headers"] {
        let wire = format!("Content-Type: {media_type}\r\n\r\nSubject: no separator");
        let MimeEntity::Message(part) = MimeEntity::load(&mut wire.as_bytes()).unwrap() else {
            panic!("message wrapper expected")
        };
        assert!(part.message().unwrap().headers().contains("Subject"));
        let MimeEntity::Part(part) = MimeEntity::load_with_options(
            &mut wire.as_bytes(),
            &MimeEntityParseOptions {
                parse_message_parts: false,
                max_depth: 1,
                ..Default::default()
            },
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(part.content().get_ref().get_ref(), b"Subject: no separator");
    }
}

#[test]
fn structured_loading_shares_input_header_and_depth_limits() {
    let record = format!("Subject: {}\r\n", "a".repeat(100));
    let wire = format!("Content-Type: text/rfc822-headers\r\n\r\n{record}\r\n");
    let mut options = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        max_header_length: record.len(),
        max_depth: 2,
        ..Default::default()
    };
    assert!(MimeEntity::load_with_options(&mut wire.as_bytes(), &options).is_ok());
    options.max_input_bytes -= 1;
    assert_eq!(
        MimeEntity::load_with_options(&mut wire.as_bytes(), &options)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    options.max_input_bytes += 1;
    options.max_header_length -= 1;
    let error = MimeEntity::load_with_options(&mut wire.as_bytes(), &options).unwrap_err();
    assert!(error.get_ref().unwrap().is::<mimecraft::HeaderTooLong>());
    options.max_header_length += 1;
    options.max_depth = 1;
    assert_eq!(
        MimeEntity::load_with_options(&mut wire.as_bytes(), &options)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn checked_metadata_edits_support_header_type_and_remain_atomic() {
    let mut part: MessagePart = MessagePart::new(MimeMessage::new());
    part.headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    let content_type = ContentType::new("text", "rfc822-headers").unwrap();
    let before = part.headers().clone();
    assert!(matches!(
        part.set_content_type_with_options(
            &content_type,
            &HeaderFormatOptions {
                new_line_format: NewLineFormat::Mixed,
                ..Default::default()
            }
        ),
        Err(MessagePartError::HeaderFormat(_))
    ));
    assert_eq!(part.headers(), &before);
    part.set_content_type(&content_type).unwrap();
    assert_eq!(part.headers().get_all("Content-Type").count(), 1);
    assert!(TextRfc822Headers::from_message_part(part).is_ok());
}

#[test]
fn writer_rechecks_raw_type_and_encoding_before_emitting_wrapper_headers() {
    for invalid in [
        "Content-Type: text/plain",
        "Content-Type: invalid",
        "Content-Transfer-Encoding: base64",
    ] {
        let mut returned: TextRfc822Headers = TextRfc822Headers::new();
        returned
            .message_part_mut()
            .headers_mut()
            .replace_all(Header::parse(invalid.as_bytes()).unwrap())
            .unwrap();
        let mut output = Vec::new();
        assert_eq!(
            returned
                .message_part_mut()
                .write_to(&mut output)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.is_empty());
    }
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("metadata operations must not read message bodies");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("metadata operations must not seek message bodies");
    }
}

#[test]
fn wrapper_preserves_borrowed_body_ownership_and_traversal_without_io() {
    let mut source = NoIo;
    let leaf = MimePart::new(MimeContent::new(&mut source));
    let message = MimeMessage::with_body(leaf.into());
    let returned = TextRfc822Headers::with_message(message);
    assert_eq!(returned.message_part().leaf_parts().count(), 1);
    let returned = TextRfc822Headers::from_message_part(returned.into_message_part()).unwrap();
    let entity: MimeEntity<_> = returned.into();
    assert_eq!(entity.body_parts().count(), 1);
    assert_eq!(entity.leaf_parts().count(), 1);
}

#[test]
fn serialization_retains_existing_body_instead_of_silently_stripping_it() {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"\0body\r".to_vec())));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    let mut source: MimeEntity =
        TextRfc822Headers::with_message(MimeMessage::with_body(part.into())).into();
    let wire = write(&mut source, NewLineFormat::Unix);
    assert!(wire.ends_with(b"\0body\r"));
    let parsed = MimeEntity::load(&mut wire.as_slice()).unwrap();
    assert_eq!(
        parsed
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref(),
        b"\0body\r"
    );
}

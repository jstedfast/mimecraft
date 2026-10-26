// Adapted from MimeKit's UnitTests/MessagePartialTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashSet, error::Error, io::Cursor};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MessageId, MessagePart, MessagePartial,
    MessagePartialError, MimeContent, MimeEntity, MimeMessage, MimePart, MimePartWriteOptions,
    Multipart, TextPart, encodings::NewLineFormat,
};

const ORIGINAL: &[u8] = include_bytes!("data/partial/message-partial.eml");

fn load(bytes: &[u8]) -> MimeMessage {
    MimeMessage::load(&mut &bytes[..]).unwrap()
}

fn write(message: &mut MimeMessage) -> Vec<u8> {
    let mut bytes = Vec::new();
    message.write_to(&mut bytes).unwrap();
    bytes
}

fn partials(messages: Vec<MimeMessage>) -> (HeaderList, Vec<MessagePartial>) {
    let headers = messages[0].headers().clone();
    let parts = messages
        .into_iter()
        .map(|message| {
            let (_, Some(MimeEntity::Part(part))) = message.into_parts() else {
                panic!("expected an opaque message/partial body")
            };
            MessagePartial::from_part(part).unwrap()
        })
        .collect();
    (headers, parts)
}

fn stored(part: &MessagePartial) -> &[u8] {
    part.part().content().get_ref().get_ref()
}

#[test]
fn upstream_photo_splits_into_eleven_fragments_and_reassembles() {
    let mut original = load(ORIGINAL);
    let expected = write(&mut original);
    let original_id = original.message_id().unwrap().unwrap();
    let messages = MessagePartial::split(original, 16 * 1024).unwrap();
    assert_eq!(messages.len(), 11);
    let mut ids = HashSet::new();
    for message in &messages {
        let id = message.message_id().unwrap().unwrap();
        assert_ne!(id, original_id);
        assert!(
            id.as_str()
                .ends_with(&format!("@{}", MessageId::default_domain()))
        );
        assert!(ids.insert(id));
        assert!(
            message
                .headers()
                .iter()
                .all(|header| header.offset().is_none())
        );
        assert_eq!(
            message.subject().unwrap().text,
            "Photo of a girl with feather earrings"
        );
        assert!(!message.headers().iter().any(Header::is_content_header));
    }
    let (enclosing, mut parts) = partials(messages);
    for (index, part) in parts.iter().enumerate() {
        assert_eq!(part.number().unwrap(), Some(index as u32 + 1));
        assert_eq!(part.total().unwrap(), Some(11));
        assert_eq!(part.id().unwrap().as_deref(), Some(original_id.as_str()));
        assert!(stored(part).len() <= 16 * 1024);
        assert!(!stored(part).is_empty());
        assert_eq!(
            part.part().content().encoding(),
            ContentEncoding::Unspecified
        );
        assert_eq!(part.part().content().get_ref().position(), 0);
        assert!(!part.part().headers().contains("Content-Transfer-Encoding"));
    }
    parts.reverse();
    let mut combined = MessagePartial::join(&enclosing, &mut parts)
        .unwrap()
        .unwrap();
    assert_eq!(write(&mut combined), expected);
}

#[test]
fn ordinary_fragment_message_wire_round_trips_preserve_the_original() {
    let mut original = load(ORIGINAL);
    let expected = write(&mut original);
    let messages = MessagePartial::split(original, 16 * 1024).unwrap();
    let reloaded = messages
        .into_iter()
        .map(|mut message| load(&write(&mut message)))
        .collect();
    let (headers, mut parts) = partials(reloaded);
    let mut joined = MessagePartial::join(&headers, &mut parts).unwrap().unwrap();
    assert_eq!(write(&mut joined), expected);
}

#[test]
fn upstream_small_message_returns_original_storage_headers_and_positions() {
    let mut part = TextPart::new("plain", "This message should not need to be split.")
        .unwrap()
        .into_part();
    part.content_mut().get_mut().set_position(u64::MAX);
    let bytes_pointer = part.content().get_ref().get_ref().as_ptr();
    let body_headers = part.headers().clone();
    let mut message = MimeMessage::with_body(part.into());
    message
        .headers_mut()
        .add(Header::parse(b"From: Sender <sender@example.com>").unwrap());
    message
        .headers_mut()
        .add(Header::parse(b"To: Recipient <recipient@example.com>").unwrap());
    message.set_subject("Small message").unwrap();
    let headers = message.headers().clone();
    let mut messages = MessagePartial::split(message, 1024 * 1024).unwrap();
    assert_eq!(messages.len(), 1);
    let message = messages.pop().unwrap();
    assert_eq!(message.headers(), &headers);
    assert!(message.message_id().unwrap().is_none());
    let Some(MimeEntity::Part(part)) = message.body() else {
        panic!("original text body")
    };
    assert_eq!(part.headers(), &body_headers);
    assert_eq!(part.content().get_ref().get_ref().as_ptr(), bytes_pointer);
    assert_eq!(part.content().get_ref().position(), u64::MAX);
}

#[test]
fn small_message_keeps_nested_and_truncated_parsed_framing() {
    for wire in [
        b"Subject: only".as_slice(),
        b"Content-Type: text/plain\r\n",
        b"Content-Type: message/rfc822\r\n\r\nSubject: no separator",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b \t\n\
Content-Type: message/rfc822\r\n\r\nContent-Type: text/plain\r\n\r\ninside\r\n--b--",
    ] {
        let mut original = load(wire);
        let expected = write(&mut original);
        let headers = original.headers().clone();
        let pointers: Vec<_> = original
            .leaf_parts()
            .map(|part| part.content().get_ref().get_ref().as_ptr())
            .collect();
        let mut result = MessagePartial::split(original, usize::MAX)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(result.headers(), &headers);
        assert_eq!(
            result
                .leaf_parts()
                .map(|part| part.content().get_ref().get_ref().as_ptr())
                .collect::<Vec<_>>(),
            pointers
        );
        assert_eq!(write(&mut result), expected);
    }
}

#[test]
fn small_message_restores_distinct_positions_across_nested_sources() {
    fn leaf(position: u64) -> MimeEntity {
        let mut source = Cursor::new(b"hello".to_vec());
        source.set_position(position);
        MimePart::new(MimeContent::new(source)).into()
    }
    let mut mixed = Multipart::with_boundary("mixed", "b").unwrap();
    mixed.children_mut().push(leaf(2));
    mixed
        .children_mut()
        .push(MessagePart::new(MimeMessage::with_body(leaf(u64::MAX))).into());
    mixed.children_mut().push(leaf(4));
    let message = MimeMessage::with_body(mixed.into());
    let pointers: Vec<_> = message
        .leaf_parts()
        .map(|part| part.content().get_ref().get_ref().as_ptr())
        .collect();
    let result = MessagePartial::split(message, usize::MAX)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        result
            .leaf_parts()
            .map(|part| part.content().get_ref().position())
            .collect::<Vec<_>>(),
        [2, u64::MAX, 4]
    );
    assert_eq!(
        result
            .leaf_parts()
            .map(|part| part.content().get_ref().get_ref().as_ptr())
            .collect::<Vec<_>>(),
        pointers
    );
}

#[test]
fn filtered_size_threshold_is_inclusive_and_ignores_outer_headers() {
    let enclosed = b"Subject: original\r\nContent-Type: text/plain\r\n\r\nabc";
    let wire = [
        b"From: sender@example.com\r\nX-Outer: excluded\r\n".as_slice(),
        enclosed,
    ]
    .concat();
    for limit in [enclosed.len(), enclosed.len() + 1, usize::MAX] {
        let mut result = MessagePartial::split(load(&wire), limit).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(write(&mut result[0]), wire);
    }
    let (_, parts) = partials(MessagePartial::split(load(&wire), enclosed.len() - 1).unwrap());
    assert!(parts.len() > 1);
    assert_eq!(
        parts.iter().flat_map(stored).copied().collect::<Vec<_>>(),
        enclosed
    );
}

#[test]
fn every_small_size_retains_exact_bytes_and_never_exceeds_the_limit() {
    for body in [
        b"abc\r\ndefghijklmnop\r\nlast".as_slice(),
        b"\0\xffbinary\r\n",
        b"\r\n\r\n",
    ] {
        let wire = [b"Subject: original\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: binary\r\n\r\n".as_slice(), body].concat();
        for limit in 1..wire.len() {
            let messages = MessagePartial::split(load(&wire), limit).unwrap();
            let (headers, mut parts) = partials(messages);
            for part in &parts {
                assert!((1..=limit).contains(&stored(part).len()), "limit={limit}");
            }
            assert_eq!(
                parts.iter().flat_map(stored).copied().collect::<Vec<_>>(),
                wire,
                "limit={limit}"
            );
            if limit > 1 {
                for pair in parts.windows(2) {
                    assert!(
                        !(stored(&pair[0]).ends_with(b"\r") && stored(&pair[1]).starts_with(b"\n")),
                        "limit={limit}"
                    );
                }
            }
            let mut joined = MessagePartial::join(&headers, &mut parts).unwrap().unwrap();
            assert_eq!(write(&mut joined), wire, "limit={limit}");
        }
    }
}

#[test]
fn line_scan_prefers_complete_lines_without_including_lf_past_the_limit() {
    let options = MimePartWriteOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    // Including the empty header block, LF lies exactly one byte past the
    // second size boundary. MimeKit's scan can exceed maxSize in this case.
    let message = MimeMessage::with_body(
        MimePart::from_parts(
            HeaderList::new(),
            MimeContent::new(Cursor::new(b"abcdefg\nlast".to_vec())),
        )
        .into(),
    );
    let (_, parts) = partials(MessagePartial::split_with_options(message, 7, &options).unwrap());
    assert_eq!(
        parts.iter().map(stored).collect::<Vec<_>>(),
        [b"\n".as_slice(), b"abcdefg", b"\nlast"]
    );
}

#[test]
fn explicit_writer_options_control_the_enclosed_serialization() {
    let wire = b"Subject: original\r\nContent-Type: text/plain\r\n\r\nline\r\nlast";
    let options = MimePartWriteOptions {
        new_line_format: NewLineFormat::Unix,
        ensure_new_line: true,
        ..Default::default()
    };
    let mut expected = Vec::new();
    load(wire)
        .write_to_with_options(&mut expected, &options)
        .unwrap();
    let (headers, mut parts) =
        partials(MessagePartial::split_with_options(load(wire), 20, &options).unwrap());
    assert_eq!(
        parts.iter().flat_map(stored).copied().collect::<Vec<_>>(),
        expected
    );
    let mut joined = MessagePartial::join(&headers, &mut parts).unwrap().unwrap();
    let mut actual = Vec::new();
    joined.write_to_with_options(&mut actual, &options).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn root_header_filtering_preserves_duplicates_extensions_and_nested_messages() {
    let wire = b"From: outer@example.com\r\nX-Outer: first\r\nSubject: first\r\n\
Subject: second\r\nMessage-Id: <original@example.com>\r\nEncrypted: value\r\n\
MIME-Version: 1.0\r\nX-Outer: second\r\nContent-X-Extension: retained\r\n\
Content-Type: message/rfc822\r\n\r\nFrom: inner@example.com\r\nX-Inner: retained\r\n\
To: nested@example.com\r\nSubject: nested\r\nContent-Type: text/plain\r\n\r\nnested body";
    let (headers, mut parts) = partials(MessagePartial::split(load(wire), 80).unwrap());
    assert_eq!(headers.get_all("X-Outer").count(), 2);
    assert_eq!(headers.get_all("Subject").count(), 2);
    let data = parts.iter().flat_map(stored).copied().collect::<Vec<_>>();
    let enclosed = load(&data);
    assert!(!enclosed.headers().contains("From"));
    assert!(!enclosed.headers().contains("X-Outer"));
    assert_eq!(enclosed.headers().get_all("Subject").count(), 2);
    assert!(enclosed.headers().contains("Encrypted"));
    let Some(MimeEntity::Message(wrapper)) = enclosed.body() else {
        panic!("nested message")
    };
    assert!(wrapper.headers().contains("Content-X-Extension"));
    let nested = wrapper.message().unwrap();
    assert!(nested.headers().contains("From"));
    assert!(nested.headers().contains("To"));
    assert!(nested.headers().contains("X-Inner"));
    let mut joined = MessagePartial::join(&headers, &mut parts).unwrap().unwrap();
    assert_eq!(write(&mut joined), wire);
}

#[test]
fn missing_original_id_generates_shared_fragment_id_but_not_an_enclosed_header() {
    let wire = b"From: sender@example.com\r\nSubject: original\r\nContent-Type: text/plain\r\n\r\nlong enough to split";
    let (headers, mut parts) = partials(MessagePartial::split(load(wire), 20).unwrap());
    let id = parts[0].id().unwrap().unwrap();
    assert!(id.ends_with(&format!("@{}", MessageId::default_domain())));
    assert!(
        parts
            .iter()
            .all(|part| part.id().unwrap().as_deref() == Some(id.as_str()))
    );
    let enclosed = parts.iter().flat_map(stored).copied().collect::<Vec<_>>();
    assert!(!load(&enclosed).headers().contains("Message-Id"));
    let mut joined = MessagePartial::join(&headers, &mut parts).unwrap().unwrap();
    assert!(joined.message_id().unwrap().is_none());
    assert_eq!(write(&mut joined), wire);
}

fn bad_encoding() -> MimeMessage {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"content".to_vec())));
    part.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: invalid").unwrap());
    MimeMessage::with_body(part.into())
}

#[test]
fn invalid_options_and_serialization_errors_return_no_partial_result() {
    // The in-memory API cannot adopt MimeKit's ThrowingReadStream. Invalid
    // authoring metadata exercises the corresponding writer-error cleanup.
    assert!(matches!(
        MessagePartial::split(bad_encoding(), 0),
        Err(MessagePartialError::InvalidMaxSize)
    ));
    let error = MessagePartial::split(bad_encoding(), 1024).unwrap_err();
    assert!(matches!(error, MessagePartialError::Io(_)));
    assert!(error.source().is_some());
    for options in [
        MimePartWriteOptions {
            max_line_length: 1,
            ..Default::default()
        },
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
    ] {
        let error =
            MessagePartial::split_with_options(MimeMessage::new(), 1, &options).unwrap_err();
        assert!(matches!(error, MessagePartialError::Io(_)));
        assert!(error.source().is_some());
    }
    let wire = b"Message-Id: <>\r\nContent-Type: text/plain\r\n\r\nbody";
    assert_eq!(
        MessagePartial::split(load(wire), usize::MAX).unwrap().len(),
        1
    );
    let error = MessagePartial::split(load(wire), 10).unwrap_err();
    assert!(matches!(error, MessagePartialError::MessageId(_)));
    assert!(error.source().is_some());
    for error in [
        MessagePartialError::InvalidMaxSize,
        MessagePartialError::TooManyFragments,
    ] {
        assert!(error.source().is_none());
        assert!(!error.to_string().is_empty());
    }
}

// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, io::Cursor, iter::FusedIterator};

use mimecraft::{
    ContentType, Header, HeaderFormatOptions, HeaderList, HeaderParser, MessagePart,
    MessagePartError, MimeContent, MimeEntity, MimeMessage, MimePart, Multipart,
    encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

fn leaf(body: &[u8]) -> MimeEntity {
    let mut source = Cursor::new(body.to_vec());
    source.set_position(2);
    MimePart::new(MimeContent::new(source)).into()
}

#[test]
fn construction_adds_only_wrapper_content_type_without_read_trait_bounds() {
    struct NoIo;
    let part = MessagePart::new(MimeMessage::<NoIo>::new());
    assert_eq!(part.headers().len(), 1);
    assert_eq!(
        part.headers().get("Content-Type").unwrap().raw(),
        b"Content-Type: message/rfc822\r\n"
    );
    assert!(part.message().unwrap().headers().is_empty());
    assert!(part.message().unwrap().body().is_none());
    assert_eq!(part.leaf_parts().count(), 0);
    let entity: MimeEntity<NoIo> = part.into();
    assert!(matches!(entity, MimeEntity::Message(_)));
    assert_eq!(entity.leaf_parts().count(), 0);
    for subtype in ["news", "global", "RFC822", "x-custom", "rfc822/custom"] {
        let part = MessagePart::with_subtype(subtype, MimeMessage::<NoIo>::new()).unwrap();
        assert!(
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("message", subtype)
        );
    }
    for subtype in ["", "rfc822 bad", "rfc822\r\nX: bad"] {
        assert!(matches!(
            MessagePart::with_subtype(subtype, MimeMessage::<NoIo>::new()),
            Err(MessagePartError::ContentType(_))
        ));
    }
}

#[test]
fn raw_components_and_payloads_move_without_copying_or_repair() {
    let raw = headers(b"Broken record\nX-Raw: \xff\nContent-Type: text/plain\n");
    let raw_pointer = raw.as_slice()[0].raw().as_ptr();
    let body = leaf(b"payload");
    let payload_pointer = body
        .leaf_parts()
        .next()
        .unwrap()
        .content()
        .get_ref()
        .get_ref()
        .as_ptr();
    let message_headers = headers(b"Subject: attached\nContent-X: misplaced\n");
    let message = MimeMessage::from_parts(message_headers.clone(), Some(body));
    let part = MessagePart::from_parts(raw, Some(message));
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("text", "plain")
    );
    assert_eq!(part.headers().as_slice()[0].raw().as_ptr(), raw_pointer);
    assert_eq!(part.message().unwrap().headers(), &message_headers);
    let (raw, message) = part.into_parts();
    assert_eq!(raw.as_slice()[0].raw().as_ptr(), raw_pointer);
    let message = message.unwrap();
    let content = message.leaf_parts().next().unwrap().content();
    assert_eq!(content.get_ref().get_ref().as_ptr(), payload_pointer);
    assert_eq!(content.get_ref().position(), 2);
}

#[test]
fn replacing_and_removing_borrowed_messages_preserves_source_positions() {
    let mut source = Cursor::new(b"payload".to_vec());
    source.set_position(4);
    {
        let message = MimeMessage::with_body(MimePart::new(MimeContent::new(&mut source)).into());
        let mut part = MessagePart::new(message);
        let original_headers = part.headers().clone();
        let old = part.set_message(None).unwrap();
        assert!(part.message().is_none());
        assert_eq!(part.leaf_parts().count(), 0);
        assert_eq!(
            old.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            4
        );
        assert!(part.set_message(Some(old)).is_none());
        part.message_mut()
            .unwrap()
            .headers_mut()
            .add(Header::parse(b"Subject: edited").unwrap());
        assert_eq!(part.headers(), &original_headers);
        assert!(part.message().unwrap().headers().contains("Subject"));
        assert_eq!(part.leaf_parts().count(), 1);
        let (_, message) = part.into_parts();
        assert!(message.unwrap().headers().contains("Subject"));
    }
    assert_eq!(source.position(), 4);
}

#[test]
fn metadata_getters_observe_raw_missing_invalid_and_duplicate_values() {
    let mut part: MessagePart = MessagePart::from_parts(HeaderList::new(), None);
    assert!(part.content_type().unwrap().is_none());
    part.headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    part.headers_mut()
        .add(Header::parse(b"Content-Type: message/rfc822").unwrap());
    assert!(part.content_type().is_err());
    part.headers_mut().remove_all("Content-Type");
    part.headers_mut()
        .add(Header::parse(b"Content-Type: message/global; name=first").unwrap());
    let mut detached = part.content_type().unwrap().unwrap();
    detached.set_name(Some("changed"));
    assert_eq!(part.content_type().unwrap().unwrap().name(), Some("first"));
    part.set_content_type(&detached).unwrap();
    assert_eq!(
        part.content_type().unwrap().unwrap().name(),
        Some("changed")
    );
}

#[test]
fn checked_content_type_updates_are_atomic_and_collapse_duplicates_in_place() {
    let mut part = MessagePart::from_parts(
        headers(
            b"X-First: 1\nContent-Type: message/rfc822\nX-Middle: 2\nCONTENT-TYPE: message/news\n",
        ),
        Some(MimeMessage::with_body(leaf(b"data"))),
    );
    let before = part.headers().clone();
    let error = part
        .set_content_type(&ContentType::new("text", "plain").unwrap())
        .unwrap_err();
    assert!(matches!(error, MessagePartError::NotMessage(_)));
    assert!(error.source().is_none());
    assert_eq!(part.headers(), &before);
    let value = ContentType::new("message", "global").unwrap();
    let error = part
        .set_content_type_with_options(
            &value,
            &HeaderFormatOptions {
                new_line_format: NewLineFormat::Mixed,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, MessagePartError::HeaderFormat(_)));
    assert!(error.source().is_some());
    assert_eq!(part.headers(), &before);
    part.set_content_type_with_options(
        &value,
        &HeaderFormatOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(part.headers().get_all("Content-Type").count(), 1);
    assert_eq!(part.headers().as_slice()[0].field(), Some("X-First"));
    assert_eq!(
        part.headers().as_slice()[1].raw(),
        b"Content-Type: message/global\n"
    );
    assert_eq!(part.headers().as_slice()[2].field(), Some("X-Middle"));
    assert_eq!(
        part.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        2
    );
}

#[test]
fn traversal_descends_nested_messages_in_order_and_is_fused() {
    fn assert_fused<T: FusedIterator>(_: &T) {}
    let mut mixed: Multipart = Multipart::with_boundary("mixed", "outer").unwrap();
    let absent = MessagePart::new(MimeMessage::new());
    let mut no_message = MessagePart::new(MimeMessage::new());
    no_message.set_message(None);
    let attached = MessagePart::new(MimeMessage::with_body(leaf(b"second")));
    let mut inner: Multipart = Multipart::with_boundary("mixed", "inner").unwrap();
    inner
        .children_mut()
        .extend([leaf(b"first"), attached.into(), leaf(b"third")]);
    mixed.children_mut().extend([
        absent.into(),
        MessagePart::new(MimeMessage::with_body(inner.into())).into(),
        no_message.into(),
        leaf(b"fourth"),
    ]);
    let message = MimeMessage::with_body(mixed.into());
    let mut iter = message.leaf_parts();
    assert_fused(&iter);
    let bodies: Vec<&[u8]> = iter
        .by_ref()
        .map(|part| part.content().get_ref().get_ref().as_slice())
        .collect();
    assert_eq!(
        bodies,
        [b"first".as_slice(), b"second", b"third", b"fourth"]
    );
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
}

#[test]
fn entity_header_access_edits_only_the_wrapper() {
    let mut message: MimeMessage = MimeMessage::new();
    message
        .headers_mut()
        .add(Header::parse(b"Subject: inside").unwrap());
    let mut entity: MimeEntity = MessagePart::new(message).into();
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Description: attached").unwrap());
    assert!(!entity.headers().contains("Subject"));
    let MimeEntity::Message(part) = entity else {
        panic!("message wrapper expected")
    };
    assert!(part.headers().contains("Content-Description"));
    assert!(
        !part
            .message()
            .unwrap()
            .headers()
            .contains("Content-Description")
    );
}

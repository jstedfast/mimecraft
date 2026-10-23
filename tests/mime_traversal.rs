// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::FusedIterator;

use mimecraft::{
    Attachments, BodyParts, ContentHeaderParseErrorKind, Header, HeaderList, HeaderParser,
    MessagePart, MimeContent, MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, Multipart,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut result = Vec::new();
    parser.parse(raw, &mut result).unwrap();
    parser.finish(&mut result).unwrap();
    result.into()
}

fn label<R>(entity: &MimeEntity<R>) -> String {
    entity.headers().get("X-Label").unwrap().decode_value().text
}

fn tag<R>(mut entity: MimeEntity<R>, name: &str, disposition: Option<&str>) -> MimeEntity<R> {
    entity
        .headers_mut()
        .add(Header::parse(format!("X-Label: {name}").as_bytes()).unwrap());
    if let Some(disposition) = disposition {
        entity
            .headers_mut()
            .add(Header::parse(format!("Content-Disposition: {disposition}").as_bytes()).unwrap());
    }
    entity
}

fn leaf(name: &str, disposition: Option<&str>) -> MimeEntity {
    tag(
        MimePart::new(MimeContent::new(Cursor::new(name.as_bytes().to_vec()))).into(),
        name,
        disposition,
    )
}

fn message_part(name: &str, disposition: Option<&str>, body: Option<MimeEntity>) -> MimeEntity {
    let message = MimeMessage::from_parts(HeaderList::new(), body);
    tag(MessagePart::new(message).into(), name, disposition)
}

#[test]
fn body_parts_preserve_depth_first_order_but_stop_at_message_wrappers() {
    let mut inner: Multipart = Multipart::new("mixed", "inner").unwrap();
    inner.children_mut().extend([
        leaf("second", Some("attachment")),
        message_part(
            "wrapper",
            Some("attachment"),
            Some(leaf("hidden", Some("attachment"))),
        ),
        leaf("third", None),
    ]);
    let missing: MessagePart = MessagePart::from_parts(HeaderList::new(), None);
    let mut root: Multipart = Multipart::new("mixed", "outer").unwrap();
    root.children_mut().extend([
        leaf("first", None),
        inner.into(),
        tag(missing.into(), "absent", Some("attachment")),
        leaf("fourth", Some("attachment")),
    ]);
    let message = MimeMessage::with_body(root.into());
    let parts: BodyParts<'_, _> = message.body_parts();
    assert_eq!(
        parts.map(label).collect::<Vec<_>>(),
        ["first", "second", "wrapper", "third", "absent", "fourth"]
    );
    let attachments: Attachments<'_, _> = message.attachments();
    assert_eq!(
        attachments
            .map(|item| label(item.unwrap()))
            .collect::<Vec<_>>(),
        ["second", "wrapper", "absent", "fourth"]
    );
    assert_eq!(
        message
            .leaf_parts()
            .map(|part| part.headers().get("X-Label").unwrap().decode_value().text)
            .collect::<Vec<_>>(),
        ["first", "second", "hidden", "third", "fourth"]
    );
    let root = message.body().unwrap();
    assert_eq!(
        root.body_parts().map(label).collect::<Vec<_>>(),
        message.body_parts().map(label).collect::<Vec<_>>()
    );
    let expected = root.body_parts().nth(2).unwrap();
    assert!(std::ptr::eq(
        message.attachments().nth(1).unwrap().unwrap(),
        expected
    ));
}

#[test]
fn roots_empty_trees_and_absent_messages_follow_distinct_traversal_rules() {
    let empty: MimeMessage = MimeMessage::new();
    assert_eq!(empty.body_parts().count(), 0);
    assert_eq!(empty.attachments().count(), 0);
    let leaf = leaf("root", Some("attachment"));
    assert!(std::ptr::eq(leaf.body_parts().next().unwrap(), &leaf));
    assert!(std::ptr::eq(
        leaf.attachments().next().unwrap().unwrap(),
        &leaf
    ));
    for has_message in [false, true] {
        let message = has_message.then(MimeMessage::new);
        let root: MimeEntity =
            MessagePart::from_parts(headers(b"Content-Disposition: attachment\r\n"), message)
                .into();
        assert_eq!(root.body_parts().count(), 1);
        assert_eq!(root.attachments().count(), 1);
        assert_eq!(root.leaf_parts().count(), 0);
    }
    let container: Multipart = Multipart::new("mixed", "empty").unwrap();
    let root = tag(container.into(), "empty", Some("attachment"));
    assert!(root.is_attachment().unwrap());
    assert_eq!(root.body_parts().count(), 0);
    assert_eq!(root.attachments().count(), 0);
}

#[test]
fn attachment_status_uses_disposition_not_names_types_or_message_headers() {
    for (value, expected) in [
        (None, false),
        (Some("inline; filename=attachment.txt"), false),
        (Some("x-custom; filename=attachment.txt"), false),
        (Some("attachment"), true),
        (
            Some("(comment) AtTaChMeNt;\r\n\tfilename*=utf-8''report.txt"),
            true,
        ),
    ] {
        let mut entity = leaf("part", value);
        entity
            .headers_mut()
            .replace_all(
                Header::parse(b"Content-Type: application/octet-stream; name=attachment.txt")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(entity.is_attachment().unwrap(), expected);
        assert_eq!(entity.attachments().count(), usize::from(expected));
        let message = MimeMessage::from_parts(
            headers(b"Content-Disposition: attachment\r\n"),
            Some(entity),
        );
        assert_eq!(message.attachments().count(), usize::from(expected));
    }
}

#[test]
fn raw_disposition_edits_are_visible_and_first_duplicate_wins() {
    let mut entity = leaf("part", Some("inline"));
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Disposition: attachment").unwrap());
    assert!(!entity.is_attachment().unwrap());
    assert_eq!(entity.attachments().count(), 0);
    entity
        .headers_mut()
        .replace_all(Header::parse(b"Content-Disposition: attachment").unwrap())
        .unwrap();
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Disposition:").unwrap());
    assert!(entity.is_attachment().unwrap());
    assert_eq!(entity.attachments().count(), 1);
    entity
        .headers_mut()
        .replace_all(Header::parse(b"Content-Disposition:").unwrap())
        .unwrap();
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Disposition: attachment").unwrap());
    assert_eq!(
        entity.is_attachment().unwrap_err().kind(),
        ContentHeaderParseErrorKind::InvalidDisposition
    );
    assert!(entity.attachments().next().unwrap().is_err());
}

#[test]
fn malformed_dispositions_keep_typed_offsets_without_affecting_body_enumeration() {
    for value in ["", "\"attachment\"", "attachment invalid", "(unclosed"] {
        let entity = leaf("bad", Some(value));
        let expected = entity
            .headers()
            .get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap_err();
        assert_eq!(entity.is_attachment().unwrap_err(), expected);
        assert_eq!(entity.attachments().next().unwrap().unwrap_err(), expected);
        assert_eq!(entity.body_parts().count(), 1);
        assert_eq!(entity.leaf_parts().count(), 1);
    }
}

#[test]
fn attachment_filter_is_lazy_can_resume_after_errors_and_is_fused() {
    let mut container: Multipart = Multipart::new("mixed", "boundary").unwrap();
    container.children_mut().extend([
        leaf("first", Some("attachment")),
        leaf("bad", Some("")),
        leaf("inline", Some("inline")),
        leaf("last", Some("attachment")),
    ]);
    let message = MimeMessage::with_body(container.into());
    let mut attachments = message.attachments();
    assert_eq!(label(attachments.next().unwrap().unwrap()), "first");
    assert_eq!(
        attachments.next().unwrap().unwrap_err().kind(),
        ContentHeaderParseErrorKind::InvalidDisposition
    );
    assert_eq!(label(attachments.next().unwrap().unwrap()), "last");
    assert!(attachments.next().is_none());
    assert!(attachments.next().is_none());
    assert!(
        message
            .attachments()
            .collect::<Result<Vec<_>, _>>()
            .is_err()
    );
    fn fused<I: FusedIterator>(_: I) {}
    fused(attachments);
    let mut parts = message.body_parts();
    assert_eq!(parts.by_ref().count(), 4);
    assert!(parts.next().is_none());
    assert!(parts.next().is_none());
    fused(parts);
    fused(message.leaf_parts());
}

#[test]
fn containers_and_attached_contents_are_not_classified_by_attachment_filtering() {
    let mut container: Multipart = Multipart::new("mixed", "boundary").unwrap();
    container
        .headers_mut()
        .add(Header::parse(b"Content-Disposition:").unwrap());
    container.children_mut().extend([
        leaf("child", Some("attachment")),
        message_part(
            "wrapper",
            Some("inline"),
            Some(leaf("hidden-bad", Some(""))),
        ),
    ]);
    let root: MimeEntity = container.into();
    assert!(root.is_attachment().is_err());
    let found = root.attachments().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(label(found[0]), "child");
    assert_eq!(root.body_parts().count(), 2);
    assert_eq!(root.leaf_parts().count(), 2);
}

#[test]
fn iteration_uses_model_variants_without_parsing_unrelated_metadata() {
    let mut container: Multipart = Multipart::new("mixed", "boundary").unwrap();
    container
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    let mut child = leaf("leaf", Some("attachment"));
    child
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: message/rfc822").unwrap())
        .unwrap();
    child
        .headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: invalid").unwrap());
    container.children_mut().push(child);
    let root: MimeEntity = container.into();
    assert!(matches!(
        root.body_parts().next(),
        Some(MimeEntity::Part(_))
    ));
    assert_eq!(
        root.attachments()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn metadata_and_traversal_work_without_readable_or_seekable_source_bounds() {
    struct NoIo;
    let part =
        MessagePart::<NoIo>::from_parts(headers(b"Content-Disposition: attachment\r\n"), None);
    let mut container: Multipart<NoIo> = Multipart::new("mixed", "boundary").unwrap();
    container.children_mut().push(part.into());
    let mut root: MimeEntity<NoIo> = container.into();
    root.headers_mut()
        .add(Header::parse(b"X-Outer: kept").unwrap());
    assert!(root.headers().contains("X-Outer"));
    assert_eq!(root.body_parts().count(), 1);
    assert!(root.body_parts().next().unwrap().is_attachment().unwrap());
    assert_eq!(root.attachments().count(), 1);
    let message = MimeMessage::with_body(root);
    assert_eq!(message.body_parts().count(), 1);
    assert_eq!(message.attachments().count(), 1);
}

#[test]
fn enumeration_never_reads_seeks_or_mutates_payloads_or_raw_headers() {
    struct NoIo;
    impl Read for NoIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("read is forbidden")
        }
    }
    impl Seek for NoIo {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            panic!("seek is forbidden")
        }
    }
    let raw = headers(
        b"content-disposition: (comment) attachment;\n\tfilename=raw.txt\nX-Unknown: raw\n",
    );
    let part = MimePart::from_parts(raw.clone(), MimeContent::new(NoIo));
    let entity: MimeEntity<_> = part.into();
    assert!(entity.is_attachment().unwrap());
    assert_eq!(entity.body_parts().count(), 1);
    assert_eq!(entity.attachments().count(), 1);
    assert_eq!(entity.leaf_parts().count(), 1);
    assert_eq!(entity.headers(), &raw);

    let mut source = Cursor::new(b"unchanged body");
    source.set_position(7);
    {
        let part = MimePart::from_parts(raw.clone(), MimeContent::new(&mut source));
        let message = MimeMessage::with_body(part.into());
        assert_eq!(message.attachments().count(), 1);
        assert_eq!(message.body_parts().count(), 1);
        assert_eq!(message.body().unwrap().headers(), &raw);
    }
    assert_eq!(source.position(), 7);
    assert_eq!(*source.get_ref(), b"unchanged body");
}

#[test]
fn loaded_digest_and_explicit_wrappers_are_enumerated_without_entering_messages() {
    let raw = b"Subject: digest\r\nContent-Type: multipart/digest; boundary=d\r\n\r\n\
--d\r\nContent-Disposition: attachment\r\n\r\nSubject: first\r\n\r\nbody\r\n\
--d\r\nContent-Type: message/rfc822\r\nContent-Disposition: inline\r\n\r\nSubject: second\r\nContent-Disposition: attachment\r\n\r\ninner\r\n--d--\r\n";
    for structured in [true, false] {
        let message = MimeMessage::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                parse_message_parts: structured,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(message.body_parts().count(), 2);
        let attachments = message
            .attachments()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(matches!(attachments[0], MimeEntity::Message(_)), structured);
        assert!(!attachments[0].headers().contains("Content-Type"));
        assert_eq!(message.leaf_parts().count(), 2);
    }
}

#[test]
fn deep_multipart_enumeration_uses_an_explicit_stack() {
    let mut root = leaf("deep", Some("attachment"));
    for _ in 0..4000 {
        let mut parent: Multipart = Multipart::new("mixed", "same").unwrap();
        parent.children_mut().push(root);
        root = parent.into();
    }
    let parts = root.body_parts().count();
    let attachments = root
        .attachments()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .len();
    let leaves = root.leaf_parts().count();
    while let MimeEntity::Multipart(mut parent) = root {
        root = parent.children_mut().pop().unwrap();
    }
    assert_eq!((parts, attachments, leaves), (1, 1, 1));
}

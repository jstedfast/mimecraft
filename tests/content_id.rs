// Adapted from MimeKit's UnitTests/MimePartTests.cs and MimeEntity.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    Header, HeaderList, MessageId, MessageIdParseError, MessageIdParseErrorKind, MessagePart,
    MimeContent, MimeEntity, MimeMessage, MimePart, Multipart, MultipartRelated, TextFormat,
    TextPart,
};

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("Content-Id metadata must not read content")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("Content-Id metadata must not seek content")
    }
}

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn leaf(raw: &[u8]) -> MimeEntity<NoIo> {
    MimePart::from_parts(headers(raw), MimeContent::new(NoIo)).into()
}

fn entities(raw: &[u8]) -> [MimeEntity<NoIo>; 3] {
    [
        leaf(raw),
        Multipart::from_parts(
            headers(raw),
            vec![leaf(b"Content-Id: <child@x>\n")],
            b"preamble".to_vec(),
            b"epilogue".to_vec(),
        )
        .into(),
        MessagePart::from_parts(
            headers(raw),
            Some(MimeMessage::from_parts(
                headers(b"Message-Id: <message@x>\nContent-Id: <misplaced@x>\n"),
                Some(leaf(b"Content-Id: <body@x>\n")),
            )),
        )
        .into(),
    ]
}

fn get_concrete<R>(entity: &MimeEntity<R>) -> Result<Option<MessageId>, MessageIdParseError> {
    match entity {
        MimeEntity::Part(part) => part.content_id(),
        MimeEntity::Multipart(part) => part.content_id(),
        MimeEntity::Message(part) => part.content_id(),
    }
}

fn set_concrete<R>(
    entity: &mut MimeEntity<R>,
    value: Option<&str>,
) -> Result<(), MessageIdParseError> {
    match entity {
        MimeEntity::Part(part) => part.set_content_id(value),
        MimeEntity::Multipart(part) => part.set_content_id(value),
        MimeEntity::Message(part) => part.set_content_id(value),
    }
}

#[test]
fn absent_ids_are_not_inherited_or_generated_and_none_removes_all_occurrences() {
    for mut entity in entities(b"X-Keep: raw\n") {
        assert_eq!(entity.content_id(), Ok(None));
        assert_eq!(get_concrete(&entity), Ok(None));
        set_concrete(&mut entity, Some("image@example.com")).unwrap();
        assert_eq!(
            entity.content_id().unwrap().unwrap().as_str(),
            "image@example.com"
        );
        entity
            .headers_mut()
            .add(Header::parse(b"CONTENT-ID: <duplicate@x>").unwrap());
        entity.set_content_id(None).unwrap();
        assert_eq!(get_concrete(&entity), Ok(None));
        assert_eq!(entity.headers(), &headers(b"X-Keep: raw\n"));
        set_concrete(&mut entity, None).unwrap();
        assert_eq!(entity.headers(), &headers(b"X-Keep: raw\n"));
    }
}

#[test]
fn getter_uses_raw_prefix_grammar_and_only_the_first_header() {
    for (value, expected) in [
        (
            " (before) <first@Example.COM> (unterminated",
            "first@Example.COM",
        ),
        (" <first@x> <second@x>", "first@x"),
        (" <a b@@example.com.>", "ab@example.com"),
        (" local", "local"),
        (" <local@>", "local@"),
        (" <caf\u{e9}@example.com>", "caf\u{e9}@example.com"),
        (
            " <=?utf-8?q?local?=@example.com>",
            "=?utf-8?q?local?=@example.com",
        ),
    ] {
        let raw = format!("cOnTeNt-Id:{value}\r\nContent-Id: <ignored@x>\r\n");
        for entity in entities(raw.as_bytes()) {
            let before = entity.headers().clone();
            assert_eq!(entity.content_id().unwrap().unwrap().as_str(), expected);
            assert_eq!(get_concrete(&entity), entity.content_id());
            assert_eq!(entity.headers(), &before);
        }
    }
}

#[test]
fn invalid_first_ids_report_original_value_offsets_not_absence_or_later_duplicates() {
    for value in [b"".as_slice(), b" <>", b" <missing", b" <a@\xff>"] {
        let mut raw = b"Content-Id:".to_vec();
        raw.extend_from_slice(value);
        raw.extend_from_slice(b"\r\nContent-Id: <valid@x>\r\n");
        for entity in entities(&raw) {
            let before = entity.headers().clone();
            let error =
                MessageId::parse_prefix(before.get("Content-Id").unwrap().raw_value()).unwrap_err();
            assert_eq!(entity.content_id(), Err(error));
            assert_eq!(get_concrete(&entity), Err(error));
            assert_eq!(entity.headers(), &before);
        }
    }
}

#[test]
fn setters_normalize_compatible_values_for_all_concrete_types() {
    for (value, expected) in [
        (" \t<abc@Example.COM>\r\n", "abc@Example.COM"),
        ("<a b@example.com>", "ab@example.com"),
        ("<a@@example.com.>", "a@example.com"),
        ("local", "local"),
        ("<local@>", "local@"),
        ("(paren@example.com", "(paren@example.com"),
        ("\"x\r\n y\"@example.com", "\"x y\"@example.com"),
        ("caf\u{e9}@example.com", "caf\u{e9}@example.com"),
    ] {
        for mut entity in entities(b"X-Keep: raw\n") {
            set_concrete(&mut entity, Some(value)).unwrap();
            assert_eq!(entity.content_id().unwrap().unwrap().as_str(), expected);
            assert_eq!(
                entity.headers().get("Content-Id").unwrap().raw(),
                format!("Content-Id: <{expected}>\r\n").as_bytes()
            );
            assert_eq!(entity.headers().len(), 2);
            entity.set_content_id(Some(value)).unwrap();
            assert_eq!(get_concrete(&entity).unwrap().unwrap().as_str(), expected);
        }
    }
}

#[test]
fn assignments_canonicalize_equal_ids_and_collapse_duplicates_at_the_first_slot() {
    for old in ["(comment) <same@x> <trailing@x>", "<>", "<old@x>"] {
        let raw = format!(
            "X-Before: one\ncontent-id \t: {old}\nX-Middle: two\nCONTENT-ID: <duplicate@x>\nX-After: three\n"
        );
        for mut entity in entities(raw.as_bytes()) {
            entity.set_content_id(Some("same@x")).unwrap();
            let expected =
                headers(b"X-Before: one\nContent-Id: <same@x>\r\nX-Middle: two\nX-After: three\n");
            assert_eq!(entity.headers(), &expected);
            set_concrete(&mut entity, Some("<same@x>")).unwrap();
            assert_eq!(entity.headers(), &expected);
        }
    }
    for mut entity in entities(b"content-id: (comment) <same@x>\n") {
        entity.set_content_id(Some("same@x")).unwrap();
        assert_eq!(entity.headers(), &headers(b"Content-Id: <same@x>\r\n"));
    }
}

#[test]
fn invalid_assignments_are_atomic_and_use_argument_offsets() {
    for value in [
        "",
        "<>",
        "a b@x",
        "a@x>",
        "a\0@x",
        "a@x\r\nBcc: injected@x",
        "a@x\nBcc: injected@x",
        "a@x\rBcc: injected@x",
        "a@x\r\n\r\nInjected body",
        "<one@x> <two@x>",
        " \u{2003}a\0@x ",
    ] {
        let expected = MessageId::from_value(value).unwrap_err();
        for mut entity in entities(b"Content-Id: <old@x>\nX-Keep: raw\nCONTENT-ID: <duplicate@x>\n")
        {
            let before = entity.headers().clone();
            assert_eq!(entity.set_content_id(Some(value)), Err(expected));
            assert_eq!(entity.headers(), &before);
            assert_eq!(set_concrete(&mut entity, Some(value)), Err(expected));
            assert_eq!(entity.headers(), &before);
        }
    }
}

#[test]
fn successfully_read_raw_ids_are_revalidated_before_assignment() {
    for mut entity in entities(b"Content-Id: <a\0@x>\n") {
        let before = entity.headers().clone();
        let id = entity.content_id().unwrap().unwrap();
        assert_eq!(id.as_str(), "a\0@x");
        let error = entity.set_content_id(Some(id.as_str())).unwrap_err();
        assert_eq!(error.kind(), MessageIdParseErrorKind::NulCharacter);
        assert_eq!(entity.headers(), &before);
    }
}

#[test]
fn raw_edits_and_returned_values_do_not_create_a_cache() {
    for mut entity in entities(b"Content-Id: <first@x>\nContent-Id: <second@x>\n") {
        let saved = entity.content_id().unwrap().unwrap();
        entity.headers_mut().remove("Content-Id");
        assert_eq!(get_concrete(&entity).unwrap().unwrap().as_str(), "second@x");
        entity
            .headers_mut()
            .replace_all(Header::parse(b"Content-Id: <third@x>").unwrap())
            .unwrap();
        assert_eq!(entity.content_id().unwrap().unwrap().as_str(), "third@x");
        assert_eq!(saved.as_str(), "first@x");
        entity.headers_mut().clear();
        assert_eq!(entity.content_id(), Ok(None));
    }
}

#[test]
fn container_and_wrapper_edits_leave_children_messages_and_sources_unchanged() {
    for mut entity in entities(b"Content-Type: invalid\n") {
        entity.set_content_id(Some("root@x")).unwrap();
        match &entity {
            MimeEntity::Multipart(part) => {
                assert_eq!(
                    part.children()[0].content_id().unwrap().unwrap().as_str(),
                    "child@x"
                );
                assert_eq!(part.preamble(), b"preamble");
                assert_eq!(part.epilogue(), b"epilogue");
            }
            MimeEntity::Message(part) => {
                let message = part.message().unwrap();
                assert_eq!(message.message_id().unwrap().unwrap().as_str(), "message@x");
                assert_eq!(
                    message.headers().get("Content-Id").unwrap().raw_value(),
                    b" <misplaced@x>\n"
                );
                assert_eq!(
                    message
                        .body()
                        .unwrap()
                        .content_id()
                        .unwrap()
                        .unwrap()
                        .as_str(),
                    "body@x"
                );
            }
            MimeEntity::Part(_) => {}
        }
        assert_eq!(
            entity.headers().get("Content-Type").unwrap().raw_value(),
            b" invalid\n"
        );
    }
    let mut source = Cursor::new(b"body".to_vec());
    source.set_position(3);
    {
        let mut part = MimePart::new(MimeContent::new(&mut source));
        part.set_content_id(Some("body@x")).unwrap();
        assert_eq!(part.content_id().unwrap().unwrap().as_str(), "body@x");
        part.set_content_id(None).unwrap();
    }
    assert_eq!(source.position(), 3);
    assert_eq!(source.into_inner(), b"body");
}

#[test]
fn source_trait_bounds_are_not_needed_for_metadata_access() {
    let mut part: Multipart<()> = Multipart::with_boundary("mixed", "boundary").unwrap();
    part.set_content_id(Some("container@x")).unwrap();
    assert_eq!(part.content_id().unwrap().unwrap().as_str(), "container@x");
    let mut wrapper = MessagePart::new(MimeMessage::<()>::new());
    wrapper.set_content_id(Some("wrapper@x")).unwrap();
    assert_eq!(wrapper.content_id().unwrap().unwrap().as_str(), "wrapper@x");
    let mut entity: MimeEntity<()> = wrapper.into();
    entity.set_content_id(None).unwrap();
    assert_eq!(entity.content_id(), Ok(None));
}

#[test]
fn related_root_lookup_observes_id_changes_without_updating_selectors() {
    let mut related: MultipartRelated = MultipartRelated::with_boundary("related").unwrap();
    related
        .set_root(TextPart::new("plain", "fallback").unwrap().into())
        .unwrap();
    let mut html = TextPart::new("html", "<p>root</p>").unwrap();
    html.part_mut().set_content_id(Some("root@x")).unwrap();
    related.multipart_mut().children_mut().push(html.into());
    let mut content_type = related.multipart().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("start", "<root@x>")
        .unwrap();
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(1));

    let before = related.multipart().headers().clone();
    related
        .root_mut()
        .unwrap()
        .unwrap()
        .set_content_id(Some("new@x"))
        .unwrap();
    assert_eq!(related.multipart().headers(), &before);
    assert_eq!(related.root_index().unwrap(), Some(0));
    related.multipart_mut().children_mut()[1]
        .set_content_id(Some("root@x"))
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(1));

    let mut replacement = TextPart::new("html", "<p>replacement</p>").unwrap();
    replacement
        .part_mut()
        .set_content_id(Some("replacement@x"))
        .unwrap();
    related.set_root(replacement.into()).unwrap();
    assert_eq!(
        related
            .multipart()
            .content_type()
            .unwrap()
            .unwrap()
            .parameters()
            .get("start"),
        Some("<replacement@x>")
    );
    let entity: MimeEntity = related.into();
    assert_eq!(
        entity
            .text_body_part(TextFormat::Html)
            .unwrap()
            .unwrap()
            .content_id()
            .unwrap()
            .unwrap()
            .as_str(),
        "replacement@x"
    );
}

#[test]
fn serialized_ids_stay_on_the_body_and_invalid_raw_ids_do_not_reject_loading() {
    let mut text = TextPart::new("plain", "body").unwrap();
    text.part_mut()
        .set_content_id(Some("body@example.com"))
        .unwrap();
    let mut message = MimeMessage::with_body(text.into());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert!(!loaded.headers().contains("Content-Id"));
    assert_eq!(
        loaded
            .body()
            .unwrap()
            .content_id()
            .unwrap()
            .unwrap()
            .as_str(),
        "body@example.com"
    );

    let raw = b"Subject: tolerant\r\nContent-Type: text/plain\r\nContent-Id: <>\r\n\r\nbody";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    assert!(message.body().unwrap().content_id().is_err());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert_eq!(wire, raw);
}

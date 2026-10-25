// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    Header, HeaderList, MessagePart, MimeContent, MimeEntity, MimeEntityParseOptions, MimeMessage,
    MimePart, Multipart, MultipartError, TextBodyError, TextFormat,
};

fn header<R>(entity: &mut MimeEntity<R>, field: &str, value: &str) {
    entity
        .headers_mut()
        .replace_all(Header::parse(format!("{field}: {value}").as_bytes()).unwrap())
        .unwrap();
}

fn leaf(media_type: Option<&str>, name: &str) -> MimeEntity {
    let mut entity = MimePart::from_parts(
        HeaderList::new(),
        MimeContent::new(Cursor::new(name.as_bytes().to_vec())),
    )
    .into();
    if let Some(value) = media_type {
        header(&mut entity, "Content-Type", value);
    }
    header(&mut entity, "X-Label", name);
    entity
}

fn multipart(subtype: &str, children: Vec<MimeEntity>) -> MimeEntity {
    let mut part = Multipart::with_boundary(subtype, "boundary").unwrap();
    *part.children_mut() = children;
    part.into()
}

fn label<R>(part: &MimePart<R>) -> String {
    part.headers().get("X-Label").unwrap().decode_value().text
}

fn selected(entity: &MimeEntity, format: TextFormat) -> Option<String> {
    entity.text_body_part(format).unwrap().map(label)
}

#[test]
fn root_formats_match_mimekit_without_converting_content() {
    use TextFormat::*;
    for (media_type, formats) in [
        ("text/plain", vec![Plain]),
        ("TEXT/PLAIN; format=\" Flowed \"", vec![Plain, Flowed]),
        ("text/plain; format=fixed", vec![Plain]),
        ("Text/Html", vec![Html]),
        ("text/enriched", vec![Enriched]),
        ("text/richtext", vec![Enriched]),
        ("text/rtf", vec![RichText]),
        ("application/rtf", vec![RichText]),
        ("application/octet-stream", vec![]),
        ("text/calendar", vec![]),
    ] {
        let entity = leaf(Some(media_type), "body");
        for format in [Plain, Flowed, Html, Enriched, RichText] {
            assert_eq!(
                selected(&entity, format).is_some(),
                formats.contains(&format),
                "{media_type}: {format:?}"
            );
        }
    }
}

#[test]
fn absent_types_default_without_materializing_headers_and_digest_is_scoped() {
    let entity = leaf(None, "implicit");
    assert_eq!(
        selected(&entity, TextFormat::Plain).as_deref(),
        Some("implicit")
    );
    assert!(!entity.headers().contains("Content-Type"));
    let digest = multipart("digest", vec![entity, leaf(Some("text/plain"), "explicit")]);
    assert_eq!(
        selected(&digest, TextFormat::Plain).as_deref(),
        Some("explicit")
    );
    let nested = multipart("mixed", vec![leaf(None, "nested")]);
    let digest = multipart("digest", vec![nested]);
    assert_eq!(
        selected(&digest, TextFormat::Plain).as_deref(),
        Some("nested")
    );
}

#[test]
fn roots_and_mixed_skip_attachments_but_not_inline_filenames() {
    let mut attachment = leaf(Some("text/plain"), "attachment");
    header(
        &mut attachment,
        "Content-Disposition",
        "AtTaChMeNt; filename=file.txt",
    );
    assert!(selected(&attachment, TextFormat::Plain).is_none());
    let mut body = leaf(Some("text/plain; name=file.txt"), "body");
    header(
        &mut body,
        "Content-Disposition",
        "inline; filename=file.txt",
    );
    let entity = multipart("mixed", vec![attachment, body]);
    assert_eq!(
        selected(&entity, TextFormat::Plain).as_deref(),
        Some("body")
    );
}

#[test]
fn ordinary_multipart_stops_at_first_non_attachment_text_in_any_format() {
    for subtype in ["mixed", "signed", "report", "x-custom"] {
        for media_type in ["text/html", "text/calendar", "application/rtf"] {
            let entity = multipart(
                subtype,
                vec![
                    leaf(Some("image/png"), "image"),
                    leaf(Some(media_type), "first"),
                    leaf(Some("text/plain"), "later"),
                ],
            );
            assert!(selected(&entity, TextFormat::Plain).is_none());
        }
    }
}

#[test]
fn ordinary_multipart_stops_after_a_nested_multipart_even_without_a_match() {
    for nested in [
        multipart("alternative", vec![]),
        multipart("alternative", vec![leaf(Some("text/html"), "html")]),
    ] {
        let entity = multipart("mixed", vec![nested, leaf(Some("text/plain"), "later")]);
        assert!(selected(&entity, TextFormat::Plain).is_none());
    }
}

#[test]
fn alternatives_prefer_last_match_and_resume_after_unsuitable_nested_trees() {
    let mut entity = multipart(
        "alternative",
        vec![
            leaf(Some("text/plain"), "first"),
            leaf(Some("text/plain; format=flowed"), "last"),
            multipart("mixed", vec![leaf(Some("text/html"), "html")]),
        ],
    );
    assert_eq!(
        selected(&entity, TextFormat::Plain).as_deref(),
        Some("last")
    );
    assert_eq!(
        selected(&entity, TextFormat::Flowed).as_deref(),
        Some("last")
    );
    assert_eq!(selected(&entity, TextFormat::Html).as_deref(), Some("html"));
    let part = entity
        .text_body_part_mut(TextFormat::Plain)
        .unwrap()
        .unwrap();
    part.headers_mut()
        .replace_all(Header::parse(b"X-Label: edited").unwrap())
        .unwrap();
    assert_eq!(
        selected(&entity, TextFormat::Plain).as_deref(),
        Some("edited")
    );
    let MimeEntity::Multipart(container) = &entity else {
        panic!()
    };
    let MimeEntity::Part(expected) = &container.children()[1] else {
        panic!()
    };
    assert!(std::ptr::eq(
        entity.text_body_part(TextFormat::Plain).unwrap().unwrap(),
        expected
    ));
}

#[test]
fn alternative_and_related_roots_do_not_filter_dispositions() {
    for subtype in ["alternative", "related"] {
        for disposition in ["attachment", ""] {
            let mut body = leaf(Some("text/plain"), "body");
            header(&mut body, "Content-Disposition", disposition);
            let mut entity = multipart(subtype, vec![body]);
            header(&mut entity, "Content-Disposition", "");
            assert_eq!(
                selected(&entity, TextFormat::Plain).as_deref(),
                Some("body")
            );
        }
    }
}

fn related(start: Option<&str>, media_type: Option<&str>) -> MimeEntity {
    let mut first = leaf(Some("text/plain"), "first");
    header(&mut first, "Content-Id", "<first@example.com>");
    let mut second = leaf(Some("text/html"), "second");
    header(
        &mut second,
        "Content-Id",
        "(comment) <root@example.com> (tail)",
    );
    let mut entity = multipart("related", vec![first, second]);
    let mut value = "multipart/related".to_owned();
    if let Some(start) = start {
        value.push_str(&format!("; start=\"{start}\""));
    }
    if let Some(media_type) = media_type {
        value.push_str(&format!("; type=\"{media_type}\""));
    }
    header(&mut entity, "Content-Type", &value);
    entity
}

#[test]
fn related_uses_start_then_type_then_first_child_not_arbitrary_text_resources() {
    for start in [
        "<root@example.com>",
        "root@example.com",
        "prose <root@example.com>",
    ] {
        let entity = related(Some(start), Some("text/plain"));
        assert_eq!(
            selected(&entity, TextFormat::Html).as_deref(),
            Some("second")
        );
        assert!(selected(&entity, TextFormat::Plain).is_none());
    }
    let entity = related(None, Some("TEXT/HTML"));
    assert_eq!(
        selected(&entity, TextFormat::Html).as_deref(),
        Some("second")
    );
    for (start, media_type) in [
        (None, None),
        (None, Some("image/png")),
        (Some("<absent@example.com>"), Some("text/html")),
        (Some("<ROOT@example.com>"), Some("text/html")),
    ] {
        let entity = related(start, media_type);
        assert_eq!(
            selected(&entity, TextFormat::Plain).as_deref(),
            Some("first")
        );
        assert!(selected(&entity, TextFormat::Html).is_none());
    }
}

#[test]
fn related_root_can_be_nested_alternative_and_mutable_lookup_follows_it() {
    let mut root = multipart(
        "alternative",
        vec![
            leaf(Some("text/plain"), "plain"),
            leaf(Some("text/html"), "html"),
        ],
    );
    header(&mut root, "Content-Id", "<root>");
    let mut entity = multipart("related", vec![leaf(Some("image/png"), "image"), root]);
    header(
        &mut entity,
        "Content-Type",
        "multipart/related; start=\"<root>\"",
    );
    for (format, expected) in [(TextFormat::Plain, "plain"), (TextFormat::Html, "html")] {
        assert_eq!(
            label(entity.text_body_part_mut(format).unwrap().unwrap()),
            expected
        );
    }
}

#[test]
fn attached_messages_and_empty_bodies_are_not_candidates() {
    let inner = MimeMessage::with_body(leaf(Some("text/plain"), "hidden"));
    let wrapper: MimeEntity = MessagePart::new(inner).into();
    assert!(selected(&wrapper, TextFormat::Plain).is_none());
    let mixed = multipart("mixed", vec![wrapper, leaf(Some("text/plain"), "body")]);
    assert_eq!(selected(&mixed, TextFormat::Plain).as_deref(), Some("body"));
    let mut empty: MimeMessage = MimeMessage::new();
    assert!(empty.text_body_part(TextFormat::Plain).unwrap().is_none());
    assert!(
        empty
            .text_body_part_mut(TextFormat::Plain)
            .unwrap()
            .is_none()
    );
    for subtype in ["mixed", "related", "alternative"] {
        assert!(selected(&multipart(subtype, vec![]), TextFormat::Plain).is_none());
    }
}

#[test]
fn first_headers_and_raw_edits_are_authoritative() {
    let mut entity = leaf(Some("text/plain"), "body");
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Type:").unwrap());
    assert!(selected(&entity, TextFormat::Plain).is_some());
    header(&mut entity, "Content-Type", "text/html");
    assert!(selected(&entity, TextFormat::Plain).is_none());
    assert!(selected(&entity, TextFormat::Html).is_some());
    header(&mut entity, "Content-Type", "");
    entity
        .headers_mut()
        .add(Header::parse(b"Content-Type: text/plain").unwrap());
    let expected = entity
        .headers()
        .get("Content-Type")
        .unwrap()
        .parse_content_type()
        .unwrap_err();
    match entity.text_body_part(TextFormat::Plain).unwrap_err() {
        TextBodyError::ContentType(error) => assert_eq!(error, expected),
        error => panic!("{error:?}"),
    }
    let mut entity = related(Some("<root@example.com>"), None);
    let MimeEntity::Multipart(part) = &mut entity else {
        panic!()
    };
    header(&mut part.children_mut()[1], "Content-Id", "<other>");
    assert_eq!(
        selected(&entity, TextFormat::Plain).as_deref(),
        Some("first")
    );
}

#[test]
fn metadata_errors_are_explicit_and_do_not_fall_back_to_earlier_alternatives() {
    let mut invalid = leaf(Some(""), "bad");
    let mut entity = multipart(
        "alternative",
        vec![leaf(Some("text/plain"), "valid"), invalid],
    );
    assert!(matches!(
        entity.text_body_part(TextFormat::Plain),
        Err(TextBodyError::ContentType(_))
    ));
    assert!(matches!(
        entity.text_body_part_mut(TextFormat::Plain),
        Err(TextBodyError::ContentType(_))
    ));
    invalid = leaf(Some("text/plain"), "bad");
    header(&mut invalid, "Content-Disposition", "");
    let entity = multipart("mixed", vec![invalid]);
    let error = entity.text_body_part(TextFormat::Plain).unwrap_err();
    assert!(matches!(error, TextBodyError::Disposition(_)));
    assert!(error.source().is_some());
    assert!(error.to_string().contains("Content-Disposition"));
    let missing: MimeEntity =
        Multipart::from_parts(HeaderList::new(), vec![], vec![], vec![]).into();
    assert!(matches!(
        missing.text_body_part(TextFormat::Plain),
        Err(TextBodyError::Multipart(MultipartError::MissingContentType))
    ));
    let mut wrong = multipart("mixed", vec![]);
    header(&mut wrong, "Content-Type", "text/plain");
    assert!(matches!(
        wrong.text_body_part(TextFormat::Plain),
        Err(TextBodyError::Multipart(MultipartError::NotMultipart(_)))
    ));
}

#[test]
fn related_identifier_errors_are_explicit_but_unneeded_fields_are_ignored() {
    let entity = related(Some("<>"), None);
    assert!(matches!(
        entity.text_body_part(TextFormat::Html),
        Err(TextBodyError::RelatedStart(_))
    ));
    let mut entity = related(Some("<root@example.com>"), None);
    let MimeEntity::Multipart(part) = &mut entity else {
        panic!()
    };
    header(&mut part.children_mut()[0], "Content-Id", "<>");
    assert!(matches!(
        entity.text_body_part(TextFormat::Html),
        Err(TextBodyError::ContentId(_))
    ));
    header(
        &mut entity,
        "Content-Type",
        "multipart/related; type=\"text/html\"",
    );
    assert_eq!(
        selected(&entity, TextFormat::Html).as_deref(),
        Some("second")
    );
    let entity = multipart(
        "mixed",
        vec![
            leaf(Some("text/plain"), "body"),
            leaf(Some(""), "unvisited"),
        ],
    );
    assert!(selected(&entity, TextFormat::Plain).is_some());
}

#[test]
fn message_api_selects_encoded_loaded_content_and_preserves_raw_headers() {
    let input = b"Subject: hello\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n\
--x\r\nContent-Type: text/plain\r\n\r\nplain\r\n\
--x\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n\
PGI+aGk8L2I+\r\n--x--\r\n";
    let mut message = MimeMessage::load(&mut &input[..]).unwrap();
    message
        .headers_mut()
        .add(Header::parse(b"Content-Disposition: attachment").unwrap());
    let body = message
        .text_body_part_mut(TextFormat::Html)
        .unwrap()
        .unwrap();
    let raw = body.headers().get("Content-Type").unwrap().raw().to_vec();
    let mut decoded = Vec::new();
    body.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, b"<b>hi</b>");
    assert_eq!(body.headers().get("Content-Type").unwrap().raw(), raw);
    assert!(message.text_body_part(TextFormat::Plain).unwrap().is_some());
    let digest = b"Content-Type: multipart/digest; boundary=x\r\n\r\n--x\r\n\r\n\
Subject: attached\r\n\r\nbody\r\n--x--\r\n";
    for parse_message_parts in [false, true] {
        let options = MimeEntityParseOptions {
            parse_message_parts,
            ..Default::default()
        };
        let message = MimeMessage::load_with_options(&mut &digest[..], &options).unwrap();
        assert!(message.text_body_part(TextFormat::Plain).unwrap().is_none());
    }
}

#[test]
fn selection_requires_no_io_bounds_and_never_touches_sources() {
    struct NoIo;
    fn unbounded<R>(message: &mut MimeMessage<R>) {
        assert!(message.text_body_part(TextFormat::Plain).unwrap().is_none());
        assert!(
            message
                .text_body_part_mut(TextFormat::Plain)
                .unwrap()
                .is_none()
        );
    }
    let mut empty: MimeMessage<NoIo> = MimeMessage::with_body(
        Multipart::with_boundary("mixed", "boundary")
            .unwrap()
            .into(),
    );
    unbounded(&mut empty);
    struct PanicIo;
    impl Read for PanicIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("unexpected read")
        }
    }
    impl Seek for PanicIo {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            panic!("unexpected seek")
        }
    }
    let mut entity: MimeEntity<_> =
        MimePart::from_parts(HeaderList::new(), MimeContent::new(PanicIo)).into();
    assert!(entity.text_body_part(TextFormat::Plain).unwrap().is_some());
    assert!(
        entity
            .text_body_part_mut(TextFormat::Plain)
            .unwrap()
            .is_some()
    );
    let mut source = Cursor::new(b"unchanged");
    source.set_position(3);
    {
        let mut entity: MimeEntity<_> =
            MimePart::from_parts(HeaderList::new(), MimeContent::new(&mut source)).into();
        assert!(
            entity
                .text_body_part_mut(TextFormat::Plain)
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(source.position(), 3);
}

#[test]
fn deep_selection_and_mutable_path_resolution_are_iterative() {
    let mut entity = leaf(Some("text/plain"), "deep");
    for depth in 0..4000 {
        entity = multipart(
            if depth % 2 == 0 {
                "alternative"
            } else {
                "mixed"
            },
            vec![entity],
        );
    }
    let immutable = selected(&entity, TextFormat::Plain);
    let mutable = label(
        entity
            .text_body_part_mut(TextFormat::Plain)
            .unwrap()
            .unwrap(),
    );
    let absent = selected(&entity, TextFormat::Html);
    while let MimeEntity::Multipart(mut part) = entity {
        entity = part.children_mut().pop().unwrap();
    }
    assert_eq!(immutable.as_deref(), Some("deep"));
    assert_eq!(mutable, "deep");
    assert!(absent.is_none());
}

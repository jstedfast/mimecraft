// Adapted from MimeKit's TnefInlineAttachmentTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::super::super::bodies::compressed;
use super::*;
use mimecraft::{Multipart, tnef::TnefConversionResult};

const HTML: &str = "<html><body><img src=\"cid:image1@example.com\"></body></html>";

fn flagged(id: &str) -> Vec<Vec<u8>> {
    vec![
        integer(Tag::ATTACH_FLAGS, 4),
        string(Tag::ATTACH_CONTENT_ID_W, id),
    ]
}

fn convert(fields: &[Vec<u8>], attachments: &[Vec<Vec<u8>>]) -> TnefConversionResult {
    let mut wire = document(Attribute::MAPI_PROPERTIES, &properties(fields));
    for (index, fields) in attachments.iter().enumerate() {
        attachment(&mut wire, &format!("{index}.bin"), &[index as u8, 0, 255]);
        attr(&mut wire, Attribute::ATTACHMENT, &properties(fields));
    }
    // Conversion must outlive both the owned source and its original wire bytes.
    load(&wire).to_mime_message().unwrap()
}

fn multipart<'a, R>(entity: &'a MimeEntity<R>, subtype: &str) -> &'a Multipart<R> {
    let MimeEntity::Multipart(part) = entity else {
        panic!("expected {subtype}")
    };
    assert_eq!(
        part.content_type().unwrap().unwrap().mime_type(),
        format!("multipart/{subtype}")
    );
    part
}

fn assert_related<R>(body: &MimeEntity<R>, root_type: &str, indices: &[usize]) {
    let related = multipart(body, "related");
    let content_type = related.content_type().unwrap().unwrap();
    assert_eq!(content_type.parameters().get("type"), Some(root_type));
    assert!(content_type.parameters().get("start").is_none());
    assert_eq!(related.children().len(), indices.len() + 1);
    let content_type = match &related.children()[0] {
        MimeEntity::Part(part) => part.content_type(),
        MimeEntity::Multipart(part) => part.content_type(),
        _ => panic!("unexpected related root"),
    };
    assert_eq!(content_type.unwrap().unwrap().mime_type(), root_type);
    for (child, index) in related.children()[1..].iter().zip(indices) {
        let MimeEntity::Part(part) = child else {
            panic!("attachment")
        };
        assert_eq!(part.file_name().unwrap(), Some(format!("{index}.bin")));
        assert!(!part.content_disposition().unwrap().unwrap().is_attachment());
    }
}

#[test]
fn upstream_html_reference_and_flag_rules() {
    let cases = [
        (HTML, Some("image1@example.com"), None, None, 4, true),
        (
            "<table background='CID:Image%31@Example.com'>",
            Some(" <image1@example.com> "),
            None,
            None,
            4,
            true,
        ),
        (
            "<x data-anything='cid:image1&#64;example.com'>",
            Some("image1@example.com"),
            None,
            None,
            4,
            true,
        ),
        (HTML, Some("other@example.com"), None, None, 4, false),
        (HTML, Some("image1@example.com"), None, None, 0, false),
        (HTML, None, None, None, 4, false),
        (
            "<x src='http://example.com/images/logo.png'>",
            None,
            Some("http://example.com/images/logo.png"),
            None,
            4,
            true,
        ),
        (
            "<x src='http://example.com/images/logo.png'>",
            None,
            Some("logo.png"),
            Some("http://example.com/images/"),
            4,
            true,
        ),
        ("<x src='logo.png'>", None, Some("logo.png"), None, 4, true),
        ("<x src='Logo.png'>", None, Some("logo.png"), None, 4, false),
        (
            "<x src='http://EXAMPLE.COM:80/images/./logo%2Epng'>",
            None,
            Some("http://example.com/images/logo.png"),
            None,
            4,
            true,
        ),
        (
            "<x src='http://example.com/Images/logo.png'>",
            None,
            Some("http://example.com/images/logo.png"),
            None,
            4,
            false,
        ),
        // URLs in text, CSS, comments, or an attribute substring are not references.
        (
            "<!-- <img src='cid:image1@example.com'> -->cid:image1@example.com",
            Some("image1@example.com"),
            None,
            None,
            4,
            false,
        ),
        (
            "<x style='background:url(cid:image1@example.com)'>",
            Some("image1@example.com"),
            None,
            None,
            4,
            false,
        ),
        (
            "<x src='cid:space+id@example.com'>",
            Some("space+id@example.com"),
            None,
            None,
            4,
            true,
        ),
        (
            "<x src='cid:%2531@example.com'>",
            Some("%31@example.com"),
            None,
            None,
            4,
            true,
        ),
        (
            "<x src='cid:%2531@example.com'>",
            Some("1@example.com"),
            None,
            None,
            4,
            false,
        ),
    ];
    for (html, id, location, base, flags, inline) in cases {
        let mut fields = vec![integer(Tag::ATTACH_FLAGS, flags)];
        for (tag, value) in [
            (Tag::ATTACH_CONTENT_ID_W, id),
            (Tag::ATTACH_CONTENT_LOCATION_W, location),
            (Tag::ATTACH_CONTENT_BASE_W, base),
        ] {
            if let Some(value) = value {
                fields.push(string(tag, value));
            }
        }
        let result = convert(&[string(Tag::BODY_HTML_W, html)], &[fields]);
        assert!(result.losses.is_empty(), "{html}: {:?}", result.losses);
        let body = result.message.body().unwrap();
        if inline {
            assert_related(body, "text/html", &[0]);
        } else {
            let mixed = multipart(body, "mixed");
            let MimeEntity::Part(part) = &mixed.children()[1] else {
                panic!("attachment")
            };
            assert!(
                part.content_disposition().unwrap().unwrap().is_attachment(),
                "{html}"
            );
        }
    }
}

#[test]
fn upstream_content_id_non_ascii_whitespace_is_significant() {
    for (reference, id, inline) in [
        (
            "image1@example.com",
            "\u{a0}image1@example.com\u{a0}",
            false,
        ),
        (
            "%C2%A0image1@example.com%C2%A0",
            "image1@example.com",
            false,
        ),
        (
            "%C2%A0image1@example.com%C2%A0",
            "\u{a0}image1@example.com\u{a0}",
            true,
        ),
    ] {
        let html = format!("<img src='cid:{reference}'>");
        let result = convert(&[string(Tag::BODY_HTML_W, &html)], &[flagged(id)]);
        assert!(result.losses.is_empty(), "{:?}", result.losses);
        multipart(
            result.message.body().unwrap(),
            if inline { "related" } else { "mixed" },
        );
    }
}

#[test]
fn upstream_plain_and_empty_bodies_keep_metadata_without_related() {
    for fields in [vec![], vec![string(Tag::BODY_W, "Hello")]] {
        for disposition in ["attachment", "inline"] {
            let mut fields_attachment = flagged("image1@example.com");
            fields_attachment.push(string(Tag::ATTACH_DISPOSITION_W, disposition));
            let result = convert(&fields, &[fields_attachment]);
            assert!(result.losses.is_empty());
            let mixed = multipart(result.message.body().unwrap(), "mixed");
            assert_eq!(mixed.children().len(), 1 + usize::from(!fields.is_empty()));
            let MimeEntity::Part(part) = mixed.children().last().unwrap() else {
                panic!("attachment")
            };
            assert_eq!(
                part.content_disposition().unwrap().unwrap().disposition(),
                disposition
            );
            assert_eq!(
                part.content_id().unwrap().unwrap().as_str(),
                "image1@example.com"
            );
        }
    }
}

#[test]
fn upstream_related_alternative_and_ordinary_peers_preserve_order_and_round_trip() {
    for plain in [false, true] {
        let html = "<img src='cid:image1@example.com'><img src='cid:image2@example.com'>";
        let mut fields = vec![string(Tag::BODY_HTML_W, html)];
        if plain {
            fields.push(string(Tag::BODY_W, "Hello"));
        }
        let mut custom = flagged("image1@example.com");
        custom.push(string(
            Tag::ATTACH_DISPOSITION_W,
            "attachment; size=3; x-custom=retained",
        ));
        let mut result = convert(
            &fields,
            &[
                vec![],
                custom,
                flagged("image2@example.com"),
                flagged("unreferenced@example.com"),
            ],
        );
        assert!(result.losses.is_empty());
        let mixed = multipart(result.message.body().unwrap(), "mixed");
        assert_eq!(mixed.children().len(), 3);
        let root_type = if plain {
            "multipart/alternative"
        } else {
            "text/html"
        };
        assert_related(&mixed.children()[0], root_type, &[1, 2]);
        let related = multipart(&mixed.children()[0], "related");
        let MimeEntity::Part(part) = &related.children()[1] else {
            panic!("attachment")
        };
        let disposition = part.content_disposition().unwrap().unwrap();
        assert_eq!(disposition.parameters().get("size"), Some("3"));
        assert_eq!(disposition.parameters().get("x-custom"), Some("retained"));
        for (child, index) in mixed.children()[1..].iter().zip([0, 3]) {
            assert_eq!(raw_body(child), [index, 0, 255]);
        }
        let mut wire = Vec::new();
        result.message.write_to(&mut wire).unwrap();
        assert!(wire.is_ascii());
        let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
        let mixed = multipart(parsed.body().unwrap(), "mixed");
        assert_related(&mixed.children()[0], root_type, &[1, 2]);
        let MimeEntity::Multipart(mixed) = parsed.body_mut().unwrap() else {
            panic!("mixed")
        };
        let MimeEntity::Multipart(related) = &mut mixed.children_mut()[0] else {
            panic!("related")
        };
        for (child, index) in related.children_mut()[1..].iter_mut().zip([1, 2]) {
            let MimeEntity::Part(part) = child else {
                panic!("attachment")
            };
            let mut bytes = Vec::new();
            part.content_mut().decode_to(&mut bytes).unwrap();
            assert_eq!(bytes, [index, 0, 255]);
        }
    }
}

#[test]
fn upstream_duplicate_identifiers_and_unreferenced_candidates() {
    let result = convert(
        &[string(Tag::BODY_HTML_W, HTML)],
        &[
            flagged("image1@example.com"),
            flagged("<IMAGE1@example.com>"),
            flagged("missing@example.com"),
        ],
    );
    assert!(result.losses.is_empty());
    let mixed = multipart(result.message.body().unwrap(), "mixed");
    assert_related(&mixed.children()[0], "text/html", &[0, 1]);
    assert_eq!(raw_body(&mixed.children()[1]), [2, 0, 255]);
}

#[test]
fn upstream_embedded_methods_are_never_related_even_if_content_is_unrecognized() {
    for embedded in [false, true] {
        let mut wire = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[string(Tag::BODY_HTML_W, HTML)]),
        );
        attachment(&mut wire, "message.msg", b"invalid message");
        let mut fields = flagged("image1@example.com");
        fields.push(integer(Tag::ATTACH_METHOD, 5));
        if embedded {
            fields.push(object(
                Tag::ATTACH_DATA_OBJ.id(),
                Type::OBJECT,
                &[IID.as_slice(), &empty()].concat(),
            ));
        }
        attr(&mut wire, Attribute::ATTACHMENT, &properties(&fields));
        let result = load(&wire).to_mime_message().unwrap();
        let mixed = multipart(result.message.body().unwrap(), "mixed");
        assert_eq!(mixed.children().len(), 2);
        let MimeEntity::Part(part) = &mixed.children()[1] else {
            panic!("attachment")
        };
        assert!(part.content_disposition().unwrap().unwrap().is_attachment());
    }
}

#[test]
fn upstream_best_body_uses_only_emitted_html_references() {
    for native in [0, 1, 2, 3] {
        for encapsulated in [false, true] {
            let rtf = if encapsulated {
                br"{\rtf1\fromhtml1 {\*\htmltag <img src='cid:image1@example.com'>}}".as_slice()
            } else {
                br"{\rtf1 Hello}"
            };
            let fields = [
                string(Tag::BODY_W, "Hello"),
                string(Tag::BODY_HTML_W, HTML),
                integer(Tag::NATIVE_BODY, native),
                integer(Tag::RTF_IN_SYNC, 1),
                object(Tag::RTF_COMPRESSED.id(), Type::BINARY, &compressed(rtf)),
            ];
            let result = convert(&fields, &[flagged("image1@example.com")]);
            let inline = encapsulated || matches!(native, 1 | 3);
            multipart(
                result.message.body().unwrap(),
                if inline { "related" } else { "mixed" },
            );
            assert!(result.losses.is_empty());
        }
    }
    // Stale stored HTML must not make an attachment related to extracted HTML.
    let result = convert(
        &[
            string(Tag::BODY_HTML_W, HTML),
            integer(Tag::RTF_IN_SYNC, 1),
            object(
                Tag::RTF_COMPRESSED.id(),
                Type::BINARY,
                &compressed(br"{\rtf1\fromhtml1 {\*\htmltag <p>No images</p>}}"),
            ),
        ],
        &[flagged("image1@example.com")],
    );
    assert!(result.losses.is_empty());
    multipart(result.message.body().unwrap(), "mixed");
}

#[test]
fn references_use_emitted_charset_and_report_replacement_without_rewriting_bytes() {
    for (prefix, suffix, id, charset, loss) in [
        (
            "<meta charset=windows-1251>",
            b"\xe0".as_slice(),
            "\u{430}@example.com",
            1252,
            false,
        ),
        ("", b"\xff".as_slice(), "image1@example.com", 65001, true),
    ] {
        let mut html = prefix.as_bytes().to_vec();
        if loss {
            html.extend_from_slice(suffix);
            html.extend_from_slice(HTML.as_bytes());
        } else {
            html.extend_from_slice(b"<img src='cid:");
            html.extend_from_slice(suffix);
            html.extend_from_slice(b"@example.com'>");
        }

        let result = convert(
            &[
                integer(Tag::INTERNET_CODEPAGE, charset),
                object(Tag::BODY_HTML_B.id(), Type::BINARY, &html),
            ],
            &[flagged(id)],
        );
        assert_eq!(result.losses.len(), usize::from(loss));
        if loss {
            assert_eq!(result.losses[0].kind, Loss::InvalidHtmlBody);
        }
        let related = multipart(result.message.body().unwrap(), "related");
        assert_eq!(raw_body(&related.children()[0]), html);
    }
}

#[test]
fn missing_content_and_rejected_locations_do_not_create_related_parts() {
    let mut wire = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[string(Tag::BODY_HTML_W, HTML)]),
    );
    attr(&mut wire, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
    attr(
        &mut wire,
        Attribute::ATTACHMENT,
        &properties(&flagged("image1@example.com")),
    );
    attachment(&mut wire, "second.bin", b"payload");
    attr(
        &mut wire,
        Attribute::ATTACHMENT,
        &properties(&flagged("image1@example.com")),
    );
    let result = load(&wire).to_mime_message().unwrap();
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::AttachmentWithoutContent);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    let related = multipart(result.message.body().unwrap(), "related");
    assert_eq!(related.children().len(), 2);
    assert_eq!(raw_body(&related.children()[1]), b"payload");

    let result = convert(
        &[string(Tag::BODY_HTML_W, "<img src='bad location'>")],
        &[vec![
            integer(Tag::ATTACH_FLAGS, 4),
            string(Tag::ATTACH_CONTENT_LOCATION_W, "bad location"),
        ]],
    );
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidHeader);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    multipart(result.message.body().unwrap(), "mixed");
}

#[test]
fn inline_metadata_and_unresolvable_bases_return_checked_errors() {
    for (fields, resolution) in [
        (
            vec![string(
                Tag::new(Tag::ATTACH_FLAGS.id(), Type::UNICODE),
                "not a flag",
            )],
            false,
        ),
        (
            vec![
                integer(Tag::ATTACH_FLAGS, 4),
                string(Tag::ATTACH_CONTENT_LOCATION_W, "image.png"),
                string(Tag::ATTACH_CONTENT_BASE_W, "urn:example"),
            ],
            true,
        ),
    ] {
        let mut wire = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[string(Tag::BODY_HTML_W, HTML)]),
        );
        attachment(&mut wire, "image.png", b"payload");
        attr(&mut wire, Attribute::ATTACHMENT, &properties(&fields));
        let error = load(&wire).to_mime_message().unwrap_err();
        if resolution {
            assert!(
                matches!(error, TnefConversionError::UriResolution(_)),
                "{error}"
            );
        } else {
            assert!(matches!(error, TnefConversionError::Property(_)), "{error}");
        }
    }
}

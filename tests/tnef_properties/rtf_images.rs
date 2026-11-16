// Adapted from MimeKit's TnefInlineAttachmentTests RTF image cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::super::super::bodies::compressed;
use super::*;
use mimecraft::{MimePart, tnef::TnefConversionResult};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF\0";
const TWO: &[u8] = br"{\rtf1 A\objattph\'20 B\objattph\'20 C}";

struct Attachment<'a> {
    bytes: Option<&'a [u8]>,
    fields: Vec<Vec<u8>>,
}

fn image(bytes: &[u8], position: i32) -> Attachment<'_> {
    Attachment {
        bytes: Some(bytes),
        fields: vec![integer(Tag::RENDERING_POSITION, position)],
    }
}

fn source(rtf: &[u8], fields: &[Vec<u8>], attachments: &[Attachment<'_>]) -> Message {
    let mut fields = fields.to_vec();
    fields.push(object(
        Tag::RTF_COMPRESSED.id(),
        Type::BINARY,
        &compressed(rtf),
    ));
    let mut wire = document(Attribute::MAPI_PROPERTIES, &properties(&fields));
    for (index, attachment) in attachments.iter().enumerate() {
        attr(&mut wire, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
        let mut fields = attachment.fields.clone();
        fields.push(string(Tag::ATTACH_LONG_FILENAME_W, &format!("{index}.bin")));
        if let Some(bytes) = attachment.bytes {
            fields.push(object(Tag::ATTACH_DATA_BIN.id(), Type::BINARY, bytes));
        }
        attr(&mut wire, Attribute::ATTACHMENT, &properties(&fields));
    }
    load(&wire)
}

fn convert(rtf: &[u8], attachments: &[Attachment<'_>]) -> TnefConversionResult {
    let result = source(rtf, &[], attachments).to_mime_message().unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    result
}

fn text<'a>(result: &'a TnefConversionResult, subtype: &str) -> &'a str {
    let part = result
        .message
        .leaf_parts()
        .find(|part| {
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("text", subtype)
        })
        .unwrap();
    std::str::from_utf8(part.content().get_ref().get_ref()).unwrap()
}

fn attachments(result: &TnefConversionResult) -> Vec<&MimePart<Cursor<Vec<u8>>>> {
    result
        .message
        .leaf_parts()
        .filter(|part| part.file_name().unwrap().is_some())
        .collect()
}

fn img(part: &MimePart<Cursor<Vec<u8>>>) -> String {
    format!(
        "<img src=\"cid:{}\" alt=\"{}\"/>",
        part.content_id().unwrap().unwrap().as_str(),
        part.file_name().unwrap().unwrap()
    )
}

#[test]
fn upstream_matching_placeholders_use_images_only_and_round_trip() {
    let mut result = convert(TWO, &[image(PNG, 0), image(b"payload", 1)]);
    let parts = attachments(&result);
    assert_eq!(parts.len(), 2);
    assert!(text(&result, "plain").contains("A B C"));
    // The placeholder character following \objattph is not part of the text.
    assert!(text(&result, "html").contains(&format!("A{} B C", img(parts[0]))));
    assert_eq!(
        parts[0].content_type().unwrap().unwrap().mime_type(),
        "image/png"
    );
    assert!(
        !parts[0]
            .content_disposition()
            .unwrap()
            .unwrap()
            .is_attachment()
    );
    assert!(
        parts[1]
            .content_disposition()
            .unwrap()
            .unwrap()
            .is_attachment()
    );
    let MimeEntity::Multipart(mixed) = result.message.body().unwrap() else {
        panic!("mixed")
    };
    assert_eq!(mixed.children().len(), 2);
    let MimeEntity::Multipart(related) = &mixed.children()[0] else {
        panic!("related")
    };
    assert_eq!(
        related
            .content_type()
            .unwrap()
            .unwrap()
            .parameters()
            .get("type"),
        Some("multipart/alternative")
    );
    assert_eq!(related.children().len(), 2);
    let id = parts[0].content_id().unwrap().unwrap().to_string();
    let mut wire = Vec::new();
    result.message.write_to(&mut wire).unwrap();
    let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    let MimeEntity::Multipart(mixed) = parsed.body_mut().unwrap() else {
        panic!("mixed")
    };
    let MimeEntity::Multipart(related) = &mut mixed.children_mut()[0] else {
        panic!("related")
    };
    let MimeEntity::Part(image) = &mut related.children_mut()[1] else {
        panic!("image")
    };
    assert_eq!(image.content_id().unwrap().unwrap().to_string(), id);
    let mut bytes = Vec::new();
    image.content_mut().decode_to(&mut bytes).unwrap();
    assert_eq!(bytes, PNG);
}

#[test]
fn upstream_rendering_positions_are_unsigned_and_stable_not_mime_order() {
    let rtf = br"{\rtf1 A\objattph\'20 B\objattph\'20 C\objattph\'20 D\objattph\'20 E}";
    let result = convert(
        rtf,
        &[
            image(PNG, -2),
            image(PNG, 20),
            image(PNG, 10),
            image(PNG, 10),
        ],
    );
    let parts = attachments(&result);
    for (index, part) in parts.iter().enumerate() {
        assert_eq!(part.file_name().unwrap(), Some(format!("{index}.bin")));
    }
    assert!(text(&result, "html").contains(&format!(
        "A{} B{} C{} D{} E",
        img(parts[2]),
        img(parts[3]),
        img(parts[1]),
        img(parts[0])
    )));
}

#[test]
fn upstream_count_mismatch_and_no_placeholders_append_images_in_rendering_order() {
    for rtf in [br"{\rtf1 A\objattph\'20 B}".as_slice(), br"{\rtf1 A B}"] {
        let result = convert(rtf, &[image(PNG, 20), image(JPEG, 10)]);
        let parts = attachments(&result);
        let html = text(&result, "html");
        assert!(html.contains(&format!(
            "<div>{}</div><div>{}</div>",
            img(parts[1]),
            img(parts[0])
        )));
        assert!(html.find("A B").unwrap() < html.find("<img").unwrap());
        assert_eq!(html.matches("<img").count(), 2);
        assert!(!text(&result, "plain").contains(".bin"));
    }
}

#[test]
fn upstream_hidden_and_unrendered_attachments_do_not_occupy_slots() {
    let mut hidden = image(PNG, 0);
    hidden.fields.push(integer(Tag::ATTACHMENT_HIDDEN, 1));
    let result = convert(
        br"{\rtf1 A\objattph\'20 B}",
        &[hidden, image(PNG, -1), image(PNG, 2)],
    );
    let parts = attachments(&result);
    assert_eq!(parts[0].file_name().unwrap().as_deref(), Some("2.bin"));
    assert!(text(&result, "html").contains(&format!("A{} B", img(parts[0]))));
    assert_eq!(text(&result, "html").matches("<img").count(), 1);
    for part in &parts[1..] {
        assert!(part.content_id().unwrap().is_none());
        assert!(part.content_disposition().unwrap().unwrap().is_attachment());
    }
}

#[test]
fn missing_content_retains_its_placeholder_slot_and_loss_index() {
    let result = source(
        TWO,
        &[],
        &[
            Attachment {
                bytes: None,
                fields: vec![integer(Tag::RENDERING_POSITION, 0)],
            },
            image(PNG, 1),
        ],
    )
    .to_mime_message()
    .unwrap();
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    let parts = attachments(&result);
    assert_eq!(parts.len(), 1);
    assert!(text(&result, "html").contains(&format!("A B{} C", img(parts[0]))));
}

#[test]
fn upstream_image_signatures_override_only_compatible_declared_types() {
    let mut bmp = b"BM".to_vec();
    bmp.resize(14, 0);
    bmp.extend_from_slice(&40u32.to_le_bytes());
    let cases = [
        (PNG, "png"),
        (JPEG, "jpeg"),
        (b"GIF87a".as_slice(), "gif"),
        (b"GIF89a".as_slice(), "gif"),
        (b"RIFF\0\0\0\0WEBP".as_slice(), "webp"),
        (bmp.as_slice(), "bmp"),
    ];
    for (bytes, subtype) in cases {
        for declared in ["application/octet-stream", "image/wrong"] {
            let mut attachment = image(bytes, 0);
            attachment.fields.push(string(
                Tag::ATTACH_MIME_TAG_W,
                &format!("{declared}; x-test=kept"),
            ));
            attachment
                .fields
                .push(string(Tag::ATTACH_DISPOSITION_W, "attachment; size=123"));
            let result = convert(br"{\rtf1 A\objattph\'20 B}", &[attachment]);
            let parts = attachments(&result);
            let ty = parts[0].content_type().unwrap().unwrap();
            assert_eq!(ty.mime_type(), format!("image/{subtype}"));
            assert_eq!(ty.parameters().get("x-test"), Some("kept"));
            assert_eq!(
                parts[0]
                    .content_disposition()
                    .unwrap()
                    .unwrap()
                    .parameters()
                    .get("size"),
                Some("123")
            );
            assert!(text(&result, "html").contains(&img(parts[0])));
        }
        // A signature prefix alone is insufficient.
        let result = convert(br"{\rtf1 A\objattph\'20 B}", &[image(&bytes[..2], 0)]);
        assert!(!text(&result, "html").contains("<img"));
    }
    for declared in ["application/pdf", "text/html", "application/ms-tnef"] {
        let mut attachment = image(PNG, 0);
        attachment
            .fields
            .push(string(Tag::ATTACH_MIME_TAG_W, declared));
        let result = convert(TWO, &[attachment]);
        assert!(!text(&result, "html").contains("<img"));
        assert_eq!(
            attachments(&result)[0]
                .content_type()
                .unwrap()
                .unwrap()
                .mime_type(),
            declared
        );
    }
    for bytes in [
        b"\x89PNG\r\n\x1a".as_slice(),
        b"GIF88a",
        b"RIFF\0\0\0\0WAVE",
        b"BM\0\0\0\0\0\0\0\0\0\0\0\0\x13\0\0\0",
    ] {
        let result = convert(TWO, &[image(bytes, 0)]);
        assert!(!text(&result, "html").contains("<img"));
    }
}

#[test]
fn upstream_duplicate_cids_include_ordinary_and_hidden_parts_and_leave_source_unchanged() {
    for hidden in [false, true] {
        let mut first = image(PNG, 20);
        first
            .fields
            .push(string(Tag::ATTACH_CONTENT_ID_W, "same@example.com"));
        let mut second = image(PNG, 10);
        second
            .fields
            .push(string(Tag::ATTACH_CONTENT_ID_W, "SAME@example.com"));
        if hidden {
            second.fields.push(integer(Tag::ATTACHMENT_HIDDEN, 1));
        }
        let source = source(TWO, &[], &[first, second]);
        let result = source.to_mime_message().unwrap();
        assert!(result.losses.is_empty());
        let parts = attachments(&result);
        let ids: Vec<_> = parts
            .iter()
            .map(|part| part.content_id().unwrap().unwrap().to_string())
            .collect();
        assert!(!ids[0].eq_ignore_ascii_case(&ids[1]));
        assert_eq!(
            ids[usize::from(hidden)],
            if hidden {
                "SAME@example.com"
            } else {
                "same@example.com"
            }
        );
        assert_eq!(
            source.attachments()[0].content_id().unwrap().as_deref(),
            Some("same@example.com")
        );
        assert!(text(&result, "html").contains(&img(parts[0])));
        if !hidden {
            assert!(text(&result, "html").contains(&format!(
                "A{} B{} C",
                img(parts[1]),
                img(parts[0])
            )));
        }
    }
    let mut image = image(PNG, 0);
    image
        .fields
        .push(string(Tag::ATTACH_CONTENT_ID_W, "unique@example.com"));
    let result = convert(TWO, &[image]);
    assert_eq!(
        attachments(&result)[0]
            .content_id()
            .unwrap()
            .unwrap()
            .as_str(),
        "unique@example.com"
    );
}

#[test]
fn upstream_image_markup_encodes_cids_and_escapes_filenames() {
    let mut attachment = image(PNG, 0);
    attachment
        .fields
        .push(string(Tag::ATTACH_CONTENT_ID_W, "a%b\"c/?#@example.com"));
    attachment
        .fields
        .push(string(Tag::ATTACH_LONG_FILENAME_W, "a&\"<b>.png"));
    let result = convert(br"{\rtf1 A\objattph\'20 B}", &[attachment]);
    assert!(text(&result, "html").contains(
        "<img src=\"cid:a%25b%22c%2F%3F%23@example.com\" alt=\"a&amp;&quot;&lt;b&gt;.png\"/>"
    ));
}

#[test]
fn encapsulated_and_unselected_rtf_do_not_prepare_images() {
    let mut attachment = image(PNG, 0);
    attachment.fields.push(integer(Tag::ATTACH_FLAGS, 4));
    attachment
        .fields
        .push(string(Tag::ATTACH_CONTENT_ID_W, "image@example.com"));
    attachment
        .fields
        .push(string(Tag::ATTACH_MIME_TAG_W, "image/wrong"));
    let rtf = br"{\rtf1\fromhtml1 {\*\htmltag <p><img src='cid:image@example.com'>}A\objattph\'20 B{\*\htmltag </p>}}";
    let result = convert(rtf, &[attachment]);
    let parts = attachments(&result);
    assert_eq!(
        parts[0].content_type().unwrap().unwrap().mime_type(),
        "image/wrong"
    );
    assert_eq!(
        parts[0].content_id().unwrap().unwrap().as_str(),
        "image@example.com"
    );
    assert_eq!(text(&result, "html").matches("<img").count(), 1);
    assert!(!text(&result, "html").contains("alt="));

    let result = source(
        TWO,
        &[
            string(Tag::BODY_W, "Stored plain"),
            integer(Tag::NATIVE_BODY, 1),
        ],
        &[image(PNG, 0)],
    )
    .to_mime_message()
    .unwrap();
    assert!(result.losses.is_empty());
    assert_eq!(text(&result, "plain"), "Stored plain");
    assert!(attachments(&result)[0].content_id().unwrap().is_none());

    let result = convert(
        br"{\rtf1\fromhtml1 {\*\htmltag <p>}A\objattph\'20 B{\*\htmltag </p>}}",
        &[image(PNG, 0)],
    );
    assert!(!text(&result, "html").contains("<img"));
    assert!(attachments(&result)[0].content_id().unwrap().is_none());
}

#[test]
fn visible_placeholder_counts_agree_across_rtf_renderers() {
    for rtf in [
        br"{\rtf1 {\v hidden\objattph\'20}{\*\unknown \objattph\'20}A\objattph\'20 B}".as_slice(),
        br"{\rtf1 {\fonttbl{\f0 Arial;}}{\field{\*\fldinst HYPERLINK https://example.com}{\fldrslt A\objattph\'20 B}}}",
        br"{\rtf1 A\objattph\'20 B",
    ] {
        let result = source(rtf, &[], &[image(PNG, 0)]).to_mime_message().unwrap();
        assert!(result.losses.iter().all(|loss| loss.kind == Loss::RtfRenderingIssue));
        let html = text(&result, "html");
        assert_eq!(html.matches("<img").count(), 1);
        assert!(html.contains(&format!("A{} B", img(attachments(&result)[0]))), "{html}");
        assert!(text(&result, "plain").contains("A B"));
    }
}

#[test]
fn rtf_placement_metadata_uses_checked_conversions() {
    for tag in [Tag::RENDERING_POSITION, Tag::ATTACHMENT_HIDDEN] {
        let mut attachment = image(PNG, 0);
        if tag == Tag::RENDERING_POSITION {
            attachment.fields.clear();
        }
        attachment
            .fields
            .push(string(Tag::new(tag.id(), Type::UNICODE), "invalid"));
        assert!(matches!(
            source(TWO, &[], &[attachment]).to_mime_message(),
            Err(TnefConversionError::Property(_))
        ));
    }
}

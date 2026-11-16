// Adapted from MimeKit's TnefMessageConverterTests attachment cases,
// TnefPartConversionTests, and TnefInlineAttachmentTests OLE passthrough case.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::{
    ContentEncoding as Encoding, DateTimeOffset, MimeEntity, MimeParser,
    tnef::{
        TnefAttachmentConversionError as MimeError, TnefConversionLossKind as Loss, TnefDateTime,
        TnefDateTimeKind,
    },
};

#[test]
fn upstream_attachment_content_type_is_sanitized() {
    for (mime, expected) in [
        ("image/png", "image/png"),
        ("text/plain", "text/plain"),
        ("multipart/mixed; boundary=x", "application/octet-stream"),
        ("message/rfc822", "application/octet-stream"),
        ("message/partial", "application/octet-stream"),
        ("application/applefile", "application/octet-stream"),
        ("application/mac-binhex40", "application/octet-stream"),
        ("not a mime type", "application/octet-stream"),
        ("MESSAGE/global", "application/octet-stream"),
    ] {
        let attachment = Attachment::new(
            set(&[
                string(Tag::ATTACH_LONG_FILENAME_W, "file.bin"),
                string(Tag::ATTACH_MIME_TAG_W, mime),
            ]),
            Some(Content::Binary(b"ABC".to_vec())),
        );
        let result = attachment.to_mime_part().unwrap();
        assert_eq!(result.losses.len(), usize::from(mime != expected), "{mime}");
        let mut part = result.part.unwrap();
        assert_eq!(part.content_type().unwrap().unwrap().mime_type(), expected);
        assert_eq!(part.file_name().unwrap().as_deref(), Some("file.bin"));
        // A forged composite type must not become structure on a MIME round trip.
        let mut wire = Vec::new();
        part.write_to(&mut wire).unwrap();
        let MimeEntity::Part(mut parsed) = MimeParser::new(wire.as_slice()).parse_entity().unwrap()
        else {
            panic!("attachment became MIME structure");
        };
        let mut decoded = Vec::new();
        parsed.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, b"ABC");
    }
}

#[test]
fn missing_external_empty_and_invalid_embedded_content_are_distinct() {
    for method in [0i32, 1, 2, 3, 4, 5, 6, 77] {
        let properties = set(&[fixed(
            Tag::ATTACH_METHOD.id(),
            Type::LONG,
            &method.to_le_bytes(),
        )]);
        let missing = Attachment::new(properties.clone(), None)
            .to_mime_part()
            .unwrap();
        assert!(missing.part.is_none());
        assert_eq!(missing.losses.len(), 1);
        assert_eq!(
            missing.losses[0].kind,
            if (2..=4).contains(&method) {
                Loss::UnsupportedAttachMethod
            } else {
                Loss::AttachmentWithoutContent
            }
        );
        let present = Attachment::new(properties, Some(Content::Binary(Vec::new())))
            .to_mime_part()
            .unwrap();
        assert!(
            present
                .part
                .unwrap()
                .content()
                .get_ref()
                .get_ref()
                .is_empty()
        );
        let expected = match method {
            5 => vec![Loss::InvalidEmbeddedMessage],
            6 => vec![Loss::OleObjectNotRendered],
            _ => vec![],
        };
        assert_eq!(
            present
                .losses
                .iter()
                .map(|loss| loss.kind)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn upstream_embedded_messages_pass_through_and_outlive_the_source() {
    let attachment = embedded_attachment(None);
    let original = attachment.as_bytes().unwrap().to_vec();
    let result = attachment.to_mime_part().unwrap();
    assert_eq!(attachment.as_bytes(), Some(original.as_slice()));
    drop(attachment);
    assert!(result.losses.is_empty());
    let mut part = result.part.unwrap();
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("application", "ms-tnef")
    );
    assert!(part.content_disposition().unwrap().is_none());
    assert_eq!(part.file_name().unwrap(), None);
    assert_eq!(part.content_transfer_encoding().unwrap(), Encoding::Base64);
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, original);
    let nested = mimecraft::tnef::TnefMessage::load(decoded.as_slice()).unwrap();
    assert_eq!(nested.text_body().unwrap().text().unwrap(), "nested");
}

#[test]
fn upstream_truncated_object_and_unrendered_ole_keep_exposed_bytes() {
    for length in [0, 7, 15, 16, 17, 32] {
        let bytes: Vec<_> = (0..length).collect();
        let attachment = Attachment::new(
            set(&[fixed(
                Tag::ATTACH_METHOD.id(),
                Type::LONG,
                &6i32.to_le_bytes(),
            )]),
            Some(Content::Object(bytes.clone())),
        );
        let result = attachment.to_mime_part().unwrap();
        assert_eq!(
            result
                .losses
                .iter()
                .map(|loss| loss.kind)
                .collect::<Vec<_>>(),
            [Loss::OleObjectNotRendered]
        );
        assert_eq!(
            result.part.unwrap().content().get_ref().get_ref(),
            &bytes[bytes.len().min(16)..]
        );
    }
}

#[test]
fn attachment_metadata_uses_filename_display_name_and_utc_date_precedence() {
    let date = TnefDateTime::new(2026, 5, 6, 7, 8, 9, TnefDateTimeKind::Utc).unwrap();
    let attachment = Attachment::new(
        set(&[
            string(Tag::ATTACH_FILENAME_W, "SHORT.TXT"),
            string(Tag::ATTACH_LONG_FILENAME_W, "caf\u{e9}.txt"),
            string(Tag::DISPLAY_NAME_W, "Display name"),
            string(
                Tag::ATTACH_MIME_TAG_W,
                "text/plain; charset=windows-1252; name=old",
            ),
            string(
                Tag::ATTACH_DISPOSITION_W,
                "inline; filename=old; x-keep=yes",
            ),
            fixed(Tag::ATTACH_SIZE.id(), Type::LONG, &123i32.to_le_bytes()),
            fixed(
                Tag::CREATION_TIME.id(),
                Type::SYS_TIME,
                &date.to_file_time().unwrap().to_le_bytes(),
            ),
            fixed(
                Tag::LAST_MODIFICATION_TIME.id(),
                Type::SYS_TIME,
                &date.to_file_time().unwrap().to_le_bytes(),
            ),
            string(Tag::ATTACH_CONTENT_ID_W, " <Part@EXAMPLE.COM> (comment)"),
            string(Tag::ATTACH_CONTENT_LOCATION_W, " logo.png "),
            string(Tag::ATTACH_CONTENT_BASE_W, " https://example.com/images/ "),
        ]),
        Some(Content::Binary(b"content".to_vec())),
    );
    let result = attachment.to_mime_part().unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let part = result.part.unwrap();
    let content_type = part.content_type().unwrap().unwrap();
    assert_eq!(content_type.name(), Some("Display name"));
    assert_eq!(content_type.charset(), Some("windows-1252"));
    let disposition = part.content_disposition().unwrap().unwrap();
    assert_eq!(disposition.disposition(), "inline");
    assert_eq!(disposition.file_name(), Some("caf\u{e9}.txt"));
    assert_eq!(disposition.parameters().get("x-keep"), Some("yes"));
    assert_eq!(disposition.size().unwrap(), Some(123));
    let expected = Some(DateTimeOffset::new(2026, 5, 6, 7, 8, 9, 0).unwrap());
    assert_eq!(disposition.creation_date().unwrap(), expected);
    assert_eq!(disposition.modification_date().unwrap(), expected);
    assert_eq!(
        part.content_id().unwrap().unwrap().as_str(),
        "Part@EXAMPLE.COM"
    );
    assert_eq!(
        part.headers()
            .get("Content-Location")
            .unwrap()
            .decode_value()
            .text,
        "logo.png"
    );
    assert_eq!(
        part.headers()
            .get("Content-Base")
            .unwrap()
            .decode_value()
            .text,
        "https://example.com/images/"
    );
}

#[test]
fn conversion_retains_encoding_parameters_and_does_not_infer_inline_from_flags() {
    for (bytes, expected) in [
        (b"plain\r\n".as_slice(), Encoding::SevenBit),
        (b"From someone".as_slice(), Encoding::QuotedPrintable),
        (b"hello\0world".as_slice(), Encoding::Base64),
        (b"\xff\xfe\xfd".as_slice(), Encoding::Base64),
        (b"".as_slice(), Encoding::SevenBit),
    ] {
        let attachment = Attachment::new(
            set(&[
                string(Tag::ATTACH_MIME_TAG_W, "text/plain; charset=windows-1252"),
                string(Tag::ATTACH_FILENAME_W, "short.txt"),
                fixed(Tag::ATTACH_FLAGS.id(), Type::LONG, &4i32.to_le_bytes()),
                fixed(Tag::ATTACH_SIZE.id(), Type::LONG, &(-1i32).to_le_bytes()),
            ]),
            Some(Content::Binary(bytes.to_vec())),
        );
        let result = attachment.to_mime_part().unwrap();
        assert!(result.losses.is_empty());
        let mut part = result.part.unwrap();
        assert_eq!(part.content_transfer_encoding().unwrap(), expected);
        assert_eq!(part.content().get_ref().get_ref(), bytes);
        assert!(part.is_attachment().unwrap());
        assert_eq!(
            part.content_disposition().unwrap().unwrap().size().unwrap(),
            None
        );
        let mut wire = Vec::new();
        part.write_to(&mut wire).unwrap();
        let MimeEntity::Part(mut parsed) = MimeParser::new(wire.as_slice()).parse_entity().unwrap()
        else {
            panic!("expected a MIME leaf");
        };
        let mut decoded = Vec::new();
        parsed.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, bytes);
    }
}

#[test]
fn invalid_headers_are_reported_without_injection_or_silent_defaults() {
    let attachment = Attachment::new(
        set(&[
            string(Tag::ATTACH_MIME_TAG_W, "message/rfc822"),
            string(Tag::ATTACH_DISPOSITION_W, "@invalid"),
            string(Tag::ATTACH_CONTENT_ID_W, "<a\0b@example.com>"),
            string(Tag::ATTACH_CONTENT_LOCATION_W, "x\r\nX-Injected: yes"),
            string(Tag::ATTACH_CONTENT_BASE_W, "relative/path"),
        ]),
        Some(Content::Binary(b"abc".to_vec())),
    );
    let result = attachment.to_mime_part().unwrap();
    assert_eq!(
        result
            .losses
            .iter()
            .map(|loss| loss.kind)
            .collect::<Vec<_>>(),
        [
            Loss::InvalidHeader,
            Loss::InvalidHeader,
            Loss::InvalidMessageId,
            Loss::InvalidHeader,
            Loss::InvalidHeader
        ]
    );
    let part = result.part.unwrap();
    assert!(part.content_disposition().unwrap().is_none());
    assert!(part.content_id().unwrap().is_none());
    assert!(!part.headers().contains("Content-Location"));
    assert!(!part.headers().contains("Content-Base"));
    assert!(!part.headers().contains("X-Injected"));

    // The Rust property model reports incompatible values instead of treating
    // them as missing metadata (the source's unexpected-type tests are tolerant).
    for tag in [Tag::ATTACH_MIME_TAG_W, Tag::ATTACH_CONTENT_LOCATION_W] {
        let attachment = Attachment::new(
            set(&[fixed(tag.id(), Type::LONG, &1234i32.to_le_bytes())]),
            Some(Content::Binary(b"abc".to_vec())),
        );
        assert!(matches!(
            attachment.to_mime_part(),
            Err(MimeError::Property(ConversionError::InvalidType))
        ));
    }
}

#[test]
fn authored_filename_parameters_cannot_inject_headers_and_empty_names_do_not_fallback() {
    for name in ["caf\u{e9}\r\nX-Injected: yes.txt", ""] {
        let attachment = Attachment::new(
            set(&[
                string(Tag::ATTACH_FILENAME_W, "short.txt"),
                string(Tag::ATTACH_LONG_FILENAME_W, name),
            ]),
            Some(Content::Binary(b"abc".to_vec())),
        );
        let result = attachment.to_mime_part().unwrap();
        assert!(result.losses.is_empty());
        let mut part = result.part.unwrap();
        assert_eq!(
            part.file_name().unwrap().as_deref(),
            (!name.is_empty()).then_some(name)
        );
        let mut wire = Vec::new();
        part.write_to(&mut wire).unwrap();
        let parsed = MimeParser::new(wire.as_slice()).parse_entity().unwrap();
        assert!(!parsed.headers().contains("X-Injected"));
    }
}

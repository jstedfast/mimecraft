// Adapted from MimeKit's TnefInlineAttachmentTests placeholder callback cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::tnef::{TnefAttachment, TnefConversionOptions};
use std::{
    io,
    sync::{Arc, Mutex},
};

fn options() -> TnefConversionOptions {
    TnefConversionOptions {
        attachment_placeholder_callback: Some(Arc::new(|attachment, _| {
            Ok(attachment.file_name()?.map(|name| format!("<<{name}>>")))
        })),
        ..Default::default()
    }
}

fn named_image<'a>(bytes: &'a [u8], position: i32, name: &str) -> Attachment<'a> {
    let mut attachment = image(bytes, position);
    attachment
        .fields
        .push(string(Tag::ATTACH_LONG_FILENAME_W, name));
    attachment
}

#[test]
fn upstream_callback_uses_text_for_all_attachments_but_html_prefers_images_and_escapes_text() {
    let mut result = source(
        TWO,
        &[],
        &[
            named_image(PNG, 0, "a.png"),
            named_image(b"payload", 1, "b&c.bin"),
        ],
    )
    .to_mime_message_with_options(&options())
    .unwrap();
    assert!(result.losses.is_empty());
    assert!(text(&result, "plain").contains("A<<a.png>> B<<b&c.bin>> C"));
    let parts = attachments(&result);
    assert!(text(&result, "html").contains(&format!(
        "A{} B&lt;&lt;b&amp;c.bin&gt;&gt; C",
        img(parts[0])
    )));
    assert!(
        parts[1]
            .content_disposition()
            .unwrap()
            .unwrap()
            .is_attachment()
    );
    let expected = text(&result, "plain").to_owned();
    let mut wire = Vec::new();
    result.message.write_to(&mut wire).unwrap();
    let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    let MimeEntity::Multipart(mixed) = parsed.body_mut().unwrap() else {
        panic!("mixed")
    };
    let MimeEntity::Multipart(related) = &mut mixed.children_mut()[0] else {
        panic!("related")
    };
    let MimeEntity::Multipart(alternative) = &mut related.children_mut()[0] else {
        panic!("alternative")
    };
    let MimeEntity::Part(plain) = &mut alternative.children_mut()[0] else {
        panic!("plain")
    };
    let mut decoded = Vec::new();
    plain.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, expected.as_bytes());
}

#[test]
fn upstream_callback_arguments_follow_rendering_order_and_none_or_empty_emit_nothing() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let options = TnefConversionOptions {
        attachment_placeholder_callback: Some(Arc::new(move |attachment, entity| {
            let name = attachment.file_name()?.unwrap().into_owned();
            let part = match entity.unwrap() {
                MimeEntity::Part(part) => part,
                _ => panic!("leaf"),
            };
            assert_eq!(part.file_name().unwrap().as_deref(), Some(name.as_str()));
            observed.lock().unwrap().push(name.clone());
            Ok((name == "second.bin").then(String::new))
        })),
        ..Default::default()
    };
    let mut hidden = named_image(b"hidden", 0, "hidden.bin");
    hidden.fields.push(integer(Tag::ATTACHMENT_HIDDEN, 1));
    let result = source(
        TWO,
        &[],
        &[
            named_image(b"second", 2, "second.bin"),
            hidden,
            named_image(b"first", 1, "first.bin"),
            named_image(b"excluded", -1, "excluded.bin"),
        ],
    )
    .to_mime_message_with_options(&options)
    .unwrap();
    assert!(result.losses.is_empty());
    assert_eq!(*calls.lock().unwrap(), ["first.bin", "second.bin"]);
    assert!(text(&result, "plain").contains("A B C"));
    assert!(text(&result, "html").contains("A B C"));
}

#[test]
fn upstream_count_mismatch_appends_text_and_images_with_crlf_and_escaped_divs() {
    for rtf in [
        br"{\rtf1 A B}".as_slice(),
        br"{\rtf1 A\objattph\'20 B}",
        br"{\rtf1 A\objattph\'20 B\objattph\'20 C\objattph\'20 D}",
    ] {
        let result = source(
            rtf,
            &[],
            &[
                named_image(PNG, 1, "a.png"),
                named_image(b"data", 2, "b&c.bin"),
            ],
        )
        .to_mime_message_with_options(&options())
        .unwrap();
        assert!(result.losses.is_empty());
        assert!(text(&result, "plain").ends_with("\r\n<<a.png>>\r\n<<b&c.bin>>\r\n"));
        assert!(text(&result, "html").contains(&format!(
            "<div>{}</div><div>&lt;&lt;b&amp;c.bin&gt;&gt;</div>",
            img(attachments(&result)[0])
        )));
    }
}

#[test]
fn upstream_encapsulated_html_stays_original_but_plain_text_uses_callback() {
    let rtf = br"{\rtf1\fromhtml1 {\*\htmltag <p>}A\objattph\'20 B{\*\htmltag </p>}}";
    let source = source(rtf, &[], &[named_image(b"data", 0, "a.bin")]);
    let default = source.to_mime_message().unwrap();
    let result = source.to_mime_message_with_options(&options()).unwrap();
    assert!(result.losses.is_empty());
    assert!(text(&result, "plain").contains("A<<a.bin>> B"));
    assert_eq!(text(&result, "html"), text(&default, "html"));
    assert!(!text(&result, "html").contains("a.bin"));
}

#[test]
fn callback_is_bypassed_for_non_rtf_best_bodies_and_successful_skeletons() {
    let skeleton = concat!(
        "Content-Type: multipart/mixed; boundary=x\r\n\r\n",
        "--x\r\nContent-Type: text/plain\r\n\r\n",
        "--x\r\nContent-Type: application/octet-stream\r\n",
        "X-Exchange-MIME-Skeleton-Content-Id: a\r\n\r\n--x--\r\n",
    );
    let options = TnefConversionOptions {
        attachment_placeholder_callback: Some(Arc::new(|_, _| panic!("callback must not run"))),
        ..Default::default()
    };
    for fields in [
        vec![string(Tag::BODY_W, "plain"), integer(Tag::RTF_IN_SYNC, 0)],
        vec![
            object(Tag::BODY_HTML_B.id(), Type::BINARY, b"<p>stored HTML</p>"),
            integer(Tag::NATIVE_BODY, 3),
        ],
        vec![
            string(Tag::BODY_W, "plain"),
            object(Tag::MIME_SKELETON.id(), Type::BINARY, skeleton.as_bytes()),
        ],
    ] {
        let mut attachment = image(b"data", 0);
        attachment
            .fields
            .push(string(Tag::ATTACH_CONTENT_ID_W, "a"));
        let result = source(TWO, &fields, &[attachment])
            .to_mime_message_with_options(&options)
            .unwrap();
        assert!(result.losses.is_empty());
    }
}

#[test]
fn invalid_skeleton_falls_back_to_callback_and_rendered_ole_entity_is_supplied() {
    let mut attachment = named_image(b"object", 0, "object.bin");
    attachment.fields.push(integer(Tag::ATTACH_METHOD, 6));
    let calls = Arc::new(Mutex::new(0));
    let seen = calls.clone();
    let options = TnefConversionOptions {
        ole_object_converter: Some(Arc::new(|_: &TnefAttachment| {
            Ok(Some(
                Box::new(Cursor::new(PNG.to_vec())) as Box<dyn io::Read>
            ))
        })),
        attachment_placeholder_callback: Some(Arc::new(move |source, entity| {
            *seen.lock().unwrap() += 1;
            assert_eq!(source.file_name()?.as_deref(), Some("object.bin"));
            let MimeEntity::Part(part) = entity.unwrap() else {
                panic!("rendered image")
            };
            assert_eq!(part.file_name().unwrap().as_deref(), Some("object.bin.png"));
            assert!(
                part.content_type()
                    .unwrap()
                    .unwrap()
                    .is_mime_type("image", "png")
            );
            assert!(part.content_id().unwrap().is_none());
            assert!(part.content_disposition().unwrap().unwrap().is_attachment());
            Ok(Some("OLE label".into()))
        })),
        ..Default::default()
    };
    let result = source(
        br"{\rtf1 A\objattph\'20 B}",
        &[object(
            Tag::MIME_SKELETON.id(),
            Type::BINARY,
            b"not a skeleton",
        )],
        &[attachment],
    )
    .to_mime_message_with_options(&options)
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidMimeSkeleton);
    assert!(text(&result, "plain").contains("AOLE label B"));
    assert!(!text(&result, "html").contains("OLE label"));
    assert!(text(&result, "html").contains(&img(attachments(&result)[0])));
}

#[test]
fn missing_content_retains_callback_slot_and_unicode_adjacent_placeholders_are_safe() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let options = TnefConversionOptions {
        attachment_placeholder_callback: Some(Arc::new(move |attachment, entity| {
            seen.lock().unwrap().push(entity.is_some());
            Ok(Some(format!("[{}]", attachment.file_name()?.unwrap())))
        })),
        ..Default::default()
    };
    let result = source(
        br"{\rtf1 \u233?\objattph\objattph\'20 end}",
        &[],
        &[
            Attachment {
                bytes: None,
                fields: vec![integer(Tag::RENDERING_POSITION, 0)],
            },
            image(b"data", 1),
        ],
    )
    .to_mime_message_with_options(&options)
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), [false, true]);
    assert_eq!(result.losses[0].kind, Loss::AttachmentWithoutContent);
    assert!(
        text(&result, "plain").contains("\u{e9}[0.bin][1.bin] end"),
        "{}",
        text(&result, "plain")
    );
    assert!(text(&result, "html").contains("[0.bin][1.bin] end"));
    assert_eq!(attachments(&result).len(), 1);
}

#[test]
fn callback_errors_propagate_and_clones_share_the_callback_during_recursive_conversion() {
    let options = TnefConversionOptions {
        attachment_placeholder_callback: Some(Arc::new(|_, _| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "callback").into())
        })),
        ..Default::default()
    };
    assert!(
        matches!(source(TWO, &[], &[image(b"data", 0)]).to_mime_message_with_options(&options),
        Err(TnefConversionError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let options = TnefConversionOptions {
        convert_embedded_messages: true,
        attachment_placeholder_callback: Some(Arc::new(move |attachment: &TnefAttachment, _| {
            seen.lock()
                .unwrap()
                .push(attachment.file_name()?.unwrap().into_owned());
            Ok(Some("nested label".into()))
        })),
        ..Default::default()
    };
    let cloned = options.clone();
    assert!(Arc::ptr_eq(
        options.attachment_placeholder_callback.as_ref().unwrap(),
        cloned.attachment_placeholder_callback.as_ref().unwrap()
    ));
    let mut nested = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[object(
            Tag::RTF_COMPRESSED.id(),
            Type::BINARY,
            &compressed(br"{\rtf1 A\objattph\'20 B}"),
        )]),
    );
    attachment(&mut nested, "nested.bin", b"data");
    attr(
        &mut nested,
        Attribute::ATTACHMENT,
        &properties(&[integer(Tag::RENDERING_POSITION, 0)]),
    );
    let result = load(&wrap(&nested))
        .to_mime_message_with_options(&cloned)
        .unwrap();
    assert!(result.losses.is_empty());
    assert_eq!(*calls.lock().unwrap(), ["nested.bin"]);
    let MimeEntity::Multipart(mixed) = result.message.body().unwrap() else {
        panic!("mixed")
    };
    let MimeEntity::Message(child) = &mixed.children()[0] else {
        panic!("message")
    };
    let plain = child
        .message()
        .unwrap()
        .leaf_parts()
        .find(|part| {
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("text", "plain")
        })
        .unwrap();
    assert!(
        std::str::from_utf8(plain.content().get_ref().get_ref())
            .unwrap()
            .contains("Anested label B")
    );
}

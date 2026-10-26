// Adapted from MimeKit's UnitTests/BodyBuilderTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{Cursor, Read},
};

use mimecraft::{
    BodyBuilder, BodyBuilderError, ContentEncoding, Header, HeaderList, MessagePart, MimeContent,
    MimeEntity, MimeMessage, MimePart, Multipart, MultipartAlternative, MultipartRelated,
    TextFormat, TextPart, TextPartError, charsets::Charset,
};

fn leaf(name: &str) -> MimeEntity {
    let fields = HeaderList::load(
        &mut &format!(
            "Content-Type: application/octet-stream; name=\"{name}\"\r\n\
Content-Disposition: attachment; filename=\"{name}\"\r\n"
        )
        .as_bytes()[..],
    )
    .unwrap();
    MimePart::from_parts(
        fields,
        MimeContent::new(Cursor::new(name.as_bytes().to_vec())),
    )
    .into()
}

fn multipart(entity: &MimeEntity) -> &Multipart {
    let MimeEntity::Multipart(part) = entity else {
        panic!("expected multipart")
    };
    part
}

fn shape(entity: &MimeEntity) -> String {
    let media_type = entity
        .headers()
        .get("Content-Type")
        .unwrap()
        .parse_content_type()
        .unwrap()
        .mime_type();
    if let MimeEntity::Multipart(part) = entity {
        format!(
            "{media_type}({})",
            part.children()
                .iter()
                .map(shape)
                .collect::<Vec<_>>()
                .join(",")
        )
    } else {
        media_type
    }
}

fn assert_text(entity: &mut MimeEntity, subtype: &str, charset: Charset, expected: &str) {
    let MimeEntity::Part(part) = entity else {
        panic!("expected text part")
    };
    let content_type = part.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("text", subtype));
    assert_eq!(content_type.charset_encoding().unwrap(), Some(charset));
    assert!(part.headers().get("Content-Transfer-Encoding").is_none());
    let mut bytes = Vec::new();
    part.content_mut().decode_to(&mut bytes).unwrap();
    assert_eq!(bytes, charset.encode(expected).unwrap().as_ref());
}

#[test]
fn default_body_is_empty_utf8_plain_text() {
    let builder = BodyBuilder::new();
    assert_eq!(builder.body_encoding, Charset::UTF8);
    assert!(builder.text_body.is_none());
    assert!(builder.html_body.is_none());
    assert!(builder.attachments.is_empty());
    assert!(builder.linked_resources.is_empty());
    let mut body = builder.into_message_body().unwrap();
    assert_text(&mut body, "plain", Charset::UTF8, "");
    assert_eq!(
        body.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("")
    );
}

#[test]
fn configured_body_encoding_applies_to_plain_html_and_both_alternatives() {
    for (plain, html) in [(true, false), (false, true), (true, true)] {
        let builder = BodyBuilder {
            text_body: plain.then(|| "This is the text body. Caf\u{e9}\n".into()),
            html_body: html.then(|| "<p>This is the html body. Caf\u{e9}</p>\r\n".into()),
            body_encoding: Charset::LATIN1,
            ..BodyBuilder::new()
        };
        let mut body = builder.into_message_body().unwrap();
        if plain && html {
            let MimeEntity::Multipart(part) = &mut body else {
                panic!()
            };
            assert_eq!(
                part.content_type().unwrap().unwrap().mime_type(),
                "multipart/alternative"
            );
            assert_eq!(part.children().len(), 2);
            assert_text(
                &mut part.children_mut()[0],
                "plain",
                Charset::LATIN1,
                "This is the text body. Caf\u{e9}\n",
            );
            assert_text(
                &mut part.children_mut()[1],
                "html",
                Charset::LATIN1,
                "<p>This is the html body. Caf\u{e9}</p>\r\n",
            );
        } else if plain {
            assert_text(
                &mut body,
                "plain",
                Charset::LATIN1,
                "This is the text body. Caf\u{e9}\n",
            );
        } else {
            assert_text(
                &mut body,
                "html",
                Charset::LATIN1,
                "<p>This is the html body. Caf\u{e9}</p>\r\n",
            );
        }
        assert_eq!(
            body.text_body(TextFormat::Plain).unwrap().as_deref(),
            plain.then_some("This is the text body. Caf\u{e9}\n")
        );
        assert_eq!(
            body.text_body(TextFormat::Html).unwrap().as_deref(),
            html.then_some("<p>This is the html body. Caf\u{e9}</p>\r\n")
        );
    }
}

#[test]
fn a_single_attachment_is_moved_without_wrapping_copying_or_rewinding() {
    let mut attachment = TextPart::new("plain", "attachment").unwrap().into_part();
    attachment.set_file_name(Some("attachment.txt")).unwrap();
    attachment.content_mut().get_mut().set_position(4);
    let bytes = attachment.content().get_ref().get_ref().as_ptr();
    let headers = attachment.headers().clone();
    let body = BodyBuilder {
        attachments: vec![attachment.into()],
        ..BodyBuilder::new()
    }
    .into_message_body()
    .unwrap();
    let MimeEntity::Part(part) = body else {
        panic!()
    };
    assert_eq!(part.headers(), &headers);
    assert_eq!(part.content().get_ref().get_ref().as_ptr(), bytes);
    assert_eq!(part.content().get_ref().position(), 4);
}

#[test]
fn all_body_resource_and_attachment_combinations_have_the_upstream_layout() {
    for plain in [None, Some(""), Some("plain")] {
        for html in [None, Some(""), Some("<p>html</p>")] {
            for resource_count in [0, 2] {
                for attachment_count in 0..=2 {
                    let builder = BodyBuilder {
                        text_body: plain.map(str::to_owned),
                        html_body: html.map(str::to_owned),
                        linked_resources: (0..resource_count)
                            .map(|i| leaf(&format!("resource{i}")))
                            .collect(),
                        attachments: (0..attachment_count)
                            .map(|i| leaf(&format!("attachment{i}")))
                            .collect(),
                        ..BodyBuilder::new()
                    };
                    let mut body = builder.into_message_body().unwrap();
                    let html_shape = if resource_count > 0 {
                        "multipart/related(text/html,application/octet-stream,application/octet-stream)"
                    } else {
                        "text/html"
                    };
                    let body_shape = match (plain, html) {
                        (Some(_), Some(_)) => {
                            Some(format!("multipart/alternative(text/plain,{html_shape})"))
                        }
                        (Some(_), None) => Some("text/plain".into()),
                        (None, Some(_)) => Some(html_shape.into()),
                        (None, None) => None,
                    };
                    let expected = match (body_shape, attachment_count) {
                        (None, 0) => "text/plain".into(),
                        (None, 1) => "application/octet-stream".into(),
                        (Some(body), 0) => body,
                        (body, count) => {
                            let mut parts: Vec<_> = body.into_iter().collect();
                            parts.extend((0..count).map(|_| "application/octet-stream".into()));
                            format!("multipart/mixed({})", parts.join(","))
                        }
                    };
                    assert_eq!(
                        shape(&body),
                        expected,
                        "{plain:?}, {html:?}, {resource_count}, {attachment_count}"
                    );
                    let expected_plain =
                        if plain.is_none() && html.is_none() && attachment_count == 0 {
                            Some("")
                        } else {
                            plain
                        };
                    assert_eq!(
                        body.text_body(TextFormat::Plain).unwrap().as_deref(),
                        expected_plain
                    );
                    assert_eq!(body.text_body(TextFormat::Html).unwrap().as_deref(), html);
                    if html.is_some() && resource_count > 0 {
                        let content = if attachment_count > 0 {
                            &multipart(&body).children()[0]
                        } else {
                            &body
                        };
                        let related = if plain.is_some() {
                            multipart(&multipart(content).children()[1])
                        } else {
                            multipart(content)
                        };
                        for (i, resource) in related.children()[1..].iter().enumerate() {
                            let MimeEntity::Part(part) = resource else {
                                panic!()
                            };
                            assert_eq!(part.file_name().unwrap(), Some(format!("resource{i}")));
                        }
                    }
                    if attachment_count > 0 {
                        let attachments =
                            if plain.is_none() && html.is_none() && attachment_count == 1 {
                                std::slice::from_ref(&body)
                            } else {
                                let children = multipart(&body).children();
                                &children[children.len() - attachment_count..]
                            };
                        for (i, part) in attachments.iter().enumerate() {
                            let MimeEntity::Part(part) = part else {
                                panic!()
                            };
                            assert_eq!(part.file_name().unwrap(), Some(format!("attachment{i}")));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn linked_html_has_root_metadata_and_preserves_resource_headers_and_content() {
    let mut resource = leaf("image.png");
    resource.set_content_id(Some("image@example.com")).unwrap();
    resource
        .headers_mut()
        .add(Header::parse(b"Content-Location: image.png").unwrap());
    let MimeEntity::Part(part) = &mut resource else {
        panic!()
    };
    part.set_content_transfer_encoding(ContentEncoding::Base64);
    part.content_mut().get_mut().set_position(2);
    let original_headers = part.headers().clone();
    let mut body = BodyBuilder {
        text_body: Some("plain".into()),
        html_body: Some("<img src=\"cid:image@example.com\">".into()),
        linked_resources: vec![resource],
        body_encoding: Charset::LATIN1,
        ..BodyBuilder::new()
    }
    .into_message_body()
    .unwrap();
    let MimeEntity::Multipart(ref mut alternative) = body else {
        panic!()
    };
    let related = multipart(&alternative.children()[1]);
    let content_type = related.content_type().unwrap().unwrap();
    assert_eq!(content_type.parameters().get("type"), Some("text/html"));
    assert_eq!(content_type.parameters().get("start"), None);
    assert!(related.children()[0].content_id().unwrap().is_none());
    assert_eq!(related.children()[1].headers(), &original_headers);
    let MimeEntity::Part(resource) = &related.children()[1] else {
        panic!()
    };
    assert_eq!(resource.content().get_ref().position(), 2);
    assert_eq!(resource.content().get_ref().get_ref(), b"image.png");
    let MimeEntity::Multipart(related) = alternative.children_mut().remove(1) else {
        panic!()
    };
    let mut related = MultipartRelated::from_multipart(related).unwrap();
    assert_eq!(related.root_index().unwrap(), Some(0));
    assert_eq!(
        related.index_of_uri("cid:image@example.com").unwrap(),
        Some(1)
    );
    assert_eq!(related.index_of_uri("image.png").unwrap(), Some(1));
    let mut decoded = Vec::new();
    related
        .open_by_uri("image.png")
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, b"image.png");
}

#[test]
fn linked_resources_without_html_are_ignored_and_empty_fallback_uses_the_charset() {
    let mut resource = leaf("ignored");
    resource
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    let mut body = BodyBuilder {
        linked_resources: vec![resource],
        body_encoding: Charset::LATIN1,
        ..BodyBuilder::new()
    }
    .into_message_body()
    .unwrap();
    assert_text(&mut body, "plain", Charset::LATIN1, "");
}

#[test]
fn unmappable_text_returns_errors_without_replacement_or_fallback() {
    for (text_body, html_body) in [
        (Some("caf\u{e9}"), None),
        (None, Some("<p>caf\u{e9}</p>")),
        (Some("plain"), Some("<p>caf\u{e9}</p>")),
    ] {
        let error = BodyBuilder {
            text_body: text_body.map(str::to_owned),
            html_body: html_body.map(str::to_owned),
            body_encoding: Charset::ASCII,
            attachments: vec![leaf("attachment")],
            ..BodyBuilder::new()
        }
        .into_message_body()
        .unwrap_err();
        assert!(matches!(
            error,
            BodyBuilderError::Text(TextPartError::Charset(_))
        ));
        assert!(error.source().unwrap().is::<TextPartError>());
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn assembled_bodies_serialize_and_round_trip_without_flattening_attachments() {
    let attached_message = MessagePart::new(MimeMessage::with_body(
        TextPart::new("plain", "attached message").unwrap().into(),
    ));
    let body = BodyBuilder {
        text_body: Some("plain".into()),
        html_body: Some("<p>html</p>".into()),
        linked_resources: vec![leaf("resource")],
        attachments: vec![leaf("attachment"), attached_message.into()],
        ..BodyBuilder::new()
    }
    .into_message_body()
    .unwrap();
    let original_shape = shape(&body);
    let mut message = MimeMessage::with_body(body);
    let mut bytes = Vec::new();
    message.write_to(&mut bytes).unwrap();
    let mut parsed = MimeMessage::load(&mut Cursor::new(&bytes)).unwrap();
    assert_eq!(shape(parsed.body().unwrap()), original_shape);
    assert_eq!(
        parsed.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("plain")
    );
    assert_eq!(
        parsed.text_body(TextFormat::Html).unwrap().as_deref(),
        Some("<p>html</p>")
    );
    let mut output = Vec::new();
    parsed.write_to(&mut output).unwrap();
    assert_eq!(output, bytes);
}

#[test]
fn container_errors_expose_the_underlying_cause() {
    let errors = [
        BodyBuilderError::Multipart(Multipart::<()>::with_boundary("mixed", "").unwrap_err()),
        BodyBuilderError::Alternative(MultipartAlternative::<()>::with_boundary("").unwrap_err()),
        BodyBuilderError::Related(MultipartRelated::<()>::with_boundary("").unwrap_err()),
    ];
    for error in errors {
        assert_eq!(error.to_string(), error.source().unwrap().to_string());
    }
}

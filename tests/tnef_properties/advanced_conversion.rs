// Adapted from MimeKit's TnefMessageConverterTests and TnefInlineAttachmentTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::super::super::bodies::compressed;
use super::*;
use mimecraft::{
    MessagePart, MimePart,
    tnef::{TnefAttachment, TnefConversionOptions, TnefConversionResult},
};
use std::{
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrendered";
const RTF: &[u8] = br"{\rtf1 Before\objattph\'20 after}";

fn wire(fields: &[Vec<u8>], attachments: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let mut bytes = document(Attribute::MAPI_PROPERTIES, &properties(fields));
    for fields in attachments {
        attr(&mut bytes, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
        attr(&mut bytes, Attribute::ATTACHMENT, &properties(fields));
    }
    bytes
}

fn binary(tag: Tag, bytes: &[u8]) -> Vec<u8> {
    object(tag.id(), Type::BINARY, bytes)
}

fn attached(id: &str, bytes: &[u8]) -> Vec<Vec<u8>> {
    vec![
        string(Tag::ATTACH_CONTENT_ID_W, id),
        string(Tag::ATTACH_LONG_FILENAME_W, "source.bin"),
        binary(Tag::ATTACH_DATA_BIN, bytes),
    ]
}

fn embedded(id: &str, bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut fields = attached(id, &[IID.as_slice(), bytes].concat());
    fields.push(integer(Tag::ATTACH_METHOD, 5));
    fields
}

fn run(
    fields: &[Vec<u8>],
    attachments: &[Vec<Vec<u8>>],
    options: &TnefConversionOptions,
) -> TnefConversionResult {
    load(&wire(fields, attachments))
        .to_mime_message_with_options(options)
        .unwrap()
}

fn recursive() -> TnefConversionOptions {
    TnefConversionOptions {
        convert_embedded_messages: true,
        ..Default::default()
    }
}

fn children(message: &MimeMessage) -> &[MimeEntity] {
    let MimeEntity::Multipart(part) = message.body().unwrap() else {
        panic!("multipart")
    };
    part.children()
}

fn child(entity: &MimeEntity) -> &MessagePart<Cursor<Vec<u8>>> {
    let MimeEntity::Message(part) = entity else {
        panic!("message/rfc822")
    };
    part
}

fn leaf(entity: &MimeEntity) -> &MimePart<Cursor<Vec<u8>>> {
    let MimeEntity::Part(part) = entity else {
        panic!("leaf")
    };
    part
}

#[test]
fn upstream_embedded_passthrough_and_opt_in_preserve_wrapper_metadata_and_round_trip() {
    let nested = wire(
        &[
            string(Tag::SUBJECT_W, "Embedded"),
            string(Tag::BODY_W, "Inside"),
        ],
        &[],
    );
    let mut attachment = embedded("child@example.com", &nested);
    attachment.extend([
        string(Tag::ATTACH_CONTENT_LOCATION_W, "child.eml"),
        string(Tag::ATTACH_CONTENT_BASE_W, "https://example.com/"),
        string(
            Tag::ATTACH_DISPOSITION_W,
            "inline; creation-date=\"Fri, 15 Mar 2024 10:30:00 +0000\"",
        ),
        integer(Tag::ATTACH_FLAGS, 4),
    ]);
    let fields = [binary(
        Tag::BODY_HTML_B,
        b"<img src='cid:child@example.com'>",
    )];
    let default = run(&fields, &[attachment.clone()], &Default::default());
    assert!(default.losses.is_empty());
    let part = leaf(&children(&default.message)[1]);
    assert_eq!(
        part.content_type().unwrap().unwrap().mime_type(),
        "application/ms-tnef"
    );
    assert_eq!(part.content().get_ref().get_ref(), &nested);
    let mut result = run(&fields, &[attachment], &recursive());
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let part = child(&children(&result.message)[1]);
    assert_eq!(part.message().unwrap().subject().unwrap().text, "Embedded");
    assert_eq!(raw_body(part.message().unwrap().body().unwrap()), b"Inside");
    assert_eq!(
        part.headers()
            .get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap()
            .file_name(),
        Some("source.bin")
    );
    assert_eq!(
        part.content_id().unwrap().unwrap().as_str(),
        "child@example.com"
    );
    assert!(part.headers().contains("Content-Location"));
    assert!(part.headers().contains("Content-Base"));
    assert!(!part.headers().contains("Content-Transfer-Encoding"));
    assert!(
        !part
            .headers()
            .get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap()
            .is_attachment()
    );
    let mut output = Vec::new();
    result.message.write_to(&mut output).unwrap();
    let parsed = MimeParser::new(output.as_slice()).parse_message().unwrap();
    assert_eq!(
        child(&children(&parsed)[1])
            .message()
            .unwrap()
            .subject()
            .unwrap()
            .text,
        "Embedded"
    );
}

#[test]
fn recursive_embedded_conversion_relaxes_only_exchange_headers_and_retains_nested_loss_context() {
    let mut invalid_rtf = [0; 16];
    invalid_rtf[..4].copy_from_slice(&12u32.to_le_bytes());
    let nested = wire(
        &[
            string(Tag::SUBJECT_W, "deep"),
            binary(Tag::RTF_COMPRESSED, &invalid_rtf),
            named(1, "X-MS-Exchange-Organization-SCL", &["1"]),
            named(2, "X-Microsoft-Exchange-Forest-Thing", &["2"]),
            named(3, "Content-Type", &["bad"]),
            named(4, "MIME-Version", &["bad"]),
        ],
        &[],
    );
    let middle = wire(&[], &[embedded("deep@example.com", &nested)]);
    let result = run(
        &[],
        &[embedded("middle@example.com", &middle)],
        &recursive(),
    );
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidRtfBody);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    assert!(
        result.losses[0]
            .description
            .contains("Embedded attachment 0:")
    );
    let middle = child(&children(&result.message)[0]).message().unwrap();
    let deep = child(&children(middle)[0]).message().unwrap();
    assert_eq!(deep.subject().unwrap().text, "deep");
    assert!(deep.headers().contains("X-MS-Exchange-Organization-SCL"));
    assert!(deep.headers().contains("X-Microsoft-Exchange-Forest-Thing"));
    assert!(!deep.headers().contains("Content-Type"));
    assert!(!deep.headers().contains("MIME-Version"));
}

#[test]
fn upstream_corrupt_embedded_header_falls_back_but_checked_nesting_limits_propagate() {
    let result = run(
        &[],
        &[embedded("bad@example.com", b"not tnef")],
        &recursive(),
    );
    assert_eq!(
        result
            .losses
            .iter()
            .map(|loss| loss.kind)
            .collect::<Vec<_>>(),
        [Loss::InvalidEmbeddedMessage]
    );
    assert_eq!(raw_body(&children(&result.message)[0]), b"not tnef");
    let leaf = wire(&[string(Tag::BODY_W, "leaf")], &[]);
    let middle = wire(&[], &[embedded("leaf@example.com", &leaf)]);
    let bytes = wire(&[], &[embedded("middle@example.com", &middle)]);
    let options = TnefReaderOptions {
        max_nesting_depth: 1,
        ..Default::default()
    };
    let source = TnefReader::with_options(bytes.as_slice(), &options)
        .unwrap()
        .load_message()
        .unwrap();
    assert!(matches!(
        source.to_mime_message_with_options(&recursive()),
        Err(TnefConversionError::Embedded(Error::NestingLimitExceeded))
    ));
}

#[test]
fn embedded_recovery_issues_are_not_silently_discarded() {
    let mut bytes = wire(&[string(Tag::BODY_W, "body")], &[]);
    let end = bytes.len() - 1;
    bytes[end] ^= 1;
    let result = run(&[], &[embedded("child@example.com", &bytes)], &recursive());
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::EmbeddedMessageIssue);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    assert_eq!(
        raw_body(
            child(&children(&result.message)[0])
                .message()
                .unwrap()
                .body()
                .unwrap()
        ),
        b"body"
    );
}

fn ole(fields: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut all = fields.to_vec();
    all.extend(attached("ole@example.com", b"original OLE"));
    all.extend([
        integer(Tag::ATTACH_METHOD, 6),
        integer(Tag::RENDERING_POSITION, 0),
    ]);
    all
}

fn renderer(bytes: Option<&[u8]>) -> TnefConversionOptions {
    let bytes = bytes.map(<[u8]>::to_vec);
    TnefConversionOptions {
        ole_object_converter: Some(Arc::new(move |_: &TnefAttachment| {
            Ok(bytes
                .clone()
                .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read>))
        })),
        ..Default::default()
    }
}

#[test]
fn upstream_ole_rendered_images_use_display_name_metadata_and_rtf_placement() {
    let result = run(
        &[binary(Tag::RTF_COMPRESSED, &compressed(RTF))],
        &[ole(&[
            string(Tag::DISPLAY_NAME_W, "Chart & data"),
            string(
                Tag::ATTACH_DISPOSITION_W,
                "attachment; size=999; modification-date=\"Fri, 15 Mar 2024 10:30:00 +0000\"",
            ),
            integer(Tag::ATTACH_SIZE, 999),
            string(Tag::ATTACH_CONTENT_LOCATION_W, "image.png"),
        ])],
        &renderer(Some(PNG)),
    );
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let related = children(&result.message);
    assert_eq!(related.len(), 2);
    let part = leaf(&related[1]);
    assert_eq!(part.content().get_ref().get_ref(), PNG);
    assert_eq!(
        part.file_name().unwrap().as_deref(),
        Some("Chart & data.png")
    );
    assert_eq!(
        part.content_type().unwrap().unwrap().name(),
        Some("Chart & data.png")
    );
    assert_eq!(part.content_description().unwrap().text, "Chart & data.png");
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Base64
    );
    let disposition = part.content_disposition().unwrap().unwrap();
    assert!(!disposition.is_attachment());
    assert_eq!(disposition.size().unwrap(), None);
    assert!(disposition.parameters().get("modification-date").is_some());
    assert!(part.headers().contains("Content-Location"));
    let html = result
        .message
        .leaf_parts()
        .find(|part| {
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("text", "html")
        })
        .unwrap();
    let html = std::str::from_utf8(html.content().get_ref().get_ref()).unwrap();
    assert!(
        html.contains(
            "Before<img src=\"cid:ole@example.com\" alt=\"Chart &amp; data.png\"/> after"
        ),
        "{html}"
    );
}

#[test]
fn upstream_ole_naming_formats_and_fallback_preserve_original_bytes() {
    let mut bmp = vec![0; 18];
    bmp[..2].copy_from_slice(b"BM");
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    for (bytes, subtype, name) in [
        (PNG, "png", "image.png"),
        (b"\xff\xd8\xff", "jpeg", "image.jpg"),
        (b"GIF87a", "gif", "image.gif"),
        (b"GIF89a", "gif", "image.gif"),
        (b"RIFF1234WEBP", "webp", "image.webp"),
        (bmp.as_slice(), "bmp", "image.bmp"),
    ] {
        let result = run(
            &[string(Tag::BODY_W, "plain")],
            &[ole(&[string(Tag::DISPLAY_NAME_W, "image")])],
            &renderer(Some(bytes)),
        );
        assert!(result.losses.is_empty());
        let part = leaf(&children(&result.message)[1]);
        assert_eq!(
            part.content_type().unwrap().unwrap().media_subtype(),
            subtype
        );
        assert_eq!(part.file_name().unwrap().as_deref(), Some(name));
        assert!(part.content_disposition().unwrap().unwrap().is_attachment());
    }
    for (display, filename, expected) in [
        (Some("Photo.JPG"), "file", "Photo.JPG"),
        (None, "photo.JPG", "photo.JPG"),
        (Some(" \t"), "photo", "photo.jpg"),
        (Some(""), "", "image.jpg"),
    ] {
        let mut fields = vec![string(Tag::ATTACH_LONG_FILENAME_W, filename)];
        if let Some(display) = display {
            fields.push(string(Tag::DISPLAY_NAME_W, display));
        }
        let result = run(&[], &[ole(&fields)], &renderer(Some(b"\xff\xd8\xff")));
        assert!(result.losses.is_empty());
        assert_eq!(
            leaf(&children(&result.message)[0])
                .file_name()
                .unwrap()
                .as_deref(),
            Some(expected)
        );
    }
    for bytes in [
        None,
        Some(b"".as_slice()),
        Some(b"BMgarbage"),
        Some(b"\x89PNG"),
        Some(b"\xff\xd8"),
        Some(b"RIFF1234WAVE"),
        Some(b"GIF89z"),
    ] {
        let result = run(&[], &[ole(&[])], &renderer(bytes));
        assert_eq!(
            result
                .losses
                .iter()
                .map(|loss| loss.kind)
                .collect::<Vec<_>>(),
            [Loss::OleObjectNotRendered]
        );
        assert_eq!(raw_body(&children(&result.message)[0]), b"original OLE");
    }
}

struct RenderStream {
    bytes: Cursor<Vec<u8>>,
    drops: Arc<AtomicUsize>,
    fail: bool,
}
impl Read for RenderStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.fail {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "render stream"))
        } else {
            self.bytes.read(output)
        }
    }
}
impl Drop for RenderStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn upstream_ole_shared_renderer_reads_current_position_without_seek_and_drops_on_errors() {
    for fail in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (render_calls, render_drops) = (calls.clone(), drops.clone());
        let options = TnefConversionOptions {
            ole_object_converter: Some(Arc::new(move |_: &TnefAttachment| {
                render_calls.fetch_add(1, Ordering::SeqCst);
                let mut bytes = Cursor::new([b"skip".as_slice(), PNG].concat());
                bytes.set_position(4);
                Ok(Some(Box::new(RenderStream {
                    bytes,
                    drops: render_drops.clone(),
                    fail,
                }) as Box<dyn Read>))
            })),
            ..recursive()
        };
        let cloned = options.clone();
        assert!(Arc::ptr_eq(
            options.ole_object_converter.as_ref().unwrap(),
            cloned.ole_object_converter.as_ref().unwrap()
        ));
        let nested = wire(&[], &[ole(&[])]);
        let source = load(&wire(
            &[],
            &[
                embedded("child@example.com", &nested),
                attached("plain@example.com", b"raw"),
            ],
        ));
        let result = source.to_mime_message_with_options(&cloned);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        if fail {
            assert!(
                matches!(result, Err(TnefConversionError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
            );
        } else {
            let result = result.unwrap();
            let nested = child(&children(&result.message)[0]).message().unwrap();
            assert_eq!(raw_body(&children(nested)[0]), PNG);
        }
    }
    let options = TnefConversionOptions {
        ole_object_converter: Some(Arc::new(|_: &TnefAttachment| {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "renderer"))
        })),
        ..Default::default()
    };
    let source = load(&wire(&[], &[ole(&[])]));
    assert!(matches!(source.to_mime_message_with_options(&options),
        Err(TnefConversionError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied));
}

#[test]
fn ole_html_references_work_and_missing_content_does_not_invoke_renderer() {
    let result = run(
        &[binary(Tag::BODY_HTML_B, b"<img src='cid:ole@example.com'>")],
        &[ole(&[integer(Tag::ATTACH_FLAGS, 4)])],
        &renderer(Some(PNG)),
    );
    assert!(result.losses.is_empty());
    assert_eq!(
        leaf(&children(&result.message)[1])
            .content_type()
            .unwrap()
            .unwrap()
            .mime_type(),
        "image/png"
    );
    assert!(
        !leaf(&children(&result.message)[1])
            .content_disposition()
            .unwrap()
            .unwrap()
            .is_attachment()
    );
    let options = TnefConversionOptions {
        ole_object_converter: Some(Arc::new(
            |_: &TnefAttachment| -> io::Result<Option<Box<dyn Read>>> { panic!("no content") },
        )),
        ..Default::default()
    };
    let result = run(&[], &[vec![integer(Tag::ATTACH_METHOD, 6)]], &options);
    assert_eq!(result.losses[0].kind, Loss::AttachmentWithoutContent);
}

fn skeleton(parts: &[&str]) -> String {
    format!(
        "Subject: Original\r\nTo: original@example.com\r\nImportance: low\r\nX-Original: yes\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n{}\r\n--b--\r\n",
        parts.join("\r\n--b\r\n")
    )
}

const PLAIN: &str = "Content-Type: text/plain; charset=us-ascii; format=flowed\r\nContent-Transfer-Encoding: quoted-printable\r\nX-Exchange-MIME-Skeleton-Content-Id: <body@example.com>\r\n\r\n";
const IMAGE: &str = "Content-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nX-Exchange-MIME-Skeleton-Content-Id: <image@example.com>\r\n\r\n";
const EMBEDDED: &str = "Content-Type: message/rfc822\r\nX-Exchange-MIME-Skeleton-Content-Id: <image@example.com>\r\nContent-Disposition: attachment; filename=original.eml\r\n\r\n";

fn skeleton_fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    vec![
        binary(Tag::MIME_SKELETON, bytes),
        string(Tag::SUBJECT_W, "Property"),
        string(Tag::BODY_W, "Hello = body"),
        string(Tag::BODY_CONTENT_ID_W, "body@example.com"),
        string(
            Tag::TRANSPORT_MESSAGE_HEADERS_W,
            "Received: from transport.example.com\r\n",
        ),
        named(1, "X-Property", &["yes"]),
        integer(Tag::IMPORTANCE, 2),
    ]
}

fn fallback(result: &TnefConversionResult) {
    assert!(
        result
            .losses
            .iter()
            .any(|loss| loss.kind == Loss::InvalidMimeSkeleton),
        "{:?}",
        result.losses
    );
    assert_eq!(result.message.subject().unwrap().text, "Property");
    assert!(result.message.headers().contains("X-Property"));
    assert_eq!(
        result.message.headers().iter().next().unwrap().field(),
        Some("Received")
    );
}

#[test]
fn upstream_skeleton_restores_headers_bodies_and_raw_attachments_preserving_transfer_encoding() {
    let bytes = skeleton(&[PLAIN, IMAGE]);
    let mut result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[attached(" \t< image@example.com >\r\n", PNG)],
        &Default::default(),
    );
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    assert_eq!(result.message.subject().unwrap().text, "Original");
    assert!(!result.message.headers().contains("X-Property"));
    assert!(!result.message.headers().contains("Received"));
    assert!(result.message.headers().contains("X-Original"));
    assert_eq!(
        result.message.importance().unwrap(),
        mimecraft::MessageImportance::High
    );
    let parts = children(&result.message);
    let body = leaf(&parts[0]);
    assert_eq!(
        body.content_type().unwrap().unwrap().charset(),
        Some("utf-8")
    );
    assert_eq!(
        body.content_type()
            .unwrap()
            .unwrap()
            .parameters()
            .get("format"),
        Some("flowed")
    );
    assert_eq!(
        body.content_transfer_encoding().unwrap(),
        ContentEncoding::QuotedPrintable
    );
    assert_eq!(raw_body(&parts[0]), b"Hello = body");
    assert_eq!(raw_body(&parts[1]), PNG);
    for part in parts {
        assert!(
            !part
                .headers()
                .contains("X-Exchange-MIME-Skeleton-Content-Id")
        );
        assert!(part.content_id().unwrap().is_none());
    }
    let mut output = Vec::new();
    result.message.write_to(&mut output).unwrap();
    let mut parsed = MimeParser::new(output.as_slice()).parse_message().unwrap();
    let MimeEntity::Multipart(parts) = parsed.body_mut().unwrap() else {
        panic!("mixed")
    };
    for (entity, expected) in parts
        .children_mut()
        .iter_mut()
        .zip([b"Hello = body".as_slice(), PNG])
    {
        let MimeEntity::Part(part) = entity else {
            panic!("leaf")
        };
        let mut bytes = Vec::new();
        part.content_mut().decode_to(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
    }
}

#[test]
fn skeleton_decodes_folded_encoded_body_and_attachment_identifiers() {
    let plain = PLAIN.replace(
        "<body@example.com>",
        "=?us-ascii?q?=3Cbody=40?=\r\n\t=?us-ascii?q?example.com=3E?=",
    );
    let image = IMAGE.replace(
        "<image@example.com>",
        "=?us-ascii?q?=3Cimage=40example.com=3E?=",
    );
    let bytes = skeleton(&[&plain, &image]);
    let mut result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[attached("image@example.com", PNG)],
        &Default::default(),
    );
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    assert_eq!(result.message.subject().unwrap().text, "Original");
    assert!(result.message.headers().contains("X-Original"));
    assert!(!result.message.headers().contains("X-Property"));
    let parts = children(&result.message);
    assert_eq!(raw_body(&parts[0]), b"Hello = body");
    assert_eq!(raw_body(&parts[1]), PNG);
    for part in parts {
        assert!(
            !part
                .headers()
                .contains("X-Exchange-MIME-Skeleton-Content-Id")
        );
        assert!(part.content_id().unwrap().is_none());
    }
    let mut wire = Vec::new();
    result.message.write_to(&mut wire).unwrap();
    let parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    assert_eq!(parsed.subject().unwrap().text, "Original");
    assert_eq!(children(&parsed).len(), 2);
}

#[test]
fn skeleton_identifier_charset_recovery_is_reported_as_fallback() {
    for id in [
        "=?unknown-charset?q?=3Cimage=40example.com=3E?=",
        "=?utf-8?q?=3Cimage=FF=40example.com=3E?=",
    ] {
        let image = IMAGE.replace("<image@example.com>", id);
        let bytes = skeleton(&[PLAIN, &image]);
        fallback(&run(
            &skeleton_fields(bytes.as_bytes()),
            &[attached("image@example.com", PNG)],
            &Default::default(),
        ));
    }
}

#[test]
fn upstream_skeleton_matching_is_case_sensitive_and_does_not_trim_unicode_whitespace() {
    let bytes = skeleton(&[
        PLAIN,
        &IMAGE.replace("X-Exchange-MIME-Skeleton-Content-Id", "Content-Id"),
    ]);
    for id in [
        "image@example.com",
        "IMAGE@example.com",
        "\u{a0}image@example.com\u{a0}",
    ] {
        let result = run(
            &skeleton_fields(bytes.as_bytes()),
            &[attached(id, PNG)],
            &Default::default(),
        );
        if id == "image@example.com" {
            assert!(result.losses.is_empty());
            assert_eq!(
                children(&result.message)[1]
                    .content_id()
                    .unwrap()
                    .unwrap()
                    .as_str(),
                id
            );
        } else {
            fallback(&result);
        }
    }
}

#[test]
fn upstream_skeleton_rejects_missing_extra_duplicate_and_contentless_attachments() {
    let bytes = skeleton(&[PLAIN, IMAGE]);
    let candidates = [
        vec![],
        vec![attached("other@example.com", PNG)],
        vec![
            attached("image@example.com", PNG),
            attached("extra@example.com", b"extra"),
        ],
        vec![
            attached("image@example.com", PNG),
            attached("image@example.com", b"duplicate"),
        ],
        vec![vec![string(Tag::ATTACH_CONTENT_ID_W, "image@example.com")]],
    ];
    for attachments in candidates {
        fallback(&run(
            &skeleton_fields(bytes.as_bytes()),
            &attachments,
            &Default::default(),
        ));
    }
    let duplicated = skeleton(&[PLAIN, IMAGE, IMAGE]);
    fallback(&run(
        &skeleton_fields(duplicated.as_bytes()),
        &[attached("image@example.com", PNG)],
        &Default::default(),
    ));
}

#[test]
fn upstream_skeleton_headers_only_unknown_bodies_malformed_and_empty_inputs() {
    let result = run(
        &skeleton_fields(b"Subject: Headers only\r\n"),
        &[],
        &Default::default(),
    );
    assert!(result.losses.is_empty());
    assert_eq!(result.message.subject().unwrap().text, "Headers only");
    assert_eq!(raw_body(result.message.body().unwrap()), b"Hello = body");
    for bytes in [
        b"Not a header\r\n\r\nBody\r\n".as_slice(),
        b"\0\0\0\0",
        b"Subject: Original\r\nContent-Type: text/html\r\n\r\n",
    ] {
        fallback(&run(&skeleton_fields(bytes), &[], &Default::default()));
    }
    fallback(&run(
        &skeleton_fields(b"Subject: Headers only\r\n"),
        &[attached("extra@example.com", PNG)],
        &Default::default(),
    ));
    let duplicate = skeleton(&[PLAIN, PLAIN]);
    fallback(&run(
        &skeleton_fields(duplicate.as_bytes()),
        &[],
        &Default::default(),
    ));
    let result = run(&skeleton_fields(b""), &[], &Default::default());
    assert!(result.losses.is_empty());
    assert_eq!(result.message.subject().unwrap().text, "Property");
}

#[test]
fn upstream_skeleton_populated_parts_claim_attachments_without_replacing_content() {
    let image = IMAGE.to_owned() + "R0lGODlh";
    let bytes = skeleton(&[
        PLAIN,
        &image,
        "Content-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR",
    ]);
    let result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[
            attached("image@example.com", PNG),
            vec![string(Tag::ATTACH_LONG_FILENAME_W, "missing.bin")],
        ],
        &Default::default(),
    );
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::AttachmentWithoutContent);
    assert_eq!(result.losses[0].attachment_index, Some(1));
    assert_eq!(result.message.subject().unwrap().text, "Original");
    assert_eq!(raw_body(&children(&result.message)[1]), b"R0lGODlh");
    assert!(raw_body(&children(&result.message)[2]).starts_with(b"BEGIN:VCALENDAR"));
}

#[test]
fn upstream_skeleton_forces_embedded_conversion_and_rolls_back_tentative_losses_on_fallback() {
    let bytes = skeleton(&[PLAIN, EMBEDDED]);
    let nested = wire(
        &[
            string(Tag::SUBJECT_W, "Nested"),
            string(Tag::BODY_W, "inside"),
        ],
        &[],
    );
    let result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[embedded("image@example.com", &nested)],
        &Default::default(),
    );
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let part = child(&children(&result.message)[1]);
    assert_eq!(
        part.headers()
            .get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap()
            .file_name(),
        Some("original.eml")
    );
    assert_eq!(part.message().unwrap().subject().unwrap().text, "Nested");
    for attachment in [
        attached("image@example.com", PNG),
        embedded("image@example.com", b"not tnef"),
    ] {
        let result = run(
            &skeleton_fields(bytes.as_bytes()),
            &[attachment],
            &Default::default(),
        );
        fallback(&result);
        assert_eq!(result.losses.len(), 1);
    }
    let mut unknown = [0; 16];
    unknown[..4].copy_from_slice(&12u32.to_le_bytes());
    let nested = wire(&[binary(Tag::RTF_COMPRESSED, &unknown)], &[]);
    let result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[
            embedded("image@example.com", &nested),
            attached("extra@example.com", b"extra"),
        ],
        &Default::default(),
    );
    fallback(&result);
    assert_eq!(
        result
            .losses
            .iter()
            .map(|loss| loss.kind)
            .collect::<Vec<_>>(),
        [Loss::InvalidMimeSkeleton]
    );
}

#[test]
fn upstream_skeleton_reuses_rtf_and_preserves_preexisting_rtf_losses_on_fallback() {
    for subtype in ["text/rtf", "application/rtf"] {
        let bytes = skeleton(&[&format!("Content-Type: {subtype}\r\n\r\n"), IMAGE]);
        let mut fields = skeleton_fields(bytes.as_bytes());
        fields.push(binary(Tag::RTF_COMPRESSED, &compressed(RTF)));
        fields.push(integer(Tag::NATIVE_BODY, 2));
        let result = run(
            &fields,
            &[attached("image@example.com", PNG)],
            &Default::default(),
        );
        assert!(result.losses.is_empty(), "{:?}", result.losses);
        assert_eq!(raw_body(&children(&result.message)[0]), RTF);
        assert_eq!(
            leaf(&children(&result.message)[0])
                .content_type()
                .unwrap()
                .unwrap()
                .charset(),
            None
        );
        let result = run(
            &fields,
            &[attached("other@example.com", PNG)],
            &Default::default(),
        );
        fallback(&result);
        assert!(result.message.leaf_parts().any(|part| {
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("text", "html")
        }));
    }
    let bytes = skeleton(&["Content-Type: text/rtf\r\n\r\n"]);
    let mut fields = skeleton_fields(bytes.as_bytes());
    let mut unknown = [0; 16];
    unknown[..4].copy_from_slice(&12u32.to_le_bytes());
    fields.push(binary(Tag::RTF_COMPRESSED, &unknown));
    let result = run(&fields, &[], &Default::default());
    assert_eq!(
        result
            .losses
            .iter()
            .map(|loss| loss.kind)
            .collect::<Vec<_>>(),
        [Loss::InvalidRtfBody, Loss::InvalidMimeSkeleton]
    );
}

#[test]
fn skeleton_uses_original_ole_payload_and_does_not_consult_renderer() {
    let options = TnefConversionOptions {
        ole_object_converter: Some(Arc::new(
            |_: &TnefAttachment| -> io::Result<Option<Box<dyn Read>>> {
                panic!("skeleton bypasses OLE rendering")
            },
        )),
        ..Default::default()
    };
    let bytes = skeleton(&[
        PLAIN,
        &IMAGE.replace("image@example.com", "ole@example.com"),
    ]);
    let result = run(&skeleton_fields(bytes.as_bytes()), &[ole(&[])], &options);
    assert!(result.losses.is_empty());
    assert_eq!(raw_body(&children(&result.message)[1]), b"original OLE");
}

#[test]
fn skeleton_body_charset_selection_preserves_bytes_and_existing_parameters() {
    for (body, expected, charset) in [
        (
            binary(Tag::BODY_HTML_B, b"<meta charset=windows-1251>\xcf"),
            b"<meta charset=windows-1251>\xcf".as_slice(),
            "windows-1251",
        ),
        (
            string(Tag::BODY_HTML_W, "HTML \u{e9}"),
            "HTML \u{e9}".as_bytes(),
            "utf-8",
        ),
    ] {
        let mut fields = skeleton_fields(b"Subject: Original\r\nContent-Type: text/html; charset=us-ascii; format=flowed\r\nContent-Transfer-Encoding: base64\r\n\r\n");
        fields.push(body);
        let result = run(&fields, &[], &Default::default());
        assert!(result.losses.is_empty(), "{:?}", result.losses);
        let part = leaf(result.message.body().unwrap());
        assert_eq!(part.content().get_ref().get_ref(), expected);
        assert_eq!(
            part.content_type().unwrap().unwrap().charset(),
            Some(charset)
        );
        assert_eq!(
            part.content_type()
                .unwrap()
                .unwrap()
                .parameters()
                .get("format"),
            Some("flowed")
        );
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::Base64
        );
    }
}

#[test]
fn skeleton_nested_multiparts_remove_all_private_leaf_ids_and_keep_unmatched_messages() {
    let image = IMAGE.replace("\r\n\r\n", "\r\nX-Exchange-MIME-Skeleton-Content-Id: <ignored@example.com>\r\nContent-Id: <real@example.com>\r\n\r\n");
    let inner = format!(
        "Content-Type: multipart/related; boundary=c\r\nX-Exchange-MIME-Skeleton-Content-Id: <multipart@example.com>\r\n\r\n--c\r\n{PLAIN}\r\n--c\r\n{image}\r\n--c--\r\n"
    );
    let existing = "Content-Type: message/rfc822\r\n\r\nSubject: Existing\r\nContent-Type: text/plain\r\n\r\nkept";
    let bytes = skeleton(&[&inner, existing]);
    let result = run(
        &skeleton_fields(bytes.as_bytes()),
        &[attached("image@example.com", PNG)],
        &Default::default(),
    );
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let parts = children(&result.message);
    assert_eq!(
        child(&parts[1]).message().unwrap().subject().unwrap().text,
        "Existing"
    );
    let MimeEntity::Multipart(related) = &parts[0] else {
        panic!("related")
    };
    assert!(
        related
            .headers()
            .contains("X-Exchange-MIME-Skeleton-Content-Id")
    );
    let image = &related.children()[1];
    assert!(
        !image
            .headers()
            .contains("X-Exchange-MIME-Skeleton-Content-Id")
    );
    assert_eq!(
        image.content_id().unwrap().unwrap().as_str(),
        "real@example.com"
    );
    assert_eq!(raw_body(image), PNG);
}

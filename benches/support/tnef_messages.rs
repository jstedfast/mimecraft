// Original whole-message workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    attachment_workloads::{integer, value},
    embedded_workloads, recipient_workloads, rtf_compressed_workloads,
};
use mimecraft::tnef::{
    TnefAttachment, TnefAttributeTag as Attribute, TnefConversionLossKind, TnefConversionOptions,
    TnefMessage, TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReader,
};
use mimecraft::{MimeEntity, MimeMessage, MimeParser};
use std::{
    hint::black_box,
    io::{self, Cursor, Read},
    sync::Arc,
};

pub const CASES: [&str; 20] = [
    "EmbeddedRecursive",
    "OleImages",
    "OleFallback",
    "MimeSkeleton",
    "MimeSkeletonFallback",
    "Legacy",
    "Mapi",
    "Mixed",
    "Embedded",
    "ShortReads",
    "Rtf",
    "RtfHtml",
    "RtfStale",
    "HtmlRelated",
    "HtmlUnreferenced",
    "RtfHtmlRelated",
    "RtfImages",
    "RtfImagesFooter",
    "RtfPlaceholders",
    "RtfPlaceholdersFooter",
];
const RTF_PARAGRAPHS: usize = 128;
pub const MODES: [Mode; 4] = [
    Mode::Load,
    Mode::Traverse,
    Mode::Convert,
    Mode::ConvertWrite,
];

#[derive(Clone, Copy)]
pub enum Mode {
    Load,
    Traverse,
    Convert,
    ConvertWrite,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Load => "Load",
            Self::Traverse => "Traverse",
            Self::Convert => "Convert",
            Self::ConvertWrite => "ConvertWrite",
        }
    }
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    payload: Vec<u8>,
    message: TnefMessage,
    rtf: Option<Vec<u8>>,
    html: String,
    options: TnefConversionOptions,
}

fn advanced(name: &str) -> bool {
    matches!(
        name,
        "EmbeddedRecursive" | "OleImages" | "OleFallback" | "MimeSkeleton" | "MimeSkeletonFallback"
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub messages: usize,
    pub attachments: usize,
    pub recipients: usize,
    pub bytes: usize,
    pub checksum: u64,
}

impl Summary {
    fn bytes(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len();
        for &byte in bytes {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }
}

struct Source<'a> {
    bytes: &'a [u8],
    chunk: usize,
}
impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.chunk);
        self.bytes.read(&mut output[..length])
    }
}

fn attribute(output: &mut Vec<u8>, tag: Attribute, bytes: &[u8]) {
    let start = output.len();
    embedded_workloads::attribute(output, tag, bytes);
    if matches!(
        tag,
        Attribute::ATTACH_RENDER_DATA | Attribute::ATTACH_TITLE | Attribute::ATTACH_DATA
    ) {
        output[start] = 2;
    }
}

fn properties(fields: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = (fields.len() as u32).to_le_bytes().to_vec();
    for field in fields {
        bytes.extend(field);
    }
    bytes
}

fn reference_html(unreferenced: bool) -> String {
    let mut html = "<p>HTML body</p>".to_owned();
    for index in (0..32).step_by(2) {
        let target = index + if unreferenced { 32 } else { 0 };
        if index % 4 == 0 {
            html.push_str(&format!("<img data-image='CID:image{target}@EXAMPLE.com'>"));
        } else {
            html.push_str(&format!(
                "<img src='http://EXAMPLE.COM/assets/image{target}.bin'>"
            ));
        }
    }
    // Unmatched attributes must not grow the candidate index.
    html.push_str(&"<span data-unrelated='http://elsewhere.example/x'>text</span>".repeat(128));
    html
}

fn rtf_source(name: &str) -> Option<Vec<u8>> {
    let name = match name {
        "RtfPlaceholders" => "RtfImages",
        "RtfPlaceholdersFooter" => "RtfImagesFooter",
        _ => name,
    };
    if !matches!(
        name,
        "Rtf" | "RtfHtml" | "RtfStale" | "RtfHtmlRelated" | "RtfImages" | "RtfImagesFooter"
    ) {
        return None;
    }
    let encapsulated = matches!(name, "RtfHtml" | "RtfHtmlRelated");
    let mut bytes = if encapsulated {
        br"{\rtf1\fromhtml1 {\*\htmltag <html><body>}".to_vec()
    } else {
        br"{\rtf1 ".to_vec()
    };
    let paragraph = if encapsulated {
        br"{\*\htmltag <p>}Hello from RTF{\*\htmltag </p>}{\htmlrtf \par }".as_slice()
    } else {
        br"Hello from RTF\par "
    };
    for index in 0..RTF_PARAGRAPHS {
        if (name == "RtfImages" && index < 32) || (name == "RtfImagesFooter" && index == 0) {
            bytes.extend_from_slice(br"Hello\objattph\'20 from RTF\par ");
        } else {
            bytes.extend_from_slice(paragraph);
        }
    }
    if name == "RtfHtmlRelated" {
        bytes.extend_from_slice(br"{\*\htmltag ");
        bytes.extend_from_slice(reference_html(false).as_bytes());
        bytes.push(b'}');
    }
    if encapsulated {
        bytes.extend_from_slice(br"{\*\htmltag </body></html>}");
    }
    bytes.push(b'}');
    Some(bytes)
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    if advanced(name) {
        return advanced_workload(name);
    }
    let legacy = name != "Mapi";
    let mapi = name != "Legacy";
    let images = matches!(
        name,
        "RtfImages" | "RtfImagesFooter" | "RtfPlaceholders" | "RtfPlaceholdersFooter"
    );
    let rtf = rtf_source(name);
    let html = if matches!(name, "HtmlRelated" | "HtmlUnreferenced") {
        reference_html(name == "HtmlUnreferenced")
    } else {
        "<p>HTML body</p>".to_owned()
    };
    let mut input =
        embedded_workloads::document(Attribute::MESSAGE_CLASS, b"IPM.Microsoft Mail.Note\0");
    if legacy {
        attribute(&mut input, Attribute::SUBJECT, b"Legacy subject\0");
        attribute(&mut input, Attribute::BODY, b"Legacy body\0");
    }
    if mapi {
        let mut fields = vec![
            recipient_workloads::property(Tag::SUBJECT_W, "MAPI subject", Type::UNICODE),
            recipient_workloads::property(Tag::BODY_W, "MAPI body", Type::UNICODE),
            value(Tag::BODY_HTML_B, html.as_bytes()),
            integer(Tag::INTERNET_CODEPAGE, 65001),
        ];
        if let Some(rtf) = &rtf {
            let mut builder = rtf_compressed_workloads::Builder::new();
            builder.literals(rtf);
            builder.end();
            fields.push(value(Tag::RTF_COMPRESSED, &builder.build()));
            fields.push(integer(Tag::RTF_IN_SYNC, i32::from(name != "RtfStale")));
        }
        attribute(&mut input, Attribute::MAPI_PROPERTIES, &properties(fields));
    }
    let row = properties(vec![
        recipient_workloads::property(Tag::DISPLAY_NAME_W, "Alice", Type::UNICODE),
        recipient_workloads::property(Tag::EMAIL_ADDRESS_W, "alice@example.com", Type::UNICODE),
        recipient_workloads::property(Tag::ADDRTYPE_W, "SMTP", Type::UNICODE),
        integer(Tag::RECIPIENT_TYPE, 1),
    ]);
    attribute(
        &mut input,
        Attribute::RECIPIENT_TABLE,
        &[&1u32.to_le_bytes()[..], &row].concat(),
    );
    let mut payload = if name == "Embedded" {
        let mut child = embedded_workloads::document(Attribute::SUBJECT, b"Inner\0");
        attribute(&mut child, Attribute::BODY, b"embedded body");
        child
    } else {
        (0..2048).map(|i| (i * 31 + 7) as u8).collect()
    };
    if images {
        payload[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    }
    for index in 0..32 {
        let render = [&1u16.to_le_bytes()[..], &(-1i32).to_le_bytes(), &[0; 8]].concat();
        attribute(&mut input, Attribute::ATTACH_RENDER_DATA, &render);
        if legacy {
            attribute(
                &mut input,
                Attribute::ATTACH_TITLE,
                format!("legacy{index}.bin\0").as_bytes(),
            );
            attribute(
                &mut input,
                Attribute::ATTACH_DATA,
                if mapi { b"replaced" } else { &payload },
            );
        }
        if mapi {
            let bytes = if name == "Embedded" {
                [embedded_workloads::IID.as_slice(), &payload].concat()
            } else {
                payload.clone()
            };
            let mut fields = vec![
                // Exercise retrospective Binary recognition when the method follows data.
                value(Tag::ATTACH_DATA_BIN, &bytes),
                integer(Tag::ATTACH_METHOD, if name == "Embedded" { 5 } else { 1 }),
                recipient_workloads::property(
                    Tag::ATTACH_LONG_FILENAME_W,
                    &format!("mapi{index}.bin"),
                    Type::UNICODE,
                ),
            ];
            if images {
                fields.push(integer(Tag::RENDERING_POSITION, 31 - index));
                // Half are displayable; the other half contradict the sniffed type.
                fields.push(recipient_workloads::property(
                    Tag::ATTACH_MIME_TAG_W,
                    if index % 2 == 0 {
                        "application/octet-stream"
                    } else {
                        "application/pdf"
                    },
                    Type::UNICODE,
                ));
            }
            if matches!(name, "HtmlRelated" | "HtmlUnreferenced" | "RtfHtmlRelated") {
                fields.push(integer(Tag::ATTACH_FLAGS, 4));
                if index % 4 == 0 {
                    fields.push(recipient_workloads::property(
                        Tag::ATTACH_CONTENT_ID_W,
                        &format!("image{index}@example.com"),
                        Type::UNICODE,
                    ));
                } else {
                    fields.push(recipient_workloads::property(
                        Tag::ATTACH_CONTENT_LOCATION_W,
                        &format!("image{index}.bin"),
                        Type::UNICODE,
                    ));
                    fields.push(recipient_workloads::property(
                        Tag::ATTACH_CONTENT_BASE_W,
                        "http://example.com/assets/",
                        Type::UNICODE,
                    ));
                }
            }
            attribute(&mut input, Attribute::ATTACHMENT, &properties(fields));
        }
    }
    Workload {
        name,
        message: TnefMessage::load(input.as_slice()).unwrap(),
        input,
        payload,
        rtf,
        html,
        options: TnefConversionOptions {
            attachment_placeholder_callback: if matches!(
                name,
                "RtfPlaceholders" | "RtfPlaceholdersFooter"
            ) {
                Some(Arc::new(|attachment, _| {
                    Ok(attachment.file_name()?.map(|name| format!("<<{name}>>")))
                }))
            } else {
                None
            },
            ..Default::default()
        },
    }
}

fn advanced_workload(name: &'static str) -> Workload {
    let recursive = name == "EmbeddedRecursive";
    let ole = matches!(name, "OleImages" | "OleFallback");
    let skeleton = matches!(name, "MimeSkeleton" | "MimeSkeletonFallback");
    let mut payload: Vec<u8> = (0..2048).map(|i| (i * 31 + 7) as u8).collect();
    let mut image = payload.clone();
    image[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    if recursive {
        let leaf = embedded_workloads::document(Attribute::BODY, b"deep body\0");
        payload = embedded_workloads::document(Attribute::SUBJECT, b"Inner\0");
        attribute(&mut payload, Attribute::BODY, b"embedded body\0");
        let render = [&1u16.to_le_bytes()[..], &(-1i32).to_le_bytes(), &[0; 8]].concat();
        attribute(&mut payload, Attribute::ATTACH_RENDER_DATA, &render);
        attribute(
            &mut payload,
            Attribute::ATTACHMENT,
            &properties(vec![
                value(
                    Tag::ATTACH_DATA_BIN,
                    &[embedded_workloads::IID.as_slice(), &leaf].concat(),
                ),
                integer(Tag::ATTACH_METHOD, 5),
            ]),
        );
    }
    let mut fields = vec![
        recipient_workloads::property(Tag::SUBJECT_W, "Property subject", Type::UNICODE),
        recipient_workloads::property(Tag::BODY_W, "MAPI body", Type::UNICODE),
        recipient_workloads::property(
            Tag::RECEIVED_BY_EMAIL_ADDRESS_W,
            "alice@example.com",
            Type::UNICODE,
        ),
    ];
    if skeleton {
        let mut mime = String::from(
            "Subject: Skeleton subject\r\nTo: alice@example.com\r\nContent-Type: multipart/mixed; boundary=skeleton\r\n\r\n--skeleton\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
        );
        for index in 0..32 {
            let id = if name == "MimeSkeletonFallback" && index == 31 {
                32
            } else {
                index
            };
            mime.push_str(&format!("\r\n--skeleton\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=part{index}.bin\r\nContent-Transfer-Encoding: base64\r\nX-Exchange-MIME-Skeleton-Content-Id: <part{id}@example.com>\r\n\r\n"));
        }
        mime.push_str("\r\n--skeleton--\r\n");
        fields.push(value(Tag::MIME_SKELETON, mime.as_bytes()));
    }
    let mut input = embedded_workloads::document(Attribute::MAPI_PROPERTIES, &properties(fields));
    for index in 0..32 {
        let render = [&1u16.to_le_bytes()[..], &(-1i32).to_le_bytes(), &[0; 8]].concat();
        attribute(&mut input, Attribute::ATTACH_RENDER_DATA, &render);
        let bytes = if recursive {
            [embedded_workloads::IID.as_slice(), &payload].concat()
        } else {
            payload.clone()
        };
        attribute(
            &mut input,
            Attribute::ATTACHMENT,
            &properties(vec![
                value(Tag::ATTACH_DATA_BIN, &bytes),
                integer(
                    Tag::ATTACH_METHOD,
                    if recursive {
                        5
                    } else if ole {
                        6
                    } else {
                        1
                    },
                ),
                recipient_workloads::property(
                    Tag::ATTACH_LONG_FILENAME_W,
                    &format!("part{index}.bin"),
                    Type::UNICODE,
                ),
                recipient_workloads::property(
                    Tag::ATTACH_CONTENT_ID_W,
                    &format!("part{index}@example.com"),
                    Type::UNICODE,
                ),
            ]),
        );
    }
    let mut options = TnefConversionOptions {
        convert_embedded_messages: recursive,
        ..Default::default()
    };
    if ole {
        options.ole_object_converter = Some(Arc::new(move |_: &TnefAttachment| {
            let bytes = if name == "OleImages" {
                image.clone()
            } else {
                b"not an image".to_vec()
            };
            Ok(Some(Box::new(Cursor::new(bytes)) as Box<dyn Read>))
        }));
    }
    Workload {
        name,
        message: TnefMessage::load(input.as_slice()).unwrap(),
        input,
        payload,
        rtf: None,
        html: String::new(),
        options,
    }
}

fn traverse(message: &TnefMessage, summary: &mut Summary, content: bool) {
    summary.messages += 1;
    summary.attachments += message.attachments().len();
    summary.recipients += message.recipients().len();
    if !content {
        return;
    }
    let mut buffer = [0; 4096];
    for body in [message.text_body(), message.html_body(), message.rtf_body()]
        .into_iter()
        .flatten()
    {
        let mut reader = body.open_decoded_read();
        loop {
            let count = reader.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            summary.bytes(black_box(&buffer[..count]));
        }
    }
    for attachment in message.attachments() {
        if attachment.is_embedded_message() {
            let child = attachment
                .load_embedded_message_with_logger(&mut |issue| panic!("invalid child: {issue:?}"))
                .unwrap();
            traverse(&child, summary, true);
        } else {
            let mut reader = attachment.open_read();
            loop {
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                summary.bytes(black_box(&buffer[..count]));
            }
        }
    }
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    if matches!(mode, Mode::Convert | Mode::ConvertWrite) {
        let result = black_box(&case.message)
            .to_mime_message_with_options(&case.options)
            .unwrap();
        let mut message = result.message;
        let mut summary = Summary {
            messages: 1 + message
                .iter()
                .filter(|entity| matches!(entity, MimeEntity::Message(_)))
                .count(),
            attachments: case.message.attachments().len(),
            recipients: message.to().addresses.mailboxes().count(),
            ..Default::default()
        };
        for loss in result.losses {
            summary.bytes(loss.description.as_bytes());
            summary.checksum = summary.checksum.wrapping_add(loss.kind as u64);
        }
        if matches!(mode, Mode::ConvertWrite) {
            let mut bytes = Vec::new();
            message.write_to(&mut bytes).unwrap();
            summary.bytes(black_box(&bytes));
        } else {
            for header in message.headers().iter() {
                summary.bytes(header.raw());
            }
            for entity in message.iter() {
                for header in entity.headers().iter() {
                    summary.bytes(header.raw());
                }
                match entity {
                    MimeEntity::Part(part) => {
                        summary.bytes(black_box(part.content().get_ref().get_ref()))
                    }
                    MimeEntity::Message(part) => {
                        for header in part.message().unwrap().headers().iter() {
                            summary.bytes(header.raw());
                        }
                    }
                    MimeEntity::Multipart(_) => (),
                }
            }
        }
        return summary;
    }
    let mut reader = TnefReader::new(Source {
        bytes: black_box(&case.input),
        chunk: if case.name == "ShortReads" {
            7
        } else {
            usize::MAX
        },
    });
    let message = reader
        .load_message_with_logger(&mut |issue| panic!("invalid message fixture: {issue:?}"))
        .unwrap();
    let mut summary = Summary::default();
    traverse(&message, &mut summary, matches!(mode, Mode::Traverse));
    summary
}

impl Workload {
    fn check_advanced(&self) {
        let recursive = self.name == "EmbeddedRecursive";
        let mut expected = Summary {
            messages: 1,
            attachments: 32,
            recipients: 0,
            ..Default::default()
        };
        assert_eq!(read(self, Mode::Load), expected);
        expected.bytes(
            &"MAPI body"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        for _ in 0..32 {
            if recursive {
                expected.messages += 2;
                expected.attachments += 1;
                expected.bytes(b"embedded body");
                expected.bytes(b"deep body");
            } else {
                expected.bytes(&self.payload);
            }
        }
        assert_eq!(read(self, Mode::Traverse), expected);
        let mut result = self
            .message
            .to_mime_message_with_options(&self.options)
            .unwrap();
        let expected_losses = match self.name {
            "OleFallback" => vec![TnefConversionLossKind::OleObjectNotRendered; 32],
            "MimeSkeletonFallback" => vec![TnefConversionLossKind::InvalidMimeSkeleton],
            _ => vec![],
        };
        assert_eq!(
            result
                .losses
                .iter()
                .map(|loss| loss.kind)
                .collect::<Vec<_>>(),
            expected_losses
        );
        assert_eq!(
            result.message.subject().unwrap().text,
            if self.name == "MimeSkeleton" {
                "Skeleton subject"
            } else {
                "Property subject"
            }
        );
        self.check_advanced_mime(&mut result.message);
        let mut bytes = Vec::new();
        result.message.write_to(&mut bytes).unwrap();
        let mut parsed = MimeParser::new(bytes.as_slice()).parse_message().unwrap();
        self.check_advanced_mime(&mut parsed);
        for mode in [Mode::Convert, Mode::ConvertWrite] {
            let summary = read(self, mode);
            assert_eq!(summary.messages, if recursive { 65 } else { 1 });
            assert_eq!(summary.attachments, 32);
            assert_eq!(summary.recipients, 1);
            assert!(
                summary.bytes
                    > 32 * if recursive {
                        b"deep body".len()
                    } else {
                        self.payload.len()
                    }
            );
        }
    }

    fn check_advanced_mime(&self, message: &mut MimeMessage) {
        let MimeEntity::Multipart(mixed) = message.body_mut().unwrap() else {
            panic!("mixed")
        };
        assert_eq!(mixed.children().len(), 33);
        let MimeEntity::Part(body) = &mut mixed.children_mut()[0] else {
            panic!("plain body")
        };
        let mut decoded = Vec::new();
        body.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, b"MAPI body");
        for (index, entity) in mixed.children_mut()[1..].iter_mut().enumerate() {
            assert!(
                !entity
                    .headers()
                    .contains("X-Exchange-MIME-Skeleton-Content-Id")
            );
            if self.name == "EmbeddedRecursive" {
                let MimeEntity::Message(part) = entity else {
                    panic!("recursive message")
                };
                let nested = part.message_mut().unwrap();
                assert_eq!(nested.subject().unwrap().text, "Inner");
                let MimeEntity::Multipart(mixed) = nested.body_mut().unwrap() else {
                    panic!("inner mixed")
                };
                assert_eq!(mixed.children().len(), 2);
                let MimeEntity::Part(body) = &mut mixed.children_mut()[0] else {
                    panic!("inner body")
                };
                decoded.clear();
                body.content_mut().decode_to(&mut decoded).unwrap();
                assert_eq!(decoded, b"embedded body");
                let MimeEntity::Message(part) = &mut mixed.children_mut()[1] else {
                    panic!("deep message")
                };
                let MimeEntity::Part(body) = part.message_mut().unwrap().body_mut().unwrap() else {
                    panic!("deep body")
                };
                decoded.clear();
                body.content_mut().decode_to(&mut decoded).unwrap();
                assert_eq!(decoded, b"deep body");
            } else {
                let MimeEntity::Part(part) = entity else {
                    panic!("attachment")
                };
                let image = self.name == "OleImages";
                assert_eq!(
                    part.file_name().unwrap(),
                    Some(format!(
                        "part{index}.bin{}",
                        if image { ".png" } else { "" }
                    ))
                );
                assert_eq!(
                    part.content_type().unwrap().unwrap().mime_type(),
                    if image {
                        "image/png"
                    } else {
                        "application/octet-stream"
                    }
                );
                decoded.clear();
                part.content_mut().decode_to(&mut decoded).unwrap();
                let mut expected = self.payload.clone();
                if image {
                    expected[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
                }
                assert_eq!(decoded, expected);
            }
        }
    }

    pub fn check(&self) {
        if advanced(self.name) {
            self.check_advanced();
            return;
        }
        let message = TnefMessage::load(self.input.as_slice()).unwrap();
        let legacy = self.name == "Legacy";
        assert_eq!(
            message.message_class().unwrap().as_deref(),
            Some("IPM.Note")
        );
        assert_eq!(
            message.subject().unwrap().as_deref(),
            Some(if legacy {
                "Legacy subject"
            } else {
                "MAPI subject"
            })
        );
        assert_eq!(
            message.text_body().unwrap().text().unwrap(),
            if legacy { "Legacy body" } else { "MAPI body" }
        );
        assert_eq!(
            message.html_body().map(|body| body.text().unwrap()),
            (!legacy).then_some(self.html.as_str().into())
        );
        assert_eq!(
            message
                .rtf_body()
                .map(|body| {
                    let mut bytes = Vec::new();
                    body.open_decoded_read().read_to_end(&mut bytes).unwrap();
                    bytes
                })
                .as_deref(),
            self.rtf.as_deref()
        );
        assert_eq!(message.recipients().len(), 1);
        assert_eq!(
            message.recipients()[0].display_name().unwrap().as_deref(),
            Some("Alice")
        );
        assert_eq!(
            message.recipients()[0].email_address().unwrap().as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(message.attachments().len(), 32);
        for (index, attachment) in message.attachments().iter().enumerate() {
            assert_eq!(
                attachment.file_name().unwrap().as_deref(),
                Some(format!("{}{index}.bin", if legacy { "legacy" } else { "mapi" }).as_str())
            );
            assert_eq!(attachment.as_bytes(), Some(self.payload.as_slice()));
            assert_eq!(attachment.is_embedded_message(), self.name == "Embedded");
            if attachment.is_embedded_message() {
                assert_eq!(
                    attachment
                        .load_embedded_message()
                        .unwrap()
                        .subject()
                        .unwrap()
                        .as_deref(),
                    Some("Inner")
                );
            }
        }
        let mut expected = Summary {
            messages: 1,
            attachments: 32,
            recipients: 1,
            ..Default::default()
        };
        assert_eq!(read(self, Mode::Load), expected);
        // Decoded Unicode bodies still expose UTF-16LE bytes, not UTF-8 text.
        if legacy {
            expected.bytes(b"Legacy body");
        } else {
            expected.bytes(
                &"MAPI body"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            expected.bytes(self.html.as_bytes());
        }
        if let Some(rtf) = &self.rtf {
            expected.bytes(rtf);
        }
        for _ in 0..32 {
            if self.name == "Embedded" {
                expected.messages += 1;
                expected.bytes(b"embedded body");
            } else {
                expected.bytes(&self.payload);
            }
        }
        assert_eq!(read(self, Mode::Traverse), expected);
        self.check_conversion();
    }

    fn expected_rtf_text(&self) -> String {
        let mut text = String::new();
        for index in 0..RTF_PARAGRAPHS {
            if self.name == "RtfPlaceholders" && index < 32 {
                text.push_str(&format!("Hello<<mapi{}.bin>> from RTF\r\n", 31 - index));
            } else {
                text.push_str("Hello from RTF\r\n");
            }
        }
        if self.name == "RtfPlaceholdersFooter" {
            for index in (0..32).rev() {
                text.push_str(&format!("\r\n<<mapi{index}.bin>>"));
            }
            text.push_str("\r\n");
        }
        text
    }

    fn check_conversion(&self) {
        let legacy = self.name == "Legacy";
        let images = matches!(
            self.name,
            "RtfImages" | "RtfImagesFooter" | "RtfPlaceholders" | "RtfPlaceholdersFooter"
        );
        let mut result = self
            .message
            .to_mime_message_with_options(&self.options)
            .unwrap();
        assert!(result.losses.is_empty(), "{:?}", result.losses);
        assert_eq!(
            result.message.subject().unwrap().text,
            if legacy {
                "Legacy subject"
            } else {
                "MAPI subject"
            }
        );
        assert_eq!(
            result
                .message
                .to()
                .addresses
                .mailboxes()
                .next()
                .unwrap()
                .address()
                .to_string(),
            "alice@example.com"
        );
        let MimeEntity::Multipart(mixed) = result.message.body().unwrap() else {
            panic!("mixed");
        };
        assert_eq!(
            mixed.content_type().unwrap().unwrap().mime_type(),
            "multipart/mixed"
        );
        let inline = images || matches!(self.name, "HtmlRelated" | "RtfHtmlRelated");
        assert_eq!(mixed.children().len(), if inline { 17 } else { 33 });
        let root = if inline {
            let MimeEntity::Multipart(related) = &mixed.children()[0] else {
                panic!("related")
            };
            let content_type = related.content_type().unwrap().unwrap();
            assert_eq!(content_type.mime_type(), "multipart/related");
            assert_eq!(
                content_type.parameters().get("type"),
                Some("multipart/alternative")
            );
            assert!(content_type.parameters().get("start").is_none());
            assert_eq!(related.children().len(), 17);
            &related.children()[0]
        } else {
            &mixed.children()[0]
        };
        if legacy {
            let MimeEntity::Part(part) = root else {
                panic!("plain");
            };
            assert_eq!(part.content().get_ref().get_ref(), b"Legacy body");
        } else {
            let MimeEntity::Multipart(alternative) = root else {
                panic!("alternative");
            };
            assert_eq!(
                alternative.content_type().unwrap().unwrap().mime_type(),
                "multipart/alternative"
            );
            assert_eq!(alternative.children().len(), 2);
            let generated = images || matches!(self.name, "Rtf" | "RtfHtml" | "RtfHtmlRelated");
            let plain = if generated {
                self.expected_rtf_text()
            } else {
                "MAPI body".to_owned()
            };
            for (entity, mime, bytes) in [
                (&alternative.children()[0], "text/plain", plain.as_bytes()),
                (
                    &alternative.children()[1],
                    "text/html",
                    self.html.as_bytes(),
                ),
            ] {
                let MimeEntity::Part(part) = entity else {
                    panic!("body part");
                };
                assert_eq!(part.content_type().unwrap().unwrap().mime_type(), mime);
                let content = part.content().get_ref().get_ref();
                if generated && mime == "text/html" {
                    let html = std::str::from_utf8(content).unwrap();
                    assert_eq!(html.matches("from RTF").count(), RTF_PARAGRAPHS);
                    assert_eq!(html.contains("HTML body"), self.name == "RtfHtmlRelated");
                    assert!(html.starts_with('<'));
                    if images {
                        let parts: Vec<_> = result
                            .message
                            .leaf_parts()
                            .filter(|part| {
                                part.content_type()
                                    .unwrap()
                                    .unwrap()
                                    .is_mime_type("image", "png")
                            })
                            .collect();
                        assert_eq!(parts.len(), 16);
                        assert_eq!(html.matches("<img").count(), 16);
                        let mut previous = 0;
                        let mut ids = std::collections::HashSet::new();
                        for part in parts.into_iter().rev() {
                            let id = part.content_id().unwrap().unwrap();
                            assert!(ids.insert(id.to_string()));
                            let image = format!(
                                "<img src=\"cid:{}\" alt=\"{}\"/>",
                                id.as_str(),
                                part.file_name().unwrap().unwrap()
                            );
                            let offset = html.find(&image).unwrap();
                            assert!(offset > previous);
                            previous = offset;
                            if matches!(self.name, "RtfImages" | "RtfPlaceholders") {
                                assert!(html.contains(&format!("Hello{image} from RTF")));
                            } else {
                                assert!(html.contains(&format!("<div>{image}</div>")));
                                assert!(offset > html.rfind("from RTF").unwrap());
                            }
                        }
                        if matches!(self.name, "RtfPlaceholders" | "RtfPlaceholdersFooter") {
                            let mut previous = 0;
                            for index in (0..32).rev() {
                                let label = format!("&lt;&lt;mapi{index}.bin&gt;&gt;");
                                if index % 2 == 0 {
                                    assert!(!html.contains(&label));
                                } else {
                                    let offset = html.find(&label).unwrap();
                                    assert!(offset > previous);
                                    previous = offset;
                                    if self.name == "RtfPlaceholders" {
                                        assert!(html.contains(&format!("Hello{label} from RTF")));
                                    } else {
                                        assert!(html.contains(&format!("<div>{label}</div>")));
                                        assert!(offset > html.rfind("from RTF").unwrap());
                                    }
                                }
                            }
                        }
                    }
                    if matches!(self.name, "RtfHtml" | "RtfHtmlRelated") {
                        assert_eq!(
                            html,
                            format!(
                                "<html><body>{}{}</body></html>",
                                "<p>Hello from RTF</p>".repeat(RTF_PARAGRAPHS),
                                if inline {
                                    reference_html(false)
                                } else {
                                    String::new()
                                },
                            )
                        );
                    }
                } else {
                    assert_eq!(content, bytes);
                }
            }
        }
        let indices: Vec<_> = if inline {
            (0..32).step_by(2).chain((1..32).step_by(2)).collect()
        } else {
            (0..32).collect()
        };
        let parts: Vec<_> = result
            .message
            .leaf_parts()
            .filter(|part| part.file_name().unwrap().is_some())
            .collect();
        assert_eq!(parts.len(), 32);
        for (part, index) in parts.into_iter().zip(indices) {
            assert_eq!(
                part.file_name().unwrap(),
                Some(format!(
                    "{}{index}.bin",
                    if legacy { "legacy" } else { "mapi" }
                ))
            );
            assert_eq!(part.content().get_ref().get_ref(), &self.payload);
            assert_eq!(
                part.content_disposition().unwrap().unwrap().is_attachment(),
                !(inline && index % 2 == 0)
            );
            assert_eq!(
                part.content_type().unwrap().unwrap().mime_type(),
                if self.name == "Embedded" {
                    "application/ms-tnef"
                } else if images {
                    if index % 2 == 0 {
                        "image/png"
                    } else {
                        "application/pdf"
                    }
                } else {
                    "application/octet-stream"
                }
            );
        }
        let mut wire = Vec::new();
        result.message.write_to(&mut wire).unwrap();
        let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
        let MimeEntity::Multipart(mixed) = parsed.body_mut().unwrap() else {
            panic!("mixed");
        };
        assert_eq!(mixed.children().len(), if inline { 17 } else { 33 });
        let (body, ordinary) = mixed.children_mut().split_at_mut(1);
        let (root, mut attachments) = if inline {
            let MimeEntity::Multipart(related) = &mut body[0] else {
                panic!("related")
            };
            assert_eq!(related.children().len(), 17);
            let (root, images) = related.children_mut().split_at_mut(1);
            (&mut root[0], images.iter_mut().collect::<Vec<_>>())
        } else {
            (&mut body[0], vec![])
        };
        attachments.extend(ordinary);
        if images || matches!(self.name, "Rtf" | "RtfHtml" | "RtfHtmlRelated") {
            let MimeEntity::Multipart(alternative) = root else {
                panic!("alternative");
            };
            let MimeEntity::Part(plain) = &mut alternative.children_mut()[0] else {
                panic!("plain");
            };
            let mut decoded = Vec::new();
            plain.content_mut().decode_to(&mut decoded).unwrap();
            assert_eq!(decoded, self.expected_rtf_text().as_bytes());
        }
        for child in attachments {
            let MimeEntity::Part(part) = child else {
                panic!("attachment");
            };
            let mut decoded = Vec::new();
            part.content_mut().decode_to(&mut decoded).unwrap();
            assert_eq!(decoded, self.payload);
        }
        for mode in [Mode::Convert, Mode::ConvertWrite] {
            let summary = read(self, mode);
            assert_eq!(summary.messages, 1);
            assert_eq!(summary.attachments, 32);
            assert_eq!(summary.recipients, 1);
            assert!(summary.bytes > self.payload.len() * 32);
        }
    }
}

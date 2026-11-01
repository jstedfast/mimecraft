// Original attachment-model workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{embedded_workloads, recipient_workloads};
use mimecraft::tnef::{
    TnefAttachment as Attachment, TnefAttributeTag as Attribute, TnefPropertySet,
    TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReader,
};
use std::{hint::black_box, io::Read};

pub const CASES: [&str; 4] = ["Direct", "BinaryText", "Embedded", "Empty"];
pub const MODES: [Mode; 3] = [Mode::Load, Mode::Access, Mode::Stream];

#[derive(Clone, Copy)]
pub enum Mode {
    Load,
    Access,
    Stream,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Load => "Load",
            Self::Access => "Access",
            Self::Stream => "Stream",
        }
    }
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    attachments: Vec<Attachment>,
    expected: Vec<Expected>,
}

struct Expected {
    name: Option<String>,
    mime: Option<String>,
    cid: Option<String>,
    method: i32,
    flags: u32,
    embedded: bool,
    content: Option<Vec<u8>>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub attachments: usize,
    pub bytes: usize,
    pub checksum: u64,
}

impl Summary {
    fn number(&mut self, value: u64) {
        self.checksum = self.checksum.wrapping_mul(31).wrapping_add(value);
    }

    fn text(&mut self, text: Option<&str>) {
        self.number(u64::from(text.is_some()));
        if let Some(text) = text {
            for byte in text.bytes() {
                self.number(u64::from(byte));
            }
        }
    }

    fn content(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len();
        for &byte in bytes {
            self.number(u64::from(byte));
        }
    }
}

fn integer(tag: Tag, value: i32) -> Vec<u8> {
    [tag.0.to_le_bytes(), value.to_le_bytes()].concat()
}

fn value(tag: Tag, content: &[u8]) -> Vec<u8> {
    let mut bytes = tag.0.to_le_bytes().to_vec();
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(content.len() as u32).to_le_bytes());
    bytes.extend_from_slice(content);
    bytes.extend(std::iter::repeat_n(0, (4 - (content.len() & 3)) & 3));
    bytes
}

fn load(input: &[u8]) -> Vec<Attachment> {
    let mut reader = TnefReader::new(input);
    let mut logger = |issue| panic!("invalid attachment fixture: {issue:?}");
    let mut attachments = Vec::new();
    while reader.read_next_with_logger(&mut logger).unwrap() {
        assert_eq!(reader.attribute().unwrap().tag, Attribute::ATTACHMENT);
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        let mut values = Vec::new();
        let mut content = None;
        while props.read_next_property().unwrap() {
            if props.property().unwrap().tag.id() == Tag::ATTACH_DATA_BIN.id() {
                assert!(
                    content.is_none(),
                    "one content value per generated attachment"
                );
                content = Some(props.read_attachment_content().unwrap());
            } else {
                values.push(props.read_property().unwrap());
            }
        }
        attachments.push(Attachment::new(TnefPropertySet::from(values), content));
    }
    attachments
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    let mut expected = Vec::new();
    for index in 0..32 {
        let empty = name == "Empty";
        let embedded = name == "Embedded";
        let object = !empty && (embedded || index % 2 == 1);
        let mut fields = Vec::new();
        let file_name = (!empty).then(|| format!("Caf\u{e9} {index}.bin"));
        let mime = (!empty).then(|| "application/octet-stream".to_owned());
        let cid = (!empty).then(|| format!("part{index}@example.com"));
        let method = if embedded {
            5
        } else if object {
            6
        } else {
            1
        };
        let flags = if empty { 0 } else { 0x80000004u32 };
        if let Some(file_name) = &file_name {
            let kind = if name == "BinaryText" {
                Type::BINARY
            } else if index % 2 == 0 {
                Type::UNICODE
            } else {
                Type::STRING8
            };
            let tag = if index % 2 == 0 {
                Tag::ATTACH_LONG_FILENAME_W
            } else {
                Tag::ATTACH_FILENAME_W
            };
            fields.push(recipient_workloads::property(tag, file_name, kind));
            fields.push(recipient_workloads::property(
                Tag::ATTACH_MIME_TAG_W,
                mime.as_deref().unwrap(),
                kind,
            ));
            fields.push(recipient_workloads::property(
                Tag::ATTACH_CONTENT_ID_W,
                cid.as_deref().unwrap(),
                kind,
            ));
            fields.push(integer(Tag::ATTACH_METHOD, method));
            fields.push(integer(Tag::ATTACH_FLAGS, flags as i32));
        }
        let content = if empty {
            (index % 2 == 0).then(Vec::new)
        } else {
            let bytes: Vec<_> = (0..2048).map(|i| (i * 31 + index) as u8).collect();
            Some(if embedded {
                embedded_workloads::document(Attribute::BODY, &bytes)
            } else {
                bytes
            })
        };
        if let Some(content) = &content {
            let tag = if object {
                Tag::ATTACH_DATA_OBJ
            } else {
                Tag::ATTACH_DATA_BIN
            };
            let mut raw = if embedded {
                embedded_workloads::IID.to_vec()
            } else if object {
                vec![0x55; 16]
            } else {
                Vec::new()
            };
            raw.extend_from_slice(content);
            fields.push(value(tag, &raw));
        }
        let mut payload = (fields.len() as u32).to_le_bytes().to_vec();
        for field in fields {
            payload.extend(field);
        }
        embedded_workloads::attribute(&mut input, Attribute::ATTACHMENT, &payload);
        expected.push(Expected {
            name: file_name,
            mime,
            cid,
            method,
            flags,
            embedded,
            content,
        });
    }
    let attachments = load(&input);
    Workload {
        name,
        input,
        attachments,
        expected,
    }
}

fn access(attachments: &[Attachment]) -> Summary {
    let mut result = Summary::default();
    for attachment in black_box(attachments) {
        let attachment = black_box(attachment);
        result.attachments += 1;
        result.bytes += attachment.len();
        result.text(attachment.file_name().unwrap().as_deref());
        result.text(attachment.mime_type().unwrap().as_deref());
        result.text(attachment.content_id().unwrap().as_deref());
        result.number(i32::from(attachment.method().unwrap()) as u64);
        result.number(u64::from(attachment.flags().unwrap().bits()));
        result.number(u64::from(attachment.is_embedded_message()));
        result.number(u64::from(attachment.has_content()));
    }
    result
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    match mode {
        Mode::Load => access(&load(black_box(&case.input))),
        Mode::Access => access(&case.attachments),
        Mode::Stream => {
            let mut result = Summary::default();
            let mut buffer = [0; 4096];
            for attachment in black_box(&case.attachments) {
                result.attachments += 1;
                let mut reader = attachment.open_read();
                loop {
                    let count = reader.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    result.content(black_box(&buffer[..count]));
                }
            }
            result
        }
    }
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(self.attachments.len(), 32);
        assert_eq!(self.attachments.len(), self.expected.len());
        let mut access_expected = Summary::default();
        let mut stream_expected = Summary::default();
        for (attachment, expected) in self.attachments.iter().zip(&self.expected) {
            assert_eq!(
                attachment.file_name().unwrap().as_deref(),
                expected.name.as_deref()
            );
            assert_eq!(
                attachment.mime_type().unwrap().as_deref(),
                expected.mime.as_deref()
            );
            assert_eq!(
                attachment.content_id().unwrap().as_deref(),
                expected.cid.as_deref()
            );
            assert_eq!(attachment.as_bytes(), expected.content.as_deref());
            assert!(
                attachment
                    .properties()
                    .get_by_id(Tag::ATTACH_DATA_BIN.id())
                    .is_none()
            );
            access_expected.attachments += 1;
            access_expected.bytes += expected.content.as_ref().map_or(0, Vec::len);
            access_expected.text(expected.name.as_deref());
            access_expected.text(expected.mime.as_deref());
            access_expected.text(expected.cid.as_deref());
            access_expected.number(expected.method as u64);
            access_expected.number(u64::from(expected.flags));
            access_expected.number(u64::from(expected.embedded));
            access_expected.number(u64::from(expected.content.is_some()));
            stream_expected.attachments += 1;
            stream_expected.content(expected.content.as_deref().unwrap_or_default());
        }
        assert_eq!(read(self, Mode::Load), access_expected, "{}", self.name);
        assert_eq!(read(self, Mode::Access), access_expected, "{}", self.name);
        assert_eq!(read(self, Mode::Stream), stream_expected, "{}", self.name);
    }
}

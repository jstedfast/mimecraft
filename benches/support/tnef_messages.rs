// Original whole-message workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    attachment_workloads::{integer, value},
    embedded_workloads, recipient_workloads,
};
use mimecraft::tnef::{
    TnefAttributeTag as Attribute, TnefMessage, TnefPropertyTag as Tag, TnefPropertyType as Type,
    TnefReader,
};
use std::{
    hint::black_box,
    io::{self, Read},
};

pub const CASES: [&str; 5] = ["Legacy", "Mapi", "Mixed", "Embedded", "ShortReads"];
pub const MODES: [Mode; 2] = [Mode::Load, Mode::Traverse];

#[derive(Clone, Copy)]
pub enum Mode {
    Load,
    Traverse,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Load => "Load",
            Self::Traverse => "Traverse",
        }
    }
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    payload: Vec<u8>,
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

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let legacy = name != "Mapi";
    let mapi = name != "Legacy";
    let mut input =
        embedded_workloads::document(Attribute::MESSAGE_CLASS, b"IPM.Microsoft Mail.Note\0");
    if legacy {
        attribute(&mut input, Attribute::SUBJECT, b"Legacy subject\0");
        attribute(&mut input, Attribute::BODY, b"Legacy body\0");
    }
    if mapi {
        attribute(
            &mut input,
            Attribute::MAPI_PROPERTIES,
            &properties(vec![
                recipient_workloads::property(Tag::SUBJECT_W, "MAPI subject", Type::UNICODE),
                recipient_workloads::property(Tag::BODY_W, "MAPI body", Type::UNICODE),
                value(Tag::BODY_HTML_B, b"<p>HTML body</p>"),
                integer(Tag::INTERNET_CODEPAGE, 65001),
            ]),
        );
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
    let payload = if name == "Embedded" {
        let mut child = embedded_workloads::document(Attribute::SUBJECT, b"Inner\0");
        attribute(&mut child, Attribute::BODY, b"embedded body");
        child
    } else {
        (0..2048).map(|i| (i * 31 + 7) as u8).collect()
    };
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
            attribute(
                &mut input,
                Attribute::ATTACHMENT,
                &properties(vec![
                    // Exercise retrospective Binary recognition when the method follows data.
                    value(Tag::ATTACH_DATA_BIN, &bytes),
                    integer(Tag::ATTACH_METHOD, if name == "Embedded" { 5 } else { 1 }),
                    recipient_workloads::property(
                        Tag::ATTACH_LONG_FILENAME_W,
                        &format!("mapi{index}.bin"),
                        Type::UNICODE,
                    ),
                ]),
            );
        }
    }
    Workload {
        name,
        input,
        payload,
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
    pub fn check(&self) {
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
            (!legacy).then_some("<p>HTML body</p>".into())
        );
        assert!(message.rtf_body().is_none());
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
            expected.bytes(b"<p>HTML body</p>");
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
    }
}

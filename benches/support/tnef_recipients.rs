// Original recipient model workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::tnef::{
    TnefAttributeTag, TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReader, TnefRecipient,
};

pub const CASES: [&str; 3] = ["Direct", "Fallbacks", "BinaryText"];

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Load,
    Access,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Load => "Load",
            Self::Access => "Access",
        }
    }
}

pub const MODES: [Mode; 2] = [Mode::Load, Mode::Access];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    recipients: Vec<TnefRecipient>,
    expected: Vec<Expected>,
}

struct Expected {
    kind: i32,
    name: Option<String>,
    address: Option<String>,
    address_type: Option<String>,
    properties: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub rows: usize,
    pub fields: usize,
    pub checksum: u64,
}

impl Summary {
    fn recipient_type(&mut self, value: i32) {
        self.rows += 1;
        self.fields += 1;
        self.checksum = self.checksum.wrapping_mul(31).wrapping_add(value as u64);
    }

    fn text(&mut self, value: Option<&str>) {
        if let Some(value) = value {
            self.fields += 1;
            for &byte in value.as_bytes() {
                self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
            }
        }
    }
}

fn property(tag: Tag, value: &str, kind: Type) -> Vec<u8> {
    let raw: Vec<_> = if kind == Type::UNICODE {
        value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    } else {
        value
            .chars()
            .map(|ch| {
                if ch == '\u{e9}' {
                    0xe9
                } else {
                    assert!(ch.is_ascii(), "benchmark text must be Windows-1252");
                    ch as u8
                }
            })
            .chain([0])
            .collect()
    };
    let mut data = Tag::new(tag.id(), kind).0.to_le_bytes().to_vec();
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    data.extend_from_slice(&raw);
    data.extend(std::iter::repeat_n(0, (4 - (raw.len() & 3)) & 3));
    data
}

fn load(input: &[u8]) -> Vec<TnefRecipient> {
    let mut reader = TnefReader::new(input);
    let mut logger = |issue| panic!("invalid recipient benchmark input: {issue:?}");
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let rows = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_rows_as_property_sets()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    rows.into_iter().map(TnefRecipient::from).collect()
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let mut expected = Vec::new();
    let mut payload = 256u32.to_le_bytes().to_vec();
    for index in 0..256 {
        let fallback = name == "Fallbacks";
        let absent = fallback && index % 4 == 3;
        let kind = if name == "BinaryText" {
            Type::BINARY
        } else if index % 2 == 0 {
            Type::UNICODE
        } else {
            Type::STRING8
        };
        let recipient_type = if absent { 1 } else { index % 5 };
        let display_name = (!absent).then(|| format!("Caf\u{e9} Recipient {index}"));
        let email = (!absent).then(|| format!("recipient{index}@example.com"));
        let address_type = (!absent).then(|| "SMTP".to_owned());
        let mut row = Vec::new();
        let mut count = 0;
        if !absent {
            row.extend_from_slice(&Tag::RECIPIENT_TYPE.0.to_le_bytes());
            // High-order PidTagRecipientType bits indicate resent recipients,
            // not a different underlying recipient type ([MS-OXOMSG] 2.2.3.1).
            let flags = if index % 2 == 0 {
                0x10000000
            } else {
                0x80000000
            };
            row.extend_from_slice(&(recipient_type as u32 | flags).to_le_bytes());
            count += 1;
        }
        if let Some(value) = &display_name {
            let tag = if fallback {
                [
                    Tag::RECIPIENT_DISPLAY_NAME_W,
                    Tag::TRANSMITABLE_DISPLAY_NAME_W,
                    Tag::DISPLAY_NAME_W,
                ][index as usize % 4]
            } else {
                Tag::RECIPIENT_DISPLAY_NAME_W
            };
            if fallback && index % 4 < 2 {
                row.extend(property(Tag::DISPLAY_NAME_W, "lower priority", kind));
                count += 1;
            }
            row.extend(property(tag, value, kind));
            count += 1;
        }
        if let Some(value) = &email {
            let tag = if fallback {
                Tag::EMAIL_ADDRESS_W
            } else {
                Tag::SMTP_ADDRESS_W
            };
            row.extend(property(tag, value, kind));
            count += 1;
        }
        if let Some(value) = &address_type {
            row.extend(property(Tag::ADDRTYPE_W, value, kind));
            count += 1;
        }
        payload.extend_from_slice(&(count as u32).to_le_bytes());
        payload.extend(row);
        expected.push(Expected {
            kind: recipient_type,
            name: display_name,
            address: email,
            address_type,
            properties: count,
        });
    }
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0, 1];
    input.extend_from_slice(&TnefAttributeTag::RECIPIENT_TABLE.0.to_le_bytes());
    input.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    input.extend_from_slice(&payload);
    let checksum = payload
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
    input.extend_from_slice(&checksum.to_le_bytes());
    let recipients = load(&input);
    Workload {
        name,
        input,
        recipients,
        expected,
    }
}

fn access(recipients: &[TnefRecipient]) -> Summary {
    let mut summary = Summary::default();
    for recipient in black_box(recipients) {
        summary.recipient_type(black_box(recipient).recipient_type().unwrap().into());
        summary.text(black_box(recipient).display_name().unwrap().as_deref());
        summary.text(black_box(recipient).email_address().unwrap().as_deref());
        summary.text(black_box(recipient).address_type().unwrap().as_deref());
    }
    summary
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    match mode {
        Mode::Load => access(&load(black_box(&case.input))),
        Mode::Access => access(black_box(&case.recipients)),
    }
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(self.recipients.len(), 256);
        assert_eq!(self.expected.len(), self.recipients.len());
        let mut summary = Summary::default();
        for (recipient, expected) in self.recipients.iter().zip(&self.expected) {
            assert_eq!(
                i32::from(recipient.recipient_type().unwrap()),
                expected.kind
            );
            assert_eq!(
                recipient.display_name().unwrap().as_deref(),
                expected.name.as_deref()
            );
            assert_eq!(
                recipient.email_address().unwrap().as_deref(),
                expected.address.as_deref()
            );
            assert_eq!(
                recipient.address_type().unwrap().as_deref(),
                expected.address_type.as_deref()
            );
            assert_eq!(recipient.properties().len(), expected.properties);
            summary.recipient_type(expected.kind);
            summary.text(expected.name.as_deref());
            summary.text(expected.address.as_deref());
            summary.text(expected.address_type.as_deref());
        }
        for mode in MODES {
            assert_eq!(read(self, mode), summary, "{}/{}", self.name, mode.name());
        }
    }
}

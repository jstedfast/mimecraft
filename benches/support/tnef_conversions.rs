// Original owned-value access workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::tnef::{
    TnefAttributeTag, TnefPropertySet, TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReader,
};

pub const CASES: [&str; 5] = ["Numeric", "Text", "BinaryText", "Bytes", "Lookup"];

pub struct Workload {
    pub name: &'static str,
    /// Original wire size, used only as an equivalent-byte throughput unit.
    pub input_length: usize,
    properties: TnefPropertySet,
    queries: Vec<Tag>,
    expected: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub values: usize,
    pub checksum: u64,
}

fn byte_sum(bytes: &[u8]) -> u64 {
    bytes.iter().map(|&byte| u64::from(byte)).sum()
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let mut payload = 256u32.to_le_bytes().to_vec();
    let mut expected = Vec::new();
    let mut queries = Vec::new();
    for index in 0..256u16 {
        let number = i64::from(index) + 1;
        let (kind, raw) = match name {
            "Numeric" => {
                let (kind, raw, integer, float) = match index % 8 {
                    0 => (
                        Type::LONG,
                        (number as i32).to_le_bytes().to_vec(),
                        number,
                        number as f64,
                    ),
                    1 => (
                        Type::DOUBLE,
                        (number as f64 + 0.25).to_le_bytes().to_vec(),
                        number,
                        number as f64 + 0.25,
                    ),
                    2 => (
                        Type::CURRENCY,
                        (number * 10000 + 2500).to_le_bytes().to_vec(),
                        number,
                        number as f64 + 0.25,
                    ),
                    3 => (
                        Type::BOOLEAN,
                        (index as i32 % 2).to_le_bytes().to_vec(),
                        i64::from(index % 2),
                        f64::from(index % 2),
                    ),
                    4 => (
                        Type::ERROR,
                        (number as i32).to_le_bytes().to_vec(),
                        number,
                        number as f64,
                    ),
                    5 => (
                        Type::I2,
                        (-(number as i32)).to_le_bytes().to_vec(),
                        -number,
                        -number as f64,
                    ),
                    6 => (
                        Type::R4,
                        (number as f32 + 0.25).to_le_bytes().to_vec(),
                        number,
                        number as f64 + 0.25,
                    ),
                    _ => (
                        Type::I8,
                        (number * 65537).to_le_bytes().to_vec(),
                        number * 65537,
                        (number * 65537) as f64,
                    ),
                };
                expected.extend([integer as u64, float.to_bits()]);
                (kind, raw)
            }
            "Text" | "BinaryText" => {
                let text = format!("Caf\u{e9}-{index}");
                expected.push(byte_sum(text.as_bytes()));
                if name == "Text" && index % 2 == 0 {
                    (
                        Type::UNICODE,
                        text.encode_utf16()
                            .chain([0])
                            .flat_map(u16::to_le_bytes)
                            .collect(),
                    )
                } else {
                    let mut bytes = b"Caf\xe9-".to_vec();
                    bytes.extend_from_slice(index.to_string().as_bytes());
                    bytes.push(0);
                    (
                        if name == "Text" {
                            Type::STRING8
                        } else {
                            Type::BINARY
                        },
                        bytes,
                    )
                }
            }
            "Bytes" => {
                let kind = [Type::BINARY, Type::OBJECT, Type::CLASS_ID][index as usize % 3];
                let length = if kind == Type::CLASS_ID { 16 } else { 64 };
                expected.push(u64::from(index) * length as u64);
                (kind, vec![index as u8; length])
            }
            "Lookup" => {
                queries.push(Tag::new(0x5000 + index, Type::UNSPECIFIED));
                expected.push(number as u64);
                (Type::LONG, (number as i32).to_le_bytes().to_vec())
            }
            _ => unreachable!(),
        };
        payload.extend_from_slice(&Tag::new(0x5000 + index, kind).0.to_le_bytes());
        if matches!(
            kind,
            Type::UNICODE | Type::STRING8 | Type::BINARY | Type::OBJECT
        ) {
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        }
        payload.extend_from_slice(&raw);
        payload.extend(std::iter::repeat_n(0, (4 - (raw.len() & 3)) & 3));
    }
    if name == "Lookup" {
        for index in 0..64 {
            queries.push(Tag::new(0x6000 + index, Type::LONG));
            expected.push(0);
        }
    }
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0, 1];
    input.extend_from_slice(&TnefAttributeTag::MAPI_PROPERTIES.0.to_le_bytes());
    input.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    input.extend_from_slice(&payload);
    input.extend_from_slice(&(byte_sum(&payload) as u16).to_le_bytes());
    let mut reader = TnefReader::new(input.as_slice());
    let mut logger = |issue| panic!("invalid conversion benchmark input: {issue:?}");
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let properties = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    Workload {
        name,
        input_length: input.len(),
        properties,
        queries,
        expected,
    }
}

fn visit(case: &Workload, mut accept: impl FnMut(u64)) {
    match case.name {
        "Numeric" => {
            for property in black_box(&case.properties) {
                accept(black_box(property).to_i64().unwrap() as u64);
                accept(black_box(property).to_f64().unwrap().to_bits());
            }
        }
        "Text" | "BinaryText" => {
            for property in black_box(&case.properties) {
                accept(byte_sum(black_box(property).to_text().unwrap().as_bytes()));
            }
        }
        "Bytes" => {
            for property in black_box(&case.properties) {
                accept(byte_sum(black_box(property).as_bytes().unwrap()));
            }
        }
        "Lookup" => {
            for &tag in black_box(&case.queries) {
                accept(match black_box(&case.properties).get_i64(tag).unwrap() {
                    Some(value) => value as u64,
                    None => 0,
                });
            }
        }
        _ => unreachable!(),
    }
}

pub fn read(case: &Workload) -> Summary {
    let mut summary = Summary::default();
    visit(case, |value| {
        summary.values += 1;
        summary.checksum = summary.checksum.wrapping_add(black_box(value));
    });
    summary
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(self.properties.len(), 256);
        let mut expected = self.expected.iter();
        visit(self, |value| {
            assert_eq!(Some(&value), expected.next(), "{}", self.name)
        });
        assert_eq!(expected.next(), None, "missing conversion");
        assert_eq!(
            read(self),
            Summary {
                values: self.expected.len(),
                checksum: self
                    .expected
                    .iter()
                    .fold(0u64, |sum, value| sum.wrapping_add(*value)),
            }
        );
    }
}

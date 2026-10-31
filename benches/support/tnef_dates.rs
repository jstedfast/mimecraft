// Original date-reading workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefAttributeTag as Attribute, TnefDateTime, TnefDateTimeKind as Kind, TnefPropertyTag,
    TnefPropertyType as Type, TnefReader,
};
use std::{
    hint::black_box,
    io::{self, Read},
};

pub const CASES: [&str; 5] = ["Legacy", "AppTime", "SysTime", "MultiValue", "ShortReads"];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    expected: Vec<(u64, Kind)>,
    chunk: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub values: usize,
    pub ticks: u64,
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
}
impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.chunk);
        self.input.read(&mut output[..count])
    }
}

fn attribute(output: &mut Vec<u8>, tag: Attribute, value: &[u8]) {
    output.push(1);
    output.extend_from_slice(&tag.0.to_le_bytes());
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
    let checksum = value
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    output.extend_from_slice(&checksum.to_le_bytes());
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    let mut expected = Vec::new();
    if name == "Legacy" {
        for i in 0..512u16 {
            let second = i % 60;
            let bytes: Vec<_> = [2024, 3, 17, 9, 45, second, 0]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect();
            attribute(&mut input, Attribute::DATE_SENT, &bytes);
            expected.push((
                638462655000000000 + u64::from(second) * 10_000_000,
                Kind::Unspecified,
            ));
        }
    } else {
        let multi = matches!(name, "MultiValue" | "ShortReads");
        let count = if multi { 256u32 } else { 1024 };
        let mut bytes = count.to_le_bytes().to_vec();
        for property in 0..count {
            let app_time = name == "AppTime" || (multi && property % 2 == 0);
            let kind = if app_time {
                Type::APP_TIME
            } else {
                Type::SYS_TIME
            };
            let kind = if multi { Type(kind.0 | 0x1000) } else { kind };
            bytes.extend_from_slice(
                &TnefPropertyTag::new(0x1000 + property as u16, kind)
                    .0
                    .to_le_bytes(),
            );
            if multi {
                bytes.extend_from_slice(&4u32.to_le_bytes());
            }
            for value in 0..if multi { 4 } else { 1 } {
                let i = property * if multi { 4 } else { 1 } + value;
                if app_time {
                    let days = if i % 2 == 0 {
                        i64::from(i)
                    } else {
                        -i64::from(i)
                    };
                    let raw = if days < 0 {
                        days as f64 - 0.25
                    } else {
                        days as f64 + 0.25
                    };
                    bytes.extend_from_slice(&raw.to_le_bytes());
                    // Quarter days are exact binary fractions. Expected ticks
                    // come from the source model, not the date converter.
                    expected.push((
                        (599264352000000000i64 + days * 864000000000 + 216000000000) as u64,
                        Kind::Unspecified,
                    ));
                } else {
                    let raw = 133551423120000000i64 + i64::from(i);
                    bytes.extend_from_slice(&raw.to_le_bytes());
                    expected.push((638462655120000000 + u64::from(i), Kind::Utc));
                }
            }
        }
        attribute(&mut input, Attribute::MAPI_PROPERTIES, &bytes);
    }
    Workload {
        name,
        input,
        expected,
        chunk: if name == "ShortReads" { 7 } else { usize::MAX },
    }
}

fn visit(case: &Workload, mut accept: impl FnMut(TnefDateTime)) {
    let mut reader = TnefReader::new(Source {
        input: black_box(&case.input),
        chunk: case.chunk,
    });
    let mut logger = |issue| panic!("invalid date benchmark input: {issue:?}");
    while reader.read_next_with_logger(&mut logger).unwrap() {
        if reader.attribute().unwrap().tag == Attribute::DATE_SENT {
            accept(reader.read_value_datetime_with_logger(&mut logger).unwrap());
        } else {
            let mut properties = reader.open_properties_with_logger(&mut logger).unwrap();
            while properties.read_next_property().unwrap() {
                while properties.has_value() {
                    accept(properties.read_value_datetime().unwrap());
                    if !properties.read_next_value().unwrap() {
                        break;
                    }
                }
            }
        }
    }
}

pub fn read(case: &Workload) -> Summary {
    let mut summary = Summary::default();
    visit(case, |date| {
        summary.values += 1;
        summary.ticks = summary.ticks.wrapping_add(black_box(date).ticks());
    });
    summary
}

impl Workload {
    pub fn check(&self) {
        let mut expected = self.expected.iter();
        visit(self, |date| {
            let &(ticks, kind) = expected.next().expect("unexpected date");
            assert_eq!((date.ticks(), date.kind()), (ticks, kind), "{}", self.name);
        });
        assert_eq!(expected.next(), None, "missing date");
        assert_eq!(
            read(self),
            Summary {
                values: self.expected.len(),
                ticks: self
                    .expected
                    .iter()
                    .fold(0u64, |sum, &(ticks, _)| sum.wrapping_add(ticks)),
            }
        );
    }
}

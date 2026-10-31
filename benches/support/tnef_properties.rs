// Original MAPI property workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefAttributeTag as Attribute, TnefName, TnefNameId, TnefPropertyReader, TnefPropertySet,
    TnefPropertyTag as Tag, TnefPropertyType as Type, TnefReader, TnefValue as Value,
};
use std::{
    hint::black_box,
    io::{self, Read},
};

pub const CASES: [&str; 6] = [
    "Fixed",
    "MultiValue",
    "Named",
    "Recipients",
    "LargeBinary",
    "ShortReads",
];

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Skip,
    Stream,
    Decode,
    Property,
    Set,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::Stream => "Stream",
            Self::Decode => "Decode",
            Self::Property => "Property",
            Self::Set => "Set",
        }
    }
}

pub const MODES: [Mode; 5] = [
    Mode::Skip,
    Mode::Stream,
    Mode::Decode,
    Mode::Property,
    Mode::Set,
];

struct ExpectedProperty {
    tag: Tag,
    name: Option<TnefNameId>,
    values: Vec<Value>,
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    rows: Vec<Vec<ExpectedProperty>>,
    table: bool,
    chunk: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub rows: usize,
    pub properties: usize,
    pub values: usize,
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
}

impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.chunk);
        self.input.read(&mut output[..length])
    }
}

fn bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::I32(value) => value.to_le_bytes().to_vec(),
        Value::Text(value) => value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect(),
        Value::Binary(value) => value.clone(),
        _ => unreachable!("only generated benchmark types"),
    }
}

fn dword(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u32).to_le_bytes());
}

fn variable(output: &mut Vec<u8>, value: &[u8]) {
    dword(output, value.len());
    output.extend_from_slice(value);
    output.extend(std::iter::repeat_n(0, (4 - (value.len() & 3)) & 3));
}

fn property(index: usize, kind: Type, values: Vec<Value>) -> ExpectedProperty {
    ExpectedProperty {
        tag: Tag::new(0x1000 + index as u16, kind),
        name: None,
        values,
    }
}

pub fn workload(name: &'static str) -> Workload {
    let table = matches!(name, "Recipients" | "ShortReads");
    let rows = match name {
        "Fixed" => vec![
            (0..1024)
                .map(|i| property(i, Type::LONG, vec![Value::I32(i as i32)]))
                .collect(),
        ],
        "MultiValue" => vec![
            (0..256)
                .map(|i| {
                    property(
                        i,
                        Type(Type::LONG.0 | 0x1000),
                        (0..4).map(|j| Value::I32((i * 4 + j) as i32)).collect(),
                    )
                })
                .collect(),
        ],
        "Named" => vec![
            (0..128)
                .map(|i| ExpectedProperty {
                    tag: Tag::new(0x8000 + i as u16, Type::LONG),
                    name: Some(TnefNameId {
                        guid: [0x42; 16],
                        name: TnefName::String(format!("property-{i}")),
                    }),
                    values: vec![Value::I32(i)],
                })
                .collect(),
        ],
        "Recipients" | "ShortReads" => (0..256)
            .map(|i| {
                vec![
                    property(0, Type::LONG, vec![Value::I32(i)]),
                    property(
                        1,
                        Type::UNICODE,
                        vec![Value::Text(format!("Recipient {i}"))],
                    ),
                    property(
                        2,
                        Type::UNICODE,
                        vec![Value::Text(format!("recipient{i}@example.com"))],
                    ),
                ]
            })
            .collect(),
        "LargeBinary" => vec![vec![property(
            0,
            Type::BINARY,
            vec![Value::Binary(
                (0..1024 * 1024).map(|i| (i & 255) as u8).collect(),
            )],
        )]],
        _ => panic!("unknown TNEF property workload: {name}"),
    };
    let mut payload = Vec::new();
    if table {
        dword(&mut payload, rows.len());
    }
    for row in &rows {
        dword(&mut payload, row.len());
        for prop in row {
            payload.extend_from_slice(&prop.tag.0.to_le_bytes());
            if let Some(name) = &prop.name {
                payload.extend_from_slice(&name.guid);
                dword(&mut payload, 1);
                let TnefName::String(name) = &name.name else {
                    unreachable!()
                };
                variable(&mut payload, &bytes(&Value::Text(name.clone())));
            }
            let variable_type = prop.tag.value_type() != Type::LONG;
            if prop.tag.is_multi_valued() || variable_type {
                dword(&mut payload, prop.values.len());
            }
            for value in &prop.values {
                let data = bytes(value);
                if variable_type {
                    variable(&mut payload, &data);
                } else {
                    payload.extend_from_slice(&data);
                }
            }
        }
    }
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0, 1];
    let kind = if table {
        Attribute::RECIPIENT_TABLE
    } else {
        Attribute::MAPI_PROPERTIES
    };
    input.extend_from_slice(&kind.0.to_le_bytes());
    dword(&mut input, payload.len());
    input.extend_from_slice(&payload);
    let checksum = payload
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    input.extend_from_slice(&checksum.to_le_bytes());
    Workload {
        name,
        input,
        rows,
        table,
        chunk: if name == "ShortReads" { 7 } else { usize::MAX },
    }
}

fn read_properties<R: Read>(
    props: &mut TnefPropertyReader<'_, R>,
    mode: Mode,
    summary: &mut Summary,
) {
    let mut buffer = [0; 4096];
    while props.read_next_property().unwrap() {
        summary.properties += 1;
        summary.values += props.value_count();
        if matches!(mode, Mode::Property) {
            black_box(props.read_property().unwrap());
            continue;
        }
        if matches!(mode, Mode::Skip) || !props.has_value() {
            continue;
        }
        loop {
            if matches!(mode, Mode::Stream)
                && matches!(
                    props.property().unwrap().tag.value_type(),
                    Type::BINARY | Type::UNICODE
                )
            {
                let mut stream = props.open_value().unwrap();
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    black_box(&buffer[..count]);
                }
            } else {
                black_box(props.read_value().unwrap());
            }
            if !props.read_next_value().unwrap() {
                break;
            }
        }
    }
}

fn summarize_set(set: &TnefPropertySet, summary: &mut Summary) {
    summary.properties += set.len();
    summary.values += set.iter().map(|p| p.value_count()).sum::<usize>();
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    let mut reader = TnefReader::new(Source {
        input: black_box(&case.input),
        chunk: case.chunk,
    });
    let mut summary = Summary::default();
    while reader.read_next().unwrap() {
        let mut props = reader.open_properties().unwrap();
        if matches!(mode, Mode::Set) {
            if props.is_table() {
                let rows = props.read_rows_as_property_sets().unwrap();
                summary.rows += rows.len();
                for row in &rows {
                    summarize_set(row, &mut summary);
                }
                black_box(rows);
            } else {
                let set = props.read_property_set().unwrap();
                summarize_set(&set, &mut summary);
                black_box(set);
            }
            continue;
        }
        if props.is_table() {
            while props.read_next_row().unwrap() {
                summary.rows += 1;
                read_properties(&mut props, mode, &mut summary);
            }
        } else {
            read_properties(&mut props, mode, &mut summary);
        }
    }
    summary
}

impl Workload {
    pub fn check(&self) {
        let expected = Summary {
            rows: if self.table { self.rows.len() } else { 0 },
            properties: self.rows.iter().map(Vec::len).sum(),
            values: self
                .rows
                .iter()
                .flatten()
                .map(|prop| prop.values.len())
                .sum(),
        };
        // Expected metadata and values come from the generator's source model,
        // not another pass through the parser. Validate streams byte-for-byte,
        // decoded values, and checksums outside timing and allocation tracking.
        for stream in [false, true] {
            let mut reader = TnefReader::new(Source {
                input: &self.input,
                chunk: self.chunk,
            });
            let mut logger = |issue| panic!("invalid benchmark input: {issue:?}");
            assert!(reader.read_next_with_logger(&mut logger).unwrap());
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            for row in &self.rows {
                if self.table {
                    assert!(props.read_next_row().unwrap());
                }
                for prop in row {
                    assert!(props.read_next_property().unwrap());
                    assert_eq!(props.property().unwrap().tag, prop.tag);
                    assert_eq!(props.property().unwrap().name, prop.name);
                    assert_eq!(props.value_count(), prop.values.len());
                    for (i, expected) in prop.values.iter().enumerate() {
                        if i != 0 {
                            assert!(props.read_next_value().unwrap());
                        }
                        if stream && prop.tag.value_type() != Type::LONG {
                            let mut value = Vec::new();
                            props.open_value().unwrap().read_to_end(&mut value).unwrap();
                            assert_eq!(value, bytes(expected));
                        } else {
                            assert_eq!(props.read_value().unwrap(), *expected);
                        }
                    }
                    assert!(!props.read_next_value().unwrap());
                }
                assert!(!props.read_next_property().unwrap());
            }
            assert!(!props.read_next_row().unwrap());
            drop(props);
            assert!(!reader.read_next_with_logger(&mut logger).unwrap());
            assert!(!reader.is_truncated());
        }
        let mut reader = TnefReader::new(Source {
            input: &self.input,
            chunk: self.chunk,
        });
        let mut logger = |issue| panic!("invalid benchmark collection: {issue:?}");
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        let sets = if self.table {
            props.read_rows_as_property_sets().unwrap()
        } else {
            vec![props.read_property_set().unwrap()]
        };
        assert_eq!(sets.len(), self.rows.len());
        for (set, expected_row) in sets.iter().zip(&self.rows) {
            assert_eq!(set.len(), expected_row.len());
            for (property, expected_property) in set.iter().zip(expected_row) {
                assert_eq!(property.tag(), expected_property.tag);
                assert_eq!(property.name(), expected_property.name.as_ref());
                assert_eq!(property.values(), expected_property.values);
            }
        }
        drop(props);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert!(!reader.is_truncated());
        for mode in MODES {
            assert_eq!(read(self, mode), expected, "{}/{mode:?}", self.name);
        }
    }
}

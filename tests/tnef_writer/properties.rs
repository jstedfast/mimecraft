// Adapted from MimeKit's TnefWriterTests property and recipient-table cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{Tag as Attribute, TnefWriter, attribute, header};
use mimecraft::tnef::{
    TnefName, TnefNameId, TnefProperty, TnefPropertyTag as Tag, TnefPropertyType as Type,
    TnefReader, TnefValue as Value,
};
use std::io::{self, Cursor, Seek};

fn multi(kind: Type) -> Type {
    Type(kind.0 | Type::MULTI_VALUED.0)
}
fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
pub(super) fn variable(value: &[u8]) -> Vec<u8> {
    let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(value);
    bytes.resize(bytes.len() + (4 - value.len() % 4) % 4, 0);
    bytes
}
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}
fn named(name: TnefName) -> TnefNameId {
    TnefNameId {
        guid: [0x42; 16],
        name,
    }
}
fn document(tag: Attribute, data: &[u8]) -> Vec<u8> {
    let mut bytes = header(1252, 0);
    attribute(&mut bytes, 1, tag, data);
    bytes
}

pub(super) fn read_sets(bytes: &[u8], selected: Attribute) -> Vec<Vec<TnefProperty>> {
    let mut reader = TnefReader::new(bytes);
    let mut sets = Vec::new();
    let mut logger = |issue| panic!("nonconformant property output: {issue:?}");
    while reader.read_next_with_logger(&mut logger).unwrap() {
        if reader.attribute().unwrap().tag != selected {
            continue;
        }
        let mut properties = reader.open_properties_with_logger(&mut logger).unwrap();
        loop {
            if properties.is_table() && !properties.read_next_row().unwrap() {
                break;
            }
            let mut values = Vec::new();
            while properties.read_next_property().unwrap() {
                values.push(properties.read_property().unwrap());
            }
            sets.push(values);
            if !properties.is_table() {
                break;
            }
        }
    }
    sets
}

struct Case {
    kind: Type,
    values: Vec<Value>,
    wire: Vec<Vec<u8>>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            kind: Type::I2,
            values: vec![Value::I16(-2), Value::I16(i16::MIN), Value::I16(i16::MAX)],
            wire: vec![
                vec![254, 255, 0, 0],
                vec![0, 128, 0, 0],
                vec![255, 127, 0, 0],
            ],
        },
        Case {
            kind: Type::LONG,
            values: vec![Value::I32(0x12345678), Value::I32(-1)],
            wire: vec![vec![0x78, 0x56, 0x34, 0x12], vec![255; 4]],
        },
        Case {
            kind: Type::R4,
            values: vec![Value::F32(1.5), Value::F32(-0.25)],
            wire: vec![
                1.5f32.to_le_bytes().to_vec(),
                (-0.25f32).to_le_bytes().to_vec(),
            ],
        },
        Case {
            kind: Type::DOUBLE,
            values: vec![Value::F64(std::f64::consts::PI), Value::F64(1e300)],
            wire: vec![
                std::f64::consts::PI.to_le_bytes().to_vec(),
                1e300f64.to_le_bytes().to_vec(),
            ],
        },
        Case {
            kind: Type::CURRENCY,
            values: vec![
                Value::Currency(123456789),
                Value::Currency(i64::MAX),
                Value::Currency(i64::MIN),
            ],
            wire: vec![
                123456789i64.to_le_bytes().to_vec(),
                i64::MAX.to_le_bytes().to_vec(),
                i64::MIN.to_le_bytes().to_vec(),
            ],
        },
        Case {
            kind: Type::APP_TIME,
            values: vec![Value::AppTime(43832.0), Value::AppTime(-1.25)],
            wire: vec![
                43832f64.to_le_bytes().to_vec(),
                (-1.25f64).to_le_bytes().to_vec(),
            ],
        },
        Case {
            kind: Type::ERROR,
            values: vec![Value::Error(0x8004010f)],
            wire: vec![vec![15, 1, 4, 128]],
        },
        Case {
            kind: Type::BOOLEAN,
            values: vec![Value::Bool(true)],
            wire: vec![vec![1, 0, 0, 0]],
        },
        Case {
            kind: Type::I8,
            values: vec![Value::I64(i64::MIN), Value::I64(i64::MAX)],
            wire: vec![
                i64::MIN.to_le_bytes().to_vec(),
                i64::MAX.to_le_bytes().to_vec(),
            ],
        },
        Case {
            kind: Type::STRING8,
            values: vec![
                Value::Text("ascii text".into()),
                Value::Text("\u{e9}".into()),
                Value::Text("".into()),
            ],
            wire: vec![
                variable(b"ascii text\0"),
                variable(&[0xe9, 0]),
                variable(&[0]),
            ],
        },
        Case {
            kind: Type::UNICODE,
            values: vec![
                Value::Text("unicode \u{2603} \u{1f600}".into()),
                Value::Text("".into()),
                Value::Text("x".into()),
            ],
            wire: vec![
                variable(&utf16("unicode \u{2603} \u{1f600}")),
                variable(&[0, 0]),
                variable(&utf16("x")),
            ],
        },
        Case {
            kind: Type::SYS_TIME,
            values: vec![Value::SysTime(132675269500000000), Value::SysTime(0)],
            wire: vec![132675269500000000i64.to_le_bytes().to_vec(), vec![0; 8]],
        },
        Case {
            kind: Type::CLASS_ID,
            values: vec![Value::Guid([0x42; 16]), Value::Guid([0; 16])],
            wire: vec![vec![0x42; 16], vec![0; 16]],
        },
        Case {
            kind: Type::BINARY,
            values: vec![
                Value::Binary(vec![1, 2, 3]),
                Value::Binary(vec![]),
                Value::Binary(vec![1, 2, 3, 4, 5]),
            ],
            wire: vec![
                variable(&[1, 2, 3]),
                variable(&[]),
                variable(&[1, 2, 3, 4, 5]),
            ],
        },
        Case {
            kind: Type::OBJECT,
            values: vec![Value::Object((1..=17).collect())],
            wire: vec![variable(&(1..=17).collect::<Vec<_>>())],
        },
    ]
}

#[test]
fn all_native_types_and_multivalues_match_independent_wire_and_copy_snapshots() {
    let cases = cases();
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    properties.write_tag(Tag::new(0x6000, Type::NULL)).unwrap();
    let mut expected = Vec::new();
    word(&mut expected, 28);
    word(&mut expected, Tag::new(0x6000, Type::NULL).0);
    let mut native = vec![vec![Value::Null]];
    for (i, case) in cases.iter().enumerate() {
        let tag = Tag::new(0x6001 + i as u16, case.kind);
        properties.write_tag(tag).unwrap();
        properties.write_value(&case.values[0]).unwrap();
        word(&mut expected, tag.0);
        if matches!(
            case.kind,
            Type::STRING8 | Type::UNICODE | Type::BINARY | Type::OBJECT
        ) {
            word(&mut expected, 1);
        }
        expected.extend_from_slice(&case.wire[0]);
        native.push(vec![case.values[0].clone()]);
        if matches!(case.kind, Type::ERROR | Type::BOOLEAN | Type::OBJECT) {
            continue;
        }
        let tag = Tag::new(0x6040 + i as u16, multi(case.kind));
        properties.write_tag(tag).unwrap();
        word(&mut expected, tag.0);
        word(&mut expected, case.values.len() as u32);
        for (value, bytes) in case.values.iter().zip(&case.wire) {
            properties.write_value(value).unwrap();
            expected.extend_from_slice(bytes);
        }
        native.push(case.values.clone());
    }
    assert_eq!(properties.property_count(), 28);
    properties.finish().unwrap();
    writer.finish().unwrap();
    let bytes = writer.into_inner();
    assert_eq!(bytes, document(Attribute::MAPI_PROPERTIES, &expected));
    let sets = read_sets(&bytes, Attribute::MAPI_PROPERTIES);
    for (property, expected) in sets[0].iter().zip(native) {
        assert_eq!(property.values(), expected);
    }
    let mut copy = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = copy.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    for property in &sets[0] {
        properties.write_property(property).unwrap();
    }
    properties.finish().unwrap();
    copy.finish().unwrap();
    assert_eq!(copy.into_inner(), bytes);
}

#[test]
fn named_descriptors_reuse_ids_across_rows_and_attributes() {
    let names = [
        named(TnefName::Id(0x8501)),
        named(TnefName::String("Custom Name".into())),
        named(TnefName::String("".into())),
        named(TnefName::String("\u{1f600}".into())),
        named(TnefName::String("custom name".into())),
    ];
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    for attribute in [Attribute::RECIPIENT_TABLE, Attribute::MAPI_PROPERTIES] {
        let mut properties = writer.open_properties(attribute).unwrap();
        if properties.is_table() {
            properties.begin_row().unwrap();
        }
        for name in &names {
            properties.write_named_tag(name, Type::LONG).unwrap();
            properties.write_value(&Value::I32(7)).unwrap();
        }
        properties
            .write_named_tag(&names[0], Type::UNICODE)
            .unwrap();
        properties
            .write_value(&Value::Text("value".into()))
            .unwrap();
        properties.finish().unwrap();
    }
    writer
        .write_attribute(Attribute::ATTACH_RENDER_DATA, &[0; 14])
        .unwrap();
    let mut properties = writer.open_properties(Attribute::ATTACHMENT).unwrap();
    properties.write_named_tag(&names[1], Type::LONG).unwrap();
    properties.write_value(&Value::I32(1)).unwrap();
    properties.finish().unwrap();
    writer.finish().unwrap();
    for attribute in [Attribute::RECIPIENT_TABLE, Attribute::MAPI_PROPERTIES] {
        let sets = read_sets(writer.get_ref(), attribute);
        assert_eq!(
            sets[0].iter().map(|p| p.tag().id()).collect::<Vec<_>>(),
            [0x8000, 0x8001, 0x8002, 0x8003, 0x8004, 0x8000]
        );
        for (property, name) in sets[0].iter().zip(names.iter().chain([&names[0]])) {
            assert_eq!(property.name(), Some(name));
        }
    }
    assert_eq!(
        read_sets(writer.get_ref(), Attribute::ATTACHMENT)[0][0]
            .tag()
            .id(),
        0x8001
    );
}

#[test]
fn named_snapshot_copy_remaps_ids_and_reencodes_string8() {
    let mut source = TnefWriter::new(Vec::new(), 65001, 0).unwrap();
    let name = named(TnefName::String("Keywords".into()));
    let mut properties = source.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    properties
        .write_named_tag(&name, multi(Type::STRING8))
        .unwrap();
    properties
        .write_value(&Value::Text("\u{e9}".into()))
        .unwrap();
    properties.write_value(&Value::Text("two".into())).unwrap();
    properties.finish().unwrap();
    source.finish().unwrap();
    let sets = read_sets(source.get_ref(), Attribute::MAPI_PROPERTIES);
    let mut target = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = target.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    properties
        .write_named_tag(&named(TnefName::Id(1)), Type::NULL)
        .unwrap();
    properties.write_property(&sets[0][0]).unwrap();
    let scratch = properties.finish().unwrap().to_vec().unwrap();
    assert!(
        scratch
            .windows(8)
            .any(|bytes| bytes == [2, 0, 0, 0, 0xe9, 0, 0, 0])
    );
    target.finish().unwrap();
    let copied = read_sets(target.get_ref(), Attribute::MAPI_PROPERTIES);
    assert_eq!(copied[0][1].tag().id(), 0x8001);
    assert_eq!(copied[0][1].name(), Some(&name));
    assert_eq!(copied[0][1].values(), sets[0][0].values());
}

#[test]
fn recipient_rows_include_empty_rows_and_require_begin_row() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(Attribute::RECIPIENT_TABLE).unwrap();
    assert!(properties.is_table());
    assert!(properties.write_tag(Tag::DISPLAY_NAME_W).is_err());
    for (name, recipient_type) in [("Alice", 1), ("Bob", 2)] {
        properties.begin_row().unwrap();
        assert_eq!(properties.property_count(), 0);
        properties.write_tag(Tag::DISPLAY_NAME_W).unwrap();
        properties.write_value(&Value::Text(name.into())).unwrap();
        properties.write_tag(Tag::RECIPIENT_TYPE).unwrap();
        properties.write_value(&Value::I32(recipient_type)).unwrap();
        assert_eq!(properties.property_count(), 2);
    }
    properties.begin_row().unwrap();
    assert_eq!(properties.row_count(), 3);
    properties.finish().unwrap();
    writer.finish().unwrap();
    let rows = read_sets(writer.get_ref(), Attribute::RECIPIENT_TABLE);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0].value(), Some(&Value::Text("Alice".into())));
    assert_eq!(rows[1][1].value(), Some(&Value::I32(2)));
    assert!(rows[2].is_empty());
}

#[test]
fn empty_lists_tables_and_multivalues_have_exact_counts() {
    for attribute in [Attribute::MAPI_PROPERTIES, Attribute::RECIPIENT_TABLE] {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        writer.open_properties(attribute).unwrap().finish().unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.into_inner(), document(attribute, &[0; 4]));
    }
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    for kind in [Type::LONG, Type::BINARY, Type::UNICODE] {
        properties.write_tag(Tag::new(0x6000, multi(kind))).unwrap();
    }
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), Attribute::MAPI_PROPERTIES);
    assert_eq!(sets[0].len(), 3);
    assert!(sets[0].iter().all(|p| p.values().is_empty()));
}

#[test]
fn invalid_tags_values_and_incomplete_transitions_are_recoverable() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    assert!(properties.begin_row().is_err());
    assert!(properties.write_value(&Value::I32(1)).is_err());
    for kind in [
        Type::UNSPECIFIED,
        Type(0x99),
        multi(Type::NULL),
        multi(Type::BOOLEAN),
        multi(Type::ERROR),
        multi(Type::OBJECT),
    ] {
        assert!(properties.write_tag(Tag::new(0x6000, kind)).is_err());
    }
    assert!(properties.write_tag(Tag::new(0x8000, Type::LONG)).is_err());
    assert!(
        properties
            .write_named_tag(&named(TnefName::Unknown(2)), Type::LONG)
            .is_err()
    );
    assert_eq!(properties.get_ref().to_vec().unwrap(), [0; 4]);
    properties.write_tag(Tag::SUBJECT_A).unwrap();
    let before = properties.get_ref().to_vec().unwrap();
    assert!(properties.write_value(&Value::I32(1)).is_err());
    assert!(
        properties
            .write_value(&Value::Text("\u{1f600}".into()))
            .is_err()
    );
    assert!(properties.write_tag(Tag::IMPORTANCE).is_err());
    assert_eq!(properties.get_ref().to_vec().unwrap(), before);
    properties
        .write_value(&Value::Text("subject".into()))
        .unwrap();
    assert!(
        properties
            .write_value(&Value::Text("again".into()))
            .is_err()
    );
    properties.write_tag(Tag::ATTACH_DATA_OBJ).unwrap();
    assert!(properties.write_value(&Value::Object(vec![0; 15])).is_err());
    properties.write_value(&Value::Object(vec![0; 16])).unwrap();
    properties.write_tag(Tag::new(0x6000, Type::NULL)).unwrap();
    assert!(properties.write_value(&Value::Null).is_err());
    properties.finish().unwrap();
    writer.finish().unwrap();
    assert_eq!(
        read_sets(writer.get_ref(), Attribute::MAPI_PROPERTIES)[0].len(),
        3
    );
}

#[test]
fn invalid_raw_dates_are_rejected_and_float_bits_are_preserved() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut properties = writer.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
    properties
        .write_tag(Tag::new(0x6000, Type::SYS_TIME))
        .unwrap();
    for value in [-1, i64::MAX] {
        assert!(properties.write_value(&Value::SysTime(value)).is_err());
    }
    properties.write_value(&Value::SysTime(0)).unwrap();
    properties
        .write_tag(Tag::new(0x6001, Type::APP_TIME))
        .unwrap();
    for value in [f64::NAN, f64::INFINITY, -657435.0, 2958466.0] {
        assert!(properties.write_value(&Value::AppTime(value)).is_err());
    }
    properties.write_value(&Value::AppTime(-1.25)).unwrap();
    properties
        .write_tag(Tag::new(0x6002, multi(Type::DOUBLE)))
        .unwrap();
    for bits in [0x7ff8000000001234, 0x8000000000000000, 0x7ff0000000000000] {
        properties
            .write_value(&Value::F64(f64::from_bits(bits)))
            .unwrap();
    }
    properties.finish().unwrap();
    writer.finish().unwrap();
    let sets = read_sets(writer.get_ref(), Attribute::MAPI_PROPERTIES);
    let bits: Vec<_> = sets[0][2]
        .values()
        .iter()
        .map(|value| match value {
            Value::F64(x) => x.to_bits(),
            _ => panic!("not a double"),
        })
        .collect();
    assert_eq!(
        bits,
        [0x7ff8000000001234, 0x8000000000000000, 0x7ff0000000000000]
    );
}

#[test]
fn variable_padding_and_terminators_cover_all_remainders() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut buffer = Cursor::new(Vec::new());
    let mut properties = writer
        .open_properties_with_buffer(Attribute::MAPI_PROPERTIES, &mut buffer)
        .unwrap();
    let mut expected = Vec::new();
    word(&mut expected, 3);
    for (kind, id) in [
        (Type::BINARY, 0x6000),
        (Type::STRING8, 0x6001),
        (Type::UNICODE, 0x6002),
    ] {
        let tag = Tag::new(id, multi(kind));
        properties.write_tag(tag).unwrap();
        word(&mut expected, tag.0);
        word(&mut expected, 9);
        for length in 0..9 {
            let text = "a".repeat(length);
            let value = if kind == Type::BINARY {
                Value::Binary(vec![1; length])
            } else {
                Value::Text(text.clone())
            };
            properties.write_value(&value).unwrap();
            let payload = match kind {
                Type::BINARY => vec![1; length],
                Type::STRING8 => {
                    let mut v = text.into_bytes();
                    v.push(0);
                    v
                }
                _ => utf16(&text),
            };
            expected.extend(variable(&payload));
        }
    }
    properties.finish().unwrap();
    assert_eq!(buffer.get_ref(), &expected);
    assert_eq!(buffer.stream_position().unwrap(), expected.len() as u64);
    writer.finish().unwrap();
    assert_eq!(
        writer.into_inner(),
        document(Attribute::MAPI_PROPERTIES, &expected)
    );
}

#[test]
fn incomplete_or_abandoned_property_writer_faults_parent_without_output() {
    for mode in 0..3 {
        let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
        writer.write_string(Attribute::SUBJECT, "x").unwrap();
        let before = writer.get_ref().clone();
        let mut properties = writer.open_properties(Attribute::MAPI_PROPERTIES).unwrap();
        properties.write_tag(Tag::SUBJECT_W).unwrap();
        match mode {
            0 => {
                assert_eq!(
                    properties.finish().unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
            1 => drop(properties),
            _ => drop(properties.into_inner()),
        }
        assert!(writer.is_faulted());
        assert_eq!(
            writer.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.get_ref(), &before);
    }
}

#[test]
fn forgetting_borrowed_property_writer_cannot_bypass_finalization() {
    let mut writer = TnefWriter::new(Vec::new(), 1252, 0).unwrap();
    let mut buffer = Cursor::new(Vec::new());
    let properties = writer
        .open_properties_with_buffer(Attribute::MAPI_PROPERTIES, &mut buffer)
        .unwrap();
    #[expect(
        clippy::forget_non_drop,
        reason = "Exercise forgotten borrows without leaking owned scratch storage."
    )]
    std::mem::forget(properties);
    assert!(writer.is_faulted());
    assert_eq!(
        writer.finish().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(writer.get_ref().is_empty());
    assert_eq!(buffer.into_inner(), [0; 4]);
}

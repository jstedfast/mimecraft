// Adapted from MimeKit's TnefPropertyReaderTests, TnefPropertyReaderRobustnessTests,
// TnefPropertyValueCountTests, TnefPropertyReaderTextValueTests, TnefCodepageTests,
// TnefPropertyTagTests, and TnefNameIdTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read};

use mimecraft::tnef::{
    TnefAttributeTag as Attribute, TnefComplianceIssue as Issue,
    TnefComplianceViolation as Violation, TnefName, TnefNameId, TnefPropertyTag as Tag,
    TnefPropertyType as Type, TnefReadError as Error, TnefReader, TnefReaderOptions,
    TnefValue as Value,
};

#[path = "tnef_properties/dates.rs"]
mod dates;
#[path = "tnef_properties/embedded.rs"]
mod embedded;
#[path = "tnef_properties/sets.rs"]
mod sets;

fn dword(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn tag(bytes: &mut Vec<u8>, id: u16, kind: Type) {
    bytes.extend_from_slice(&Tag::new(id, kind).0.to_le_bytes());
}

fn variable(bytes: &mut Vec<u8>, value: &[u8]) {
    dword(bytes, value.len() as i32);
    bytes.extend_from_slice(value);
    bytes.extend(std::iter::repeat_n(0, (4 - (value.len() & 3)) & 3));
}

fn unicode(value: &str) -> Vec<u8> {
    value.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn attribute(bytes: &mut Vec<u8>, kind: Attribute, value: &[u8]) {
    bytes.push(if kind == Attribute::ATTACHMENT { 2 } else { 1 });
    bytes.extend_from_slice(&kind.0.to_le_bytes());
    dword(bytes, value.len() as i32);
    bytes.extend_from_slice(value);
    bytes.extend_from_slice(
        &value
            .iter()
            .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)))
            .to_le_bytes(),
    );
}

fn document(kind: Attribute, properties: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    attribute(&mut bytes, kind, properties);
    bytes
}

fn properties(items: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    dword(&mut bytes, items.len() as i32);
    for item in items {
        bytes.extend_from_slice(item);
    }
    bytes
}

fn fixed(id: u16, kind: Type, value: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    tag(&mut bytes, id, kind);
    bytes.extend_from_slice(value);
    bytes
}

fn text(kind: Type, value: &[u8]) -> Vec<u8> {
    let mut bytes = fixed(0x0037, kind, &[]);
    dword(&mut bytes, 1);
    variable(&mut bytes, value);
    bytes
}

struct Chunked<'a> {
    input: &'a [u8],
    chunk: usize,
}

impl Read for Chunked<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.chunk);
        self.input.read(&mut output[..length])
    }
}

#[test]
fn tag_bits_and_named_identifier_boundaries() {
    for id in [0, 0x7fff, 0x8000, 0xfffe, 0xffff] {
        let value = Tag::new(id, Type(Type::UNICODE.0 | Type::MULTI_VALUED.0));
        assert_eq!(value.id(), id);
        assert_eq!(value.value_type(), Type::UNICODE);
        assert!(value.is_multi_valued());
        assert_eq!(value.is_named(), (0x8000..=0xfffe).contains(&id));
    }
    assert_eq!(Tag(0x1234ffff).value_type(), Type(0xefff));
}

#[test]
fn native_fixed_values_are_repeatable_and_ignore_padding() {
    let cases = [
        (Type::NULL, vec![], Value::Null),
        (Type::I2, vec![0xff, 0xff, 0xab, 0xcd], Value::I16(-1)),
        (
            Type::LONG,
            (-1234i32).to_le_bytes().to_vec(),
            Value::I32(-1234),
        ),
        (Type::R4, 1.25f32.to_le_bytes().to_vec(), Value::F32(1.25)),
        (
            Type::DOUBLE,
            (-2.5f64).to_le_bytes().to_vec(),
            Value::F64(-2.5),
        ),
        (
            Type::CURRENCY,
            (-98765i64).to_le_bytes().to_vec(),
            Value::Currency(-98765),
        ),
        (
            Type::APP_TIME,
            123.5f64.to_le_bytes().to_vec(),
            Value::AppTime(123.5),
        ),
        (
            Type::ERROR,
            u32::MAX.to_le_bytes().to_vec(),
            Value::Error(u32::MAX),
        ),
        (Type::BOOLEAN, vec![0, 0, 0xff, 0xff], Value::Bool(false)),
        (Type::BOOLEAN, vec![0, 1, 0, 0], Value::Bool(true)),
        (
            Type::I8,
            i64::MIN.to_le_bytes().to_vec(),
            Value::I64(i64::MIN),
        ),
        (
            Type::SYS_TIME,
            i64::MAX.to_le_bytes().to_vec(),
            Value::SysTime(i64::MAX),
        ),
        (
            Type::CLASS_ID,
            (0..16).collect(),
            Value::Guid(std::array::from_fn(|i| i as u8)),
        ),
    ];
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(
            &cases
                .iter()
                .map(|(kind, bytes, _)| fixed(0x1234, *kind, bytes))
                .collect::<Vec<_>>(),
        ),
    );
    for chunk in [1, 3, 7, 4096] {
        let options = TnefReaderOptions {
            max_value_length: 0,
            max_total_data_bytes: 0,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(
            Chunked {
                input: &data,
                chunk,
            },
            &options,
        )
        .unwrap();
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(!props.read_next_row().unwrap());
        assert!(matches!(props.read_value(), Err(Error::NoPropertyValue)));
        for (kind, _, expected) in &cases {
            assert!(props.read_next_property().unwrap());
            assert_eq!(props.property_count(), cases.len());
            assert_eq!(props.property().unwrap().tag, Tag::new(0x1234, *kind));
            assert_eq!(props.value_count(), 1);
            assert!(props.has_value());
            assert_eq!(props.read_value().unwrap(), *expected);
            assert_eq!(props.read_value().unwrap(), *expected);
            let snapshot = props.read_property().unwrap();
            assert_eq!(snapshot.info(), props.property().unwrap());
            assert_eq!(snapshot.value_count(), 1);
            assert_eq!(snapshot.value(), Some(expected));
            assert_eq!(snapshot.property_type(), *kind);
            assert!(!snapshot.is_multi_valued());
            assert!(matches!(
                props.open_value(),
                Err(Error::InvalidPropertyType(_))
            ));
            assert!(matches!(
                props.read_value_bytes(),
                Err(Error::InvalidPropertyType(_))
            ));
            assert!(!props.read_next_value().unwrap());
            assert!(!props.has_value());
        }
        assert!(!props.read_next_property().unwrap());
        assert!(props.property().is_none());
        drop(props);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn strings_binary_objects_and_padding_preserve_following_values() {
    let cases = [
        (
            Type::STRING8,
            b"Caf\xe9 au lait\0\0".to_vec(),
            Value::Text("Caf\u{e9} au lait".into()),
        ),
        (
            Type::UNICODE,
            unicode("This is the subject\0\0"),
            Value::Text("This is the subject".into()),
        ),
        (
            Type::UNICODE,
            vec![0, 1, 0, 0, 0x99],
            Value::Text("\u{100}".into()),
        ),
        (
            Type::UNICODE,
            vec![0, 0xd8, b'x', 0],
            Value::Text("\u{fffd}x".into()),
        ),
        (Type::UNICODE, vec![0xff], Value::Text(String::new())),
        (Type::BINARY, vec![1, 2, 0], Value::Binary(vec![1, 2, 0])),
        (
            Type::OBJECT,
            (0..19).collect(),
            Value::Object((0..19).collect()),
        ),
        (Type::BINARY, vec![], Value::Binary(vec![])),
    ];
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(
            &cases
                .iter()
                .map(|(kind, bytes, _)| text(*kind, bytes))
                .collect::<Vec<_>>(),
        ),
    );
    for chunk in [1, 7, 4096] {
        let mut reader = TnefReader::new(Chunked {
            input: &data,
            chunk,
        });
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        for (_, _, expected) in &cases {
            assert!(props.read_next_property().unwrap());
            assert_eq!(props.read_value().unwrap(), *expected);
            assert!(props.is_value_consumed());
            assert!(matches!(props.read_value(), Err(Error::ValueAlreadyRead)));
        }
        assert!(!props.read_next_property().unwrap());
        drop(props);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn value_counts_recover_without_hiding_the_next_property() {
    for (kind, count, values, invalid) in [
        (Type::UNICODE, 0, vec![], true),
        (Type::UNICODE, -1, vec![], true),
        (Type(Type::LONG.0 | 0x1000), 0, vec![], false),
        (Type(Type::NULL.0 | 0x1000), i32::MAX, vec![], true),
        (
            Type(Type::LONG.0 | 0x1000),
            3,
            [1i32, 2, 3]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
            false,
        ),
        (
            Type::BINARY,
            2,
            [1i32, 0x61, 0]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
            true,
        ),
    ] {
        let mut first = fixed(0x1234, kind, &[]);
        dword(&mut first, count);
        first.extend(values);
        let data = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[first, fixed(0x0017, Type::LONG, &7i32.to_le_bytes())]),
        );
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        assert_eq!(
            props.value_count(),
            if kind.0 & !0x1000 == Type::NULL.0 {
                0
            } else {
                count.max(0) as usize
            }
        );
        if props.value_count() == 0 {
            assert!(matches!(props.read_value(), Err(Error::NoPropertyValue)));
        }
        // Advancing must consume all unread values, including non-one counts on
        // nominally single-valued properties, to preserve the next boundary.
        assert!(props.read_next_property().unwrap());
        assert_eq!(props.read_value().unwrap(), Value::I32(7));
        assert!(!props.read_next_property().unwrap());
        drop(props);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), usize::from(invalid), "{kind:?}");
        if invalid {
            assert_eq!(
                issues[0],
                Issue {
                    violation: Violation::InvalidValueCount,
                    depth: 0,
                    stream_offset: 23,
                    attribute_tag: Some(Attribute::MAPI_PROPERTIES),
                    property_tag: Some(Tag::new(0x1234, kind)),
                }
            );
        }
    }
}

#[test]
fn multivalue_streams_exclude_padding_and_drop_without_consuming() {
    let mut item = fixed(0x1234, Type(Type::BINARY.0 | 0x1000), &[]);
    dword(&mut item, 3);
    for bytes in [&b"abc"[..], &b""[..], &b"12345"[..]] {
        variable(&mut item, bytes);
    }
    let data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties().unwrap();
    assert!(props.read_next_property().unwrap());
    let start = props.position();
    let mut buffer = [0; 10];
    {
        let mut stream = props.open_value().unwrap();
        assert_eq!(stream.read(&mut []).unwrap(), 0);
        assert_eq!(stream.read(&mut buffer[..1]).unwrap(), 1);
        assert_eq!(buffer[0], b'a');
    }
    assert_eq!(props.position(), start + 1);
    assert!(matches!(props.open_value(), Err(Error::ValueAlreadyRead)));
    assert!(props.read_next_value().unwrap());
    assert_eq!(props.read_value_bytes().unwrap(), []);
    assert!(props.read_next_value().unwrap());
    {
        let mut stream = props.open_value().unwrap();
        assert_eq!(stream.read(&mut buffer).unwrap(), 5);
        assert_eq!(&buffer[..5], b"12345");
        assert_eq!(stream.read(&mut buffer).unwrap(), 0);
        assert_eq!(stream.read(&mut buffer).unwrap(), 0);
    }
    assert!(!props.read_next_value().unwrap());
    assert!(!props.read_next_value().unwrap());
    drop(props);
    assert!(!reader.read_next().unwrap());
}

fn named(kind: u32, name: &[u8]) -> Vec<u8> {
    let mut item = fixed(0x8001, Type::LONG, &[0x42; 16]);
    item.extend_from_slice(&kind.to_le_bytes());
    if kind == 0 {
        item.extend_from_slice(name);
    } else if kind == 1 {
        variable(&mut item, name);
    }
    dword(&mut item, 7);
    item
}

#[test]
fn named_properties_retain_guids_names_and_unknown_kinds() {
    let items = [
        named(0, &123i32.to_le_bytes()),
        named(1, &unicode("ab\0")),
        named(99, &[]),
    ];
    let expected = [
        TnefName::Id(123),
        TnefName::String("ab".into()),
        TnefName::Unknown(99),
    ];
    let data = document(Attribute::MAPI_PROPERTIES, &properties(&items));
    let mut reader = TnefReader::new(Chunked {
        input: &data,
        chunk: 1,
    });
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
    for name in expected {
        assert!(props.read_next_property().unwrap());
        assert_eq!(
            props.property().unwrap().name,
            Some(TnefNameId {
                guid: [0x42; 16],
                name
            })
        );
        assert_eq!(props.read_value().unwrap(), Value::I32(7));
    }
    assert!(!props.read_next_property().unwrap());
    drop(props);
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidNamedPropertyKind);
    assert_eq!(
        issues[0].stream_offset,
        19 + items[0].len() as u64 + items[1].len() as u64
    );
}

#[test]
fn recipient_rows_require_selection_and_skip_unread_properties() {
    let first = properties(&[
        text(Type::UNICODE, &unicode("first")),
        fixed(1, Type::LONG, &1i32.to_le_bytes()),
    ]);
    let last = properties(&[fixed(2, Type::LONG, &2i32.to_le_bytes())]);
    let mut table = 3i32.to_le_bytes().to_vec();
    table.extend(first);
    dword(&mut table, 0);
    table.extend(last);
    let data = document(Attribute::RECIPIENT_TABLE, &table);
    for select_first in [false, true] {
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.is_table());
        assert!(!props.read_next_property().unwrap());
        assert_eq!(props.row_count(), 0);
        assert!(props.read_next_row().unwrap());
        assert_eq!(props.row_count(), 3);
        assert_eq!(props.property_count(), 2);
        if select_first {
            assert!(props.read_next_property().unwrap());
        }
        assert!(props.read_next_row().unwrap());
        assert_eq!(props.property_count(), 0);
        assert!(!props.read_next_property().unwrap());
        assert!(props.read_next_row().unwrap());
        assert!(props.read_next_property().unwrap());
        assert_eq!(props.read_value().unwrap(), Value::I32(2));
        assert!(!props.read_next_row().unwrap());
        assert!(!props.read_next_row().unwrap());
        assert!(!props.read_next_property().unwrap());
        drop(props);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn invalid_lengths_and_types_stop_only_inner_navigation() {
    let mut items = Vec::new();
    for length in [i32::MAX, i32::MAX - 3, 0x40000000, 1048576, -1, i32::MIN] {
        let mut value = fixed(0x0037, Type::UNICODE, &1i32.to_le_bytes());
        dword(&mut value, length);
        items.push(value);
        let mut name = fixed(0x8001, Type::LONG, &[0; 16]);
        dword(&mut name, 1);
        dword(&mut name, length);
        items.push(name);
    }
    items.push(fixed(0x1234, Type::UNSPECIFIED, &[]));
    items.push(fixed(0x1234, Type::CLASS_ID, &[0; 12]));
    items.push(fixed(0x8001, Type::LONG, &[0; 16]));
    for item in items {
        let kind = Tag(u32::from_le_bytes(item[..4].try_into().unwrap())).value_type();
        let mut data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
        attribute(&mut data, Attribute::BODY, b"next\0");
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(!props.read_next_property().unwrap());
        assert!(props.is_stopped());
        assert!(props.property().is_none());
        assert!(!props.read_next_property().unwrap());
        assert!(!props.read_next_value().unwrap());
        drop(props);
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(reader.read_value_string().unwrap(), "next");
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].violation,
            if kind == Type::UNSPECIFIED {
                Violation::UnsupportedPropertyType
            } else {
                Violation::InvalidPropertyLength
            }
        );
        assert!(issues[0].property_tag.is_some());
    }
}

#[test]
fn oversized_counts_are_bounded_and_reported() {
    for kind in [Attribute::MAPI_PROPERTIES, Attribute::RECIPIENT_TABLE] {
        for count in [-1, i32::MAX] {
            let data = document(kind, &count.to_le_bytes());
            let mut reader = TnefReader::new(data.as_slice());
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            assert!(reader.read_next().unwrap());
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(
                !if props.is_table() {
                    props.read_next_row()
                } else {
                    props.read_next_property()
                }
                .unwrap()
            );
            drop(props);
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].property_tag, None);
            assert_eq!(issues[0].stream_offset, 15);
        }
    }
    let mut item = fixed(1, Type(Type::LONG.0 | 0x1000), &i32::MAX.to_le_bytes());
    dword(&mut item, 42);
    let data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
    let mut reader = TnefReader::new(data.as_slice());
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
    assert!(props.read_next_property().unwrap());
    assert_eq!(props.value_count(), 1);
    assert_eq!(props.read_value().unwrap(), Value::I32(42));
    assert!(!props.read_next_value().unwrap());
    drop(props);
    assert_eq!(issues[0].violation, Violation::InvalidValueCount);
}

#[test]
fn missing_padding_is_reported_without_crossing_the_attribute() {
    let mut item = fixed(1, Type::BINARY, &1i32.to_le_bytes());
    dword(&mut item, 1);
    item.push(42);
    let mut data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
    attribute(&mut data, Attribute::BODY, b"next");
    let mut reader = TnefReader::new(data.as_slice());
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
    assert!(props.read_next_property().unwrap());
    assert_eq!(props.read_value().unwrap(), Value::Binary(vec![42]));
    assert!(!props.read_next_property().unwrap());
    drop(props);
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(reader.read_value_string().unwrap(), "next");
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidPropertyLength);
    assert_eq!(issues[0].stream_offset, 32);
}

#[test]
fn shared_byte_limits_allow_value_streaming_but_stop_oversized_names() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            named(1, &unicode("ab")),
            text(Type::BINARY, b"1234"),
            text(Type::BINARY, b"12345"),
        ]),
    );
    for (limit, budget, name_fails) in [(3, 20, true), (5, 7, false), (4, 20, false)] {
        let options = TnefReaderOptions {
            max_value_length: limit,
            max_total_data_bytes: budget,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        if name_fails {
            assert!(matches!(
                props.read_next_property(),
                Err(Error::ValueLimitExceeded)
            ));
            assert!(props.is_stopped());
            assert!(!props.read_next_property().unwrap());
        } else {
            assert!(props.read_next_property().unwrap());
            assert!(props.read_next_property().unwrap());
            if budget == 7 {
                assert!(matches!(
                    props.read_value_bytes(),
                    Err(Error::ValueLimitExceeded)
                ));
                assert!(!props.is_value_consumed());
                let mut bytes = Vec::new();
                props.open_value().unwrap().read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"1234");
            } else {
                assert_eq!(props.read_value_bytes().unwrap(), b"1234");
                assert!(props.read_next_property().unwrap());
                let position = props.position();
                assert!(matches!(
                    props.read_value_bytes(),
                    Err(Error::ValueLimitExceeded)
                ));
                assert_eq!(props.position(), position);
                assert!(!props.is_value_consumed());
                let mut bytes = Vec::new();
                props.open_value().unwrap().read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, b"12345");
            }
        }
        drop(props);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::DataSizeLimitExceeded);
    }
}

#[test]
fn property_claims_and_faults_remain_exclusive() {
    let data = document(Attribute::BODY, b"body");
    let mut reader = TnefReader::new(data.as_slice());
    assert!(matches!(reader.open_properties(), Err(Error::NoAttribute)));
    assert!(reader.read_next().unwrap());
    assert!(matches!(
        reader.open_properties(),
        Err(Error::InvalidValueType(_))
    ));
    assert_eq!(reader.read_value_string().unwrap(), "body");

    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[text(Type::BINARY, b"abc")]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    drop(reader.open_properties().unwrap());
    assert!(matches!(
        reader.open_properties(),
        Err(Error::ValueAlreadyRead)
    ));
    assert!(matches!(
        reader.read_value_bytes(),
        Err(Error::ValueAlreadyRead)
    ));

    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[fixed(1, Type::UNSPECIFIED, &[])]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let mut logger = |_| Err(io::Error::other("logger stopped"));
    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
    assert!(matches!(props.read_next_property(), Err(Error::Io(_))));
    assert!(matches!(props.read_next_property(), Err(Error::Faulted)));
    assert!(matches!(props.read_value(), Err(Error::Faulted)));
    drop(props);
    assert!(matches!(reader.read_next(), Err(Error::Faulted)));
}

#[test]
fn every_physical_truncation_and_chunk_boundary_terminates() {
    let items = properties(&[
        named(1, &unicode("name\0")),
        fixed(1, Type::CLASS_ID, &[42; 16]),
        text(Type::BINARY, b"abcde"),
    ]);
    let data = document(Attribute::MAPI_PROPERTIES, &items);
    for length in 0..data.len() {
        for chunk in [1, 7, 4096] {
            for collect in [false, true] {
                let mut reader = TnefReader::new(Chunked {
                    input: &data[..length],
                    chunk,
                });
                let mut issues = Vec::new();
                let mut logger = |issue| {
                    issues.push(issue);
                    Ok(())
                };
                if reader.read_next_with_logger(&mut logger).unwrap() {
                    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
                    let mut count = 0;
                    while props.read_next_property().unwrap() {
                        count += 1;
                        assert!(count <= 3);
                        if collect && props.has_value() {
                            props.read_value().unwrap();
                        }
                    }
                }
                assert!(!reader.read_next_with_logger(&mut logger).unwrap());
                // Ending exactly after the six-byte header is a valid empty document.
                assert_eq!(reader.is_truncated(), length != 6);
                assert_eq!(
                    issues.len(),
                    usize::from(length != 6),
                    "length={length} chunk={chunk}"
                );
                if length != 6 {
                    assert_eq!(issues[0].violation, Violation::TruncatedStream);
                }
            }
        }
    }
}

#[test]
fn internet_codepage_is_only_a_message_fallback() {
    for (oem, page, kind, expected) in [
        (None, 1251, Attribute::MAPI_PROPERTIES, 1251),
        (Some(0), 1251, Attribute::MAPI_PROPERTIES, 1251),
        (Some(65123), 1251, Attribute::MAPI_PROPERTIES, 1251),
        (Some(1252), 1251, Attribute::MAPI_PROPERTIES, 1252),
        (None, 1251, Attribute::ATTACHMENT, 1252),
        (None, 65123, Attribute::MAPI_PROPERTIES, 1252),
        (None, -1, Attribute::MAPI_PROPERTIES, 1252),
        (None, 1200, Attribute::MAPI_PROPERTIES, 1252),
        (None, 1201, Attribute::MAPI_PROPERTIES, 1252),
        (None, 12000, Attribute::MAPI_PROPERTIES, 1252),
        (None, 12001, Attribute::MAPI_PROPERTIES, 1252),
    ] {
        let mut data = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
        if let Some(oem) = oem {
            let mut bytes = Vec::new();
            dword(&mut bytes, oem);
            dword(&mut bytes, 0);
            attribute(&mut data, Attribute::OEM_CODEPAGE, &bytes);
        }
        attribute(
            &mut data,
            kind,
            &properties(&[
                fixed(0x3fde, Type::LONG, &i32::to_le_bytes(page)),
                text(Type::STRING8, b"\xcf\xf0\xe8\xe2\xe5\xf2\0"),
            ]),
        );
        let mut reader = TnefReader::new(data.as_slice());
        while reader.read_next().unwrap() {
            if reader.attribute().unwrap().tag != kind {
                continue;
            }
            let mut props = reader.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            assert!(props.read_next_property().unwrap());
            let actual = props.read_value().unwrap();
            assert_eq!(
                actual,
                Value::Text(
                    if expected == 1251 {
                        "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}"
                    } else {
                        "\u{cf}\u{f0}\u{e8}\u{e2}\u{e5}\u{f2}"
                    }
                    .into()
                )
            );
        }
        assert_eq!(reader.codepage(), expected);
    }
}

#[test]
fn recipient_and_multivalue_codepages_do_not_change_message_decoding() {
    for table in [false, true] {
        let kind = if table {
            Type::LONG
        } else {
            Type(Type::LONG.0 | 0x1000)
        };
        let mut page = fixed(0x3fde, kind, &[]);
        if !table {
            dword(&mut page, 1);
        }
        dword(&mut page, 1251);
        let row = properties(&[page, text(Type::STRING8, b"\xe9")]);
        let mut payload = Vec::new();
        if table {
            dword(&mut payload, 1);
        }
        payload.extend_from_slice(&row);
        let data = document(
            if table {
                Attribute::RECIPIENT_TABLE
            } else {
                Attribute::MAPI_PROPERTIES
            },
            &payload,
        );
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        if table {
            assert!(props.read_next_row().unwrap());
        }
        assert!(props.read_next_property().unwrap());
        assert!(props.read_next_property().unwrap());
        assert_eq!(props.read_value().unwrap(), Value::Text("\u{e9}".into()));
        drop(props);
        assert_eq!(reader.codepage(), 1252);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn source_errors_and_interruptions_propagate_through_property_reads() {
    struct Source<'a> {
        input: &'a [u8],
        interrupt: bool,
    }
    impl Read for Source<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if std::mem::take(&mut self.interrupt) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.input.is_empty() {
                return Err(io::Error::other("source stopped"));
            }
            self.input.read(output)
        }
    }
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[named(1, &unicode("name")), text(Type::BINARY, b"abcdef")]),
    );
    for length in 15..data.len() - 2 {
        for stream in [false, true] {
            let mut reader = TnefReader::new(Source {
                input: &data[..length],
                interrupt: true,
            });
            assert!(reader.read_next().unwrap());
            let mut props = reader.open_properties().unwrap();
            let error = loop {
                match props.read_next_property() {
                    Err(error) => break error,
                    Ok(false) => panic!("missing source error"),
                    Ok(true) => {
                        if props.property().unwrap().tag.value_type() == Type::BINARY {
                            if stream {
                                if let Err(error) =
                                    props.open_value().unwrap().read_to_end(&mut Vec::new())
                                {
                                    assert_eq!(error.to_string(), "source stopped");
                                    break Error::Io(error);
                                }
                            } else if let Err(error) = props.read_value() {
                                break error;
                            }
                        }
                    }
                }
            };
            assert!(matches!(error, Error::Io(_)));
            assert!(matches!(props.read_next_value(), Err(Error::Faulted)));
            drop(props);
            assert!(matches!(reader.read_next(), Err(Error::Faulted)));
        }
        let mut reader = TnefReader::new(Source {
            input: &data[..length],
            interrupt: true,
        });
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(matches!(props.read_property_set(), Err(Error::Io(_))));
        assert!(matches!(props.read_property(), Err(Error::Faulted)));
        assert!(matches!(props.read_property_set(), Err(Error::Faulted)));
        assert!(matches!(
            props.read_rows_as_property_sets(),
            Err(Error::Faulted)
        ));
        drop(props);
        assert!(matches!(reader.read_next(), Err(Error::Faulted)));
    }
}

#[test]
fn issue_budgets_are_shared_and_checksum_reports_clear_property_context() {
    let payload = properties(&[named(99, &[]), named(99, &[]), named(99, &[])]);
    let mut data = document(Attribute::MAPI_PROPERTIES, &payload);
    let last = data.len() - 1;
    data[last] ^= 1;
    attribute(&mut data, Attribute::MAPI_PROPERTIES, &payload);
    let options = TnefReaderOptions {
        max_issues_per_violation: 1,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        while props.read_next_property().unwrap() {}
    }
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].violation, Violation::InvalidNamedPropertyKind);
    assert!(issues[0].property_tag.is_some());
    assert_eq!(issues[1].violation, Violation::AttributeChecksumMismatch);
    assert_eq!(issues[1].property_tag, None);
}

#[test]
fn enormous_declared_values_do_not_require_enormous_allocations() {
    // Lift the logical limits so this tests incremental allocation, not just
    // the normal rejection of an oversized value.
    for kind in [Type::BINARY, Type::UNICODE, Type::STRING8] {
        let mut item = fixed(0x0037, kind, &1i32.to_le_bytes());
        dword(&mut item, 0x40000000);
        item.extend_from_slice(b"abc");
        let mut data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
        data.truncate(data.len() - 2);
        data[11..15].copy_from_slice(&(i32::MAX - 1024).to_le_bytes());
        let options = TnefReaderOptions {
            max_value_length: usize::MAX,
            max_total_data_bytes: u64::MAX,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        let bytes = props.read_value_bytes().unwrap();
        assert_eq!(bytes, b"abc");
        assert!(bytes.capacity() <= 65536);
        assert!(!props.read_next_property().unwrap());
        assert!(props.is_stopped());
        drop(props);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert!(reader.is_truncated());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::TruncatedStream);
        assert_eq!(issues[0].property_tag, Some(Tag::new(0x0037, kind)));
    }
}

#[derive(Debug, PartialEq)]
struct CorpusScan {
    properties: Vec<(mimecraft::tnef::TnefPropertyInfo, usize)>,
    values: Vec<Value>,
    rows: usize,
    issues: Vec<Issue>,
    position: u64,
}

fn scan_corpus(data: &[u8], chunk: usize, mode: u8) -> CorpusScan {
    let options = TnefReaderOptions {
        max_value_length: usize::MAX,
        max_total_data_bytes: u64::MAX,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(Chunked { input: data, chunk }, &options).unwrap();
    let mut result = CorpusScan {
        properties: Vec::new(),
        values: Vec::new(),
        rows: 0,
        issues: Vec::new(),
        position: 0,
    };
    let mut logger = |issue| {
        result.issues.push(issue);
        Ok(())
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        if !matches!(
            reader.attribute().unwrap().tag,
            Attribute::MAPI_PROPERTIES | Attribute::ATTACHMENT | Attribute::RECIPIENT_TABLE
        ) {
            continue;
        }
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        loop {
            if props.is_table() {
                if !props.read_next_row().unwrap() {
                    break;
                }
                result.rows += 1;
                assert!(result.rows <= data.len());
            }
            while props.read_next_property().unwrap() {
                result
                    .properties
                    .push((props.property().unwrap().clone(), props.value_count()));
                assert!(result.properties.len() <= data.len());
                if mode == 0 {
                    continue;
                }
                let variable = matches!(
                    props.property().unwrap().tag.value_type(),
                    Type::BINARY | Type::OBJECT | Type::UNICODE | Type::STRING8
                );
                if !props.has_value() {
                    continue;
                }
                loop {
                    let value = if variable {
                        let bytes = if mode == 1 {
                            props.read_value_bytes().unwrap()
                        } else {
                            let mut bytes = Vec::new();
                            let mut stream = props.open_value().unwrap();
                            let mut buffer = [0; 31];
                            loop {
                                let count = stream.read(&mut buffer).unwrap();
                                if count == 0 {
                                    break;
                                }
                                bytes.extend_from_slice(&buffer[..count]);
                            }
                            bytes
                        };
                        Value::Binary(bytes)
                    } else {
                        props.read_value().unwrap()
                    };
                    result.values.push(value);
                    assert!(result.values.len() <= data.len());
                    if !props.read_next_value().unwrap() {
                        break;
                    }
                }
            }
            if !props.is_table() {
                break;
            }
        }
    }
    result.position = reader.position();
    result
}

#[test]
fn original_corpus_properties_match_across_chunks_and_read_modes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut count = 0;
    for file in std::fs::read_dir(root).unwrap() {
        let path = file.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("tnef") {
            continue;
        }
        count += 1;
        let data = std::fs::read(&path).unwrap();
        let expected = scan_corpus(&data, 4096, 1);
        for chunk in [1, 7, 4096] {
            for mode in [1, 2] {
                assert_eq!(
                    scan_corpus(&data, chunk, mode),
                    expected,
                    "{} {chunk} {mode}",
                    path.display()
                );
            }
            let skipped = scan_corpus(&data, chunk, 0);
            assert_eq!(
                skipped.properties,
                expected.properties,
                "{}",
                path.display()
            );
            assert_eq!(skipped.rows, expected.rows);
            assert_eq!(skipped.position, expected.position);
            // A truncated variable value can be noticed at a different offset
            // while collecting versus skipping, but the violation is the same.
            assert_eq!(
                skipped
                    .issues
                    .iter()
                    .map(|i| i.violation)
                    .collect::<Vec<_>>(),
                expected
                    .issues
                    .iter()
                    .map(|i| i.violation)
                    .collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(count, 19);
}

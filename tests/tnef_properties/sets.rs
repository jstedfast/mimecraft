// Adapted from MimeKit's TnefPropertySetTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::tnef::{TnefProperty, TnefPropertySet};

fn multi(kind: Type, values: &[&[u8]]) -> Vec<u8> {
    let mut item = fixed(0x6601, Type(kind.0 | 0x1000), &[]);
    dword(&mut item, values.len() as i32);
    for value in values {
        if matches!(kind, Type::BINARY | Type::UNICODE) {
            variable(&mut item, value);
        } else {
            item.extend_from_slice(value);
        }
    }
    item
}

fn read_set(data: &[u8], chunk: usize) -> TnefPropertySet {
    let mut reader = TnefReader::new(Chunked { input: data, chunk });
    let mut logger = |issue| panic!("unexpected issue: {issue:?}");
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let set = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    set
}

#[test]
fn snapshots_survive_reader_and_input_and_preserve_native_values() {
    let set = {
        let bytes = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[
                text(Type::STRING8, b"Caf\xe9\0"),
                text(Type::UNICODE, &unicode("body\0")),
                text(Type::BINARY, &[1, 2, 3, 0]),
                text(Type::OBJECT, &[0x42; 19]),
                fixed(0x1234, Type::NULL, &[]),
            ]),
        );
        read_set(&bytes, 1)
    };
    let expected = [
        Value::Text("Caf\u{e9}".into()),
        Value::Text("body".into()),
        Value::Binary(vec![1, 2, 3, 0]),
        Value::Object(vec![0x42; 19]),
        Value::Null,
    ];
    assert_eq!(set.len(), expected.len());
    for _ in 0..2 {
        for (index, expected) in expected.iter().enumerate().rev() {
            assert_eq!(set[index].values(), std::slice::from_ref(expected));
            assert_eq!(set[index].value(), Some(expected));
            assert_eq!(set[index].name(), None);
        }
    }
    assert_eq!(set[0].info().offset, 19);
    let cloned = set.clone();
    let collected: TnefPropertySet = cloned.into_iter().collect();
    assert_eq!(set, collected);
    assert_eq!((&set).into_iter().count(), 5);
    assert_eq!(set.iter().count(), 5);
    assert_eq!(set.as_slice().get(5), None);
    let restored = TnefPropertySet::from(collected.into_vec());
    let (info, values) = restored.into_iter().next().unwrap().into_parts();
    assert_eq!(info, *set[0].info());
    assert_eq!(values, [expected[0].clone()]);
    let empty = TnefPropertySet::new();
    assert!(empty.is_empty());
    assert_eq!(empty.get_by_id(0), None);
}

#[test]
fn native_multivalues_and_empty_properties() {
    let items = [
        multi(
            Type::LONG,
            &[
                &1i32.to_le_bytes(),
                &2i32.to_le_bytes(),
                &3i32.to_le_bytes(),
            ],
        ),
        multi(Type::I8, &[&(-1i64).to_le_bytes(), &i64::MAX.to_le_bytes()]),
        multi(
            Type::UNICODE,
            &[&unicode("one\0"), &unicode("\0"), &unicode("three\0")],
        ),
        multi(Type::BINARY, &[&[1], &[2, 3, 4, 5, 6]]),
        multi(Type::LONG, &[]),
    ];
    let data = document(Attribute::MAPI_PROPERTIES, &properties(&items));
    for chunk in [1, 7, 4096] {
        let set = read_set(&data, chunk);
        assert_eq!(set.len(), 5);
        assert!(set.iter().all(TnefProperty::is_multi_valued));
        assert!(set.iter().all(|p| p.value().is_none()));
        assert_eq!(
            set[0].values(),
            [Value::I32(1), Value::I32(2), Value::I32(3)]
        );
        assert_eq!(set[1].values(), [Value::I64(-1), Value::I64(i64::MAX)]);
        assert_eq!(
            set[2].values(),
            [
                Value::Text("one".into()),
                Value::Text("".into()),
                Value::Text("three".into())
            ]
        );
        assert_eq!(
            set[3].values(),
            [Value::Binary(vec![1]), Value::Binary(vec![2, 3, 4, 5, 6])]
        );
        assert_eq!(set[4].value_count(), 0);
        assert!(set[4].values().is_empty());
    }
}

#[test]
fn lookup_preserves_order_duplicates_types_and_case_sensitive_names() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            text(Type::STRING8, b"first\0"),
            text(Type::UNICODE, &unicode("second\0")),
            named(1, &unicode("Keywords\0")),
            named(1, &unicode("Keywords\0")),
            named(0, &0x8501i32.to_le_bytes()),
        ]),
    );
    let set = read_set(&data, 7);
    assert_eq!(set.len(), 5);
    assert_eq!(set.get(Tag::new(0x0037, Type::UNICODE)), Some(&set[0]));
    assert_eq!(
        set.get_by_id(0x0037).unwrap().value(),
        Some(&Value::Text("first".into()))
    );
    let name = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::String("Keywords".into()),
    };
    assert!(std::ptr::eq(set.get_named(&name).unwrap(), &set[2]));
    assert_eq!(
        set.get_named(&TnefNameId {
            name: TnefName::String("keywords".into()),
            ..name.clone()
        }),
        None
    );
    assert_eq!(
        set.get_named(&TnefNameId {
            guid: [0; 16],
            ..name
        }),
        None
    );
    let numeric = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::Id(0x8501),
    };
    assert_eq!(set.get_named(&numeric), Some(&set[4]));
    assert_eq!(set.get_by_id(0x9999), None);

    let mut empty = text(Type::BINARY, b"");
    empty.truncate(8);
    empty[4..8].copy_from_slice(&0i32.to_le_bytes());
    // A zero-count single-valued property is malformed, but still shadows a
    // later property with the same identifier, exactly as in MimeKit.
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[empty, text(Type::STRING8, b"later")]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let set = reader
        .open_properties()
        .unwrap()
        .read_property_set()
        .unwrap();
    assert_eq!(set.get_by_id(0x0037).unwrap().value(), None);
    assert_eq!(set[1].value(), Some(&Value::Text("later".into())));
}

#[test]
fn snapshot_cursor_claims_and_remaining_set_semantics() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            text(Type::UNICODE, &unicode("subject")),
            multi(Type::LONG, &[&1i32.to_le_bytes(), &2i32.to_le_bytes()]),
            fixed(0x0026, Type::LONG, &1i32.to_le_bytes()),
        ]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties().unwrap();
    assert!(matches!(props.read_property(), Err(Error::NoProperty)));
    assert!(props.read_next_property().unwrap());
    let subject = props.read_property().unwrap();
    assert_eq!(subject.value(), Some(&Value::Text("subject".into())));
    assert!(matches!(
        props.read_property(),
        Err(Error::ValueAlreadyRead)
    ));
    assert!(props.read_next_property().unwrap());
    assert_eq!(props.read_value().unwrap(), Value::I32(1));
    assert!(props.read_next_value().unwrap());
    assert!(matches!(
        props.read_property(),
        Err(Error::ValueAlreadyRead)
    ));
    // The remainder of the current multivalued property is skipped.
    let set = props.read_property_set().unwrap();
    assert_eq!(set.len(), 1);
    assert_eq!(set[0].tag().id(), 0x0026);
    assert_eq!(set[0].value(), Some(&Value::I32(1)));
    assert!(props.read_property_set().unwrap().is_empty());
    assert!(matches!(props.read_property(), Err(Error::NoProperty)));
    drop(props);
    assert!(!reader.read_next().unwrap());
    assert_eq!(subject.value(), Some(&Value::Text("subject".into())));

    for stream in [false, true] {
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        if stream {
            let _stream = props.open_value().unwrap();
        } else {
            props.read_value().unwrap();
        }
        assert!(matches!(
            props.read_property(),
            Err(Error::ValueAlreadyRead)
        ));
    }
}

#[test]
fn empty_property_does_not_inherit_previous_claim() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            text(Type::UNICODE, &unicode("body")),
            multi(Type::UNICODE, &[]),
            fixed(0x0026, Type::LONG, &1i32.to_le_bytes()),
        ]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties().unwrap();
    assert!(props.read_next_property().unwrap());
    props.read_value().unwrap();
    assert!(props.read_next_property().unwrap());
    assert_eq!(props.read_property().unwrap().value_count(), 0);
    assert!(props.read_next_property().unwrap());
    assert_eq!(props.read_property().unwrap().value(), Some(&Value::I32(1)));
    assert_eq!(props.read_property().unwrap().value(), Some(&Value::I32(1)));
    assert!(!props.read_next_value().unwrap());
    assert!(matches!(
        props.read_property(),
        Err(Error::ValueAlreadyRead)
    ));
}

#[test]
fn nominally_single_value_snapshot_leaves_extra_values_for_skipping() {
    let mut first = text(Type::BINARY, b"first");
    first[4..8].copy_from_slice(&2i32.to_le_bytes());
    variable(&mut first, b"second");
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[first, text(Type::STRING8, b"next")]),
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
    let snapshot = props.read_property().unwrap();
    assert_eq!(snapshot.values(), [Value::Binary(b"first".to_vec())]);
    assert_eq!(props.value_count(), 2);
    let rest = props.read_property_set().unwrap();
    assert_eq!(rest[0].value(), Some(&Value::Text("next".into())));
    drop(props);
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidValueCount);
}

#[test]
fn recipient_collection_requires_a_row_and_skips_the_current_row() {
    let rows = [
        properties(&[
            text(Type::UNICODE, &unicode("Alice")),
            fixed(0x0c15, Type::LONG, &1i32.to_le_bytes()),
        ]),
        properties(&[]),
        properties(&[
            text(Type::UNICODE, &unicode("Bob")),
            fixed(0x0c15, Type::LONG, &2i32.to_le_bytes()),
        ]),
    ];
    let mut payload = 3i32.to_le_bytes().to_vec();
    for row in rows {
        payload.extend(row);
    }
    let mut data = document(Attribute::RECIPIENT_TABLE, &payload);
    attribute(
        &mut data,
        Attribute::MAPI_PROPERTIES,
        &properties(&[text(Type::STRING8, b"message")]),
    );
    for skip_current in [false, true] {
        let mut reader = TnefReader::new(Chunked {
            input: &data,
            chunk: 1,
        });
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_property_set().unwrap().is_empty());
        if skip_current {
            assert!(props.read_next_row().unwrap());
            assert!(props.read_next_property().unwrap());
        }
        let sets = props.read_rows_as_property_sets().unwrap();
        assert_eq!(sets.len(), if skip_current { 2 } else { 3 });
        if !skip_current {
            assert_eq!(sets[0][0].value(), Some(&Value::Text("Alice".into())));
        }
        assert!(sets[sets.len() - 2].is_empty());
        assert_eq!(
            sets.last().unwrap()[0].value(),
            Some(&Value::Text("Bob".into()))
        );
        assert!(props.read_rows_as_property_sets().unwrap().is_empty());
        drop(props);
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        let before = props.position();
        assert!(props.read_rows_as_property_sets().unwrap().is_empty());
        assert_eq!(props.position(), before);
        assert_eq!(
            props.read_property_set().unwrap()[0].value(),
            Some(&Value::Text("message".into()))
        );
    }
}

#[test]
fn truncated_multivalues_keep_actual_values_and_original_metadata() {
    // Claim four values, but provide two. Each Unicode value occupies eight
    // bytes, so the count initially fits the minimum-width bound.
    let mut item = multi(Type::UNICODE, &[&unicode("a\0"), &unicode("b\0")]);
    item[4..8].copy_from_slice(&4i32.to_le_bytes());
    let data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
    let mut reader = TnefReader::new(data.as_slice());
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
    let set = props.read_property_set().unwrap();
    assert!(props.is_stopped());
    assert_eq!(set.len(), 1);
    assert_eq!(set[0].info().offset, 19);
    assert_eq!(
        set[0].values(),
        [Value::Text("a".into()), Value::Text("b".into())]
    );
    assert_eq!(set[0].value_count(), 2);
    drop(props);
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidPropertyLength);
}

#[test]
fn collection_limits_are_explicit_and_failed_values_remain_streamable() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            multi(Type::BINARY, &[b"ok", b"oversized"]),
            text(Type::BINARY, b"next"),
            fixed(0x0026, Type::LONG, &7i32.to_le_bytes()),
        ]),
    );
    for entire_set in [false, true] {
        let options = TnefReaderOptions {
            max_value_length: 5,
            max_total_data_bytes: 5,
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
        if entire_set {
            assert!(matches!(
                props.read_property_set(),
                Err(Error::ValueLimitExceeded)
            ));
        } else {
            assert!(props.read_next_property().unwrap());
            assert!(matches!(
                props.read_property(),
                Err(Error::ValueLimitExceeded)
            ));
        }
        assert!(!props.is_stopped());
        assert!(!props.is_value_consumed());
        let mut bytes = Vec::new();
        props.open_value().unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"oversized");
        assert!(props.read_next_property().unwrap());
        // The already collected "ok" was not refunded after the failure.
        assert!(matches!(
            props.read_property(),
            Err(Error::ValueLimitExceeded)
        ));
        let remaining = props.read_property_set().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].value(), Some(&Value::I32(7)));
        drop(props);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 2);
        assert!(
            issues
                .iter()
                .all(|i| i.violation == Violation::DataSizeLimitExceeded)
        );
    }
}

#[test]
fn name_limit_and_logger_errors_propagate_from_collection() {
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[named(1, &unicode("long-name"))]),
    );
    let options = TnefReaderOptions {
        max_value_length: 2,
        ..Default::default()
    };
    for fail_logger in [false, true] {
        let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
        let mut logger = |_: Issue| {
            if fail_logger {
                Err(io::Error::other("logger stopped"))
            } else {
                Ok(())
            }
        };
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        let error = props.read_property_set().unwrap_err();
        if fail_logger {
            assert!(matches!(error, Error::Io(_)));
            assert!(matches!(props.read_property(), Err(Error::Faulted)));
            assert!(matches!(props.read_property_set(), Err(Error::Faulted)));
            assert!(matches!(
                props.read_rows_as_property_sets(),
                Err(Error::Faulted)
            ));
        } else {
            assert!(matches!(error, Error::ValueLimitExceeded));
            assert!(props.is_stopped());
            drop(props);
            assert!(!reader.read_next().unwrap());
        }
    }
}

#[test]
fn cumulative_limits_span_properties_and_recipient_rows() {
    let value = vec![0x5a; 100 * 1024];
    for table in [false, true] {
        let mut payload = Vec::new();
        dword(&mut payload, 50);
        for _ in 0..50 {
            if table {
                dword(&mut payload, 1);
            }
            payload.extend(text(Type::BINARY, &value));
        }
        let kind = if table {
            Attribute::RECIPIENT_TABLE
        } else {
            Attribute::MAPI_PROPERTIES
        };
        let data = document(kind, &payload);
        let options = TnefReaderOptions {
            max_value_length: 128 * 1024,
            max_total_data_bytes: 1050 * 1024,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(
            Chunked {
                input: &data,
                chunk: 4096,
            },
            &options,
        )
        .unwrap();
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        let error = if table {
            props.read_rows_as_property_sets().unwrap_err()
        } else {
            props.read_property_set().unwrap_err()
        };
        assert!(matches!(error, Error::ValueLimitExceeded));
        // Ten values fit; the eleventh fails before claiming its payload.
        let framing = 12 + if table { 4 } else { 0 };
        assert_eq!(
            props.position(),
            (19 + 10 * (value.len() + framing) + framing) as u64
        );
        assert!(!props.is_value_consumed());
        let mut bytes = Vec::new();
        props.open_value().unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, value);
        if table {
            assert!(props.read_next_row().unwrap());
        }
        assert!(props.read_next_property().unwrap());
        assert!(matches!(
            props.read_property(),
            Err(Error::ValueLimitExceeded)
        ));
        drop(props);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn unverified_counts_grow_only_with_actual_values() {
    for (declared, actual) in [(0x08000000i32, 3i32), (5000, 5000)] {
        let mut item = fixed(0x6601, Type(Type::I8.0 | 0x1000), &declared.to_le_bytes());
        for value in 0..actual {
            item.extend_from_slice(&i64::from(value).to_le_bytes());
        }
        let mut data = document(Attribute::MAPI_PROPERTIES, &properties(&[item]));
        data.truncate(data.len() - 2);
        data[11..15].copy_from_slice(&0x7ffffff0i32.to_le_bytes());
        let mut reader = TnefReader::new(Chunked {
            input: &data,
            chunk: 7,
        });
        assert!(reader.read_next().unwrap());
        let set = reader
            .open_properties()
            .unwrap()
            .read_property_set()
            .unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].value_count(), actual as usize);
        assert!(
            set[0]
                .values()
                .iter()
                .enumerate()
                .all(|(i, value)| *value == Value::I64(i as i64))
        );
        let (_, values) = set.into_iter().next().unwrap().into_parts();
        // Trusting the declared count would require gigabytes of storage.
        assert!(values.capacity() <= (actual as usize).next_power_of_two());
        assert!(!reader.read_next().unwrap());
        assert!(reader.is_truncated());
    }
}

#[test]
fn large_nonseekable_values_and_truncated_prefixes() {
    let value: Vec<_> = (0..300 * 1024 + 3)
        .map(|i| (i * 31 + (i >> 8)) as u8)
        .collect();
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            text(Type::BINARY, &value),
            text(Type::UNICODE, &unicode(&"z".repeat(100 * 1024))),
        ]),
    );
    let set = read_set(&data, 7);
    assert_eq!(set[0].value(), Some(&Value::Binary(value.clone())));
    assert_eq!(set[1].value(), Some(&Value::Text("z".repeat(100 * 1024))));
    let mut reader = TnefReader::new(Chunked {
        input: &data[..150 * 1024],
        chunk: 7,
    });
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let set = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert_eq!(set.len(), 1);
    let Some(Value::Binary(bytes)) = set[0].value() else {
        panic!("missing truncated binary")
    };
    assert!(bytes.len() < 150 * 1024);
    assert_eq!(bytes, &value[..bytes.len()]);
    assert!(reader.is_truncated());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::TruncatedStream);
}

type CursorProperty = (mimecraft::tnef::TnefPropertyInfo, Vec<Value>);

fn corpus(data: &[u8], chunk: usize, collect_sets: bool) -> (Vec<Vec<CursorProperty>>, Vec<Issue>) {
    let options = TnefReaderOptions {
        max_value_length: usize::MAX,
        max_total_data_bytes: u64::MAX,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(Chunked { input: data, chunk }, &options).unwrap();
    let mut results = Vec::new();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
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
        if collect_sets {
            let sets = if props.is_table() {
                props.read_rows_as_property_sets().unwrap()
            } else {
                vec![props.read_property_set().unwrap()]
            };
            results.extend(
                sets.into_iter()
                    .map(|set| set.into_iter().map(TnefProperty::into_parts).collect()),
            );
        } else {
            loop {
                if props.is_table() && !props.read_next_row().unwrap() {
                    break;
                }
                let mut properties = Vec::new();
                while props.read_next_property().unwrap() {
                    let info = props.property().unwrap().clone();
                    let count = if info.tag.is_multi_valued() {
                        props.value_count()
                    } else {
                        usize::from(props.has_value())
                    };
                    let mut values = Vec::new();
                    for i in 0..count {
                        if i != 0 && !props.read_next_value().unwrap() {
                            break;
                        }
                        values.push(props.read_value().unwrap());
                    }
                    properties.push((info, values));
                }
                results.push(properties);
                if !props.is_table() {
                    break;
                }
            }
        }
    }
    (results, issues)
}

#[test]
fn corpus_collections_match_native_cursor_values_across_chunks() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut count = 0;
    for file in std::fs::read_dir(root).unwrap() {
        let path = file.unwrap().path();
        if path.extension().and_then(|v| v.to_str()) != Some("tnef") {
            continue;
        }
        count += 1;
        let bytes = std::fs::read(&path).unwrap();
        let expected = corpus(&bytes, 4096, false);
        for chunk in [1, 7, 4096] {
            assert_eq!(
                corpus(&bytes, chunk, true),
                expected,
                "{} chunk={chunk}",
                path.display()
            );
        }
    }
    assert_eq!(count, 19);
}

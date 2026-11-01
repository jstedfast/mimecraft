// Adapted from MimeKit's TnefMessageTests and TnefNestingTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::tnef::TnefEmbeddedContent;

fn collect(input: &[u8], options: &TnefReaderOptions, chunk: usize) -> TnefEmbeddedContent {
    let mut reader = TnefReader::with_options(Chunked { input, chunk }, options).unwrap();
    assert!(reader.read_next().unwrap());
    let content = {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        props.read_embedded_content().unwrap()
    };
    assert!(!reader.read_next().unwrap());
    content
}

#[test]
fn repeated_loads_survive_parent_and_source_and_preserve_nested_offsets() {
    let leaf = document(Attribute::SUBJECT, b"Innermost\0");
    let mut middle = document(Attribute::SUBJECT, b"Inner\0");
    attribute(
        &mut middle,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf)]),
    );
    for chunk in [1, 7, 4096] {
        let input = wrap(&middle);
        let middle_start = input
            .windows(middle.len())
            .position(|b| b == middle)
            .unwrap();
        let leaf_start = input.windows(leaf.len()).position(|b| b == leaf).unwrap();
        let content = collect(&input, &TnefReaderOptions::default(), chunk);
        drop(input);
        assert_eq!(content.as_bytes(), middle);

        // The embedded message may be loaded any number of times.
        for _ in 0..3 {
            let mut reader = content.open_reader().unwrap();
            assert_eq!(reader.position(), middle_start as u64);
            assert_eq!(reader.depth(), 1);
            assert!(reader.read_next().unwrap());
            assert_eq!(reader.attribute().unwrap().offset, middle_start as u64 + 6);
            assert_eq!(reader.read_value_string().unwrap(), "Inner");
            assert!(reader.read_next().unwrap());
            let innermost = {
                let mut props = reader.open_properties().unwrap();
                assert!(props.read_next_property().unwrap());
                props.read_embedded_content().unwrap()
            };
            assert!(!reader.read_next().unwrap());
            drop(reader);
            let mut reader = innermost.open_reader().unwrap();
            assert_eq!(reader.position(), leaf_start as u64);
            assert_eq!(reader.depth(), 2);
            assert!(reader.read_next().unwrap());
            assert_eq!(reader.read_value_string().unwrap(), "Innermost");
            assert!(!reader.read_next().unwrap());
        }
    }
}

#[test]
fn simultaneous_readers_have_independent_cursors_and_owned_content_is_send_sync() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<TnefEmbeddedContent>();
    send_sync::<TnefReader<&[u8]>>();
    let leaf = document(Attribute::BODY, b"hello");
    let content = collect(&wrap(&leaf), &TnefReaderOptions::default(), 1);
    let mut first = content.open_reader().unwrap();
    let mut second = content.open_reader().unwrap();
    assert!(first.read_next().unwrap());
    let mut stream = first.open_value().unwrap();
    let mut byte = [0];
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"h");
    assert!(second.read_next().unwrap());
    assert_eq!(second.read_value_string().unwrap(), "hello");
    assert!(!second.read_next().unwrap());
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, b"ello");
}

#[test]
fn binary_and_object_values_exclude_iid_and_padding_and_preserve_parent_alignment() {
    for kind in [Type::OBJECT, Type::BINARY] {
        for size in 0..8 {
            let leaf = document(Attribute::BODY, &vec![b'x'; size]);
            let mut value = IID.to_vec();
            value.extend_from_slice(&leaf);
            // Binary recognition requires AttachMethod before the data on the
            // forward-only cursor; retrospective recognition needs the loader.
            let input = document(
                Attribute::ATTACHMENT,
                &properties(&[
                    fixed(0x3705, Type::LONG, &5i32.to_le_bytes()),
                    object(0x3701, kind, &value),
                    fixed(0x1234, Type::LONG, &99i32.to_le_bytes()),
                ]),
            );
            let mut reader = TnefReader::new(input.as_slice());
            assert!(reader.read_next().unwrap());
            {
                let mut props = reader.open_properties().unwrap();
                assert!(props.read_next_property().unwrap());
                assert!(matches!(
                    props.read_embedded_content(),
                    Err(Error::InvalidPropertyType(_))
                ));
                assert!(props.read_next_property().unwrap());
                let content = props.read_embedded_content().unwrap();
                assert_eq!(content.as_bytes(), leaf);
                assert!(props.is_value_consumed());
                assert!(matches!(props.open_value(), Err(Error::ValueAlreadyRead)));
                assert!(matches!(
                    props.open_embedded_message(),
                    Err(Error::ValueAlreadyRead)
                ));
                assert!(matches!(
                    props.read_embedded_content(),
                    Err(Error::ValueAlreadyRead)
                ));
                assert!(props.read_next_property().unwrap());
                assert_eq!(props.read_value().unwrap(), Value::I32(99));
                assert!(!props.read_next_property().unwrap());
            }
            assert!(!reader.read_next().unwrap());
        }
    }
}

#[test]
fn detachment_cannot_reset_nesting_depth_and_limit_errors_leave_raw_streaming_usable() {
    for limit in [0, 1, 32] {
        let mut input = document(Attribute::BODY, b"leaf");
        for _ in 0..=limit {
            input = wrap(&input);
        }
        let options = TnefReaderOptions {
            max_nesting_depth: limit,
            max_issues_per_violation: 1,
            ..Default::default()
        };
        fn descend(reader: &mut TnefReader<&[u8]>, limit: usize) {
            assert!(reader.read_next().unwrap());
            let depth = reader.depth();
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            {
                let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
                assert!(props.read_next_property().unwrap());
                if depth < limit {
                    let content = props.read_embedded_content().unwrap();
                    descend(&mut content.open_reader().unwrap(), limit);
                } else {
                    for _ in 0..2 {
                        assert!(matches!(
                            props.read_embedded_content(),
                            Err(Error::NestingLimitExceeded)
                        ));
                    }
                    assert!(!props.is_value_consumed());
                    let mut raw = Vec::new();
                    props.open_value().unwrap().read_to_end(&mut raw).unwrap();
                    assert!(raw.starts_with(&IID));
                }
            }
            if depth == limit {
                assert_eq!(issues.len(), 1);
                assert_eq!(issues[0].violation, Violation::NestingTooDeep);
                assert_eq!(issues[0].depth, limit);
                assert!(issues[0].stream_offset > 0);
                assert_eq!(issues[0].property_tag, Some(Tag::ATTACH_DATA_OBJ));
            } else {
                assert!(issues.is_empty());
            }
            assert!(!reader.read_next().unwrap());
            assert!(!reader.is_faulted());
        }
        descend(
            &mut TnefReader::with_options(input.as_slice(), &options).unwrap(),
            limit,
        );
    }
}

#[test]
fn collection_limits_include_iid_and_are_recoverable() {
    let leaf = document(Attribute::BODY, b"hello");
    let input = wrap(&leaf);
    let length = (16 + leaf.len()) as u64;
    for total_limit in [false, true] {
        for allowed in [length - 1, length] {
            let options = TnefReaderOptions {
                max_total_data_bytes: if total_limit { allowed } else { u64::MAX },
                max_value_length: if total_limit {
                    usize::MAX
                } else {
                    allowed as usize
                },
                ..Default::default()
            };
            let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
            assert!(reader.read_next().unwrap());
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            {
                let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
                assert!(props.read_next_property().unwrap());
                if allowed == length {
                    assert_eq!(props.read_embedded_content().unwrap().as_bytes(), leaf);
                } else {
                    assert!(matches!(
                        props.read_embedded_content(),
                        Err(Error::ValueLimitExceeded)
                    ));
                    assert!(!props.is_value_consumed());
                    let mut raw = Vec::new();
                    props.open_value().unwrap().read_to_end(&mut raw).unwrap();
                    assert_eq!(&raw[16..], leaf);
                }
            }
            assert!(!reader.read_next().unwrap());
            assert!(!reader.is_faulted());
            assert_eq!(issues.len(), usize::from(allowed < length));
            if !issues.is_empty() {
                assert_eq!(issues[0].violation, Violation::DataSizeLimitExceeded);
            }
        }
    }
}

#[test]
fn detached_readers_get_fresh_budgets_without_refunding_the_parent() {
    let leaf = document(Attribute::BODY, b"hello");
    let input = document(
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf), nested(&leaf)]),
    );
    let options = TnefReaderOptions {
        max_total_data_bytes: (IID.len() + leaf.len()) as u64,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let content = props.read_embedded_content().unwrap();
        for _ in 0..3 {
            let mut detached = content.open_reader().unwrap();
            assert!(detached.read_next().unwrap());
            assert_eq!(detached.read_value_bytes().unwrap(), b"hello");
            assert!(!detached.read_next().unwrap());
        }
        assert!(props.read_next_property().unwrap());
        assert!(matches!(
            props.read_embedded_content(),
            Err(Error::ValueLimitExceeded)
        ));
        assert!(!props.is_value_consumed());
    }
    assert!(!reader.read_next().unwrap());
}

#[test]
fn captured_charset_survives_parent_drop_and_child_overrides_are_local() {
    let mut leaf = document(Attribute::BODY, &[0xe0]);
    attribute(&mut leaf, Attribute::OEM_CODEPAGE, &1252u32.to_le_bytes());
    attribute(&mut leaf, Attribute::BODY, &[0xe0]);
    let mut input = document(Attribute::OEM_CODEPAGE, &1251u32.to_le_bytes());
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf)]),
    );
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    assert!(reader.read_next().unwrap());
    let content = {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        props.read_embedded_content().unwrap()
    };
    assert!(!reader.read_next().unwrap());
    drop(reader);
    for _ in 0..2 {
        let mut reader = content.open_reader().unwrap();
        assert_eq!(reader.codepage(), 1251);
        assert!(reader.read_next().unwrap());
        assert_eq!(reader.read_value_string().unwrap(), "\u{430}");
        assert!(reader.read_next().unwrap());
        assert!(reader.read_next().unwrap());
        assert_eq!(reader.read_value_string().unwrap(), "\u{e0}");
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn logger_faults_are_isolated_and_report_caps_restart_for_each_detached_reader() {
    let bad = Attribute(0x00069999);
    let mut leaf = document(bad, &[]);
    attribute(&mut leaf, bad, &[]);
    let mut input = document(bad, &[]);
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf)]),
    );
    let options = TnefReaderOptions {
        max_issues_per_violation: 1,
        ..Default::default()
    };
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut parent = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(parent.read_next_with_logger(&mut logger).unwrap());
    assert!(parent.read_next_with_logger(&mut logger).unwrap());
    let content = {
        let mut props = parent.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        props.read_embedded_content().unwrap()
    };
    let mut failed = content.open_reader().unwrap();
    let mut stop = |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "stop"));
    assert!(matches!(
        failed.read_next_with_logger(&mut stop),
        Err(Error::Io(_))
    ));
    assert!(failed.is_faulted());
    assert!(matches!(failed.read_next(), Err(Error::Faulted)));
    assert!(!parent.is_faulted());
    for _ in 0..2 {
        let mut reader = content.open_reader().unwrap();
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    }
    assert!(!parent.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 3);
    assert_eq!(
        issues.iter().map(|i| i.depth).collect::<Vec<_>>(),
        [0, 1, 1]
    );
    assert!(
        issues
            .iter()
            .all(|i| i.violation == Violation::UnknownAttribute)
    );
}

#[test]
fn checksums_and_signatures_are_validated_only_by_the_respective_readers() {
    for corrupt_signature in [false, true] {
        let mut leaf = document(Attribute::BODY, b"hello");
        if corrupt_signature {
            leaf[0] ^= 1;
        } else {
            *leaf.last_mut().unwrap() ^= 1;
        }
        let mut input = wrap(&leaf);
        *input.last_mut().unwrap() ^= 1;
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut parent = TnefReader::new(input.as_slice());
        assert!(parent.read_next_with_logger(&mut logger).unwrap());
        let content = {
            let mut props = parent.open_properties_with_logger(&mut logger).unwrap();
            assert!(props.read_next_property().unwrap());
            props.read_embedded_content().unwrap()
        };
        assert!(!parent.read_next_with_logger(&mut logger).unwrap());
        let mut child = content.open_reader().unwrap();
        let start = child.position();
        if !corrupt_signature {
            assert!(child.read_next_with_logger(&mut logger).unwrap());
            assert_eq!(child.read_value_bytes().unwrap(), b"hello");
        }
        assert!(!child.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].depth, 0);
        assert_eq!(issues[0].violation, Violation::AttributeChecksumMismatch);
        assert_eq!(issues[1].depth, 1);
        assert_eq!(
            issues[1].stream_offset,
            start + if corrupt_signature { 0 } else { 6 }
        );
        assert_eq!(
            issues[1].violation,
            if corrupt_signature {
                Violation::InvalidSignature
            } else {
                Violation::AttributeChecksumMismatch
            }
        );
    }
}

#[test]
fn partial_iids_and_headers_stay_bounded_and_report_truncation() {
    for length in 0..22 {
        let mut value = IID.to_vec();
        value.extend_from_slice(b"\x78\x9f\x3e\x22\0\0");
        value.truncate(length);
        let input = document(
            Attribute::ATTACHMENT,
            &properties(&[
                fixed(0x3705, Type::LONG, &5i32.to_le_bytes()),
                object(0x3701, Type::BINARY, &value),
            ]),
        );
        let mut parent = TnefReader::new(input.as_slice());
        assert!(parent.read_next().unwrap());
        let content = {
            let mut props = parent.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            assert!(props.read_next_property().unwrap());
            props.read_embedded_content().unwrap()
        };
        assert!(!parent.read_next().unwrap());
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut child = content.open_reader().unwrap();
        assert_eq!(content.as_bytes(), &value[length.min(16)..]);
        assert!(!child.read_next_with_logger(&mut logger).unwrap());
        assert!(!child.read_next_with_logger(&mut logger).unwrap());
        assert!(child.is_truncated());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::TruncatedStream);
        assert_eq!(issues[0].depth, 1);
        assert!(issues[0].stream_offset <= parent.position());
    }
}

#[test]
fn physical_truncation_collects_only_available_bytes_and_propagates_logger_errors() {
    let leaf = document(Attribute::BODY, b"hello");
    let mut value = IID.to_vec();
    value.extend_from_slice(&leaf);
    let complete = document(
        Attribute::ATTACHMENT,
        &properties(&[
            fixed(0x3705, Type::LONG, &5i32.to_le_bytes()),
            object(0x3701, Type::BINARY, &value),
        ]),
    );
    let start = complete
        .windows(value.len())
        .position(|b| b == value)
        .unwrap();
    for length in 0..value.len() {
        for chunk in [1, 4096] {
            let input = &complete[..start + length];
            let mut parent = TnefReader::new(Chunked { input, chunk });
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            assert!(parent.read_next().unwrap());
            let content = {
                let mut props = parent.open_properties_with_logger(&mut logger).unwrap();
                assert!(props.read_next_property().unwrap());
                assert!(props.read_next_property().unwrap());
                props.read_embedded_content().unwrap()
            };
            assert!(parent.is_truncated());
            assert!(!parent.read_next_with_logger(&mut logger).unwrap());
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].violation, Violation::TruncatedStream);
            assert_eq!(content.as_bytes(), &value[length.min(16)..length]);
            assert_eq!(
                content.open_reader().unwrap().position(),
                (start + length.min(16)) as u64
            );
        }
    }
    let mut parent = TnefReader::new(&complete[..start + 19]);
    assert!(parent.read_next().unwrap());
    let mut stop = |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "stop"));
    {
        let mut props = parent.open_properties_with_logger(&mut stop).unwrap();
        assert!(props.read_next_property().unwrap());
        assert!(props.read_next_property().unwrap());
        assert!(matches!(props.read_embedded_content(), Err(Error::Io(_))));
    }
    assert!(parent.is_faulted());
}

#[test]
fn detachment_from_a_borrowed_child_charges_the_shared_root_budget() {
    let leaf = document(Attribute::BODY, b"hello");
    let middle = wrap(&leaf);
    let input = document(
        Attribute::ATTACHMENT,
        &properties(&[nested(&middle), nested(&leaf)]),
    );
    let options = TnefReaderOptions {
        max_total_data_bytes: (IID.len() + leaf.len()) as u64,
        ..Default::default()
    };
    let mut root = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(root.read_next().unwrap());
    let content = {
        let mut props = root.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let content = {
            let mut child = props.open_embedded_message().unwrap();
            assert!(child.read_next().unwrap());
            let content = {
                let mut props = child.open_properties().unwrap();
                assert!(props.read_next_property().unwrap());
                props.read_embedded_content().unwrap()
            };
            assert!(!child.read_next().unwrap());
            content
        };
        assert!(props.read_next_property().unwrap());
        assert!(matches!(
            props.read_embedded_content(),
            Err(Error::ValueLimitExceeded)
        ));
        content
    };
    assert!(!root.read_next().unwrap());
    drop(root);
    let mut reader = content.open_reader().unwrap();
    assert_eq!(reader.depth(), 2);
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_value_bytes().unwrap(), b"hello");
    assert!(!reader.read_next().unwrap());
}

#[test]
fn collection_io_and_logger_errors_fault_the_parent() {
    let input = wrap(&document(Attribute::BODY, b"hello"));
    let mut parent = TnefReader::new(Observed {
        input: &input,
        offset: 0,
        fail: 51,
        calls: Rc::new(Cell::new(0)),
        interrupted: false,
    });
    assert!(parent.read_next().unwrap());
    {
        let mut props = parent.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        assert!(matches!(props.read_embedded_content(), Err(Error::Io(_))));
        assert!(matches!(props.read_embedded_content(), Err(Error::Faulted)));
    }
    assert!(parent.is_faulted());

    let options = TnefReaderOptions {
        max_nesting_depth: 0,
        ..Default::default()
    };
    let mut parent = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(parent.read_next().unwrap());
    let mut stop = |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "stop"));
    {
        let mut props = parent.open_properties_with_logger(&mut stop).unwrap();
        assert!(props.read_next_property().unwrap());
        assert!(matches!(props.read_embedded_content(), Err(Error::Io(_))));
    }
    assert!(parent.is_faulted());
}

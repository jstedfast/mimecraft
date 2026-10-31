// Adapted from MimeKit's TnefNestingTests and TnefPropertySetTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::tnef::{TnefEmbeddedReader, TnefPropertyReader};
use std::{cell::Cell, rc::Rc};

const IID: [u8; 16] = [7, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];

fn object(id: u16, kind: Type, value: &[u8]) -> Vec<u8> {
    let mut bytes = fixed(id, kind, &[]);
    dword(&mut bytes, 1);
    variable(&mut bytes, value);
    bytes
}

fn nested(input: &[u8]) -> Vec<u8> {
    let mut value = IID.to_vec();
    value.extend_from_slice(input);
    object(0x3701, Type::OBJECT, &value)
}

fn wrap(input: &[u8]) -> Vec<u8> {
    document(Attribute::ATTACHMENT, &properties(&[nested(input)]))
}

fn attachment_attribute(bytes: &mut Vec<u8>, kind: Attribute, data: &[u8]) {
    let offset = bytes.len();
    attribute(bytes, kind, data);
    bytes[offset] = 2;
}

#[test]
fn iid_detection_precedes_method_and_never_claims_raw_bytes() {
    let leaf = document(Attribute::BODY, b"hello");
    for kind in [Type::OBJECT, Type::BINARY] {
        for method in [None, Some(1i32), Some(5)] {
            for iid in [IID, [0; 16], [0x55; 16]] {
                for id in [0x3701, 0x3702] {
                    let mut value = iid.to_vec();
                    value.extend_from_slice(&leaf);
                    let mut items = Vec::new();
                    if let Some(method) = method {
                        items.push(fixed(0x3705, Type::LONG, &method.to_le_bytes()));
                    }
                    items.push(object(id, kind, &value));
                    // Writers commonly emit AttachMethod after AttachData.
                    items.push(fixed(0x3705, Type::LONG, &5i32.to_le_bytes()));
                    let input = document(Attribute::ATTACHMENT, &properties(&items));
                    let mut reader = TnefReader::new(Chunked {
                        input: &input,
                        chunk: 1,
                    });
                    assert!(reader.read_next().unwrap());
                    {
                        let mut props = reader.open_properties().unwrap();
                        assert!(!props.is_embedded_message());
                        assert!(matches!(
                            props.open_embedded_message(),
                            Err(Error::NoPropertyValue)
                        ));
                        if method.is_some() {
                            assert!(props.read_next_property().unwrap());
                            assert!(!props.is_embedded_message());
                        }
                        assert!(props.read_next_property().unwrap());
                        let expected = id == 0x3701
                            && if kind == Type::OBJECT && iid != [0; 16] {
                                iid == IID
                            } else {
                                method == Some(5)
                            };
                        assert_eq!(props.is_embedded_message(), expected);
                        if !expected {
                            assert!(matches!(
                                props.open_embedded_message(),
                                Err(Error::NotEmbeddedMessage)
                            ));
                        }
                        let position = props.position();
                        assert!(!props.is_value_consumed());
                        assert_eq!(props.read_value_bytes().unwrap(), value);
                        assert_eq!(props.position(), position + value.len() as u64);
                        assert!(matches!(
                            props.open_embedded_message(),
                            Err(Error::ValueAlreadyRead)
                        ));
                        assert!(props.read_next_property().unwrap());
                        assert!(!props.is_embedded_message());
                    }
                    assert!(!reader.read_next().unwrap());
                }
            }
        }
    }
}

#[test]
fn attachment_method_is_reset_at_each_render_attribute() {
    let mut input = document(
        Attribute::ATTACHMENT,
        &properties(&[fixed(0x3705, Type::LONG, &5i32.to_le_bytes())]),
    );
    attachment_attribute(&mut input, Attribute::ATTACH_RENDER_DATA, &[0, 0]);
    attachment_attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[object(0x3701, Type::BINARY, &[0; 32])]),
    );
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
    }
    assert!(reader.read_next().unwrap());
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties().unwrap();
    assert!(props.read_next_property().unwrap());
    assert!(!props.is_embedded_message());
}

type Record = (usize, u64, u32);

fn walk_properties<R: Read>(props: &mut TnefPropertyReader<'_, R>, records: &mut Vec<Record>) {
    loop {
        if props.is_table() && !props.read_next_row().unwrap() {
            break;
        }
        while props.read_next_property().unwrap() {
            while props.has_value() {
                if props.is_embedded_message() {
                    let mut child = props.open_embedded_message().unwrap();
                    walk(&mut child, records);
                }
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

fn walk(reader: &mut TnefEmbeddedReader<'_, '_>, records: &mut Vec<Record>) {
    while reader.read_next().unwrap() {
        let attr = *reader.attribute().unwrap();
        records.push((reader.depth(), attr.offset, attr.tag.0));
        if matches!(
            attr.tag,
            Attribute::ATTACHMENT | Attribute::MAPI_PROPERTIES | Attribute::RECIPIENT_TABLE
        ) {
            walk_properties(&mut reader.open_properties().unwrap(), records);
        }
    }
}

#[test]
fn nested_depth_absolute_offsets_and_parent_alignment() {
    let leaf = document(Attribute::BODY, b"hello");
    let middle = wrap(&leaf);
    let input = document(
        Attribute::ATTACHMENT,
        &properties(&[
            nested(&middle),
            fixed(0x1234, Type::LONG, &99i32.to_le_bytes()),
        ]),
    );
    let leaf_start = input
        .windows(leaf.len())
        .position(|bytes| bytes == leaf)
        .unwrap();
    let middle_start = input
        .windows(middle.len())
        .position(|bytes| bytes == middle)
        .unwrap();
    for chunk in [1, 7, 4096] {
        let mut reader = TnefReader::new(Chunked {
            input: &input,
            chunk,
        });
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert_eq!(reader.depth(), 0);
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(props.read_next_property().unwrap());
            {
                let mut child = props.open_embedded_message().unwrap();
                assert_eq!(child.depth(), 1);
                assert_eq!(child.options().max_nesting_depth, 32);
                assert_eq!(child.position(), middle_start as u64 - 16);
                let mut records = Vec::new();
                walk(&mut child, &mut records);
                assert_eq!(
                    records,
                    [
                        (1, middle_start as u64 + 6, Attribute::ATTACHMENT.0),
                        (2, leaf_start as u64 + 6, Attribute::BODY.0),
                    ]
                );
            }
            assert!(matches!(
                props.read_value_bytes(),
                Err(Error::ValueAlreadyRead)
            ));
            assert!(props.read_next_property().unwrap());
            assert_eq!(props.read_value().unwrap(), Value::I32(99));
            assert!(!props.read_next_property().unwrap());
        }
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(reader.position(), input.len() as u64);
        assert!(issues.is_empty(), "{issues:?}");
    }
}

#[test]
fn depth_limit_is_explicit_recoverable_and_shares_report_caps() {
    let leaf = document(Attribute::BODY, b"hello");
    let input = document(
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf), nested(&leaf)]),
    );
    let options = TnefReaderOptions {
        max_nesting_depth: 0,
        max_issues_per_violation: 1,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        for _ in 0..2 {
            assert!(props.read_next_property().unwrap());
            for _ in 0..2 {
                assert!(matches!(
                    props.open_embedded_message(),
                    Err(Error::NestingLimitExceeded)
                ));
            }
            assert!(!props.is_value_consumed());
            let mut raw = Vec::new();
            props.open_value().unwrap().read_to_end(&mut raw).unwrap();
            assert_eq!(&raw[..16], &IID);
            assert_eq!(&raw[16..], &leaf);
        }
    }
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert!(!reader.is_faulted());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::NestingTooDeep);
    assert_eq!(issues[0].stream_offset, 31);
    assert_eq!(issues[0].depth, 0);
    assert_eq!(issues[0].property_tag, Some(Tag::new(0x3701, Type::OBJECT)));
}

#[test]
fn grandchild_limit_and_default_depth_boundary() {
    let leaf = document(Attribute::BODY, b"leaf");
    for limit in [1, 32] {
        let mut input = leaf.clone();
        for _ in 0..=limit {
            input = wrap(&input);
        }
        let options = TnefReaderOptions {
            max_nesting_depth: limit,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let mut child = props.open_embedded_message().unwrap();
        fn descend(reader: &mut TnefEmbeddedReader<'_, '_>, limit: usize) {
            assert!(reader.read_next().unwrap());
            let depth = reader.depth();
            let mut props = reader.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            if depth == limit {
                assert!(matches!(
                    props.open_embedded_message(),
                    Err(Error::NestingLimitExceeded)
                ));
                assert!(!props.is_value_consumed());
            } else {
                descend(&mut props.open_embedded_message().unwrap(), limit);
            }
        }
        descend(&mut child, limit);
    }
}

#[test]
fn short_iids_and_invalid_signatures_do_not_damage_parent() {
    for kind in [Type::OBJECT, Type::BINARY] {
        for length in 0..=22 {
            let payload = vec![0; length];
            let mut input = document(
                Attribute::ATTACHMENT,
                &properties(&[
                    fixed(0x3705, Type::LONG, &5i32.to_le_bytes()),
                    object(0x3701, kind, &payload),
                ]),
            );
            attachment_attribute(&mut input, Attribute::ATTACH_TITLE, b"next\0");
            let mut reader = TnefReader::new(Chunked {
                input: &input,
                chunk: 1,
            });
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            assert!(reader.read_next().unwrap());
            {
                let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
                assert!(props.read_next_property().unwrap());
                assert!(props.read_next_property().unwrap());
                let start = props.position();
                let mut child = props.open_embedded_message().unwrap();
                assert!(!child.read_next().unwrap());
                assert!(!child.read_next().unwrap());
                assert_eq!(child.legacy_key(), None);
                assert_eq!(child.position(), start + if length >= 16 { 16 } else { 0 });
            }
            assert!(reader.read_next_with_logger(&mut logger).unwrap());
            assert_eq!(reader.read_value_string().unwrap(), "next");
            assert!(!reader.read_next_with_logger(&mut logger).unwrap());
            assert!(!reader.is_truncated());
            assert_eq!(issues.len(), 1, "length={length}: {issues:?}");
            assert_eq!(issues[0].depth, 1);
            assert_eq!(
                issues[0].violation,
                if length < 20 {
                    Violation::TruncatedStream
                } else {
                    Violation::InvalidSignature
                }
            );
        }
    }
}

#[test]
fn siblings_and_parent_share_owned_byte_budget() {
    let leaf = document(Attribute::BODY, b"abc");
    let mut input = document(Attribute::BODY, b"xy");
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf), nested(&leaf)]),
    );
    attachment_attribute(&mut input, Attribute::ATTACH_TITLE, b"zz");
    let options = TnefReaderOptions {
        max_total_data_bytes: 7,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_value_bytes().unwrap(), b"xy");
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        {
            let mut child = props.open_embedded_message().unwrap();
            assert!(child.read_next().unwrap());
            assert_eq!(child.read_value_bytes().unwrap(), b"abc");
            assert!(!child.read_next().unwrap());
        }
        assert!(props.read_next_property().unwrap());
        {
            let mut child = props.open_embedded_message().unwrap();
            assert!(child.read_next().unwrap());
            assert!(matches!(
                child.read_value_bytes(),
                Err(Error::ValueLimitExceeded)
            ));
            let mut raw = Vec::new();
            child.open_value().unwrap().read_to_end(&mut raw).unwrap();
            assert_eq!(raw, b"abc");
            assert!(!child.read_next().unwrap());
        }
    }
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(reader.read_value_bytes().unwrap(), b"zz");
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::DataSizeLimitExceeded);
    assert_eq!(issues[0].depth, 1);
}

#[test]
fn inherited_report_caps_survive_promotion_and_siblings() {
    let bad = Attribute(0x00069999);
    let leaf = document(bad, &[]);
    let mut input = document(bad, &[]);
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf), nested(&leaf)]),
    );
    let options = TnefReaderOptions {
        max_issues_per_violation: 1,
        ..Default::default()
    };
    for report_root in [false, true] {
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        if report_root {
            assert!(reader.read_next_with_logger(&mut logger).unwrap());
        } else {
            assert!(reader.read_next().unwrap());
        }
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            walk_properties(&mut props, &mut Vec::new());
        }
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::UnknownAttribute);
        assert_eq!(issues[0].depth, usize::from(!report_root));
    }
}

#[test]
fn child_logger_failures_fault_the_root_even_after_drop() {
    let input = wrap(&[0; 6]);
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    {
        let mut logger = |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "logger stopped",
            ))
        };
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        {
            let mut child = props.open_embedded_message().unwrap();
            assert!(
                matches!(child.read_next(), Err(Error::Io(ref e)) if e.kind() == io::ErrorKind::PermissionDenied)
            );
            assert!(child.is_faulted());
            assert!(matches!(child.read_next(), Err(Error::Faulted)));
        }
        assert!(matches!(props.read_next_property(), Err(Error::Faulted)));
    }
    assert!(reader.is_faulted());
    assert!(matches!(reader.read_next(), Err(Error::Faulted)));
}

struct Observed<'a> {
    input: &'a [u8],
    offset: usize,
    fail: usize,
    calls: Rc<Cell<usize>>,
    interrupted: bool,
}

impl Read for Observed<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.offset >= self.fail {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source stopped",
            ));
        }
        let count = output.len().min(1);
        let read = self.input.read(&mut output[..count])?;
        self.offset += read;
        Ok(read)
    }
}

#[test]
fn open_drop_do_no_io_and_source_errors_propagate_through_children() {
    let input = wrap(&document(Attribute::BODY, b"hello"));
    for fail in [usize::MAX, 51] {
        let calls = Rc::new(Cell::new(0));
        let mut reader = TnefReader::new(Observed {
            input: &input,
            offset: 0,
            fail,
            calls: Rc::clone(&calls),
            interrupted: false,
        });
        assert_eq!(calls.get(), 0);
        assert!(reader.read_next().unwrap());
        {
            let mut props = reader.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            let before = calls.get();
            {
                let mut child = props.open_embedded_message().unwrap();
                assert_eq!(calls.get(), before);
                if fail != usize::MAX {
                    assert!(
                        matches!(child.read_next(), Err(Error::Io(ref e)) if e.kind() == io::ErrorKind::PermissionDenied)
                    );
                }
            }
            if fail == usize::MAX {
                assert_eq!(calls.get(), before);
            } else {
                assert!(matches!(props.read_next_property(), Err(Error::Faulted)));
            }
        }
        if fail == usize::MAX {
            assert!(!reader.read_next().unwrap());
        } else {
            assert!(reader.is_faulted());
        }
    }
}

#[test]
fn child_charset_inherits_but_does_not_change_parent() {
    let mut leaf = document(Attribute::BODY, &[0xe0]);
    attribute(&mut leaf, Attribute::OEM_CODEPAGE, &1252u32.to_le_bytes());
    attribute(&mut leaf, Attribute::BODY, &[0xe0]);
    let mut input = document(Attribute::OEM_CODEPAGE, &1251u32.to_le_bytes());
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf)]),
    );
    attachment_attribute(&mut input, Attribute::ATTACH_TITLE, &[0xe0]);
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let mut child = props.open_embedded_message().unwrap();
        assert_eq!(child.codepage(), 1251);
        assert!(child.read_next().unwrap());
        assert_eq!(child.read_value_string().unwrap(), "\u{430}");
        assert!(child.read_next().unwrap());
        assert!(child.read_next().unwrap());
        assert_eq!(child.read_value_string().unwrap(), "\u{e0}");
        assert!(!child.read_next().unwrap());
    }
    assert_eq!(reader.codepage(), 1251);
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_value_string().unwrap(), "\u{430}");
}

#[test]
fn checksums_are_verified_only_when_each_reader_advances() {
    let mut leaf = document(Attribute::BODY, b"abc");
    let last = leaf.len() - 1;
    leaf[last] ^= 1;
    for finish_child in [false, true] {
        let mut input = wrap(&leaf);
        let last = input.len() - 1;
        input[last] ^= 1;
        let mut reader = TnefReader::new(input.as_slice());
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(props.read_next_property().unwrap());
            let mut child = props.open_embedded_message().unwrap();
            assert!(child.read_next().unwrap());
            assert_eq!(child.read_value_bytes().unwrap(), b"abc");
            if finish_child {
                assert!(!child.read_next().unwrap());
            }
        }
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1 + usize::from(finish_child));
        assert!(
            issues
                .iter()
                .all(|issue| issue.violation == Violation::AttributeChecksumMismatch)
        );
        assert_eq!(issues.last().unwrap().depth, 0);
        assert_eq!(issues.last().unwrap().property_tag, None);
        if finish_child {
            assert_eq!(issues[0].depth, 1);
            assert_eq!(issues[0].stream_offset, 53);
        }
    }
}

fn corpus(input: &[u8], chunk: usize) -> (Vec<Record>, Vec<Issue>) {
    let mut reader = TnefReader::new(Chunked { input, chunk });
    let mut records = Vec::new();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        let attr = *reader.attribute().unwrap();
        records.push((0, attr.offset, attr.tag.0));
        if matches!(
            attr.tag,
            Attribute::ATTACHMENT | Attribute::MAPI_PROPERTIES | Attribute::RECIPIENT_TABLE
        ) {
            walk_properties(
                &mut reader.open_properties_with_logger(&mut logger).unwrap(),
                &mut records,
            );
        }
    }
    (records, issues)
}

#[test]
fn all_fixtures_recurse_consistently_across_short_reads() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut count = 0;
    let mut nested_count = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|v| v.to_str()) != Some("tnef") {
            continue;
        }
        count += 1;
        let input = std::fs::read(&path).unwrap();
        let expected = corpus(&input, 4096);
        nested_count += expected.0.iter().filter(|r| r.0 > 0).count();
        for chunk in [1, 7] {
            assert_eq!(
                corpus(&input, chunk),
                expected,
                "{} chunk={chunk}",
                path.display()
            );
        }
    }
    assert_eq!(count, 19);
    assert!(nested_count > 0);
}

#[test]
fn ordinary_root_readers_remain_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TnefReader<&[u8]>>();
}

#[test]
fn every_physical_truncation_recovers_consistently() {
    let input = wrap(&wrap(&document(Attribute::BODY, b"nested payload")));
    for length in 0..=input.len() {
        let expected = corpus(&input[..length], 4096);
        for chunk in [1, 7] {
            assert_eq!(
                corpus(&input[..length], chunk),
                expected,
                "length={length}, chunk={chunk}"
            );
        }
    }
}

#[test]
fn partial_child_drop_skips_only_its_value_and_padding() {
    let leaf = document(Attribute::BODY, &vec![0x5a; 12001]);
    let input = document(
        Attribute::ATTACHMENT,
        &properties(&[
            nested(&leaf),
            fixed(0x1234, Type::LONG, &55i32.to_le_bytes()),
        ]),
    );
    for chunk in [1, 7, 4096] {
        let mut reader = TnefReader::new(Chunked {
            input: &input,
            chunk,
        });
        let mut logger = |issue| panic!("unexpected issue: {issue:?}");
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(props.read_next_property().unwrap());
            {
                let mut child = props.open_embedded_message().unwrap();
                assert!(child.read_next().unwrap());
                let mut byte = [0];
                child.open_value().unwrap().read_exact(&mut byte).unwrap();
                assert_eq!(byte, [0x5a]);
            }
            assert!(props.read_next_property().unwrap());
            assert_eq!(props.read_value().unwrap(), Value::I32(55));
            assert!(!props.read_next_property().unwrap());
        }
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    }
}

#[test]
fn skipped_named_properties_charge_the_shared_budget() {
    let mut named = fixed(0x8000, Type::LONG, &[0; 16]);
    dword(&mut named, 1);
    variable(&mut named, &unicode("name\0"));
    dword(&mut named, 7);
    let leaf = document(Attribute::MAPI_PROPERTIES, &properties(&[named]));
    let mut input = document(Attribute::BODY, b"xy");
    attribute(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[nested(&leaf)]),
    );
    attachment_attribute(&mut input, Attribute::ATTACH_TITLE, b"zz");
    attachment_attribute(&mut input, Attribute::ATTACH_TITLE, b"x");
    let options = TnefReaderOptions {
        max_total_data_bytes: 14,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_value_bytes().unwrap(), b"xy");
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let mut child = props.open_embedded_message().unwrap();
        assert!(child.read_next().unwrap());
        {
            let mut props = child.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            assert_eq!(
                props.property().unwrap().name.as_ref().unwrap().name,
                TnefName::String("name".into())
            );
            assert!(!props.read_next_property().unwrap());
        }
        assert!(!child.read_next().unwrap());
    }
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_value_bytes().unwrap(), b"zz");
    assert!(reader.read_next().unwrap());
    assert!(matches!(
        reader.read_value_bytes(),
        Err(Error::ValueLimitExceeded)
    ));
    assert!(!reader.is_faulted());
    assert!(!reader.read_next().unwrap());
}

#[test]
fn multivalued_object_iids_do_not_leak_between_values() {
    let leaf = document(Attribute::BODY, b"leaf");
    let mut payload = IID.to_vec();
    payload.extend_from_slice(&leaf);
    let mut values = fixed(0x3701, Type(Type::OBJECT.0 | Type::MULTI_VALUED.0), &[]);
    dword(&mut values, 3);
    variable(&mut values, &payload);
    variable(&mut values, &[0x55; 16]);
    variable(&mut values, &[]);
    let input = document(Attribute::ATTACHMENT, &properties(&[values]));
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    {
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        assert!(props.is_embedded_message());
        walk(&mut props.open_embedded_message().unwrap(), &mut Vec::new());
        assert!(props.read_next_value().unwrap());
        assert!(!props.is_embedded_message());
        assert_eq!(props.read_value_bytes().unwrap(), [0x55; 16]);
        assert!(props.read_next_value().unwrap());
        assert!(!props.is_embedded_message());
        assert_eq!(props.read_value_bytes().unwrap(), []);
        assert!(!props.read_next_value().unwrap());
        assert!(!props.is_embedded_message());
    }
    assert!(!reader.read_next().unwrap());
}

// Adapted from MimeKit's TnefReaderTests, TnefReaderRobustnessTests,
// TnefAttributeLevelTests, TnefReaderStreamTests, TnefChecksumTests,
// and TnefStreamOffsetTests. Fixtures retain their original MimeKit bytes;
// see tests/data/LICENSE-MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read};

use mimecraft::tnef::{
    TnefAttribute, TnefAttributeLevel as Level, TnefAttributeTag as Tag, TnefAttributeType as Type,
    TnefComplianceIssue as Issue, TnefComplianceViolation as Violation, TnefReadError as Error,
    TnefReader, TnefReaderOptions,
};

fn stream() -> Vec<u8> {
    vec![0x78, 0x9f, 0x3e, 0x22, 0x34, 0x12]
}

fn attribute(output: &mut Vec<u8>, level: u8, tag: Tag, value: &[u8]) -> usize {
    let offset = output.len();
    output.push(level);
    output.extend_from_slice(&tag.0.to_le_bytes());
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
    let sum = value
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
    output.extend_from_slice(&sum.to_le_bytes());
    offset
}

struct Chunked<'a> {
    input: &'a [u8],
    chunk: usize,
    calls: usize,
    ended: bool,
    interrupt: bool,
}

impl<'a> Chunked<'a> {
    fn new(input: &'a [u8], chunk: usize) -> Self {
        Self {
            input,
            chunk,
            calls: 0,
            ended: false,
            interrupt: false,
        }
    }
}

impl Read for Chunked<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(!self.ended, "source was read after EOF");
        self.calls += 1;
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = output.len().min(self.chunk).min(self.input.len());
        output[..count].copy_from_slice(&self.input[..count]);
        self.input = &self.input[count..];
        self.ended = count == 0;
        Ok(count)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct Scan {
    values: Vec<(TnefAttribute, Vec<u8>)>,
    issues: Vec<Issue>,
    position: u64,
    truncated: bool,
}

fn scan(input: &[u8], chunk: usize, value_chunk: usize) -> Scan {
    let mut result = Scan {
        values: Vec::new(),
        issues: Vec::new(),
        position: 0,
        truncated: false,
    };
    // Malformed corpus fixtures advertise huge values. Permit collection to
    // test truncation recovery; the reader must still avoid upfront allocation.
    let options = TnefReaderOptions {
        max_value_length: usize::MAX,
        max_total_data_bytes: u64::MAX,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(Chunked::new(input, chunk), &options).unwrap();
    assert_eq!(reader.get_ref().calls, 0, "construction performed I/O");
    let mut logger = |issue| {
        result.issues.push(issue);
        Ok(())
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        let metadata = *reader.attribute().unwrap();
        let mut bytes = Vec::new();
        if value_chunk == usize::MAX {
            bytes = reader.read_value_bytes_with_logger(&mut logger).unwrap();
        } else if value_chunk != 0 {
            let mut value = reader.open_value_with_logger(&mut logger).unwrap();
            let mut buffer = vec![0; value_chunk];
            assert_eq!(value.read(&mut []).unwrap(), 0);
            loop {
                let read = value.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
            }
            assert_eq!(value.read(&mut buffer).unwrap(), 0);
        }
        result.values.push((metadata, bytes));
    }
    assert!(reader.is_finished());
    assert!(reader.attribute().is_none());
    let calls = reader.get_ref().calls;
    for _ in 0..3 {
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    }
    assert_eq!(reader.get_ref().calls, calls);
    result.position = reader.position();
    result.truncated = reader.is_truncated();
    result
}

#[test]
fn signatures_keys_and_header_truncation_offsets() {
    let header = stream();
    for length in 0..6 {
        let result = scan(&header[..length], 1, 0);
        assert!(result.truncated);
        assert_eq!(
            result.issues,
            [Issue {
                violation: Violation::TruncatedStream,
                depth: 0,
                stream_offset: if length < 4 { 0 } else { 4 },
                attribute_tag: None,
                property_tag: None,
            }]
        );
    }
    let result = scan(b"not tnef", 1, 0);
    assert_eq!(result.position, 0);
    assert!(!result.truncated);
    assert_eq!(result.issues[0].violation, Violation::InvalidSignature);
    let mut reader = TnefReader::new(header.as_slice());
    assert_eq!(reader.legacy_key(), None);
    assert!(!reader.read_next().unwrap());
    assert_eq!(reader.legacy_key(), Some(0x1234));
    assert_eq!(reader.position(), 6);
    for extra in 1..9 {
        let mut data = header.clone();
        data.extend(std::iter::repeat_n(0, extra));
        let result = scan(&data, 2, 0);
        assert_eq!(result.issues[0].stream_offset, 6);
        assert_eq!(result.issues[0].attribute_tag, None);
    }
}

#[test]
fn option_validation_empty_values_and_preflight_errors() {
    let default = TnefReaderOptions::default();
    assert_eq!(default.max_value_length, 32 * 1024 * 1024);
    assert_eq!(default.max_total_data_bytes, 64 * 1024 * 1024);
    assert_eq!(default.default_codepage, 0);
    assert_eq!(default.max_issues_per_violation, 0);
    let mut options = TnefReaderOptions {
        default_codepage: 99999,
        ..default
    };
    assert!(matches!(
        TnefReader::with_options(Chunked::new(&[], 1), &options),
        Err(Error::UnsupportedCodepage(99999))
    ));
    options.default_codepage = 65001;
    options.max_value_length = 0;
    options.max_total_data_bytes = 0;
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, b"");
    attribute(&mut data, 1, Tag::DATE_SENT, b"raw date");
    let mut reader = TnefReader::with_options(Chunked::new(&data, 1), &options).unwrap();
    options.default_codepage = 1252;
    assert_eq!(reader.codepage(), 65001);
    assert_ne!(reader.options().default_codepage, options.default_codepage);
    assert!(matches!(reader.read_value_bytes(), Err(Error::NoAttribute)));
    assert!(matches!(
        reader.read_value_string(),
        Err(Error::NoAttribute)
    ));
    assert!(matches!(reader.open_value(), Err(Error::NoAttribute)));
    assert_eq!(reader.get_ref().calls, 0);
    reader.read_next().unwrap();
    let bytes = reader.read_value_bytes().unwrap();
    assert!(bytes.is_empty());
    assert_eq!(bytes.capacity(), 0);
    reader.read_next().unwrap();
    assert!(matches!(
        reader.read_value_string(),
        Err(Error::InvalidValueType(Tag::DATE_SENT))
    ));
    assert!(!reader.is_faulted());
    let mut bytes = Vec::new();
    reader
        .open_value()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"raw date");
    assert!(!reader.read_next().unwrap());
}

#[test]
fn raw_stream_respects_destination_slice_and_drop_performs_no_io() {
    let payload: Vec<_> = (0..64).collect();
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, &payload);
    attribute(&mut data, 1, Tag::SUBJECT, b"next");
    let mut reader = TnefReader::new(Chunked::new(&data, 4096));
    reader.read_next().unwrap();
    let calls = reader.get_ref().calls;
    let mut buffer = [0xaa; 256];
    {
        let mut value = reader.open_value().unwrap();
        assert_eq!(value.read(&mut buffer[100..164]).unwrap(), 64);
        assert_eq!(value.read(&mut buffer[100..164]).unwrap(), 0);
    }
    assert_eq!(reader.get_ref().calls, calls);
    assert_eq!(&buffer[100..164], payload);
    assert!(
        buffer[..100]
            .iter()
            .chain(&buffer[164..])
            .all(|&b| b == 0xaa)
    );
    assert_eq!(reader.position(), 79);
    assert_eq!(reader.value_remaining(), 0);
    assert!(reader.read_next().unwrap());
    let calls = reader.get_ref().calls;
    let position = reader.position();
    {
        let _value = reader.open_value().unwrap();
    }
    assert_eq!(reader.position(), position);
    assert_eq!(reader.get_ref().calls, calls);
    assert!(!reader.read_next().unwrap());
}

#[test]
fn signed_lengths_and_unknown_tags_are_preserved() {
    for length in [0x80000000u32, u32::MAX] {
        let mut data = stream();
        attribute(&mut data, 1, Tag::OWNER, b"");
        data[11..15].copy_from_slice(&length.to_le_bytes());
        let result = scan(&data, 1, 0);
        assert!(result.values.is_empty());
        assert_eq!(result.position, 15);
        assert_eq!(
            result.issues[0].violation,
            Violation::InvalidAttributeLength
        );
        assert_eq!(result.issues[0].stream_offset, 6);
        assert!(!result.truncated);
    }
    let mut data = stream();
    let tag = Tag(0xdeadbeef);
    attribute(&mut data, 0x7f, tag, b"unknown");
    let result = scan(&data, 1, 3);
    assert_eq!(result.values[0].0.tag, tag);
    assert_eq!(result.values[0].0.level, Level::Unknown(0x7f));
    assert_eq!(tag.attribute_type(), Type::Unknown(0xdead));
    assert_eq!(result.values[0].1, b"unknown");
    assert_eq!(
        result
            .issues
            .iter()
            .map(|issue| issue.violation)
            .collect::<Vec<_>>(),
        [
            Violation::InvalidAttributeLevel,
            Violation::UnknownAttribute
        ]
    );
}

#[test]
fn levels_ordering_and_null_exemption() {
    let attachment_tags = [
        Tag::ATTACH_CREATE_DATE,
        Tag::ATTACH_DATA,
        Tag::ATTACHMENT,
        Tag::ATTACH_META_FILE,
        Tag::ATTACH_MODIFY_DATE,
        Tag::ATTACH_RENDER_DATA,
        Tag::ATTACH_TITLE,
        Tag::ATTACH_TRANSPORT_FILENAME,
    ];
    let message_tags = [
        Tag::OWNER,
        Tag::SENT_FOR,
        Tag::DELEGATE,
        Tag::ORIGINAL_MESSAGE_CLASS,
        Tag::DATE_START,
        Tag::DATE_END,
        Tag::AID_OWNER,
        Tag::REQUEST_RESPONSE,
        Tag::FROM,
        Tag::SUBJECT,
        Tag::DATE_SENT,
        Tag::DATE_RECEIVED,
        Tag::MESSAGE_STATUS,
        Tag::MESSAGE_CLASS,
        Tag::MESSAGE_ID,
        Tag::PARENT_ID,
        Tag::CONVERSATION_ID,
        Tag::BODY,
        Tag::PRIORITY,
        Tag::DATE_MODIFIED,
        Tag::MAPI_PROPERTIES,
        Tag::RECIPIENT_TABLE,
        Tag::TNEF_VERSION,
        Tag::OEM_CODEPAGE,
    ];
    for (tags, valid_level) in [(&attachment_tags[..], 2), (&message_tags[..], 1)] {
        for &tag in tags {
            assert!(tag.is_known());
            for level in [1, 2, 0x7f] {
                let mut data = stream();
                attribute(&mut data, level, tag, b"IPM.Note\0");
                let result = scan(&data, 3, 0);
                assert_eq!(
                    result
                        .issues
                        .iter()
                        .any(|issue| issue.violation == Violation::AttributeLevelMismatch),
                    level != valid_level && level != 0x7f,
                    "{tag:?}/{level}"
                );
            }
        }
    }
    let mut data = stream();
    attribute(&mut data, 1, Tag::NULL, b"");
    attribute(&mut data, 2, Tag::NULL, b"");
    let offset = attribute(&mut data, 1, Tag::SUBJECT, b"subject\0");
    let result = scan(&data, 1, 0);
    // Once attachment-level attributes have begun, message level cannot resume.
    assert_eq!(
        result.issues,
        [Issue {
            violation: Violation::MessageAttributeAfterAttachment,
            depth: 0,
            stream_offset: offset as u64,
            attribute_tag: Some(Tag::SUBJECT),
            property_tag: None,
        }]
    );
}

#[test]
fn message_classes_length_ascii_and_terminal_nul() {
    for tag in [Tag::MESSAGE_CLASS, Tag::ORIGINAL_MESSAGE_CLASS] {
        for (value, valid) in [
            (b"IPM.Note\0".to_vec(), true),
            (b"IPM.Note".to_vec(), true),
            (vec![b'x'; 255], true),
            (vec![b'x'; 256], false),
            (vec![], false),
            (vec![0], true),
            (b"a\0b".to_vec(), false),
            (b"a\0\0".to_vec(), false),
            (vec![0x1f], false),
            (vec![0x7f], false),
            (vec![0x80], false),
            (b" !~".to_vec(), true),
        ] {
            let mut data = stream();
            attribute(&mut data, 1, tag, &value);
            for chunk in [1, 7, 4096] {
                let result = scan(&data, chunk, 73);
                assert_eq!(result.issues.is_empty(), valid, "{value:?}");
                assert_eq!(result.values[0].1, value);
                if !valid {
                    assert_eq!(result.issues[0].violation, Violation::InvalidMessageClass);
                }
            }
        }
    }
}

#[test]
fn codepage_peeks_strings_and_short_values_do_not_cross_boundaries() {
    let mut data = stream();
    for (page, text) in [
        (1251u32, &[0xcf, 0xf0][..]),
        (0, &[0xcf, 0xf0]),
        (99999, &[0xcf, 0xf0]),
        (65001, b"UTF8"),
    ] {
        attribute(&mut data, 1, Tag::OEM_CODEPAGE, &page.to_le_bytes());
        attribute(&mut data, 1, Tag::SUBJECT, text);
    }
    let mut reader = TnefReader::new(Chunked::new(&data, 1));
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    for (page, text) in [
        (1251, "\u{41f}\u{440}"),
        (1251, "\u{41f}\u{440}"),
        (1251, "\u{41f}\u{440}"),
        (65001, "UTF8"),
    ] {
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(
            reader.codepage(),
            page,
            "OEM codepage must apply before consuming its value"
        );
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(reader.read_value_string().unwrap(), text);
    }
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidMessageCodepage);
    for tag in [Tag::OEM_CODEPAGE, Tag::TNEF_VERSION] {
        for length in 0..4 {
            let mut data = stream();
            attribute(&mut data, 1, tag, &vec![0; length]);
            attribute(&mut data, 1, Tag::SUBJECT, b"next\0\0");
            let result = scan(&data, 1, 1);
            assert_eq!(result.values[1].1, b"next\0\0");
            assert_eq!(result.issues.len(), 1);
            assert_eq!(result.issues[0].violation, Violation::InvalidAttributeValue);
        }
    }
    let mut data = stream();
    attribute(&mut data, 1, Tag::TNEF_VERSION, &0x10000u32.to_le_bytes());
    attribute(&mut data, 1, Tag::TNEF_VERSION, &0x20000u32.to_le_bytes());
    assert_eq!(
        scan(&data, 1, 0).issues[0].violation,
        Violation::UnsupportedVersion
    );
    let mut data = stream();
    attribute(&mut data, 1, Tag::SUBJECT, b"\x80\0\0");
    let mut reader = TnefReader::new(data.as_slice());
    reader.read_next().unwrap();
    assert_eq!(reader.read_value_string().unwrap(), "\u{20ac}");
}

#[test]
fn all_truncations_and_chunk_boundaries() {
    let mut data = stream();
    attribute(&mut data, 1, Tag::TNEF_VERSION, &0x10000u32.to_le_bytes());
    attribute(&mut data, 1, Tag::OEM_CODEPAGE, &1252u32.to_le_bytes());
    attribute(&mut data, 1, Tag::MESSAGE_CLASS, b"IPM.Note\0");
    attribute(&mut data, 1, Tag::SUBJECT, b"example\0");
    for length in 0..=data.len() {
        let input = &data[..length];
        let expected = scan(input, 4096, usize::MAX);
        assert!(
            expected
                .issues
                .iter()
                .filter(|issue| issue.violation == Violation::TruncatedStream)
                .count()
                <= 1
        );
        assert!(!expected.issues.iter().any(|issue| matches!(
            issue.violation,
            Violation::InvalidMessageClass | Violation::AttributeChecksumMismatch
        )));
        for chunk in [1, 2, 3, 7, 9, 16] {
            for value_chunk in [1, 73, 8192, usize::MAX] {
                assert_eq!(
                    scan(input, chunk, value_chunk),
                    expected,
                    "length={length}, chunk={chunk}, value={value_chunk}"
                );
            }
            let skipped = expected.clone_without_values();
            assert_eq!(scan(input, chunk, 0), skipped);
        }
    }
}

impl Scan {
    fn clone_without_values(&self) -> Self {
        Self {
            values: self
                .values
                .iter()
                .map(|(attribute, _)| (*attribute, Vec::new()))
                .collect(),
            issues: self.issues.clone(),
            position: self.position,
            truncated: self.truncated,
        }
    }
}

#[test]
fn checksums_streaming_skipping_and_partial_drops() {
    let lengths = [
        0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 23, 24, 31, 32, 33, 47, 63, 64, 65, 95, 127, 128,
        129, 255, 256, 257, 1023, 1024, 1025, 4095, 4096, 4097, 4223, 4224, 4225, 8191, 8192, 8193,
        10000, 65535, 65536, 65537, 131073,
    ];
    for length in lengths {
        let payload: Vec<_> = (0..length).map(|i| (i & 255) as u8).collect();
        let mut data = stream();
        attribute(&mut data, 1, Tag::BODY, &payload);
        attribute(&mut data, 1, Tag::SUBJECT, b"next");
        for chunk in [1, 73, 4096, 8192] {
            let good = scan(&data, chunk, 73);
            assert!(good.issues.is_empty(), "{length}/{chunk}");
            assert_eq!(good.values[0].1, payload);
            assert_eq!(good.values[1].1, b"next");
        }
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        {
            let mut value = reader.open_value().unwrap();
            let mut buffer = [0; 3];
            let count = value.read(&mut buffer).unwrap();
            assert_eq!(&buffer[..count], &payload[..count]);
        }
        assert!(matches!(reader.open_value(), Err(Error::ValueAlreadyRead)));
        let mut reject = |issue: Issue| -> io::Result<()> { panic!("{issue:?}") };
        assert!(reader.read_next_with_logger(&mut reject).unwrap());
        assert_eq!(reader.read_value_bytes().unwrap(), b"next");
        assert!(!reader.read_next_with_logger(&mut reject).unwrap());
        data[15 + length] ^= 1;
        for mode in [0, 73, 8192, usize::MAX] {
            let bad = scan(&data, 4096, mode);
            assert_eq!(
                bad.issues,
                [Issue {
                    violation: Violation::AttributeChecksumMismatch,
                    depth: 0,
                    stream_offset: 6,
                    attribute_tag: Some(Tag::BODY),
                    property_tag: None,
                }]
            );
            assert_eq!(bad.values.len(), 2);
        }
    }
}

#[test]
fn scalar_types_caching_claims_and_short_values() {
    for (tag, value, expected) in [
        (Tag::PRIORITY, vec![0xff, 0xff], -1),
        (Tag::ORIGINAL_MESSAGE_CLASS, vec![0xff, 0xff], 65535),
        (Tag::AID_OWNER, (-123i32).to_le_bytes().to_vec(), -123),
        (Tag::TNEF_VERSION, u32::MAX.to_le_bytes().to_vec(), -1),
    ] {
        let mut data = stream();
        attribute(&mut data, 1, tag, &value);
        let mut reader = TnefReader::new(Chunked::new(&data, 1));
        reader.read_next().unwrap();
        assert_eq!(reader.read_value_i32().unwrap(), expected);
        let calls = reader.get_ref().calls;
        assert_eq!(reader.read_value_i32().unwrap(), expected);
        assert_eq!(reader.get_ref().calls, calls);
        assert!(matches!(
            reader.read_value_bytes(),
            Err(Error::ValueAlreadyRead)
        ));
        assert!(!reader.read_next().unwrap());
        assert!(matches!(reader.read_value_i32(), Err(Error::NoAttribute)));
    }
    let mut data = stream();
    attribute(&mut data, 1, Tag::AID_OWNER, b"x");
    let mut reader = TnefReader::new(data.as_slice());
    reader.read_next().unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert_eq!(reader.read_value_i32_with_logger(&mut logger).unwrap(), 0);
    assert_eq!(reader.read_value_i32_with_logger(&mut logger).unwrap(), 0);
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidAttributeValue);
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, b"text");
    let mut reader = TnefReader::new(data.as_slice());
    reader.read_next().unwrap();
    assert!(matches!(
        reader.read_value_i32(),
        Err(Error::InvalidValueType(Tag::BODY))
    ));
    assert_eq!(reader.read_value_string().unwrap(), "text");
    let mut data = stream();
    attribute(&mut data, 1, Tag::AID_OWNER, &123i32.to_le_bytes());
    for length in 15..19 {
        let mut reader = TnefReader::new(Chunked::new(&data[..length], 1));
        reader.read_next().unwrap();
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert_eq!(reader.read_value_i32_with_logger(&mut logger).unwrap(), 0);
        assert_eq!(reader.read_value_i32_with_logger(&mut logger).unwrap(), 0);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::TruncatedStream);
        assert_eq!(issues[0].stream_offset, length as u64);
    }
}

#[test]
fn allocation_limits_are_recoverable_and_do_not_limit_streaming() {
    let options = TnefReaderOptions {
        max_value_length: 4,
        max_total_data_bytes: 6,
        ..Default::default()
    };
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, b"12345");
    attribute(&mut data, 1, Tag::BODY, b"1234");
    attribute(&mut data, 1, Tag::BODY, b"123");
    attribute(&mut data, 1, Tag::BODY, b"12");
    let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    reader.read_next().unwrap();
    let position = reader.position();
    assert!(matches!(
        reader.read_value_bytes_with_logger(&mut logger),
        Err(Error::ValueLimitExceeded)
    ));
    assert_eq!(reader.position(), position);
    assert!(!reader.is_faulted());
    let mut bytes = Vec::new();
    reader
        .open_value()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"12345");
    reader.read_next().unwrap();
    assert_eq!(reader.read_value_bytes().unwrap(), b"1234");
    reader.read_next().unwrap();
    assert!(matches!(
        reader.read_value_string_with_logger(&mut logger),
        Err(Error::ValueLimitExceeded)
    ));
    reader.read_next().unwrap();
    assert_eq!(reader.read_value_string().unwrap(), "12");
    assert!(!reader.read_next().unwrap());
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .all(|issue| issue.violation == Violation::DataSizeLimitExceeded)
    );
    let options = TnefReaderOptions {
        max_value_length: usize::MAX,
        max_total_data_bytes: u64::MAX,
        ..Default::default()
    };
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, b"x");
    data[11..15].copy_from_slice(&i32::MAX.to_le_bytes());
    data.truncate(16);
    let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
    reader.read_next().unwrap();
    let bytes = reader.read_value_bytes().unwrap();
    assert_eq!(bytes, b"x");
    assert!(
        bytes.capacity() <= 65536,
        "unverified length allocated upfront"
    );
    assert!(reader.is_truncated());
}

#[test]
fn logger_caps_error_identity_and_terminal_faults() {
    let mut data = stream();
    for _ in 0..4 {
        attribute(&mut data, 1, Tag(0xdeadbeef), b"");
    }
    let options = TnefReaderOptions {
        max_issues_per_violation: 1,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
    reader.read_next().unwrap(); // Discarded reports do not consume the cap.
    let mut issues = Vec::new();
    while reader
        .read_next_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap()
    {}
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].stream_offset, 17);
    let mut reader = TnefReader::new(Chunked::new(&data, 1));
    let error = reader
        .read_next_with_logger(&mut |_| Err(io::Error::from_raw_os_error(123)))
        .unwrap_err();
    match error {
        Error::Io(error) => assert_eq!(error.raw_os_error(), Some(123)),
        other => panic!("{other}"),
    }
    assert!(reader.is_faulted());
    let calls = reader.get_ref().calls;
    assert!(matches!(reader.read_next(), Err(Error::Faulted)));
    assert!(matches!(reader.read_value_bytes(), Err(Error::Faulted)));
    assert!(matches!(reader.open_value(), Err(Error::Faulted)));
    assert_eq!(reader.get_ref().calls, calls);

    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, b"abc");
    data.truncate(16);
    let mut reader = TnefReader::new(data.as_slice());
    reader.read_next().unwrap();
    let mut logger = |_| Err(io::Error::from_raw_os_error(321));
    {
        let mut value = reader.open_value_with_logger(&mut logger).unwrap();
        let mut buffer = [0; 16];
        assert_eq!(value.read(&mut buffer).unwrap(), 1);
        assert_eq!(
            value.read(&mut buffer).unwrap_err().raw_os_error(),
            Some(321)
        );
        assert!(value.read(&mut buffer).is_err());
    }
    assert!(reader.is_faulted());
}

#[test]
fn interrupted_source_read_ahead_and_source_errors() {
    let mut data = stream();
    attribute(&mut data, 1, Tag::BODY, &vec![0xab; 9000]);
    let mut source = Chunked::new(&data, 1);
    source.interrupt = true;
    let mut reader = TnefReader::new(source);
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.position(), 15);

    let mut reader = TnefReader::new(data.as_slice());
    reader.read_next().unwrap();
    let position = reader.position() as usize;
    assert_eq!(position, 15);
    let (source, unread) = reader.into_parts();
    assert_eq!([unread.as_slice(), source].concat(), data[position..]);

    struct FailsAfter<'a>(&'a [u8]);
    impl Read for FailsAfter<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Err(io::Error::from_raw_os_error(456));
            }
            self.0.read(buffer)
        }
    }
    // Fail in every framing position, inside values, and on direct large reads.
    for length in [0, 1, 3, 4, 5, 6, 7, 14, 15, 16, 4096, 8192, 9015, 9016] {
        for mode in [0, 1, 8192] {
            let mut reader = TnefReader::new(FailsAfter(&data[..length]));
            let result = (|| -> Result<(), Error> {
                while reader.read_next()? {
                    if mode != 0 {
                        let mut value = reader.open_value()?;
                        let mut buffer = vec![0; mode];
                        while value.read(&mut buffer)? != 0 {}
                    }
                }
                Ok(())
            })();
            match result.unwrap_err() {
                Error::Io(error) => assert_eq!(error.raw_os_error(), Some(456)),
                other => panic!("{other}"),
            }
            assert!(reader.is_faulted());
            assert!(matches!(reader.read_next(), Err(Error::Faulted)));
        }
    }
}

#[test]
fn fixture_corpus_matches_independent_wire_slices() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 19);
    for file in files {
        let input = std::fs::read(&file).unwrap();
        let expected = scan(&input, 4096, usize::MAX);
        let mut offset = 6;
        let mut bad_checksums = Vec::new();
        for (metadata, value) in &expected.values {
            let header = &input[offset..offset + 9];
            assert_eq!(metadata.offset, offset as u64, "{file:?}");
            assert_eq!(metadata.level, Level::from(header[0]));
            assert_eq!(
                metadata.tag.0,
                u32::from_le_bytes(header[1..5].try_into().unwrap())
            );
            let length = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
            assert_eq!(metadata.length as usize, length);
            let end = (offset + 9 + length).min(input.len());
            assert_eq!(&input[offset + 9..end], value, "{file:?}");
            if end + 2 <= input.len() && value.len() == length {
                let sum = value
                    .iter()
                    .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
                if sum.to_le_bytes() != input[end..end + 2] {
                    bad_checksums.push(offset as u64);
                }
            }
            offset += 9 + length + 2;
        }
        assert_eq!(
            expected
                .issues
                .iter()
                .filter(|issue| issue.violation == Violation::AttributeChecksumMismatch)
                .map(|issue| issue.stream_offset)
                .collect::<Vec<_>>(),
            bad_checksums,
            "{file:?}",
        );
        // If parsing stopped before EOF, the remaining framing must be partial
        // or impossible, not a valid attribute silently dropped by the reader.
        if offset + 9 <= input.len() {
            assert!(i32::from_le_bytes(input[offset + 5..offset + 9].try_into().unwrap()) < 0);
        }
        for chunk in [1, 7, 4096] {
            assert_eq!(scan(&input, chunk, 73), expected, "{file:?}/{chunk}");
            assert_eq!(scan(&input, chunk, 8192), expected, "{file:?}/{chunk}");
            assert_eq!(
                scan(&input, chunk, 0),
                expected.clone_without_values(),
                "{file:?}/{chunk}"
            );
        }
    }
}

#[test]
#[ignore = "streams 2.6 GB through the checksum reader; run explicitly in release mode"]
fn offsets_do_not_wrap_past_two_gigabytes() {
    // Payloads contain zeros, so their checksums are also zero. Synthesize
    // them without allocating gigabytes, as MimeKit's HugeTnefStream does.
    let first = 1_600_000_000u32;
    let second = 1_000_000_000u32;
    let mut header1 = stream();
    header1.push(1);
    header1.extend_from_slice(&Tag::OWNER.0.to_le_bytes());
    header1.extend_from_slice(&first.to_le_bytes());
    let mut header2 = vec![0, 0, 1];
    header2.extend_from_slice(&Tag::OWNER.0.to_le_bytes());
    header2.extend_from_slice(&second.to_le_bytes());
    let source = header1
        .as_slice()
        .chain(io::repeat(0).take(u64::from(first)))
        .chain(header2.as_slice())
        .chain(io::repeat(0).take(u64::from(second) + 2));
    let mut reader = TnefReader::new(source);
    let mut reject = |issue: Issue| -> io::Result<()> { panic!("{issue:?}") };
    assert!(reader.read_next_with_logger(&mut reject).unwrap());
    assert_eq!(reader.attribute().unwrap().offset, 6);
    assert!(reader.read_next_with_logger(&mut reject).unwrap());
    assert_eq!(reader.attribute().unwrap().offset, u64::from(first) + 17);
    assert!(!reader.read_next_with_logger(&mut reject).unwrap());
    assert_eq!(reader.position(), u64::from(first) + u64::from(second) + 28);
}

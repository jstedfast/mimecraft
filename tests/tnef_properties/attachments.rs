// Adapted from MimeKit's TnefMessageTests.TestAttachments and embedded-content tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    embedded::{IID, object, wrap},
    *,
};
use mimecraft::tnef::{
    TnefAttachFlags as Flags, TnefAttachMethod as Method, TnefAttachment as Attachment,
    TnefAttachmentContent as Content, TnefPropertySet, TnefValueConversionError as ConversionError,
};
use std::{borrow::Cow, io::Seek};

fn string(tag: Tag, value: &str) -> Vec<u8> {
    let raw = if tag.value_type() == Type::UNICODE {
        unicode(&format!("{value}\0"))
    } else {
        [value.as_bytes(), &[0]].concat()
    };
    object(tag.id(), tag.value_type(), &raw)
}

fn set(items: &[Vec<u8>]) -> TnefPropertySet {
    let input = document(Attribute::ATTACHMENT, &properties(items));
    let mut reader = TnefReader::new(Chunked {
        input: &input,
        chunk: 1,
    });
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

fn embedded_attachment(method: Option<i32>) -> Attachment {
    let input = wrap(&document(Attribute::BODY, b"nested"));
    let content = {
        let mut reader = TnefReader::new(input.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        props.read_attachment_content().unwrap()
    };
    let properties = method.map_or_else(TnefPropertySet::new, |value| {
        set(&[fixed(
            Tag::ATTACH_METHOD.id(),
            Type::LONG,
            &value.to_le_bytes(),
        )])
    });
    Attachment::new(properties, Some(content))
}

#[test]
fn upstream_attachment_metadata_and_content_vectors() {
    let first = Attachment::new(
        set(&[
            string(Tag::ATTACH_FILENAME_A, "LONGFI~1.TXT"),
            string(Tag::ATTACH_LONG_FILENAME_W, "long file name.txt"),
            string(Tag::ATTACH_MIME_TAG_W, "text/plain"),
            string(Tag::ATTACH_CONTENT_ID_W, "part1@example.com"),
            fixed(Tag::ATTACH_METHOD.id(), Type::LONG, &1i32.to_le_bytes()),
            fixed(Tag::ATTACH_FLAGS.id(), Type::LONG, &4i32.to_le_bytes()),
        ]),
        Some(Content::Binary(b"first".to_vec())),
    );
    assert_eq!(
        first.file_name().unwrap().as_deref(),
        Some("long file name.txt")
    );
    assert_eq!(first.mime_type().unwrap().as_deref(), Some("text/plain"));
    assert_eq!(
        first.content_id().unwrap().as_deref(),
        Some("part1@example.com")
    );
    assert_eq!(first.method(), Ok(Method::ByValue));
    assert_eq!(first.flags(), Ok(Flags::RENDERED_IN_BODY));
    assert!(!first.is_embedded_message());
    assert!(first.has_content());
    assert_eq!(first.len(), 5);
    assert_eq!(first.properties().len(), 6);
    assert!(
        first
            .properties()
            .get_by_id(Tag::ATTACH_DATA_BIN.id())
            .is_none()
    );
    let mut bytes = Vec::new();
    first.open_read().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"first");
    let second = Attachment::new(
        set(&[string(Tag::ATTACH_FILENAME_A, "second.bin")]),
        Some(Content::Binary(vec![1, 2, 3])),
    );
    assert_eq!(second.file_name().unwrap().as_deref(), Some("second.bin"));
    assert_eq!(second.mime_type(), Ok(None));
    assert_eq!(second.content_id(), Ok(None));
    assert_eq!(second.method(), Ok(Method::ByValue));
    assert_eq!(second.flags(), Ok(Flags::NONE));
    assert_eq!(second.as_bytes(), Some([1, 2, 3].as_slice()));
    assert!(matches!(
        second.open_embedded_reader(),
        Err(Error::NotEmbeddedMessage)
    ));
}

#[test]
fn methods_and_flags_preserve_unknown_values_and_bits() {
    for (number, expected) in [
        (0, Method::None),
        (1, Method::ByValue),
        (5, Method::EmbeddedMessage),
        (6, Method::Ole),
        (2, Method::Unknown(2)),
        (-1, Method::Unknown(-1)),
        (i32::MIN, Method::Unknown(i32::MIN)),
        (i32::MAX, Method::Unknown(i32::MAX)),
    ] {
        assert_eq!(Method::from(number), expected);
        assert_eq!(i32::from(expected), number);
        let attachment = Attachment::new(
            set(&[
                fixed(Tag::ATTACH_METHOD.id(), Type::LONG, &number.to_le_bytes()),
                fixed(Tag::ATTACH_FLAGS.id(), Type::LONG, &number.to_le_bytes()),
            ]),
            None,
        );
        assert_eq!(attachment.method(), Ok(expected));
        assert_eq!(attachment.flags().unwrap().bits(), number as u32);
        assert!(!attachment.is_embedded_message());
    }
    let both = Flags::INVISIBLE_IN_HTML | Flags::INVISIBLE_IN_RTF;
    assert_eq!(u32::from(both), 3);
    assert!(both.contains(Flags::INVISIBLE_IN_HTML));
    assert!(!both.contains(Flags::RENDERED_IN_BODY));
    assert!(both.intersects(Flags::INVISIBLE_IN_RTF | Flags::RENDERED_IN_BODY));
    assert_eq!(both & Flags::RENDERED_IN_BODY, Flags::NONE);
    assert_eq!(Flags::from(u32::MAX).bits(), u32::MAX);
    assert_eq!(Flags::default(), Flags::NONE);
    assert!(Flags::NONE.contains(Flags::NONE));
    assert!(!Flags::NONE.intersects(Flags::NONE));
}

#[test]
fn name_priority_empty_values_and_untrusted_metadata_are_preserved() {
    for (items, expected) in [
        (vec![string(Tag::ATTACH_FILENAME_W, "short")], Some("short")),
        (
            vec![
                string(Tag::ATTACH_FILENAME_W, "short"),
                string(Tag::ATTACH_LONG_FILENAME_A, ""),
            ],
            Some(""),
        ),
        (
            vec![
                string(Tag::ATTACH_LONG_FILENAME_A, "first"),
                string(Tag::ATTACH_LONG_FILENAME_W, "second"),
            ],
            Some("first"),
        ),
        (
            vec![string(Tag::ATTACH_LONG_FILENAME_W, "..\\untrusted.txt")],
            Some("..\\untrusted.txt"),
        ),
        (vec![], None),
    ] {
        let attachment = Attachment::new(set(&items), None);
        assert_eq!(attachment.file_name().unwrap().as_deref(), expected);
        if expected.is_some() {
            assert!(matches!(
                attachment.file_name().unwrap(),
                Some(Cow::Borrowed(_))
            ));
        }
    }
    let attachment = Attachment::new(
        set(&[
            string(Tag::ATTACH_MIME_TAG_W, " not/a-validated type "),
            string(Tag::ATTACH_CONTENT_ID_W, " <Part@EXAMPLE.COM> "),
        ]),
        None,
    );
    assert_eq!(
        attachment.mime_type().unwrap().as_deref(),
        Some(" not/a-validated type ")
    );
    assert_eq!(
        attachment.content_id().unwrap().as_deref(),
        Some(" <Part@EXAMPLE.COM> ")
    );
}

#[test]
fn binary_metadata_uses_captured_charset_and_errors_do_not_fall_back() {
    let properties = set(&[
        object(
            Tag::ATTACH_LONG_FILENAME_W.id(),
            Type::BINARY,
            b"Caf\xe9.txt\0",
        ),
        string(Tag::ATTACH_FILENAME_W, "fallback"),
    ]);
    let attachment = Attachment::new(properties, None);
    assert_eq!(
        attachment.file_name().unwrap().as_deref(),
        Some("Caf\u{e9}.txt")
    );
    assert!(matches!(
        attachment.file_name().unwrap(),
        Some(Cow::Owned(_))
    ));
    for tag in [
        Tag::ATTACH_LONG_FILENAME_W,
        Tag::ATTACH_MIME_TAG_W,
        Tag::ATTACH_CONTENT_ID_W,
    ] {
        let attachment = Attachment::new(
            set(&[
                fixed(tag.id(), Type::LONG, &5i32.to_le_bytes()),
                string(Tag::ATTACH_FILENAME_W, "fallback"),
            ]),
            None,
        );
        let result = if tag == Tag::ATTACH_LONG_FILENAME_W {
            attachment.file_name()
        } else if tag == Tag::ATTACH_MIME_TAG_W {
            attachment.mime_type()
        } else {
            attachment.content_id()
        };
        assert_eq!(result, Err(ConversionError::InvalidType));
    }
    for tag in [Tag::ATTACH_METHOD, Tag::ATTACH_FLAGS] {
        let attachment = Attachment::new(
            set(&[string(Tag::new(tag.id(), Type::UNICODE), "bad")]),
            None,
        );
        if tag == Tag::ATTACH_METHOD {
            assert_eq!(attachment.method(), Err(ConversionError::InvalidType));
        } else {
            assert_eq!(attachment.flags(), Err(ConversionError::InvalidType));
        }
    }
}

#[test]
fn content_distinguishes_absent_empty_binary_and_object_iids() {
    for length in [0, 1, 15, 16, 17, 32] {
        let bytes = vec![0x42; length];
        for object in [false, true] {
            let content = if object {
                Content::Object(bytes.clone())
            } else {
                Content::Binary(bytes.clone())
            };
            let attachment = Attachment::new(TnefPropertySet::new(), Some(content));
            let expected = if object {
                &bytes[length.min(16)..]
            } else {
                bytes.as_slice()
            };
            assert!(attachment.has_content());
            assert_eq!(attachment.as_bytes(), Some(expected));
            assert_eq!(attachment.len(), expected.len());
            assert_eq!(attachment.is_empty(), expected.is_empty());
            let mut first = attachment.open_read();
            let mut second = attachment.open_read();
            first.seek(io::SeekFrom::End(0)).unwrap();
            assert_eq!(second.position(), 0);
            let mut actual = Vec::new();
            second.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, expected);
            let (properties, content) = attachment.clone().into_parts();
            assert!(properties.is_empty());
            match content.unwrap() {
                Content::Binary(raw) | Content::Object(raw) => assert_eq!(raw, bytes),
                Content::Embedded(_) => panic!("unexpected embedded content"),
            }
        }
    }
    let absent = Attachment::new(TnefPropertySet::new(), None);
    assert!(!absent.has_content());
    assert!(absent.is_empty());
    assert_eq!(absent.as_bytes(), None);
    assert_eq!(absent.open_read().read(&mut [0]).unwrap(), 0);
    assert!(absent.content().is_none());
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Attachment>();
}

#[test]
fn embedded_attachments_survive_parents_and_open_independent_readers() {
    for method in [None, Some(1), Some(5), Some(6), Some(-1)] {
        let attachment = embedded_attachment(method);
        assert!(attachment.is_embedded_message());
        assert_eq!(
            attachment.method(),
            Ok(method.map_or(Method::EmbeddedMessage, Method::from))
        );
        assert_eq!(
            attachment.as_bytes().unwrap(),
            document(Attribute::BODY, b"nested")
        );
        let mut first = attachment.open_embedded_reader().unwrap();
        let mut second = attachment.open_embedded_reader().unwrap();
        assert_eq!(first.depth(), 1);
        let position = first.position();
        assert!(position > 0);
        assert!(first.read_next().unwrap());
        assert_eq!(first.read_body().unwrap().as_bytes(), b"nested");
        assert_eq!(second.position(), position);
        assert!(second.read_next().unwrap());
        assert_eq!(second.read_body().unwrap().as_bytes(), b"nested");
        assert!(!first.read_next().unwrap());
        assert!(!second.read_next().unwrap());
    }
}

#[test]
fn collection_classifies_only_current_embedded_context_and_claims_once() {
    let leaf = document(Attribute::BODY, b"nested");
    for kind in [Type::BINARY, Type::OBJECT] {
        for iid in [IID, [0; 16], [0x55; 16]] {
            for method_first in [false, true] {
                let mut value = iid.to_vec();
                value.extend_from_slice(&leaf);
                let method = fixed(Tag::ATTACH_METHOD.id(), Type::LONG, &5i32.to_le_bytes());
                let data = object(Tag::ATTACH_DATA_BIN.id(), kind, &value);
                let items = if method_first {
                    vec![method, data]
                } else {
                    vec![data, method]
                };
                let input = document(Attribute::ATTACHMENT, &properties(&items));
                let mut reader = TnefReader::new(input.as_slice());
                assert!(reader.read_next().unwrap());
                let mut props = reader.open_properties().unwrap();
                assert!(matches!(
                    props.read_attachment_content(),
                    Err(Error::NoPropertyValue)
                ));
                assert!(props.read_next_property().unwrap());
                if method_first {
                    assert!(props.read_next_property().unwrap());
                }
                let content = props.read_attachment_content().unwrap();
                assert!(matches!(
                    props.read_attachment_content(),
                    Err(Error::ValueAlreadyRead)
                ));
                let embedded = if kind == Type::OBJECT && iid != [0; 16] {
                    iid == IID
                } else {
                    method_first
                };
                assert_eq!(matches!(content, Content::Embedded(_)), embedded);
                assert_eq!(
                    content.as_bytes(),
                    if embedded || kind == Type::OBJECT {
                        leaf.as_slice()
                    } else {
                        value.as_slice()
                    }
                );
                if !method_first {
                    assert!(props.read_next_property().unwrap());
                }
                assert!(!props.read_next_property().unwrap());
                drop(props);
                assert!(!reader.read_next().unwrap());
            }
        }
    }
}

#[test]
fn invalid_attachment_tags_leave_values_available() {
    for tag in [
        Tag::BODY_A,
        Tag::SUBJECT_W,
        Tag::new(Tag::ATTACH_DATA_BIN.id(), Type::STRING8),
        Tag(Tag::ATTACH_DATA_BIN.0 | u32::from(Type::MULTI_VALUED.0)),
    ] {
        let raw = b"data";
        let mut item = object(tag.id(), tag.value_type(), raw);
        item[..4].copy_from_slice(&tag.0.to_le_bytes());
        let input = document(Attribute::ATTACHMENT, &properties(&[item]));
        let mut reader = TnefReader::new(input.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(props.read_next_property().unwrap());
        let position = props.position();
        assert!(
            matches!(props.read_attachment_content(), Err(Error::InvalidPropertyType(t)) if t == tag)
        );
        assert_eq!(props.position(), position);
        assert!(!props.is_value_consumed());
        assert_eq!(props.read_value_bytes().unwrap(), raw);
    }
    let input = document(Attribute::BODY, b"body");
    let mut reader = TnefReader::new(input.as_slice());
    assert!(matches!(
        reader.read_attachment_content(),
        Err(Error::NoAttribute)
    ));
    assert!(reader.read_next().unwrap());
    assert!(matches!(
        reader.read_attachment_content(),
        Err(Error::InvalidValueType(_))
    ));
    assert_eq!(reader.read_value_bytes().unwrap(), b"body");
}

#[test]
fn property_collection_counts_iids_and_nesting_limits_are_recoverable() {
    let input = wrap(&document(Attribute::BODY, b"nested"));
    for options in [
        TnefReaderOptions {
            max_value_length: 16,
            ..Default::default()
        },
        TnefReaderOptions {
            max_total_data_bytes: 16,
            ..Default::default()
        },
        TnefReaderOptions {
            max_nesting_depth: 0,
            ..Default::default()
        },
    ] {
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        assert!(reader.read_next().unwrap());
        let mut issues: Vec<Issue> = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        let result = props.read_attachment_content();
        assert!(matches!(
            result,
            Err(Error::ValueLimitExceeded | Error::NestingLimitExceeded)
        ));
        assert!(!props.is_value_consumed());
        let mut raw = Vec::new();
        props.open_value().unwrap().read_to_end(&mut raw).unwrap();
        assert_eq!(&raw[..16], &IID);
        drop(props);
        assert_eq!(issues.len(), 1);
        assert!(!reader.read_next().unwrap());
    }
}

#[test]
fn legacy_collection_truncation_limits_and_inherited_diagnostics() {
    let mut leaf = document(Attribute::ATTACH_DATA, b"first");
    attribute(&mut leaf, Attribute::ATTACH_DATA, b"second");
    let input = wrap(&leaf);
    let options = TnefReaderOptions {
        max_total_data_bytes: 5,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    let mut issues: Vec<Issue> = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    {
        let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
        assert!(props.read_next_property().unwrap());
        let mut child = props.open_embedded_message().unwrap();
        assert!(child.read_next().unwrap());
        assert_eq!(
            child.read_attachment_content().unwrap().as_bytes(),
            b"first"
        );
        assert!(child.read_next().unwrap());
        assert!(matches!(
            child.read_attachment_content(),
            Err(Error::ValueLimitExceeded)
        ));
    }
    assert!(
        issues
            .iter()
            .any(|issue| issue.violation == Violation::DataSizeLimitExceeded && issue.depth == 1)
    );
    let mut input = document(Attribute::ATTACH_DATA, b"first");
    input.truncate(input.len() - 4);
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert_eq!(
        reader
            .read_attachment_content_with_logger(&mut logger)
            .unwrap()
            .as_bytes(),
        b"fir"
    );
    assert!(reader.is_truncated());
    assert!(!issues.is_empty());
}

#[test]
fn collection_logger_failures_fault_instead_of_returning_absent_content() {
    let input = document(Attribute::ATTACH_DATA, b"first");
    let options = TnefReaderOptions {
        max_value_length: 1,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    let mut logger = |_| Err(io::Error::other("logger failed"));
    assert!(matches!(
        reader.read_attachment_content_with_logger(&mut logger),
        Err(Error::Io(_))
    ));
    assert!(matches!(reader.read_next(), Err(Error::Faulted)));
}

#[test]
fn source_failures_never_turn_into_missing_or_partial_content() {
    struct FailsAtEnd<'a>(&'a [u8]);
    impl Read for FailsAtEnd<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                Err(io::Error::other("source failed"))
            } else {
                self.0.read(output)
            }
        }
    }
    for legacy in [false, true] {
        let mut input = if legacy {
            document(Attribute::ATTACH_DATA, b"hello")
        } else {
            document(
                Attribute::ATTACHMENT,
                &properties(&[object(Tag::ATTACH_DATA_BIN.id(), Type::BINARY, b"hello")]),
            )
        };
        input.truncate(input.len() - if legacy { 5 } else { 8 });
        let mut reader = TnefReader::new(FailsAtEnd(&input));
        assert!(reader.read_next().unwrap());
        let result = if legacy {
            reader.read_attachment_content()
        } else {
            let mut props = reader.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            props.read_attachment_content()
        };
        assert!(matches!(result, Err(Error::Io(_))));
        assert!(matches!(reader.read_next(), Err(Error::Faulted)));
    }
}

#[test]
fn construction_moves_content_and_preserves_supplied_properties_verbatim() {
    let properties = set(&[
        object(
            Tag::ATTACH_DATA_BIN.id(),
            Type::BINARY,
            b"not implicitly selected",
        ),
        string(Tag::ATTACH_FILENAME_A, "first"),
        string(Tag::ATTACH_FILENAME_W, "duplicate"),
    ]);
    let original = properties.clone();
    let bytes = b"explicitly selected".to_vec();
    let pointer = bytes.as_ptr();
    let attachment = Attachment::new(properties, Some(Content::Binary(bytes)));
    assert_eq!(attachment.as_bytes().unwrap().as_ptr(), pointer);
    assert_eq!(attachment.properties(), &original);
    assert_eq!(attachment.file_name().unwrap().as_deref(), Some("first"));
    let (properties, content) = attachment.into_parts();
    assert_eq!(properties, original);
    assert_eq!(content.unwrap().as_bytes().as_ptr(), pointer);
}

#[test]
fn multivalued_metadata_is_an_error_not_an_absent_field() {
    for tag in [
        Tag::ATTACH_LONG_FILENAME_W,
        Tag::ATTACH_MIME_TAG_W,
        Tag::ATTACH_CONTENT_ID_W,
        Tag::ATTACH_METHOD,
        Tag::ATTACH_FLAGS,
    ] {
        let multi = Tag::new(tag.id(), Type(Type::LONG.0 | Type::MULTI_VALUED.0));
        let mut item = multi.0.to_le_bytes().to_vec();
        dword(&mut item, 1);
        dword(&mut item, 5);
        let attachment = Attachment::new(set(&[item]), None);
        if tag == Tag::ATTACH_METHOD {
            assert_eq!(attachment.method(), Err(ConversionError::MultiValued));
        } else if tag == Tag::ATTACH_FLAGS {
            assert_eq!(attachment.flags(), Err(ConversionError::MultiValued));
        } else {
            let result = if tag == Tag::ATTACH_LONG_FILENAME_W {
                attachment.file_name()
            } else if tag == Tag::ATTACH_MIME_TAG_W {
                attachment.mime_type()
            } else {
                attachment.content_id()
            };
            assert_eq!(result, Err(ConversionError::MultiValued));
        }
    }
}

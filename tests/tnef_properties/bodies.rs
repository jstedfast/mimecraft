// Body vectors adapted from MimeKit's TnefMessageTests.TestBodies/TestLegacyBody.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::{
    charsets::Charset,
    tnef::{
        RtfCompressionMode, RtfDecompressionError, TnefBodyError, TnefBodyReader,
        TnefMessageBody as Body, TnefMessageBodyFormat as Format,
    },
};
use std::{borrow::Cow, io::Seek};

#[path = "../../benches/support/rtf_compressed.rs"]
mod compression;

fn body_property(tag: Tag, bytes: &[u8]) -> Vec<u8> {
    let mut value = tag.0.to_le_bytes().to_vec();
    dword(&mut value, 1);
    variable(&mut value, bytes);
    value
}

pub(super) fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut builder = compression::Builder::new();
    builder.literals(bytes);
    builder.end();
    builder.build()
}

#[test]
fn upstream_body_vectors_survive_parent_and_late_codepage() {
    let rtf = compressed(b"{\\rtf1 Hello}");
    let bodies = {
        let input = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[
                body_property(Tag::BODY_W, &unicode("MAPI body\0\0")),
                body_property(Tag::BODY_HTML_B, "<p>caf\u{e9}</p>".as_bytes()),
                body_property(Tag::RTF_COMPRESSED, &rtf),
                fixed(
                    Tag::INTERNET_CODEPAGE.id(),
                    Type::LONG,
                    &65001i32.to_le_bytes(),
                ),
            ]),
        );
        let mut reader = TnefReader::new(Chunked {
            input: &input,
            chunk: 1,
        });
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        let mut bodies = Vec::new();
        for _ in 0..3 {
            assert!(props.read_next_property().unwrap());
            bodies.push(props.read_body().unwrap());
            assert!(matches!(props.read_body(), Err(Error::ValueAlreadyRead)));
        }
        assert!(props.read_next_property().unwrap());
        assert_eq!(props.read_value().unwrap(), Value::I32(65001));
        assert!(!props.read_next_property().unwrap());
        drop(props);
        assert!(!reader.read_next().unwrap());
        for body in &mut bodies {
            body.set_message_charsets(Charset::UTF8, Some(Charset::UTF8));
        }
        bodies
    };
    let [text, html, rtf_body] = bodies.as_slice() else {
        panic!("three bodies")
    };
    assert_eq!(text.format(), Format::Text);
    assert_eq!(text.tag(), Tag::BODY_W);
    assert_eq!(
        text.charset(),
        Some(Charset::from_label("utf-16le").unwrap())
    );
    assert_eq!(text.text().unwrap(), "MAPI body");
    assert_eq!(text.len(), 18);
    assert_eq!(html.format(), Format::Html);
    assert_eq!(html.tag(), Tag::BODY_HTML_B);
    assert_eq!(html.charset(), Some(Charset::UTF8));
    assert_eq!(html.text().unwrap(), "<p>caf\u{e9}</p>");
    assert_eq!(html.as_bytes(), "<p>caf\u{e9}</p>".as_bytes());
    assert_eq!(rtf_body.format(), Format::CompressedRtf);
    assert_eq!(rtf_body.charset(), None);
    assert_eq!(rtf_body.as_bytes(), rtf);
    assert_eq!(rtf_body.text().unwrap(), "{\\rtf1 Hello}");
}

#[test]
fn upstream_legacy_body_and_repeatable_independent_cursors() {
    let body = {
        let mut input = document(Attribute::OEM_CODEPAGE, &1252i32.to_le_bytes());
        attribute(&mut input, Attribute::BODY, b"Caf\xe9\0");
        let mut reader = TnefReader::new(input.as_slice());
        assert!(reader.read_next().unwrap());
        assert!(matches!(
            reader.read_body(),
            Err(Error::InvalidValueType(_))
        ));
        assert!(reader.read_next().unwrap());
        let body = reader.read_body().unwrap();
        assert!(matches!(reader.read_body(), Err(Error::ValueAlreadyRead)));
        assert!(!reader.read_next().unwrap());
        body
    };
    assert_eq!(body.tag(), Tag::BODY_A);
    assert_eq!(
        body.charset(),
        Some(Charset::from_label("windows-1252").unwrap())
    );
    assert_eq!(body.text().unwrap(), "Caf\u{e9}");
    assert_eq!(body.len(), 4);
    assert!(!body.is_empty());
    let mut first = body.open_read();
    let mut second = body.open_read();
    let mut byte = [0];
    first.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"C");
    second.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"C");
    first.seek(io::SeekFrom::End(-1)).unwrap();
    first.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [0xe9]);
    assert_eq!(second.position(), 1);
    let mut decoded = Vec::new();
    body.open_decoded_read().read_to_end(&mut decoded).unwrap();
    assert_eq!(decoded, b"Caf\xe9");
    assert_eq!(body.clone().text().unwrap(), body.text().unwrap());
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Body>();
    send_sync::<TnefBodyReader<'static>>();
}

#[test]
fn format_recognition_rejects_invalid_and_multivalued_tags() {
    for tag in [
        Tag::BODY_A,
        Tag::BODY_W,
        Tag::BODY_HTML_A,
        Tag::BODY_HTML_W,
        Tag::BODY_HTML_B,
        Tag::RTF_COMPRESSED,
    ] {
        assert!(Format::from_tag(tag).is_some());
        assert!(Body::new(tag, vec![], Charset::UTF8).unwrap().is_empty());
        let multi = Tag(tag.0 | u32::from(Type::MULTI_VALUED.0));
        assert_eq!(Format::from_tag(multi), None);
        assert!(matches!(Body::new(multi, vec![], Charset::UTF8),
            Err(Error::InvalidPropertyType(value)) if value == multi));
    }
    for tag in [
        Tag::SUBJECT_W,
        Tag::new(0x1000, Type::BINARY),
        Tag::new(0x1009, Type::STRING8),
        Tag::new(0x1013, Type::LONG),
    ] {
        assert_eq!(Format::from_tag(tag), None);
        assert!(matches!(Body::new(tag, vec![], Charset::UTF8),
            Err(Error::InvalidPropertyType(value)) if value == tag));
    }
}

#[test]
fn terminal_nuls_respect_complete_utf16_units_and_binary_bytes() {
    for (tag, bytes, expected) in [
        (Tag::BODY_A, b"a\0b\0\0".as_slice(), b"a\0b".as_slice()),
        (Tag::BODY_W, b"a\0\0\0\0\0", b"a\0"),
        (Tag::BODY_W, b"a\0\0\0\0", b"a\0\0\0\0"),
        (Tag::BODY_W, b"\0\0", b""),
        (Tag::BODY_A, b"\0\0", b""),
        (Tag::BODY_HTML_A, b"x\0", b"x"),
        (Tag::BODY_HTML_W, b"x\0\0\0", b"x\0"),
        (Tag::BODY_HTML_B, b"x\0\0", b"x\0\0"),
        (Tag::RTF_COMPRESSED, b"x\0\0", b"x\0\0"),
    ] {
        let body = Body::new(tag, bytes.to_vec(), Charset::UTF8).unwrap();
        assert_eq!(body.as_bytes(), expected);
        assert_eq!(body.len(), expected.len());
    }
}

#[test]
fn charset_updates_preserve_unicode_and_explicit_html_selection() {
    let cp1252 = Charset::from_label("windows-1252").unwrap();
    for tag in [Tag::BODY_A, Tag::BODY_HTML_A, Tag::BODY_HTML_B] {
        let mut body = Body::new(tag, b"caf\xc3\xa9".to_vec(), cp1252).unwrap();
        assert_ne!(body.text().unwrap(), "caf\u{e9}");
        body.set_message_charsets(Charset::UTF8, None);
        assert_eq!(body.text().unwrap(), "caf\u{e9}");
        assert_eq!(
            body.charset(),
            if tag == Tag::BODY_HTML_B {
                None
            } else {
                Some(Charset::UTF8)
            }
        );
        body.set_message_charsets(cp1252, Some(Charset::UTF8));
        assert_eq!(body.text().unwrap() == "caf\u{e9}", tag == Tag::BODY_HTML_B);
    }
    for tag in [Tag::BODY_W, Tag::BODY_HTML_W] {
        let mut body = Body::new(tag, unicode("caf\u{e9}"), cp1252).unwrap();
        body.set_message_charsets(Charset::UTF8, Some(cp1252));
        assert_eq!(body.text().unwrap(), "caf\u{e9}");
    }
    let mut rtf = Body::new(
        Tag::RTF_COMPRESSED,
        compressed(b"{\\rtf1 caf\xe9}"),
        Charset::UTF8,
    )
    .unwrap();
    assert!(matches!(rtf.text(), Err(TnefBodyError::Charset(_))));
    rtf.set_message_charsets(cp1252, Some(Charset::UTF8));
    assert_eq!(rtf.charset(), None);
    assert_eq!(rtf.text().unwrap(), "{\\rtf1 caf\u{e9}}");
}

#[test]
fn text_is_strict_or_explicitly_lossy_and_preserves_boms() {
    for (tag, bytes, expected) in [
        (Tag::BODY_A, b"a\xffb".to_vec(), "a\u{fffd}b"),
        (Tag::BODY_W, vec![0x00, 0xd8], "\u{fffd}"),
        (Tag::BODY_W, vec![0x61, 0, 0], "a\u{fffd}"),
        (Tag::RTF_COMPRESSED, compressed(b"a\xffb"), "a\u{fffd}b"),
    ] {
        let body = Body::new(tag, bytes, Charset::UTF8).unwrap();
        assert!(matches!(body.text(), Err(TnefBodyError::Charset(_))));
        assert_eq!(body.text_lossy().unwrap(), (Cow::Borrowed(expected), true));
    }
    for tag in [Tag::BODY_A, Tag::BODY_W] {
        let bytes = if tag == Tag::BODY_W {
            unicode("\u{feff}text")
        } else {
            "\u{feff}text".as_bytes().to_vec()
        };
        let body = Body::new(tag, bytes, Charset::UTF8).unwrap();
        assert_eq!(body.text().unwrap(), "\u{feff}text");
        assert!(!body.text_lossy().unwrap().1);
    }
    let body = Body::new(Tag::BODY_A, b"ascii".to_vec(), Charset::UTF8).unwrap();
    assert!(matches!(body.text().unwrap(), Cow::Borrowed("ascii")));
}

#[test]
fn compressed_workloads_support_repeatable_short_output_reads() {
    for name in compression::CASES {
        let case = compression::workload(name);
        case.check();
        let body = Body::new(Tag::RTF_COMPRESSED, case.input, Charset::UTF8).unwrap();
        for size in [1, 7, 4096] {
            let mut first = body.open_decoded_read();
            let mut second = body.open_decoded_read();
            let mut buffer = vec![0; size];
            let mut output = Vec::new();
            loop {
                let count = first.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(output, case.expected, "{name}/{size}");
            output.clear();
            second.read_to_end(&mut output).unwrap();
            assert_eq!(output, case.expected);
        }
        assert_eq!(body.text().unwrap().as_bytes(), case.expected);
    }
}

#[test]
fn invalid_compression_is_an_explicit_repeatable_eof_error() {
    let valid = compressed(b"{\\rtf1 Hello}");
    let mut crc = valid.clone();
    crc[12] ^= 1;
    let mut size = valid.clone();
    size[..4].copy_from_slice(&11i32.to_le_bytes());
    let mut truncated = valid.clone();
    truncated.truncate(20);
    let unknown = compression::document(b"abc", 3, RtfCompressionMode::Unknown(42), 0);
    for (bytes, expected) in [
        (vec![0; 15], RtfDecompressionError::IncompleteHeader),
        (size, RtfDecompressionError::InvalidCompressedSize(11)),
        (truncated, RtfDecompressionError::IncompleteContent),
        (
            unknown,
            RtfDecompressionError::UnsupportedCompressionMode(42),
        ),
        (
            crc.clone(),
            RtfDecompressionError::InvalidCrc32 {
                expected: u32::from_le_bytes(crc[12..16].try_into().unwrap()),
                actual: u32::from_le_bytes(valid[12..16].try_into().unwrap()),
            },
        ),
    ] {
        let body = Body::new(Tag::RTF_COMPRESSED, bytes.clone(), Charset::UTF8).unwrap();
        assert_eq!(body.as_bytes(), bytes);
        let mut reader = body.open_decoded_read();
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        let mut provisional = Vec::new();
        let error = reader.read_to_end(&mut provisional).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<RtfDecompressionError>(),
            Some(&expected)
        );
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(
            reader.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(matches!(body.text(), Err(TnefBodyError::Io(_))));
        assert!(body.text_lossy().is_err());
    }
}

#[test]
fn unsupported_property_collection_does_not_claim_bytes() {
    for tag in [
        Tag::SUBJECT_W,
        Tag(Tag::BODY_W.0 | u32::from(Type::MULTI_VALUED.0)),
    ] {
        let input = document(
            Attribute::MAPI_PROPERTIES,
            &properties(&[body_property(tag, &unicode("hello"))]),
        );
        let mut reader = TnefReader::new(input.as_slice());
        assert!(reader.read_next().unwrap());
        let mut props = reader.open_properties().unwrap();
        assert!(matches!(props.read_body(), Err(Error::NoPropertyValue)));
        assert!(props.read_next_property().unwrap());
        let position = props.position();
        assert!(matches!(props.read_body(), Err(Error::InvalidPropertyType(t)) if t == tag));
        assert_eq!(props.position(), position);
        assert!(!props.is_value_consumed());
        assert_eq!(props.read_value_bytes().unwrap(), unicode("hello"));
    }
}

#[test]
fn body_limits_charge_original_bytes_and_remain_recoverable() {
    for legacy in [false, true] {
        for total in [false, true] {
            let input = if legacy {
                document(Attribute::BODY, b"a\0\0")
            } else {
                document(
                    Attribute::MAPI_PROPERTIES,
                    &properties(&[body_property(Tag::BODY_A, b"a\0\0")]),
                )
            };
            let options = TnefReaderOptions {
                max_value_length: if total { 3 } else { 2 },
                max_total_data_bytes: if total { 2 } else { 3 },
                ..Default::default()
            };
            let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
            assert!(reader.read_next().unwrap());
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            if legacy {
                assert!(matches!(
                    reader.read_body_with_logger(&mut logger),
                    Err(Error::ValueLimitExceeded)
                ));
                let mut raw = Vec::new();
                reader.open_value().unwrap().read_to_end(&mut raw).unwrap();
                assert_eq!(raw, b"a\0\0");
            } else {
                let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
                assert!(props.read_next_property().unwrap());
                assert!(matches!(props.read_body(), Err(Error::ValueLimitExceeded)));
                assert!(!props.is_value_consumed());
                let mut raw = Vec::new();
                props.open_value().unwrap().read_to_end(&mut raw).unwrap();
                assert_eq!(raw, b"a\0\0");
            }
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.violation == Violation::DataSizeLimitExceeded)
            );
            assert!(!reader.read_next().unwrap());
        }
    }
    let mut input = document(Attribute::BODY, b"a\0\0");
    attribute(&mut input, Attribute::BODY, b"b");
    let options = TnefReaderOptions {
        max_total_data_bytes: 3,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next().unwrap());
    assert_eq!(reader.read_body().unwrap().as_bytes(), b"a");
    assert!(reader.read_next().unwrap());
    assert!(matches!(reader.read_body(), Err(Error::ValueLimitExceeded)));
}

#[test]
fn truncation_is_reported_and_logger_errors_fault_the_reader() {
    for fail in [false, true] {
        let mut input = document(Attribute::BODY, b"hello");
        input.truncate(input.len() - 4);
        let mut reader = TnefReader::new(input.as_slice());
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            if fail {
                Err(io::Error::other("logger failed"))
            } else {
                Ok(())
            }
        };
        assert!(reader.read_next().unwrap());
        let result = reader.read_body_with_logger(&mut logger);
        if fail {
            assert!(matches!(result, Err(Error::Io(_))));
            assert!(matches!(reader.read_next(), Err(Error::Faulted)));
        } else {
            assert_eq!(result.unwrap().as_bytes(), b"hel");
            assert!(reader.is_truncated());
        }
        assert!(!issues.is_empty());
    }
}

#[test]
fn body_collection_propagates_source_errors_without_partial_success() {
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
            document(Attribute::BODY, b"hello")
        } else {
            document(
                Attribute::MAPI_PROPERTIES,
                &properties(&[body_property(Tag::BODY_A, b"hello")]),
            )
        };
        input.truncate(input.len() - if legacy { 5 } else { 8 });
        let mut reader = TnefReader::new(FailsAtEnd(&input));
        assert!(reader.read_next().unwrap());
        let result = if legacy {
            reader.read_body()
        } else {
            let mut props = reader.open_properties().unwrap();
            assert!(props.read_next_property().unwrap());
            props.read_body()
        };
        assert!(matches!(result, Err(Error::Io(_))));
        assert!(matches!(reader.read_next(), Err(Error::Faulted)));
    }
}

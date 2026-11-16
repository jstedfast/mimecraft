// Adapted from MimeKit's TnefMessageConverterTests compressed-RTF cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::super::super::bodies::compressed;
use super::*;
use mimecraft::tnef::{RtfDecompressionError, TnefConversionResult};

const RTF: &[u8] = br"{\rtf1 Hello from RTF}";
const ENCAPSULATED: &[u8] = br"{\rtf1\ansi\fromhtml1 {\*\htmltag <html><body><p>}{\htmlrtf \par }Hello {\*\htmltag <b>}encapsulated{\*\htmltag </b>}{\*\htmltag </p></body></html>}}";

fn rtf_message(
    rtf: &[u8],
    sync: Option<bool>,
    native: Option<i32>,
    plain: bool,
    html: bool,
) -> Message {
    let mut fields = vec![object(Tag::RTF_COMPRESSED.id(), Type::BINARY, rtf)];
    if plain {
        fields.push(string(Tag::BODY_W, "Stored plain"));
    }
    if html {
        fields.push(string(Tag::BODY_HTML_W, "<p>Stored HTML</p>"));
    }
    if let Some(sync) = sync {
        fields.push(integer(Tag::RTF_IN_SYNC, i32::from(sync)));
    }
    if let Some(native) = native {
        fields.push(integer(Tag::NATIVE_BODY, native));
    }
    message(&fields)
}

fn texts(result: &TnefConversionResult) -> Vec<&str> {
    result
        .message
        .leaf_parts()
        .map(|part| std::str::from_utf8(part.content().get_ref().get_ref()).unwrap())
        .collect()
}

fn generated(result: &TnefConversionResult, plain: &str, html: &str) {
    let MimeEntity::Multipart(alternative) = result.message.body().unwrap() else {
        panic!("expected multipart/alternative");
    };
    assert!(
        alternative
            .content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("multipart", "alternative")
    );
    assert_eq!(alternative.children().len(), 2);
    for (child, subtype) in alternative.children().iter().zip(["plain", "html"]) {
        let MimeEntity::Part(part) = child else {
            panic!("expected text part")
        };
        let content_type = part.content_type().unwrap().unwrap();
        assert!(content_type.is_mime_type("text", subtype));
        assert_eq!(content_type.charset(), Some("utf-8"));
        assert!(!raw_body(child).starts_with(b"\xef\xbb\xbf"));
    }
    let bodies = texts(result);
    assert_eq!(bodies[0].trim(), plain);
    assert!(bodies[1].starts_with('<'));
    assert!(bodies[1].contains(html), "{}", bodies[1]);
}

#[test]
fn upstream_compressed_and_uncompressed_rtf_generate_utf8_alternatives() {
    let mut mela = ((RTF.len() + 12) as u32).to_le_bytes().to_vec();
    mela.extend_from_slice(&(RTF.len() as u32).to_le_bytes());
    mela.extend_from_slice(b"MELA\0\0\0\0");
    mela.extend_from_slice(RTF);
    for rtf in [compressed(RTF), mela] {
        let source = rtf_message(&rtf, Some(true), None, true, false);
        let result = source.to_mime_message().unwrap();
        assert!(result.losses.is_empty(), "{:?}", result.losses);
        assert_eq!(source.rtf_body().unwrap().as_bytes(), rtf);
        drop(source);
        generated(&result, "Hello from RTF", "Hello from RTF");
    }
}

#[test]
fn upstream_best_body_table_and_native_body_precedence() {
    let rtf = compressed(RTF);
    for plain in [false, true] {
        for html in [false, true] {
            for sync in [None, Some(false), Some(true)] {
                for native in [None, Some(0), Some(1), Some(2), Some(3), Some(99)] {
                    let result = rtf_message(&rtf, sync, native, plain, html)
                        .to_mime_message()
                        .unwrap();
                    assert!(result.losses.is_empty(), "{:?}", result.losses);
                    // MS-OXBBODY: NativeBody overrides the table; absent native
                    // plain/HTML still falls back when RTF is the only body.
                    let use_rtf = (!plain && !html)
                        || match native {
                            Some(1 | 3) => false,
                            Some(2) => true,
                            _ => sync == Some(true),
                        };
                    if use_rtf {
                        generated(&result, "Hello from RTF", "Hello from RTF");
                    } else {
                        let bodies = texts(&result);
                        assert_eq!(bodies.len(), usize::from(plain) + usize::from(html));
                        assert!(!bodies.iter().any(|text| text.contains("Hello from RTF")));
                        if plain {
                            assert_eq!(bodies[0], "Stored plain");
                        }
                        if html {
                            assert_eq!(*bodies.last().unwrap(), "<p>Stored HTML</p>");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn upstream_encapsulated_html_is_extracted_not_rendered() {
    let result = rtf_message(&compressed(ENCAPSULATED), Some(true), None, true, true)
        .to_mime_message()
        .unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    generated(
        &result,
        "Hello encapsulated",
        "<p>Hello <b>encapsulated</b></p>",
    );
    assert_eq!(
        texts(&result)[1],
        "<html><body><p>Hello <b>encapsulated</b></p></body></html>"
    );

    let empty = rtf_message(&compressed(br"{\rtf1\fromhtml1}"), None, None, false, false)
        .to_mime_message()
        .unwrap();
    assert!(empty.losses.is_empty());
    assert_eq!(texts(&empty)[1], "");
}

#[test]
fn upstream_unknown_compression_is_absent_even_with_native_rtf() {
    for mode in [0u32, 0x44434241] {
        for plain in [false, true] {
            let mut rtf = compressed(RTF);
            rtf[8..12].copy_from_slice(&mode.to_le_bytes());
            let result = rtf_message(&rtf, Some(true), Some(2), plain, false)
                .to_mime_message()
                .unwrap();
            assert_eq!(result.losses.len(), 1);
            assert_eq!(result.losses[0].kind, Loss::InvalidRtfBody);
            assert!(
                result.losses[0]
                    .description
                    .contains(&format!("{mode:08X}"))
            );
            assert_eq!(result.message.body().is_some(), plain);
            if plain {
                assert_eq!(texts(&result)[0], "Stored plain");
            }
        }
    }
}

#[test]
fn upstream_bad_compressed_crc_is_reported_but_readable_rtf_is_kept() {
    let mut rtf = compressed(RTF);
    rtf[12..16].copy_from_slice(&0x12345678u32.to_le_bytes());
    for sync in [false, true] {
        let result = rtf_message(&rtf, Some(sync), None, true, false)
            .to_mime_message()
            .unwrap();
        assert_eq!(result.losses.len(), 1);
        assert_eq!(result.losses[0].kind, Loss::RtfChecksumMismatch);
        if sync {
            generated(&result, "Hello from RTF", "Hello from RTF");
        } else {
            assert_eq!(texts(&result)[0], "Stored plain");
        }
    }
}

#[test]
fn malformed_rtf_framing_remains_a_checked_error() {
    let mut incomplete = compressed(RTF);
    incomplete.truncate(18);
    let mut invalid_size = compressed(RTF);
    invalid_size[..4].copy_from_slice(&(-1i32).to_le_bytes());
    let mut bad_mela = vec![0; 16];
    bad_mela[..4].copy_from_slice(&12i32.to_le_bytes());
    bad_mela[8..12].copy_from_slice(b"MELA");
    bad_mela[12] = 1;
    for (input, expected) in [
        (vec![0; 12], RtfDecompressionError::IncompleteHeader),
        (incomplete, RtfDecompressionError::IncompleteContent),
        (
            invalid_size,
            RtfDecompressionError::InvalidCompressedSize(-1),
        ),
        (
            bad_mela,
            RtfDecompressionError::InvalidCrc32 {
                expected: 1,
                actual: 0,
            },
        ),
    ] {
        assert!(matches!(
            rtf_message(&input, None, None, true, false).to_mime_message(),
            Err(TnefConversionError::Rtf(error)) if error == expected
        ));
    }
}

#[test]
fn upstream_lone_surrogate_is_replaced_and_round_trips_with_explicit_losses() {
    let mut result = rtf_message(
        &compressed(br"{\rtf1 A\u-10240?B}"),
        None,
        None,
        false,
        false,
    )
    .to_mime_message()
    .unwrap();
    generated(&result, "A\u{fffd}B", "B");
    // Unlike MimeKit's silent replacement, surface the existing interpreters'
    // first diagnostic for each output format without retaining an unbounded list.
    assert_eq!(result.losses.len(), 2);
    assert!(
        result
            .losses
            .iter()
            .all(|loss| loss.kind == Loss::RtfRenderingIssue
                && loss.description.contains("UnpairedSurrogate"))
    );
    let mut wire = Vec::new();
    result.message.write_to(&mut wire).unwrap();
    assert!(wire.is_ascii());
    let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    let MimeEntity::Multipart(alternative) = parsed.body_mut().unwrap() else {
        panic!("alternative")
    };
    let MimeEntity::Part(plain) = &mut alternative.children_mut()[0] else {
        panic!("plain")
    };
    let mut decoded = Vec::new();
    plain.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, "A\u{fffd}B".as_bytes());
}

#[test]
fn rtf_default_nonimage_placeholders_are_omitted_without_losses() {
    let result = rtf_message(
        &compressed(br"{\rtf1 A\objattph B}"),
        None,
        None,
        false,
        false,
    )
    .to_mime_message()
    .unwrap();
    assert!(result.losses.is_empty());

    let mut wire = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[object(
            Tag::RTF_COMPRESSED.id(),
            Type::BINARY,
            &compressed(RTF),
        )]),
    );
    attachment(&mut wire, "kept.bin", b"payload");
    let result = load(&wire).to_mime_message().unwrap();
    assert!(result.losses.is_empty());
    let MimeEntity::Multipart(mixed) = result.message.body().unwrap() else {
        panic!("mixed")
    };
    assert_eq!(mixed.children().len(), 2);
    assert_eq!(raw_body(&mixed.children()[1]), b"payload");
}

#[test]
fn best_body_metadata_uses_checked_property_conversions() {
    for tag in [Tag::NATIVE_BODY, Tag::RTF_IN_SYNC] {
        let source = message(&[
            object(Tag::RTF_COMPRESSED.id(), Type::BINARY, &compressed(RTF)),
            string(Tag::new(tag.id(), Type::UNICODE), "not a number"),
        ]);
        assert!(matches!(
            source.to_mime_message(),
            Err(TnefConversionError::Property(_))
        ));
    }
}

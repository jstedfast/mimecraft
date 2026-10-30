// Standalone parsing and display vectors adapted from MimeKit's HeaderTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, error::Error};

use mimecraft::{
    ContentType, Header, HeaderId, HeaderList, HeaderParser,
    charsets::Charset,
    rfc2047::{ComplianceMode, DecodeIssue, DecodeOptions, decode_text_with_options},
    utils::unfold,
};

fn streamed(input: &[u8]) -> Vec<Header> {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    assert_eq!(parser.parse(input, &mut headers).unwrap(), input.len());
    parser.finish(&mut headers).unwrap();
    headers
}

#[test]
fn standalone_parsing_canonicalizes_only_the_missing_terminal_newline() {
    for (input, expected) in [
        (&b"Subject: value"[..], &b"Subject: value\r\n"[..]),
        (b"Subject:", b"Subject:\r\n"),
        (b"Subject: value\n", b"Subject: value\n"),
        (b"Subject: value\r\n", b"Subject: value\r\n"),
        (b"Subject: value\n\n", b"Subject: value\n\n"),
        (b"Subject: value\r", b"Subject: value\r\r\n"),
        (
            b"sUbJeCt \t: folded\r\n\tvalue\n",
            b"sUbJeCt \t: folded\r\n\tvalue\n",
        ),
        (b"X-Raw: \xff\0\rtext", b"X-Raw: \xff\0\rtext\r\n"),
    ] {
        let header = Header::parse(input).unwrap();
        assert_eq!(header.raw(), expected);
        assert!(!header.is_invalid());
        assert_eq!(header, Header::parse(expected).unwrap());
    }
    let header = Header::parse(b"sUbJeCt \t: folded\r\n\tvalue\n").unwrap();
    assert_eq!(header.field(), Some("sUbJeCt"));
    assert_eq!(header.raw_field(), b"sUbJeCt \t");
    assert_eq!(header.raw_value(), b" folded\r\n\tvalue\n");
    assert_eq!(header.id(), HeaderId::Subject);
}

#[test]
fn empty_fields_and_values_follow_source_parser_tolerance() {
    for input in [b":".as_slice(), b": value", b" \t: value"] {
        let header = Header::parse(input).unwrap();
        assert_eq!(header.field(), Some(""));
        assert_eq!(header.id(), HeaderId::Unknown);
        assert!(!header.is_invalid());
        assert!(header.matches_field(""));
    }
    for input in [b"Subject:".as_slice(), b"Subject: \t\r\n", b"Subject:\n"] {
        let header = Header::parse(input).unwrap();
        let decoded = header.decode_value();
        assert!(decoded.text.is_empty());
        assert!(decoded.issues.is_empty());
    }
}

#[test]
fn standalone_field_character_classes_match_streaming_records() {
    for byte in 0u8..=255 {
        let input = [b'X', byte, b'Y', b':', b'v'];
        let result = Header::parse(&input);
        let valid = (33..=126).contains(&byte);
        assert_eq!(result.is_ok(), valid, "byte {byte}");
        if let Ok(header) = result {
            let field_end = if byte == b':' { 1 } else { 3 };
            assert_eq!(header.field().unwrap().as_bytes(), &input[..field_end]);
            assert_eq!(header.raw(), [&input[..], b"\r\n"].concat());
            let streamed = streamed(&input);
            assert_eq!(streamed[0].field(), header.field());
            assert_eq!(streamed[0].id(), header.id());
        } else {
            let offset = if matches!(byte, b' ' | b'\t') { 2 } else { 1 };
            assert_eq!(result.unwrap_err().offset(), offset, "byte {byte}");
        }
    }
}

#[test]
fn standalone_parse_errors_identify_the_expected_colon() {
    for (input, offset) in [
        (&b""[..], 0),
        (b"Subject", 7),
        (b"Subject \t", 9),
        (b"Bad Name: value", 4),
        (b"\tSubject: value", 1),
        (b"Subject\r\n: value", 7),
        (b"\xff: value", 0),
        (b"Bad\0Name: value", 3),
    ] {
        let error = Header::parse(input).unwrap_err();
        assert_eq!(error.offset(), offset, "{input:?}");
        assert_eq!(
            error.to_string(),
            format!("expected ':' after header field at byte {offset}")
        );
        assert!(error.source().is_none());
    }
    let input = b"!!Bad Name: value??";
    assert_eq!(
        Header::parse(&input[2..input.len() - 2])
            .unwrap_err()
            .offset(),
        4
    );
    assert_eq!(
        "caf\u{e9}: value".parse::<Header>().unwrap_err().offset(),
        3
    );
}

#[test]
fn standalone_parse_is_not_a_header_block_parser() {
    let input = b"Subject: one\r\nX-Other: two\r\n\r\nbody";
    let header = Header::parse(input).unwrap();
    assert_eq!(header.field(), Some("Subject"));
    assert_eq!(header.raw_value(), b" one\r\nX-Other: two\r\n\r\nbody\r\n");
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    let consumed = parser.parse(input, &mut records).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(&input[consumed..], b"body");
    assert_eq!(parser.body_separator(), Some(&b"\r\n"[..]));
}

#[test]
fn standalone_canonicalization_does_not_change_streaming_or_decoded_records() {
    for input in [
        &b"Subject: no final newline"[..],
        b"Subject: bare carriage return\r",
        b"X-Raw: \xff\0",
    ] {
        let records = streamed(input);
        assert_eq!(records.len(), 1);
        let original = records[0].clone();
        assert_eq!(original.raw(), input);
        let _decoded = records[0].decode_value();
        assert_eq!(records[0], original);
        assert_eq!(
            Header::parse(input).unwrap().raw(),
            [input, b"\r\n"].concat()
        );
    }
    for input in [
        &b"Broken\r\n"[..],
        &b"\r\r\n"[..],
        &b"Bad Name: value\r\n"[..],
    ] {
        let records = streamed(input);
        assert!(records[0].is_invalid());
        assert!(Header::parse(input).is_err());
        let decoded = records[0].decode_value();
        assert!(decoded.text.is_empty());
        assert!(decoded.issues.is_empty());
        assert_eq!(records[0].raw(), input);
    }
}

#[test]
fn decoding_unfolds_after_encoded_word_decoding_and_preserves_other_text() {
    for (input, expected) in [
        ("Subject: hello\r\n\tworld", "hello\tworld"),
        ("Subject: \t hello\r\n  world \t", "hello  world"),
        ("Subject: a\rb\nc\r\nd", "abcd"),
        ("Subject: \"quoted\" (comment)", "\"quoted\" (comment)"),
        (
            "Subject: =?utf-8?q?=20hello=0D=0A=09world=20?=",
            "hello\tworld",
        ),
        (
            "Subject: =?utf-8?q?=00=01a=0Bb=0Cc?=",
            "\0\u{1}a\u{b}b\u{c}c",
        ),
        (
            "Subject: \u{a0}caf\u{e9}\u{1f642}\u{3000}",
            "\u{a0}caf\u{e9}\u{1f642}\u{3000}",
        ),
        ("Subject: =?utf-8?q?unterminated", "=?utf-8?q?unterminated"),
    ] {
        let header: Header = input.parse().unwrap();
        let original = header.clone();
        let decoded = header.decode_value();
        assert_eq!(decoded.text, expected, "{input:?}");
        assert!(decoded.issues.is_empty(), "{input:?}: {:?}", decoded.issues);
        assert_eq!(header, original);
    }
}

#[test]
fn mimekit_japanese_header_vector_keeps_charset_state_across_words() {
    let input = concat!(
        "Subject: =?ISO-2022-JP?B?GyRCRnxLXDhsJWEhPCVrJUYlOSVIGyhCICh0ZXN0aW5nIEph?=\n",
        " =?ISO-2022-JP?B?cGFuZXNlIGVtYWlscyk=?="
    );
    let expected = concat!(
        "\u{65e5}\u{672c}\u{8a9e}\u{30e1}\u{30fc}\u{30eb}",
        "\u{30c6}\u{30b9}\u{30c8} (testing Japanese emails)"
    );
    let header: Header = input.parse().unwrap();
    assert_eq!(header.id(), HeaderId::Subject);
    assert_eq!(header.decode_value().text, expected);
    assert!(header.decode_value().issues.is_empty());
    let raw_utf8: Header = format!("Subject: {expected}").parse().unwrap();
    assert_eq!(raw_utf8.decode_value().text, expected);
}

#[test]
fn decoding_options_apply_per_call_without_changing_raw_data() {
    let options = DecodeOptions {
        fallback_charset: Some(Charset::from_label("windows-1252").unwrap()),
        ..Default::default()
    };
    for (input, default, overridden) in [
        (&b"Subject: \x80"[..], "\u{80}", "\u{20ac}"),
        (b"Subject: =?unknown?q?=80?=", "\u{80}", "\u{20ac}"),
        (b"Subject: =?latin1?q?=80?=", "\u{80}", "\u{80}"),
        (b"Subject: =?unknown?q?=C3=A9?=", "\u{e9}", "\u{e9}"),
        (b"Subject: \xc3\xa9", "\u{e9}", "\u{e9}"),
    ] {
        let header = Header::parse(input).unwrap();
        let original = header.clone();
        assert_eq!(header.decode_value().text, default);
        assert_eq!(header.decode_value_with_options(&options).text, overridden);
        assert_eq!(header.decode_value().text, default);
        assert_eq!(header, original);
    }
    let header: Header = "Subject: prefix=?utf-8?q?word?=suffix".parse().unwrap();
    let strict = DecodeOptions {
        compliance_mode: ComplianceMode::Strict,
        ..Default::default()
    };
    assert_eq!(header.decode_value().text, "prefixwordsuffix");
    assert_eq!(
        header.decode_value_with_options(&strict).text,
        "prefix=?utf-8?q?word?=suffix"
    );
}

#[test]
fn decoding_diagnostics_still_refer_to_raw_value_offsets() {
    let header = Header::parse(b"X-Test \t: \xff =?unknown?q?=E9?= =?utf-8?q?=FF?=\r\n").unwrap();
    let options = DecodeOptions::default();
    let raw = decode_text_with_options(header.raw_value(), &options);
    let decoded = header.decode_value_with_options(&options);
    assert_eq!(decoded.text, "\u{ff} \u{e9}\u{fffd}");
    assert_eq!(decoded.issues, raw.issues);
    assert_eq!(decoded.issues.len(), 3);
    assert_eq!(
        decoded.issues[0],
        DecodeIssue::UnlabelledText {
            range: 1..2,
            selected: Charset::LATIN1
        }
    );
    assert!(matches!(
        decoded.issues[1],
        DecodeIssue::UnsupportedCharset { .. }
    ));
    assert!(matches!(
        decoded.issues[2],
        DecodeIssue::InvalidBytes { .. }
    ));
}

#[test]
fn unfolding_borrows_trimmed_text_and_preserves_internal_whitespace() {
    for (input, expected) in [
        ("", ""),
        (" \t\r\n", ""),
        ("unchanged", "unchanged"),
        (" \ttrimmed\r\n", "trimmed"),
        ("\u{a0}text\u{3000}", "\u{a0}text\u{3000}"),
        ("a  \tb", "a  \tb"),
    ] {
        let output = unfold(input);
        assert_eq!(output, expected);
        assert!(matches!(output, Cow::Borrowed(_)));
    }
    for (input, expected) in [
        ("a\r\n\tb", "a\tb"),
        (" a\rb\nc ", "abc"),
        ("a\r\nb", "ab"),
        ("a\r\r\n b", "a b"),
    ] {
        let output = unfold(input);
        assert_eq!(output, expected);
        assert!(matches!(output, Cow::Owned(_)));
    }
}

#[test]
fn unfolding_trims_only_mime_whitespace_at_the_edges() {
    let whitespace = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
        '\u{3000}',
    ];
    for ch in whitespace {
        let padded = format!("{ch}text{ch}");
        let expected = if matches!(ch, ' ' | '\t' | '\r' | '\n') {
            "text"
        } else {
            &padded
        };
        assert_eq!(unfold(&padded), expected);
        let input = format!("a{ch}b");
        let expected = if matches!(ch, '\r' | '\n') {
            "ab"
        } else {
            &input
        };
        assert_eq!(unfold(&input), expected);
    }
    for ch in ['\0', '\u{7f}', '\u{180e}', '\u{200b}', '\u{feff}'] {
        let input = format!("{ch}text{ch}");
        assert_eq!(unfold(&input), input);
    }
}

#[test]
fn decoded_in_place_unfolding_matches_the_standalone_transform() {
    let alphabet = [
        'a',
        ' ',
        '\t',
        '\r',
        '\n',
        '\0',
        '\u{a0}',
        '\u{2028}',
        '\u{e9}',
        '\u{1f642}',
    ];
    for first in alphabet {
        for middle in alphabet {
            for last in alphabet {
                let value = format!("{first}{middle}{last}");
                let expected: String = value
                    .trim_matches([' ', '\t', '\r', '\n'])
                    .chars()
                    .filter(|&ch| ch != '\r' && ch != '\n')
                    .collect();
                assert_eq!(unfold(&value), expected, "{value:?}");
                let header: Header = format!("X-Test: {value}").parse().unwrap();
                assert_eq!(header.decode_value().text, expected, "{value:?}");
            }
        }
    }
}

#[test]
fn standalone_headers_integrate_with_indexed_lists_and_typed_serializers() {
    let mut list = HeaderList::new();
    list.add(Header::parse(b"Subject: first").unwrap());
    list.add("subject: second".parse().unwrap());
    list.add("X-Trace: keep".parse().unwrap());
    let replacement: Header = "sUbJeCt: =?utf-8?q?caf=C3=A9?=".parse().unwrap();
    assert_eq!(
        list.replace_all(replacement)
            .unwrap()
            .unwrap()
            .decode_value()
            .text,
        "first"
    );
    assert_eq!(list.index_of("subject"), Some(0));
    assert_eq!(list.get_all("Subject").count(), 1);
    assert_eq!(
        list.get("SUBJECT").unwrap().decode_value().text,
        "caf\u{e9}"
    );
    assert_eq!(list.index_of("x-trace"), Some(1));

    let content_type = ContentType::new("text", "html").unwrap();
    let formatted = content_type.to_header_string().unwrap();
    let header: Header = formatted.parse().unwrap();
    assert_eq!(header.raw(), formatted.as_bytes());
    list.add(header);
    let raw = list.get("content-type").unwrap();
    assert_eq!(ContentType::parse(raw.raw_value()).unwrap(), content_type);
}

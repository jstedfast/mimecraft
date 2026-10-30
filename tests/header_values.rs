// Includes cases adapted from MimeKit's Utils/MimeUtils.cs and
// UnitTests/Utils/MimeUtilsTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding, HeaderId, HeaderParser, MimeVersion, MimeVersionParseErrorKind,
    UnknownContentEncoding,
};

#[test]
fn transfer_encoding_tokens_and_aliases() {
    let cases = [
        ("7bit", ContentEncoding::SevenBit),
        ("8bit", ContentEncoding::EightBit),
        ("binary", ContentEncoding::Binary),
        ("base64", ContentEncoding::Base64),
        ("quoted-printable", ContentEncoding::QuotedPrintable),
        ("x-uuencode", ContentEncoding::UuEncode),
        ("uuencode", ContentEncoding::UuEncode),
        ("x-uue", ContentEncoding::UuEncode),
    ];
    for (token, expected) in cases {
        assert_eq!(ContentEncoding::parse(token), Ok(expected));
        assert_eq!(token.parse(), Ok(expected));
        assert_eq!(ContentEncoding::parse(&token.to_uppercase()), Ok(expected));
        for suffix in [
            "",
            "\r\n",
            ";",
            "; charset=ignored",
            " (ignored)",
            "\tgarbage",
            " invalid\0tail",
        ] {
            let value = format!(" \t\r\n{}{suffix}", token.to_uppercase());
            assert_eq!(ContentEncoding::parse(&value), Ok(expected), "{value:?}");
        }
    }
}

#[test]
fn canonical_encoding_tokens_follow_mimepart() {
    let cases = [
        (ContentEncoding::SevenBit, "7bit"),
        (ContentEncoding::EightBit, "8bit"),
        (ContentEncoding::Binary, "binary"),
        (ContentEncoding::Base64, "base64"),
        (ContentEncoding::QuotedPrintable, "quoted-printable"),
        (ContentEncoding::UuEncode, "x-uuencode"),
    ];
    const UNSPECIFIED: Option<&str> = ContentEncoding::Unspecified.as_str();
    assert_eq!(UNSPECIFIED, None);
    for (encoding, token) in cases {
        assert_eq!(encoding.as_str(), Some(token));
        assert_eq!(token.parse(), Ok(encoding));
    }
}

#[test]
fn transfer_encoding_uses_only_mime_whitespace() {
    for whitespace in [' ', '\t', '\r', '\n'] {
        let value = format!("{whitespace}base64{whitespace}ignored");
        assert_eq!(value.parse(), Ok(ContentEncoding::Base64), "{value:?}");
    }
    for nonwhitespace in [
        '\u{000b}', '\u{000c}', '\u{0085}', '\u{00a0}', '\u{1680}', '\u{2000}', '\u{2007}',
        '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\0', '\u{001c}', '\u{200b}',
        '\u{feff}',
    ] {
        assert!(ContentEncoding::parse(&format!("{nonwhitespace}base64")).is_err());
        assert!(ContentEncoding::parse(&format!("base64{nonwhitespace}")).is_err());
        assert!(ContentEncoding::parse(&format!("base64{nonwhitespace}junk")).is_err());
    }
}

#[test]
fn unrecognized_encodings_are_errors_not_unspecified() {
    for value in [
        "",
        " \t\r\n",
        ";base64",
        "default",
        "unspecified",
        "yenc",
        "uue",
        "x-base64",
        "base64,",
        "base64(comment)",
        "(comment) base64",
        "\"base64\"",
        "Content-Transfer-Encoding: base64",
        "quoted printable",
        "7-bit",
        "base64\0",
        "b\u{00e1}se64",
        "binaryextra",
    ] {
        let error = ContentEncoding::parse(value).unwrap_err();
        assert_eq!(error, UnknownContentEncoding);
        assert_eq!(value.parse::<ContentEncoding>(), Err(error));
        assert_eq!(error.to_string(), "unrecognized content transfer encoding");
        let _: &dyn std::error::Error = &error;
    }
}

#[test]
fn source_version_vectors() {
    for (input, expected) in [
        (" 1 (comment) .\t0\r\n", "1.0"),
        (" 1 (comment) .\t0\r\n .0\r\n", "1.0.0"),
        (" 1 (comment) .\t0\r\n .0.0\r\n", "1.0.0.0"),
    ] {
        let version = MimeVersion::parse(input.as_bytes()).unwrap();
        assert_eq!(version.to_string(), expected);
        assert_eq!(input.parse::<MimeVersion>(), Ok(version));
    }
    for input in [
        "1",
        "1.2.3.4.5",
        "1x2.3",
        "(unterminated comment",
        "1 (unterminated comment",
    ] {
        assert!(MimeVersion::parse(input.as_bytes()).is_err(), "{input:?}");
    }
}

#[test]
fn version_components_and_normalized_formatting() {
    let normal: MimeVersion = "001.000".parse().unwrap();
    assert_eq!(normal, MimeVersion::V1_0);
    assert_eq!(normal.major(), 1);
    assert_eq!(normal.minor(), 0);
    assert_eq!(normal.build(), None);
    assert_eq!(normal.revision(), None);
    assert_eq!(normal.components(), [1, 0]);
    assert_eq!(normal.to_string(), "1.0");

    let build: MimeVersion = "01.00.004".parse().unwrap();
    assert_eq!(build.components(), [1, 0, 4]);
    assert_eq!(build.build(), Some(4));
    assert_eq!(build.revision(), None);
    assert_eq!(build.to_string(), "1.0.4");

    let revision: MimeVersion = "01.00.004.002".parse().unwrap();
    assert_eq!(revision.components(), [1, 0, 4, 2]);
    assert_eq!(revision.build(), Some(4));
    assert_eq!(revision.revision(), Some(2));
    assert_eq!(revision.to_string(), "1.0.4.2");

    let versions = [normal, "1.0.0".parse().unwrap(), "1.0.0.0".parse().unwrap()];
    let unique: std::collections::HashSet<_> = versions.into_iter().collect();
    assert_eq!(unique.len(), 3);
    assert_ne!("2.0".parse::<MimeVersion>().unwrap(), MimeVersion::V1_0);
}

#[test]
fn comments_can_be_nested_escaped_and_non_utf8() {
    let cases: &[&[u8]] = &[
        b"(outer (inner)) 1 (a)(b).(c) 0 (end)",
        br"(escaped \( and \)) 1.0",
        br"(escaped backslash \\)1.0",
        br"(nested (inner \) still inner) outer)1.0",
        b"(\xff\0\r\n)1.0",
        b"1(comment).0",
        b"1.0(\x01\x7f)",
    ];
    for &input in cases {
        assert_eq!(
            MimeVersion::parse(input),
            Ok(MimeVersion::V1_0),
            "{input:?}"
        );
    }
    for input in [
        br"(\)1.0".as_slice(),
        br"(trailing \",
        br"1.(\)",
        b"1.0 (outer (inner)",
    ] {
        let error = MimeVersion::parse(input).unwrap_err();
        assert_eq!(error.kind(), MimeVersionParseErrorKind::IncompleteComment);
    }
}

#[test]
fn backslash_parity_controls_comment_delimiters() {
    for count in 0..20 {
        let mut input = b"(".to_vec();
        input.extend(std::iter::repeat_n(b'\\', count));
        input.extend_from_slice(b")1.0");
        if count % 2 == 0 {
            assert_eq!(MimeVersion::parse(&input), Ok(MimeVersion::V1_0));
        } else {
            assert_eq!(
                MimeVersion::parse(&input).unwrap_err().kind(),
                MimeVersionParseErrorKind::IncompleteComment
            );
            input.extend_from_slice(b")1.0");
            assert_eq!(MimeVersion::parse(&input), Ok(MimeVersion::V1_0));
        }
    }
}

#[test]
fn deeply_nested_comments_do_not_recurse() {
    let mut input = vec![b'('; 20_000];
    input.extend(std::iter::repeat_n(b')', 20_000));
    input.extend_from_slice(b"1.0");
    assert_eq!(MimeVersion::parse(&input), Ok(MimeVersion::V1_0));
    input.remove(0);
    assert!(MimeVersion::parse(&input).is_err());
}

#[test]
fn versions_accept_only_mime_whitespace_outside_comments() {
    for whitespace in [' ', '\t', '\r', '\n'] {
        let input = format!("{whitespace}1{whitespace}.{whitespace}0{whitespace}");
        assert_eq!(input.parse(), Ok(MimeVersion::V1_0));
    }
    for whitespace in ['\u{000b}', '\u{000c}', '\u{0085}', '\u{00a0}', '\u{2000}'] {
        for input in [
            format!("{whitespace}1.0"),
            format!("1{whitespace}.0"),
            format!("1.{whitespace}0"),
            format!("1.0{whitespace}"),
        ] {
            assert!(input.parse::<MimeVersion>().is_err(), "{input:?}");
        }
        let input = format!("({whitespace})1.0");
        assert_eq!(input.parse(), Ok(MimeVersion::V1_0));
    }
}

#[test]
fn exact_end_trailing_dot_preserves_mimekit_quirk() {
    for input in ["1.0.", "1.2.3.", "1.2.3.4.", "1.0 (comment)."] {
        let actual: MimeVersion = input.parse().unwrap();
        let expected: MimeVersion = input.strip_suffix('.').unwrap().parse().unwrap();
        assert_eq!(actual, expected);
    }
    for input in [
        "1.",
        "1.0. ",
        "1.0.\r\n",
        "1.0.(comment)",
        "1.0..",
        "1.2.3.4.5.",
    ] {
        assert!(input.parse::<MimeVersion>().is_err(), "{input:?}");
    }
}

#[test]
fn integer_limits_are_exact_in_every_component() {
    for count in 2..=4 {
        let maximum = vec!["2147483647"; count].join(".");
        let version: MimeVersion = maximum.parse().unwrap();
        assert_eq!(version.components(), vec![i32::MAX as u32; count]);
        assert_eq!(version.to_string(), maximum);
        for index in 0..count {
            for overflow in ["2147483648", "4294967295", "999999999999999999999999999"] {
                let mut components = vec!["0"; count];
                components[index] = overflow;
                let input = components.join(".");
                let error = input.parse::<MimeVersion>().unwrap_err();
                assert_eq!(error.kind(), MimeVersionParseErrorKind::ComponentOverflow);
                assert!(error.offset() < input.len());
            }
        }
    }
    let many_zeros = format!("{}1.{}0", "0".repeat(10_000), "0".repeat(10_000));
    assert_eq!(many_zeros.parse(), Ok(MimeVersion::V1_0));
}

#[test]
fn error_kinds_and_byte_offsets_are_explicit() {
    use MimeVersionParseErrorKind::*;
    let cases: &[(&[u8], MimeVersionParseErrorKind, usize)] = &[
        (b"", InvalidComponent, 0),
        (b" \t", InvalidComponent, 2),
        (b"1", InvalidComponentCount, 1),
        (b"1.", InvalidComponentCount, 2),
        (b"1. ", InvalidComponent, 3),
        (b".0", InvalidComponent, 0),
        (b"1..0", InvalidComponent, 2),
        (b"-1.0", InvalidComponent, 0),
        (b"+1.0", InvalidComponent, 0),
        (b"1 2", ExpectedDot, 2),
        (b"1x2.3", ExpectedDot, 1),
        (b"1.2.3.4.5", InvalidComponentCount, 8),
        (b"2147483648.0", ComponentOverflow, 9),
        (b"0.2147483648", ComponentOverflow, 11),
        (b"1.0 (comment", IncompleteComment, 4),
        (b"(closed) (open", IncompleteComment, 9),
        (b"\xff.0", InvalidComponent, 0),
        (b"1.0\0", ExpectedDot, 3),
        (b"1.0;foo", ExpectedDot, 3),
        ("(\u{00e9})1.x".as_bytes(), InvalidComponent, 6),
    ];
    for &(input, kind, offset) in cases {
        let error = MimeVersion::parse(input).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        assert!(error.to_string().ends_with(&format!("at byte {offset}")));
        let _: &dyn std::error::Error = &error;
    }
}

#[test]
fn version_formatting_round_trips_all_component_counts() {
    let mut state = 0x1234_5678u32;
    for count in 2..=4 {
        for _ in 0..100 {
            let mut components = Vec::new();
            for _ in 0..count {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                components.push(state & i32::MAX as u32);
            }
            let input = components
                .iter()
                .map(|component| format!(" (comment) {component:010} (nested (comment)) "))
                .collect::<Vec<_>>()
                .join(".");
            let version: MimeVersion = input.parse().unwrap();
            assert_eq!(version.components(), components);
            assert_eq!(version.to_string().parse(), Ok(version));
        }
    }
}

#[test]
fn typed_values_integrate_with_incremental_raw_headers() {
    let input = b"MIME-Version: 1 (comment)\r\n\t. 0\r\n\
Content-Transfer-Encoding: BASE64;\r\n\r\nQ2F0\r\n";
    for size in 1..=input.len() {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        let mut consumed = 0;
        for chunk in input.chunks(size) {
            consumed += parser.parse(chunk, &mut headers).unwrap();
        }
        assert!(parser.is_finished());
        assert_eq!(&input[consumed..], b"Q2F0\r\n");
        assert_eq!(headers[0].id(), HeaderId::MimeVersion);
        assert_eq!(
            MimeVersion::parse(headers[0].raw_value()),
            Ok(MimeVersion::V1_0)
        );
        assert_eq!(headers[1].id(), HeaderId::ContentTransferEncoding);
        let text = std::str::from_utf8(headers[1].raw_value()).unwrap();
        assert_eq!(ContentEncoding::parse(text), Ok(ContentEncoding::Base64));
        assert_eq!(headers[0].raw_value(), b" 1 (comment)\r\n\t. 0\r\n");
        assert_eq!(headers[1].raw_value(), b" BASE64;\r\n");
    }
}

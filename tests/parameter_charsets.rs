// Compatibility vectors adapted from MimeKit's ParameterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentType, HeaderFormatError, HeaderFormatOptions, Parameter,
    ParameterComplianceMode, ParameterEncodingMethod, ParameterParseOptions,
    charsets::Charset,
    encodings::{NewLineFormat, Rfc2231Decoder},
    rfc2047,
};

const METHODS: [ParameterEncodingMethod; 2] = [
    ParameterEncodingMethod::Rfc2231,
    ParameterEncodingMethod::Rfc2047,
];
const CHINESE: &str = "\u{6d4b}\u{8bd5}\u{6587}\u{672c}.doc";

fn disposition(text: &str, charset: Option<Charset>) -> ContentDisposition {
    let mut parameter = Parameter::new("filename", text).unwrap();
    parameter.set_charset(charset);
    let mut value = ContentDisposition::default();
    value.parameters_mut().add(parameter).unwrap();
    value
}

fn round_trip(value: &ContentDisposition, options: &HeaderFormatOptions) -> String {
    let before = value.clone();
    let output = value.to_header_string_with_options(options).unwrap();
    assert_eq!(*value, before);
    assert!(output.is_ascii());
    let newline = if options.new_line_format == NewLineFormat::Dos {
        "\r\n"
    } else {
        "\n"
    };
    let body = output.strip_suffix(newline).unwrap();
    for (index, line) in body.split(newline).enumerate() {
        assert!(!line.is_empty());
        assert!(line.len() <= options.max_line_length, "{line:?}");
        assert!(
            line.bytes()
                .all(|byte| byte == b'\t' || !byte.is_ascii_control())
        );
        assert!(index == 0 || line.starts_with('\t'));
    }
    for compliance_mode in [
        ParameterComplianceMode::Loose,
        ParameterComplianceMode::Strict,
    ] {
        let parsed = ContentDisposition::parse_with_options(
            output.split_once(':').unwrap().1.as_bytes(),
            &ParameterParseOptions {
                compliance_mode,
                rfc2047_compliance_mode: rfc2047::ComplianceMode::Strict,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            parsed.parameters().issues().is_empty(),
            "{:?}: {output:?}",
            parsed.parameters().issues()
        );
        assert_eq!(
            parsed
                .parameters()
                .iter()
                .map(|p| (p.name(), p.value()))
                .collect::<Vec<_>>(),
            value
                .parameters()
                .iter()
                .map(|p| (p.name(), p.value()))
                .collect::<Vec<_>>(),
            "{output:?}"
        );
    }
    output
}

fn check_chunks(output: &str, charset: Charset, method: ParameterEncodingMethod, text: &str) {
    let mut decoded = String::new();
    if method == ParameterEncodingMethod::Rfc2231 {
        assert!(
            output.contains(&format!("{}''", charset.name())),
            "{output}"
        );
        for assignment in output.split(';').skip(1) {
            let (name, mut payload) = assignment.trim().split_once('=').unwrap();
            if !name.starts_with("filename*") {
                continue;
            }
            if let Some(suffix) = payload.strip_prefix(&format!("{}''", charset.name())) {
                payload = suffix;
            }
            let mut bytes = Vec::new();
            let mut decoder = Rfc2231Decoder::new();
            decoder.decode(payload.as_bytes(), &mut bytes);
            for (index, byte) in payload.bytes().enumerate() {
                if byte == b'%' {
                    let digits = payload.as_bytes().get(index + 1..index + 3).unwrap();
                    assert!(digits.iter().all(u8::is_ascii_hexdigit), "{payload}");
                }
            }
            let (chunk, had_errors) = charset.decode_lossy(&bytes);
            assert!(!had_errors, "{payload}");
            decoded.push_str(&chunk);
        }
    } else {
        let mut remaining = output;
        while let Some(start) = remaining.find("=?") {
            let rest = &remaining[start..];
            let payload = rest[2..].find('?').unwrap() + 5;
            let end = payload + rest[payload..].find("?=").unwrap() + 2;
            let word = &rest[..end];
            remaining = &rest[end..];
            assert!(word.len() <= 75, "{word}");
            assert!(
                word.starts_with(&format!("=?{}?", charset.name())),
                "{word}"
            );
            if charset.name() == "iso-2022-jp" {
                assert!(word.contains("?b?"));
            }
            let chunk = rfc2047::decode_text(word.as_bytes());
            assert!(chunk.issues.is_empty(), "{word}");
            decoded.push_str(&chunk.text);
        }
    }
    assert_eq!(decoded, text, "{output}");
}

#[test]
fn charset_preferences_follow_owned_parameter_edits() {
    assert_eq!(
        HeaderFormatOptions::default().parameter_charset,
        Charset::UTF8
    );
    let gb = Charset::from_label("gb18030").unwrap();
    let mut parameter = Parameter::new("FileName", CHINESE).unwrap();
    assert_eq!(parameter.charset(), None);
    parameter.set_charset(Some(gb));
    parameter.set_encoding_method(Some(ParameterEncodingMethod::Rfc2047));
    parameter.set_always_quote(true);
    let original = parameter.clone();
    let mut value = ContentDisposition::default();
    value.parameters_mut().add(parameter).unwrap();
    value.parameters_mut().set("FILENAME", "other").unwrap();
    value.set_file_name(Some(&format!("prefix-{CHINESE}")));
    let parameter = &value.parameters().as_slice()[0];
    assert_eq!(parameter.name(), "FileName");
    assert_eq!(parameter.charset(), Some(gb));
    assert_eq!(parameter.encoding_method(), original.encoding_method());
    assert!(parameter.always_quote());
    assert_eq!(original.value(), CHINESE);
    let mut parameter = parameter.clone();
    parameter.set_charset(None);
    assert_eq!(parameter.charset(), None);
    assert_eq!(value.parameters().as_slice()[0].charset(), Some(gb));
    value.parameters_mut().replace(0, parameter).unwrap();
    assert!(round_trip(&value, &HeaderFormatOptions::default()).contains("=?utf-8?"));
}

#[test]
fn canonical_gb18030_output_with_local_and_global_preferences() {
    let gb = Charset::from_label("gb18030").unwrap();
    for method in METHODS {
        for local in [None, Some(gb)] {
            let options = HeaderFormatOptions {
                parameter_encoding_method: method,
                parameter_charset: if local.is_none() { gb } else { Charset::UTF8 },
                ..Default::default()
            };
            let output = round_trip(&disposition(CHINESE, local), &options);
            let expected = match method {
                ParameterEncodingMethod::Rfc2231 => {
                    "Content-Disposition: attachment;\r\n\tfilename*=gb18030''%B2%E2%CA%D4%CE%C4%B1%BE.doc\r\n"
                }
                ParameterEncodingMethod::Rfc2047 => {
                    "Content-Disposition: attachment; filename=\"=?gb18030?b?suLK1M7Esb4uZG9j?=\"\r\n"
                }
            };
            assert_eq!(output, expected);
            check_chunks(&output, gb, method, CHINESE);
        }
        let options = HeaderFormatOptions {
            parameter_encoding_method: method,
            parameter_charset: gb,
            ..Default::default()
        };
        let output = round_trip(&disposition(CHINESE, Some(Charset::UTF8)), &options);
        check_chunks(&output, Charset::UTF8, method, CHINESE);
    }
}

#[test]
fn ascii_and_latin1_take_precedence_over_charset_preferences() {
    for method in METHODS {
        let automatic = HeaderFormatOptions {
            parameter_encoding_method: method,
            ..Default::default()
        };
        for label in ["gb18030", "utf-16", "iso-2022-jp", "us-ascii"] {
            let charset = Charset::from_label(label).unwrap();
            let preferred = HeaderFormatOptions {
                parameter_charset: charset,
                ..automatic
            };
            for text in [
                "",
                "plain.txt",
                "two words",
                "=?utf-8?q?literal?=",
                "caf\u{e9}.txt",
                "\0\t\r\n\u{7f}\u{80}\u{ff}",
                &"a".repeat(300),
            ] {
                let expected = round_trip(&disposition(text, None), &automatic);
                assert_eq!(round_trip(&disposition(text, None), &preferred), expected);
                assert_eq!(
                    round_trip(&disposition(text, Some(charset)), &automatic),
                    expected
                );
            }
        }
    }
}

#[test]
fn unrepresentable_preferences_fall_back_for_the_entire_value() {
    // Only part of this long value is representable in Latin-1. All chunks must
    // use UTF-8, not a mixture of charsets or lossy replacements.
    let upstream = "LA\u{07ac}ingFQCont60 +1\u{ff}e  Fri 6 \u{f6}Fe 14:33:60 +0100 caf\u{e9} au lait avec des croissants";
    for method in METHODS {
        for (label, text) in [
            ("us-ascii", "snowman-\u{2603}.txt".to_owned()),
            ("iso-8859-1", "caf\u{e9}-\u{2603}.txt".to_owned()),
            ("iso-8859-1", upstream.to_owned()),
            ("iso-8859-1", "caf\u{e9} ".repeat(80) + "\u{2603}.txt"),
            ("shift_jis", "\u{65e5}\u{672c}".repeat(80) + "\u{1f600}"),
            ("iso-2022-jp", "\u{65e5}\u{672c}".repeat(80) + "\u{1f600}"),
        ] {
            let charset = Charset::from_label(label).unwrap();
            assert!(charset.encode(&text).is_err());
            for local in [None, Some(charset)] {
                let options = HeaderFormatOptions {
                    parameter_encoding_method: method,
                    parameter_charset: if local.is_none() {
                        charset
                    } else {
                        Charset::UTF8
                    },
                    ..Default::default()
                };
                let output = round_trip(&disposition(&text, local), &options);
                check_chunks(&output, Charset::UTF8, method, &text);
                assert!(
                    !output.contains("%3F") && !output.contains("=3F"),
                    "{output}"
                );
                let automatic = HeaderFormatOptions {
                    parameter_encoding_method: method,
                    ..Default::default()
                };
                assert_eq!(output, round_trip(&disposition(&text, None), &automatic));
            }
        }
    }
}

#[test]
fn legacy_multibyte_stateful_and_utf16_chunks_are_complete() {
    for (label, text) in [
        ("windows-1252", "price \u{20ac} \u{201c}quoted\u{201d} "),
        (
            "windows-1251",
            "\u{041f}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442} ABC ",
        ),
        ("shift_jis", "\u{65e5}\u{672c}\u{8a9e} ABC "),
        ("iso-2022-jp", "\u{65e5}\u{672c}\u{8a9e} ABC \0\r\n"),
        ("iso-2022-jp", "\u{65e5}\u{672c}"),
        ("gb18030", "\u{6d4b}\u{8bd5}\u{21861}\u{1f600} ABC "),
        ("big5", "\u{4e2d}\u{6587} ABC "),
        ("euc-kr", "\u{ac00}\u{b098} ABC "),
        ("utf-16", "\u{65e5} \u{1f600} \0\u{feff} "),
        ("utf-16be", "\u{65e5} \u{1f600} \0\u{feff} "),
    ] {
        let charset = Charset::from_label(label).unwrap();
        let text = text.repeat(25);
        for method in METHODS {
            for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
                let options = HeaderFormatOptions {
                    parameter_encoding_method: method,
                    parameter_charset: charset,
                    max_line_length: 60,
                    new_line_format,
                    ..Default::default()
                };
                let mut value = disposition(&text, None);
                value.parameters_mut().set("tail", "last").unwrap();
                let output = round_trip(&value, &options);
                check_chunks(&output, charset, method, &text);
            }
        }
    }
}

#[test]
fn every_line_limit_accounts_for_charset_shift_and_reset_bytes() {
    let text = "\u{65e5}\u{672c}\u{8a9e} ABC ".repeat(10);
    for label in ["iso-2022-jp", "utf-16be"] {
        let charset = Charset::from_label(label).unwrap();
        let mut value = disposition(&text, Some(charset));
        value.parameters_mut().set("tail", "last").unwrap();
        for max_line_length in 60..=998 {
            let options = HeaderFormatOptions {
                max_line_length,
                ..Default::default()
            };
            let output = round_trip(&value, &options);
            check_chunks(&output, charset, ParameterEncodingMethod::Rfc2231, &text);
        }
    }
}

#[test]
fn charset_only_sections_and_exact_limits_preserve_complete_characters() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    for (label, text, name_length) in [("iso-2022-jp", "\u{65e5}", 41), ("utf-16", "\u{1f600}", 43)]
    {
        let mut parameter = Parameter::new(&"x".repeat(name_length), text).unwrap();
        let charset = Charset::from_label(label).unwrap();
        parameter.set_charset(Some(charset));
        let mut value = ContentDisposition::default();
        value.parameters_mut().add(parameter).unwrap();
        let output = round_trip(&value, &options);
        assert!(
            output.contains(&format!("*0*={}'';\r\n", charset.name())),
            "{output}"
        );
        assert_eq!(output.lines().last().unwrap().len(), 60, "{output}");
        value.parameters_mut().set("tail", "last").unwrap();
        let before = value.clone();
        assert_eq!(
            value.to_header_string_with_options(&options),
            Err(HeaderFormatError::LineTooLong {
                required: 61,
                limit: 60
            })
        );
        assert_eq!(value, before);
    }
}

#[test]
fn content_type_body_charset_is_independent_of_parameter_charset() {
    for method in METHODS {
        let options = HeaderFormatOptions {
            parameter_charset: Charset::from_label("gb18030").unwrap(),
            parameter_encoding_method: method,
            ..Default::default()
        };
        let mut value = ContentType::new("text", "plain").unwrap();
        value.set_charset(Some("utf-8"));
        value.set_name(Some(CHINESE));
        let header = value.to_header_with_options(&options).unwrap();
        assert!(header.raw().windows(7).any(|bytes| bytes == b"gb18030"));
        let parsed = header.parse_content_type().unwrap();
        assert_eq!(parsed.charset(), Some("utf-8"));
        assert_eq!(parsed.name(), Some(CHINESE));
    }
}

#[test]
fn iso2022_mime_boundaries_restart_only_at_explicit_ascii_resets() {
    for (first, second) in [
        ("%1B$BF|%1B%28B", "%1B$BK%5C%1B%28B"),
        ("%1B$BF|%1B%28B%1", "B$BK%5C%1B%28B"),
        ("%1B$BF", "|K%5C%1B%28B"),
        ("%1B$", "BF|K%5C%1B%28B"),
    ] {
        let text = format!("attachment; filename*0*=iso-2022-jp''{first}; filename*1*={second}");
        let parsed = ContentDisposition::parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.file_name(), Some("\u{65e5}\u{672c}"));
        assert!(parsed.parameters().issues().is_empty());
    }

    for (first, second) in [
        ("=1B$BF|=1B(B", "=1B$BK=5C=1B(B"),
        ("=1B$BF|=1B(B=1", "B$BK=5C=1B(B"),
        ("=1B$BF", "|K=5C=1B(B"),
        ("=1B$", "BF|K=5C=1B(B"),
    ] {
        let text = format!("=?iso-2022-jp?q?{first}?= =?iso-2022-jp?q?{second}?=");
        let decoded = rfc2047::decode_text(text.as_bytes());
        assert_eq!(decoded.text, "\u{65e5}\u{672c}");
        assert!(decoded.issues.is_empty());
    }
    let mut payload = Vec::new();
    for bytes in [b"\x1b$BF|\x1b(B", b"\x1b$BK\\\x1b(B"] {
        mimecraft::encodings::Rfc2047Base64Encoder::new().encode(bytes, &mut payload);
    }
    let payload = String::from_utf8(payload).unwrap();
    // The first padded block ends in a reset. Part of the next quartet may
    // already be pending when character decoding restarts at the word boundary.
    for split in 12..=15 {
        let (first, second) = payload.split_at(split);
        let text = format!("=?iso-2022-jp?b?{first}?= =?iso-2022-jp?b?{second}?=");
        let decoded = rfc2047::decode_text(text.as_bytes());
        assert_eq!(decoded.text, "\u{65e5}\u{672c}");
        assert!(decoded.issues.is_empty());
    }
}

#[test]
fn iso2022_boundary_restarts_do_not_hide_invalid_bytes() {
    let parsed = ContentDisposition::parse(
        b"attachment; filename*0*=iso-2022-jp''%FF%1B%28B; filename*1*=%1B$BF|%1B%28B",
    )
    .unwrap();
    assert_eq!(parsed.file_name(), Some("\u{fffd}\u{65e5}"));
    assert!(matches!(
        parsed.parameters().issues(),
        [mimecraft::ParameterIssue {
            kind: mimecraft::ParameterIssueKind::InvalidBytes { .. },
            ..
        }]
    ));
    let decoded =
        rfc2047::decode_text(b"=?iso-2022-jp?q?=FF=1B(B?= =?iso-2022-jp?q?=1B$BF|=1B(B?=");
    assert_eq!(decoded.text, "\u{fffd}\u{65e5}");
    assert!(matches!(
        decoded.issues.as_slice(),
        [rfc2047::DecodeIssue::InvalidBytes { .. }]
    ));
}

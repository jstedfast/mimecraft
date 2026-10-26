// Compatibility vectors adapted from MimeKit's ParameterTests.cs and
// ContentTypeTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentType, HeaderFormatError, HeaderFormatOptions, HeaderParser,
    Parameter, ParameterComplianceMode, ParameterEncodingMethod, ParameterParseOptions,
    encodings::{NewLineFormat, Rfc2231Decoder},
};

fn check_wire(output: &str, options: &HeaderFormatOptions) {
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => unreachable!(),
    };
    assert!(output.is_ascii());
    let content = output.strip_suffix(newline).unwrap();
    for (index, line) in content.split(newline).enumerate() {
        assert!(!line.is_empty());
        assert!(line.len() <= options.max_line_length, "{line:?}");
        assert!(
            line.bytes()
                .all(|byte| byte == b'\t' || !byte.is_ascii_control())
        );
        if index != 0 {
            assert!(line.starts_with('\t'), "{line:?}");
        }
    }
}

fn round_trip(value: &ContentDisposition, options: &HeaderFormatOptions) -> String {
    let before = value.clone();
    let output = value.to_header_string_with_options(options).unwrap();
    assert_eq!(*value, before);
    check_wire(&output, options);
    let (_, body) = output.split_once(':').unwrap();
    for compliance_mode in [
        ParameterComplianceMode::Loose,
        ParameterComplianceMode::Strict,
    ] {
        let parsed = ContentDisposition::parse_with_options(
            body.as_bytes(),
            &ParameterParseOptions {
                compliance_mode,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parsed.disposition(), value.disposition());
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
        assert!(parsed.parameters().issues().is_empty(), "{output:?}");
    }
    output
}

fn file_name(value: &str) -> ContentDisposition {
    let mut disposition = ContentDisposition::default();
    disposition.set_file_name(Some(value));
    disposition
}

fn check_encoded_words(output: &str) {
    for (start, _) in output.match_indices("=?") {
        let word = &output[start..];
        let word = &word[..word.find("?=").unwrap() + 2];
        assert!(word.len() <= 75, "{word}");
        assert!(!word.contains(['"', '\\']));
        assert!(
            mimecraft::rfc2047::decode_text(word.as_bytes())
                .issues
                .is_empty(),
            "{word}"
        );
    }
}

#[test]
fn parameter_preferences_follow_owned_edits() {
    let mut parameter = Parameter::new("FileName", "tps-report.doc").unwrap();
    assert_eq!(parameter.encoding_method(), None);
    assert!(!parameter.always_quote());
    assert_eq!(
        ParameterEncodingMethod::default(),
        ParameterEncodingMethod::Rfc2231
    );
    assert_eq!(
        HeaderFormatOptions::default().parameter_encoding_method,
        ParameterEncodingMethod::Rfc2231
    );
    for always_quote in [true, true, false, true] {
        parameter.set_always_quote(always_quote);
        assert_eq!(parameter.always_quote(), always_quote);
    }
    for method in [
        Some(ParameterEncodingMethod::Rfc2047),
        Some(ParameterEncodingMethod::Rfc2231),
        None,
    ] {
        parameter.set_encoding_method(method);
        assert_eq!(parameter.encoding_method(), method);
    }
    parameter.set_encoding_method(Some(ParameterEncodingMethod::Rfc2047));
    let mut disposition = ContentDisposition::default();
    disposition.parameters_mut().add(parameter.clone()).unwrap();
    disposition
        .parameters_mut()
        .set("FILENAME", "changed.txt")
        .unwrap();
    disposition.set_file_name(Some("final.txt"));
    let stored = &disposition.parameters().as_slice()[0];
    assert_eq!(stored.name(), "FileName");
    assert_eq!(stored.value(), "final.txt");
    assert!(stored.always_quote());
    assert_eq!(stored.encoding_method(), parameter.encoding_method());
    assert_eq!(parameter.value(), "tps-report.doc");
    assert_eq!(
        round_trip(&disposition, &HeaderFormatOptions::default()),
        "Content-Disposition: attachment; FileName=\"final.txt\"\r\n"
    );
    let previous = disposition
        .parameters_mut()
        .replace(0, Parameter::new("filename", "new").unwrap())
        .unwrap();
    assert!(previous.always_quote());
    assert_eq!(
        previous.encoding_method(),
        Some(ParameterEncodingMethod::Rfc2047)
    );
    assert!(!disposition.parameters().as_slice()[0].always_quote());
    assert_eq!(
        disposition.parameters().as_slice()[0].encoding_method(),
        None
    );
}

#[test]
fn per_parameter_quoting_and_encoding_override_header_options() {
    for method in [
        ParameterEncodingMethod::Rfc2231,
        ParameterEncodingMethod::Rfc2047,
    ] {
        let options = HeaderFormatOptions {
            parameter_encoding_method: method,
            ..Default::default()
        };
        let mut disposition = ContentDisposition::default();
        let mut parameter = Parameter::new("filename", "tps-report.doc").unwrap();
        parameter.set_always_quote(true);
        disposition.parameters_mut().add(parameter).unwrap();
        disposition.parameters_mut().set("plain", "value").unwrap();
        assert_eq!(
            round_trip(&disposition, &options),
            "Content-Disposition: attachment; filename=\"tps-report.doc\"; plain=value\r\n"
        );
        let quoted = HeaderFormatOptions {
            always_quote_parameters: true,
            ..options
        };
        assert!(round_trip(&disposition, &quoted).contains("plain=\"value\""));
        disposition.set_file_name(Some(&"a".repeat(300)));
        let output = round_trip(&disposition, &options);
        if method == ParameterEncodingMethod::Rfc2231 {
            for line in output.lines().filter(|line| line.contains("filename*")) {
                assert!(line.split_once('=').unwrap().1.starts_with('"'), "{line}");
            }
        } else {
            assert!(output.contains("filename=\"=?us-ascii?"));
            check_encoded_words(&output);
        }
    }

    for (global, local) in [
        (
            ParameterEncodingMethod::Rfc2047,
            ParameterEncodingMethod::Rfc2231,
        ),
        (
            ParameterEncodingMethod::Rfc2231,
            ParameterEncodingMethod::Rfc2047,
        ),
    ] {
        let options = HeaderFormatOptions {
            parameter_encoding_method: global,
            ..Default::default()
        };
        let mut disposition = ContentDisposition::default();
        let mut parameter = Parameter::new("filename", "caf\u{e9}.txt").unwrap();
        parameter.set_encoding_method(Some(local));
        disposition.parameters_mut().add(parameter.clone()).unwrap();
        let output = round_trip(&disposition, &options);
        assert_eq!(
            output.contains("filename*="),
            local == ParameterEncodingMethod::Rfc2231
        );
        assert_eq!(
            output.contains("filename=\"=?"),
            local == ParameterEncodingMethod::Rfc2047
        );
        parameter.set_encoding_method(None);
        disposition.parameters_mut().replace(0, parameter).unwrap();
        let output = round_trip(&disposition, &options);
        assert_eq!(
            output.contains("filename*="),
            global == ParameterEncodingMethod::Rfc2231
        );
    }
}

#[test]
fn rfc2047_parameter_compatibility_vectors() {
    let options = HeaderFormatOptions {
        parameter_encoding_method: ParameterEncodingMethod::Rfc2047,
        ..Default::default()
    };
    assert_eq!(
        round_trip(&file_name("\u{6d4b}\u{8bd5}\u{6587}\u{672c}.doc"), &options),
        "Content-Disposition: attachment;\r\n\tfilename=\"=?utf-8?b?5rWL6K+V5paH5pysLmRvYw==?=\"\r\n"
    );
    assert_eq!(
        round_trip(&file_name("caf\u{e9}.txt"), &options),
        "Content-Disposition: attachment; filename=\"=?iso-8859-1?q?caf=E9=2Etxt?=\"\r\n"
    );
    for text in [
        "dir\\fichier-\u{e9}.txt",
        "I \u{2764}\u{fe0f}\u{200d}\u{1f525} emojis.doc",
        "Some \"\u{6d4b}\u{8bd5}\u{6587}\u{672c}\" characters.doc",
        "X-ingF=?iso-8859-1?q?QJg-V2?=.2",
        "=?utf-8?q?caf=C3=A9?=",
        "start-info=\"=?utf-8?q?caf=C3=A9?=\"",
        "=?utf-8?q?caf=C3=A9?= au lait",
        "caf\u{e9} au lait et cr\u{e8}me\u{2603}.txt",
    ] {
        let output = round_trip(&file_name(text), &options);
        assert!(output.contains("filename=\"=?"));
        check_encoded_words(&output);
        let body = output.split_once(':').unwrap().1;
        for rfc2047_compliance_mode in [
            mimecraft::rfc2047::ComplianceMode::Loose,
            mimecraft::rfc2047::ComplianceMode::Strict,
        ] {
            let parsed = ContentDisposition::parse_with_options(
                body.as_bytes(),
                &ParameterParseOptions {
                    compliance_mode: ParameterComplianceMode::Strict,
                    rfc2047_compliance_mode,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(parsed.file_name(), Some(text));
            assert_eq!(parsed.parameters().as_slice()[0].encoding_method(), None);
        }
    }
    let output = round_trip(&file_name("dir\\fichier-\u{e9}.txt"), &options);
    assert!(!output.contains('\\'));
    assert!(output.contains("=5C"));
}

#[test]
fn rfc2047_controls_unicode_and_long_ascii_preserve_literal_values() {
    let options = HeaderFormatOptions {
        parameter_encoding_method: ParameterEncodingMethod::Rfc2047,
        ..Default::default()
    };
    for code in 0..=255 {
        let text = format!("before{}after", char::from_u32(code).unwrap());
        check_encoded_words(&round_trip(&file_name(&text), &options));
    }
    for text in [
        "this is a really really long filename that should force MimeKit to break it apart - yay!.html".to_owned(),
        "a".repeat(300),
        " ".repeat(300),
        " \\\") abc".repeat(80),
        "\"".repeat(300),
        "x\r\nInjected: header\0.txt".repeat(40),
        "a\u{301}\u{feff}\u{10000}\u{10ffff} \u{d7ff}\u{e000}".repeat(40),
        (0..=255).map(|code| char::from_u32(code).unwrap()).collect(),
    ] {
        let output = round_trip(&file_name(&text), &options);
        assert!(output.contains("filename=\"=?"));
        assert!(!output.contains("filename*"));
        check_encoded_words(&output);
    }
}

#[test]
fn rfc2047_every_line_limit_and_following_parameters_fit() {
    let mut value = file_name(&("a\\\" ?\u{e9}\u{1f600}".repeat(20) + &"z".repeat(100)));
    value.parameters_mut().set("tail", "final value").unwrap();
    for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for max_line_length in 60..=998 {
            let options = HeaderFormatOptions {
                parameter_encoding_method: ParameterEncodingMethod::Rfc2047,
                max_line_length,
                new_line_format,
                ..Default::default()
            };
            check_encoded_words(&round_trip(&value, &options));
        }
    }
}

#[test]
fn rfc2047_long_names_empty_values_and_exact_limits() {
    let options = HeaderFormatOptions {
        parameter_encoding_method: ParameterEncodingMethod::Rfc2047,
        max_line_length: 60,
        ..Default::default()
    };
    for name_length in 1..=70 {
        for text in ["", "abc", "a b", "\u{e9}", "\u{1f600}", "=?"] {
            let mut value = ContentDisposition::default();
            value
                .parameters_mut()
                .set(&"x".repeat(name_length), text)
                .unwrap();
            let before = value.clone();
            match value.to_header_string_with_options(&options) {
                Ok(output) => {
                    round_trip(&value, &options);
                    check_encoded_words(&output);
                }
                Err(HeaderFormatError::LineTooLong { required, limit }) => {
                    assert!(required > limit);
                    assert_eq!(limit, 60);
                }
                Err(error) => panic!("unexpected error: {error}"),
            }
            assert_eq!(value, before);
        }
    }
    let mut value = ContentDisposition::default();
    let name = "x".repeat(43);
    value.parameters_mut().set(&name, "a\u{1f525}").unwrap();
    let output = round_trip(&value, &options);
    assert!(output.contains(&format!("{name}=\"=?utf-8?q?a?=\r\n\t")));
    check_encoded_words(&output);

    let mut value = ContentDisposition::default();
    let name = "x".repeat(36);
    value.parameters_mut().set(&name, "\u{1f525}").unwrap();
    let output = round_trip(&value, &options);
    assert_eq!(output.lines().last().unwrap().len(), 60);
    value.parameters_mut().set("tail", "last").unwrap();
    assert!(matches!(
        value.to_header_string_with_options(&options),
        Err(HeaderFormatError::LineTooLong {
            required: 61,
            limit: 60
        })
    ));
}

#[test]
fn full_header_shape_and_ordinary_values() {
    let options = HeaderFormatOptions::default();
    assert_eq!(options.max_line_length, 78);
    assert_eq!(options.new_line_format, NewLineFormat::Dos);
    assert!(!options.always_quote_parameters);
    assert_eq!(
        ContentType::new("Text", "Plain")
            .unwrap()
            .to_header_string()
            .unwrap(),
        "Content-Type: Text/Plain\r\n"
    );
    assert_eq!(
        round_trip(&ContentDisposition::default(), &options),
        "Content-Disposition: attachment\r\n"
    );
    for (value, encoded) in [
        ("tps-report.doc", "tps-report.doc"),
        ("", "\"\""),
        ("two words", "\"two words\""),
        ("\\\"x", "\"\\\\\\\"x\""),
        ("100%*'done", "\"100%*'done\""),
        (" a ", "\" a \""),
    ] {
        assert_eq!(
            round_trip(&file_name(value), &options),
            format!("Content-Disposition: attachment; filename={encoded}\r\n")
        );
    }
}

#[test]
fn canonical_latin1_and_utf8_vectors() {
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    let mut value = ContentType::new("text", "plain").unwrap();
    value.set_charset(Some("iso-8859-1"));
    value.set_name(Some("Kristoffer Br\u{e5}nemyr"));
    let output = value.to_header_string_with_options(&options).unwrap();
    assert_eq!(
        output,
        "Content-Type: text/plain; charset=iso-8859-1;\n\tname*=iso-8859-1''Kristoffer%20Br%E5nemyr\n"
    );
    check_wire(&output, &options);

    value.set_charset(Some("utf-8"));
    value.set_name(Some(&"\u{e5}".repeat(40)));
    let output = value.to_header_string_with_options(&options).unwrap();
    assert_eq!(
        output,
        format!(
            "Content-Type: text/plain; charset=utf-8;\n\tname*0*=iso-8859-1''{};\n\tname*1*={}\n",
            "%E5".repeat(18),
            "%E5".repeat(22)
        )
    );
    check_wire(&output, &options);
    assert_eq!(
        ContentType::parse(output.split_once(':').unwrap().1.as_bytes()).unwrap(),
        value
    );

    let output = round_trip(&file_name(&"\u{2603}".repeat(40)), &options);
    let snowman = "%E2%98%83";
    let mut expected = format!(
        "Content-Disposition: attachment;\n\tfilename*0*=utf-8''{}",
        snowman.repeat(6)
    );
    for index in 1..=4 {
        expected.push_str(&format!(";\n\tfilename*{index}*={}", snowman.repeat(7)));
    }
    expected.push_str(&format!(";\n\tfilename*5*={}\n", snowman.repeat(6)));
    assert_eq!(output, expected);
}

#[test]
fn canonical_controls_and_literal_encoded_words() {
    let options = HeaderFormatOptions::default();
    assert_eq!(
        round_trip(&file_name("control\u{1}character.txt"), &options),
        "Content-Disposition: attachment;\r\n\tfilename*=iso-8859-1''control%01character.txt\r\n"
    );
    for value in [
        "=?utf-8?b?SGVsbG8=?=",
        "before =?iso-8859-1?q?caf=E9?= after",
        "=?",
        "x\r\nInjected: header\0.txt",
        "\u{7f}",
        "\u{80}\u{ff}",
        "\u{e9}\u{2603}",
    ] {
        let output = round_trip(&file_name(value), &options);
        assert!(!output.contains("=?"));
        assert!(output.contains("filename*="));
    }
    assert!(round_trip(&file_name("=?"), &options).contains("us-ascii''%3D%3F"));
    assert!(round_trip(&file_name("\u{e9}\u{2603}"), &options).contains("utf-8''%C3%A9%E2%98%83"));
}

#[test]
fn every_latin1_character_and_mixed_unicode_round_trip() {
    for always_quote_parameters in [false, true] {
        let options = HeaderFormatOptions {
            always_quote_parameters,
            ..Default::default()
        };
        for code in 0..=255 {
            let ch = char::from_u32(code).unwrap();
            round_trip(&file_name(&format!("before{ch}after")), &options);
        }
        let all: String = (0..=255)
            .map(|code| char::from_u32(code).unwrap())
            .collect();
        round_trip(&file_name(&all.repeat(4)), &options);
        round_trip(
            &file_name("a\u{301}\u{feff}\u{10000}\u{10ffff} \u{d7ff}\u{e000}"),
            &options,
        );
    }
}

#[test]
fn long_ascii_continuations_escape_quotes_and_preserve_spaces() {
    let options = HeaderFormatOptions::default();
    for value in [
        "this is a really really long filename that should force MimeKit to break it apart - yay!.html".to_owned(),
        "a".repeat(300),
        " \\\") abc".repeat(80),
        " ".repeat(300),
        "\"".repeat(300),
        "a".repeat(65) + " " + &"b".repeat(100),
    ] {
        let output = round_trip(&file_name(&value), &options);
        assert!(output.contains("filename*0="));
        assert!(!output.contains("filename*0*="));
    }
}

#[test]
fn quote_preference_applies_to_each_raw_section() {
    let options = HeaderFormatOptions {
        always_quote_parameters: true,
        ..Default::default()
    };
    assert_eq!(
        round_trip(&file_name("plain"), &options),
        "Content-Disposition: attachment; filename=\"plain\"\r\n"
    );
    let output = round_trip(&file_name(&"a".repeat(300)), &options);
    for line in output.lines().skip(1) {
        assert!(line.split_once('=').unwrap().1.starts_with('"'));
    }
    assert!(round_trip(&file_name("\u{e9}"), &options).contains("filename*=iso-8859-1''%E9"));
}

#[test]
fn non_bmp_characters_and_percent_triplets_are_never_split() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    for prefix_length in 0..160 {
        for unicode in ["\u{21861}", "\u{1f600}\u{1f4a9}\u{1f680}\u{1f4e7}"] {
            let value = "?".repeat(prefix_length) + unicode + &"x".repeat(80);
            let output = round_trip(&file_name(&value), &options);
            for (index, line) in output.lines().skip(1).enumerate() {
                let mut payload = line.split_once('=').unwrap().1.trim_end_matches(';');
                if index == 0 {
                    payload = payload.strip_prefix("utf-8''").unwrap();
                }
                let mut decoded = Vec::new();
                Rfc2231Decoder::new().decode(payload.as_bytes(), &mut decoded);
                assert!(std::str::from_utf8(&decoded).is_ok(), "{payload}");
                let bytes = payload.as_bytes();
                let mut offset = 0;
                while offset < bytes.len() {
                    if bytes[offset] == b'%' {
                        assert!(offset + 2 < bytes.len(), "{payload}");
                        assert!(
                            bytes[offset + 1..offset + 3]
                                .iter()
                                .all(u8::is_ascii_hexdigit)
                        );
                        offset += 3;
                    } else {
                        offset += 1;
                    }
                }
            }
        }
    }
}

#[test]
fn every_allowed_line_limit_is_obeyed() {
    let value = file_name(&("a\\\" ?".repeat(20) + "\u{e9}\u{1f600}" + &"b".repeat(1100)));
    for max_line_length in 60..=998 {
        round_trip(
            &value,
            &HeaderFormatOptions {
                max_line_length,
                ..Default::default()
            },
        );
    }
    for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in 0..160 {
            let mut value = file_name(&"x".repeat(length));
            value.parameters_mut().set("last", "a b").unwrap();
            round_trip(
                &value,
                &HeaderFormatOptions {
                    new_line_format,
                    ..Default::default()
                },
            );
        }
    }
}

#[test]
fn growing_section_numbers_and_following_parameters_fit() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    for text in ["a", " ", "\"", "\u{1f600}", "=?utf-8?q?literal?="] {
        let mut value = file_name(&text.repeat(12_000));
        value.parameters_mut().set("tail", "last").unwrap();
        let output = round_trip(&value, &options);
        assert!(output.contains("filename*100"));
        assert!(output.contains("tail=last"));
    }
}

#[test]
fn charset_only_first_section_and_long_names() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    let mut value = ContentDisposition::default();
    let name = "x".repeat(42);
    value.parameters_mut().set(&name, "\u{e9}").unwrap();
    let output = round_trip(&value, &options);
    assert!(output.contains(&format!("{name}*0*=iso-8859-1'';\r\n")));
    assert!(output.contains(&format!("{name}*1*=%E9\r\n")));

    for name_length in 1..=70 {
        for text in ["", "abc", "a b", "\u{e9}", "\u{1f600}", "=?"] {
            let mut value = ContentDisposition::default();
            value
                .parameters_mut()
                .set(&"x".repeat(name_length), text)
                .unwrap();
            let before = value.clone();
            match value.to_header_string_with_options(&options) {
                Ok(_) => {
                    round_trip(&value, &options);
                }
                Err(HeaderFormatError::LineTooLong { required, limit }) => {
                    assert!(required > limit);
                    assert_eq!(limit, 60);
                }
                Err(error) => panic!("unexpected error: {error}"),
            }
            assert_eq!(value, before);
        }
    }
}

#[test]
fn final_scalar_can_use_the_byte_not_needed_for_a_separator() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    let name = "x".repeat(43);
    let mut value = ContentDisposition::default();
    value.parameters_mut().set(&name, "\u{1f600}").unwrap();
    let output = round_trip(&value, &options);
    assert!(output.contains(&format!("{name}*0*=utf-8'';\r\n")));
    let last = output.lines().last().unwrap();
    assert_eq!(last, format!("\t{name}*1*=%F0%9F%98%80"));
    assert_eq!(last.len(), 60);

    value.parameters_mut().set("tail", "last").unwrap();
    assert_eq!(
        value.to_header_string_with_options(&options),
        Err(HeaderFormatError::LineTooLong {
            required: 61,
            limit: 60
        })
    );
}

#[test]
fn initial_value_folding_and_exact_line_limit() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    let mut disposition = ContentDisposition::new(&"a".repeat(59)).unwrap();
    assert_eq!(
        round_trip(&disposition, &options),
        format!("Content-Disposition:\r\n\t{}\r\n", "a".repeat(59))
    );
    disposition.parameters_mut().set("x", "y").unwrap();
    assert_eq!(
        disposition.to_header_string_with_options(&options),
        Err(HeaderFormatError::LineTooLong {
            required: 61,
            limit: 60
        })
    );
    let content_type = ContentType::new("text", &"x".repeat(54)).unwrap();
    let output = content_type
        .to_header_string_with_options(&options)
        .unwrap();
    assert_eq!(
        output,
        format!("Content-Type:\r\n\ttext/{}\r\n", "x".repeat(54))
    );
    check_wire(&output, &options);
    assert_eq!(
        ContentType::parse(output.split_once(':').unwrap().1.as_bytes()).unwrap(),
        content_type
    );
}

#[test]
fn option_and_unbreakable_piece_errors_are_explicit() {
    use std::error::Error;
    for length in [0, 1, 59, 999, usize::MAX] {
        let options = HeaderFormatOptions {
            max_line_length: length,
            ..Default::default()
        };
        let error = ContentDisposition::default()
            .to_header_string_with_options(&options)
            .unwrap_err();
        let HeaderFormatError::InvalidLineLength(invalid) = error else {
            panic!("{error}")
        };
        assert_eq!(invalid.requested(), length);
        assert!(error.source().is_some());
    }
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert_eq!(
        ContentType::new("text", "plain")
            .unwrap()
            .to_header_string_with_options(&options),
        Err(HeaderFormatError::InvalidNewLineFormat(
            NewLineFormat::Mixed
        ))
    );
    for text in ["", "abc", "\u{e9}", "\u{1f600}"] {
        let mut value = ContentDisposition::default();
        value.parameters_mut().set(&"x".repeat(1000), text).unwrap();
        assert!(matches!(
            value.to_header_string(),
            Err(HeaderFormatError::LineTooLong { .. })
        ));
    }
    assert!(matches!(
        ContentDisposition::new(&"a".repeat(1000))
            .unwrap()
            .to_header_string(),
        Err(HeaderFormatError::LineTooLong { .. })
    ));
}

#[test]
fn historical_diagnostics_order_and_casing_are_not_changed() {
    let mut value = ContentDisposition::parse(b"ATTACHMENT; X=one; x=two; FileName=old").unwrap();
    assert!(!value.parameters().issues().is_empty());
    value.set_file_name(Some("\u{2603}.txt"));
    let before = value.clone();
    let output = round_trip(&value, &HeaderFormatOptions::default());
    assert!(output.starts_with("Content-Disposition: ATTACHMENT; X=one;"));
    assert!(output.contains("FileName*=utf-8''"));
    assert_eq!(value, before);
}

#[test]
fn serialized_headers_integrate_with_incremental_raw_parser() {
    for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let options = HeaderFormatOptions {
            new_line_format,
            ..Default::default()
        };
        let mut content_type = ContentType::new("text", "plain").unwrap();
        content_type.set_charset(Some("utf-8"));
        content_type.set_name(Some("\u{1f4e7}.txt"));
        let disposition = file_name(&"document \u{e9}\u{2603}".repeat(30));
        let first = content_type
            .to_header_string_with_options(&options)
            .unwrap();
        let second = disposition.to_header_string_with_options(&options).unwrap();
        let block = first.clone() + &second;
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        for byte in block.as_bytes().chunks(1) {
            assert_eq!(parser.parse(byte, &mut headers).unwrap(), 1);
        }
        parser.finish(&mut headers).unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].raw(), first.as_bytes());
        assert_eq!(headers[1].raw(), second.as_bytes());
        assert_eq!(
            ContentType::parse(headers[0].raw_value()).unwrap(),
            content_type
        );
        assert_eq!(
            ContentDisposition::parse(headers[1].raw_value()).unwrap(),
            disposition
        );
    }
}

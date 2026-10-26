// Compatibility vectors adapted from MimeKit's ParameterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentType, HeaderFormatError, HeaderFormatOptions, HeaderId, HeaderList,
    Parameter, ParameterComplianceMode, ParameterEncodingMethod, ParameterParseOptions,
    charsets::Charset, encodings::NewLineFormat, rfc2047,
};

const CHINESE: &str = "\u{6d4b}\u{8bd5}\u{6587}\u{672c}";
const METHODS: [ParameterEncodingMethod; 2] = [
    ParameterEncodingMethod::Rfc2231,
    ParameterEncodingMethod::Rfc2047,
];

fn options() -> HeaderFormatOptions {
    HeaderFormatOptions {
        international: true,
        ..Default::default()
    }
}

fn disposition(text: &str) -> ContentDisposition {
    let mut value = ContentDisposition::default();
    value.set_file_name(Some(text));
    value
}

fn round_trip(value: &ContentDisposition, options: &HeaderFormatOptions) -> String {
    let before = value.clone();
    let output = value.to_header_string_with_options(options).unwrap();
    assert_eq!(*value, before);
    let newline = if options.new_line_format == NewLineFormat::Dos {
        "\r\n"
    } else {
        "\n"
    };
    for (index, line) in output
        .strip_suffix(newline)
        .unwrap()
        .split(newline)
        .enumerate()
    {
        assert!(!line.is_empty());
        assert!(line.len() <= options.max_line_length, "{line:?}");
        assert!(index == 0 || line.starts_with('\t'));
        assert!(line.bytes().all(|b| b == b'\t' || !b.is_ascii_control()));
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
        assert!(parsed.parameters().issues().is_empty(), "{parsed:?}");
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
        assert_eq!(
            parsed.to_header_string_with_options(options).unwrap(),
            output
        );
    }
    output
}

#[test]
fn upstream_short_and_long_international_values() {
    assert!(!HeaderFormatOptions::default().international);
    let short = disposition(&format!("{CHINESE}.doc"));
    assert_eq!(
        round_trip(&short, &options()),
        format!("Content-Disposition: attachment; filename=\"{CHINESE}.doc\"\r\n")
    );
    assert!(short.to_header_string().unwrap().is_ascii());

    let long = disposition(&format!("{}.doc", CHINESE.repeat(16)));
    // Retain the conservative payload budget and strict byte limit rather
    // than MimeKit's exact packing, but verify the same original value.
    let chunk = CHINESE.repeat(5);
    assert_eq!(
        round_trip(&long, &options()),
        format!(
            "Content-Disposition: attachment;\r\n\
             \tfilename*0=\"{chunk}\";\r\n\
             \tfilename*1=\"{chunk}\";\r\n\
             \tfilename*2=\"{chunk}\";\r\n\
             \tfilename*3=\"{CHINESE}.doc\"\r\n"
        )
    );
}

#[test]
fn raw_values_ignore_charset_preferences() {
    for text in ["caf\u{e9}.txt", CHINESE, "\u{1f600}", "A\u{301}"] {
        for repetitions in [1, 100] {
            let expected = round_trip(&disposition(&text.repeat(repetitions)), &options());
            assert!(!expected.is_ascii());
            assert!(!expected.contains("=?"));
            assert!(!expected.contains("*="));
            for label in ["us-ascii", "iso-8859-1", "gb18030", "iso-2022-jp", "utf-16"] {
                let charset = Charset::from_label(label).unwrap();
                for preference in [None, Some(charset)] {
                    let mut value = ContentDisposition::default();
                    let mut parameter =
                        Parameter::new("filename", text.repeat(repetitions)).unwrap();
                    parameter.set_charset(preference);
                    value.parameters_mut().add(parameter).unwrap();
                    assert_eq!(
                        round_trip(
                            &value,
                            &HeaderFormatOptions {
                                parameter_charset: charset,
                                ..options()
                            }
                        ),
                        expected
                    );
                }
            }
        }
    }
}

#[test]
fn continuations_preserve_utf8_and_quoted_pairs_at_every_limit() {
    // These non-ASCII scalars have low bytes that are ASCII attribute characters.
    for text in [
        "\u{161}",
        "\u{1041}",
        "\u{1f600}",
        "a\u{301}",
        "\"\\\u{e9} ",
    ] {
        for max_line_length in 60..=90 {
            for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
                for always_quote_parameters in [false, true] {
                    let mut value = disposition(&text.repeat(600));
                    value.parameters_mut().set("tail", "end").unwrap();
                    let output = round_trip(
                        &value,
                        &HeaderFormatOptions {
                            max_line_length,
                            new_line_format,
                            always_quote_parameters,
                            ..options()
                        },
                    );
                    assert!(output.contains("filename*10="));
                    for section in output.split(';').skip(1) {
                        let assignment = section.trim();
                        let (name, payload) = assignment.split_once('=').unwrap();
                        if name.starts_with("filename*") {
                            assert!(payload.starts_with('"'), "{assignment}");
                            assert!(payload.ends_with('"'), "{assignment}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn controls_and_encoded_word_lookalikes_still_use_encoding() {
    let mut values = (0u8..=31)
        .chain([127])
        .map(|byte| format!("{CHINESE}{}end", char::from(byte)))
        .collect::<Vec<_>>();
    values.extend([
        format!("{CHINESE}=?utf-8?q?literal?="),
        format!("{}=?utf-8?q?literal?=", CHINESE.repeat(40)),
        format!("{CHINESE}\r\nX-Injected: no"),
    ]);
    for method in METHODS {
        for text in &values {
            let output = round_trip(
                &disposition(text),
                &HeaderFormatOptions {
                    parameter_encoding_method: method,
                    max_line_length: 60,
                    ..options()
                },
            );
            assert!(output.is_ascii(), "{output:?}");
            assert!(!output.contains("\r\nX-Injected:"));
            assert!(
                output.contains(if method == ParameterEncodingMethod::Rfc2231 {
                    "utf-8''"
                } else {
                    "=?utf-8?"
                })
            );
        }
    }
}

#[test]
fn encoding_overrides_apply_only_when_raw_values_do_not_fit() {
    for method in METHODS {
        for repetitions in [1, 30] {
            let mut value = ContentDisposition::default();
            let mut parameter = Parameter::new("filename", CHINESE.repeat(repetitions)).unwrap();
            parameter.set_encoding_method(Some(method));
            parameter.set_always_quote(true);
            value.parameters_mut().add(parameter).unwrap();
            let opposite = if method == ParameterEncodingMethod::Rfc2231 {
                ParameterEncodingMethod::Rfc2047
            } else {
                ParameterEncodingMethod::Rfc2231
            };
            let output = round_trip(
                &value,
                &HeaderFormatOptions {
                    parameter_encoding_method: opposite,
                    ..options()
                },
            );
            assert_eq!(
                output.is_ascii(),
                repetitions > 1 && method == ParameterEncodingMethod::Rfc2047
            );
        }
    }
    for always_quote_parameters in [false, true] {
        let mut value = disposition("");
        value.parameters_mut().set("token", "plain").unwrap();
        let output = round_trip(
            &value,
            &HeaderFormatOptions {
                always_quote_parameters,
                ..options()
            },
        );
        assert!(output.contains("filename=\"\""));
        assert!(output.contains(if always_quote_parameters {
            "token=\"plain\""
        } else {
            "token=plain"
        }));
    }
}

#[test]
fn exact_byte_limits_include_quotes_scalars_and_separators() {
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..options()
    };
    for (name_length, scalar) in [(52, "\u{1f600}"), (53, "\u{4e00}"), (54, "\u{161}")] {
        let mut value = ContentDisposition::default();
        value
            .parameters_mut()
            .set(&"x".repeat(name_length), scalar)
            .unwrap();
        let output = round_trip(&value, &options);
        assert_eq!(output.lines().last().unwrap().len(), 60);
        value.parameters_mut().set("tail", "end").unwrap();
        let before = value.clone();
        assert_eq!(
            value.to_header_string_with_options(&options),
            Err(HeaderFormatError::LineTooLong {
                required: 63,
                limit: 60
            })
        );
        assert_eq!(value, before);
    }

    let mut value = ContentDisposition::default();
    value
        .parameters_mut()
        .set(&"x".repeat(49), "\u{1f600}\u{1f600}")
        .unwrap();
    let output = round_trip(&value, &options);
    assert_eq!(output.lines().nth(1).unwrap().len(), 60);
    assert_eq!(output.lines().nth(2).unwrap().len(), 59);
}

#[test]
fn typed_headers_and_list_edits_preserve_utf8_and_atomic_errors() {
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..options()
    };
    let mut content_type = ContentType::new("text", "plain").unwrap();
    content_type
        .parameters_mut()
        .set("x-eai-please-do-not", "abst\u{fc}rzen")
        .unwrap();
    let header = content_type.to_header_with_options(&options).unwrap();
    assert_eq!(header.id(), HeaderId::ContentType);
    assert_eq!(
        header.raw(),
        "Content-Type: text/plain; x-eai-please-do-not=\"abst\u{fc}rzen\"\n".as_bytes()
    );
    assert_eq!(
        ContentType::parse(header.raw_value()).unwrap(),
        content_type
    );

    let value = disposition(&format!("{CHINESE}.doc"));
    let mut headers = HeaderList::new();
    headers.add(header);
    headers.add(value.to_header().unwrap());
    headers
        .replace_all(value.to_header_with_options(&options).unwrap())
        .unwrap();
    assert_eq!(
        headers.get("Content-Disposition").unwrap().raw(),
        round_trip(&value, &options).as_bytes()
    );
    let before = headers.clone();
    let mut oversized = value.clone();
    oversized
        .parameters_mut()
        .set(&"x".repeat(100), CHINESE)
        .unwrap();
    let result = oversized
        .to_header_with_options(&options)
        .map(|header| headers.replace_all(header));
    assert!(matches!(result, Err(HeaderFormatError::LineTooLong { .. })));
    assert_eq!(headers, before);
}

// Adapted from MimeKit's UnitTests/HeaderTests.cs and Utils/Rfc2047Tests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::error::Error;

use mimecraft::{
    Header, HeaderId, HeaderList, UnstructuredHeaderFormatError as ErrorKind,
    UnstructuredHeaderFormatOptions as Options,
    charsets::Charset,
    encodings::NewLineFormat,
    rfc2047::{ComplianceMode, DecodeOptions, EncodeError, EncodeOptions, decode_text},
    utils::unfold,
};

fn round_trip(field: &str, value: &str, options: &Options) -> Header {
    let header = Header::from_unstructured_with_options(field, value, options).unwrap();
    assert_eq!(header.field(), Some(field));
    let raw = std::str::from_utf8(header.raw()).unwrap();
    if !options.international {
        assert!(raw.is_ascii());
    }
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => unreachable!(),
    };
    assert!(raw.ends_with(newline));
    let lines: Vec<_> = raw.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() <= options.max_line_length, "{line}");
        assert!(!line.is_empty());
        assert!(
            line.bytes().all(|byte| !byte.is_ascii_control()),
            "{line:?}"
        );
        if index > 0 {
            assert!(line.starts_with(' '));
        }
    }
    for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
        let decoded = header.decode_value_with_options(&DecodeOptions {
            compliance_mode: mode,
            ..Default::default()
        });
        assert_eq!(decoded.text, unfold(value), "{raw}");
        assert!(decoded.issues.is_empty(), "{:?}", decoded.issues);
    }
    let mut reader = header.raw();
    let loaded = HeaderList::load(&mut reader).unwrap();
    assert_eq!(loaded.len(), 1, "{raw}");
    assert_eq!(loaded.get(field), Some(&header));
    header
}

#[test]
fn defaults_and_simple_ascii_output() {
    let options = Options::default();
    assert_eq!(options.max_line_length, 78);
    assert_eq!(options.new_line_format, NewLineFormat::Dos);
    assert_eq!(options.encoding, EncodeOptions::default());
    assert!(!options.international);
    let header = round_trip("sUbJeCt", "Hello world", &options);
    assert_eq!(header.id(), HeaderId::Subject);
    assert_eq!(header.raw(), b"sUbJeCt: Hello world\r\n");
    assert_eq!(
        Header::from_unstructured("Subject", "Hello").unwrap().raw(),
        b"Subject: Hello\r\n"
    );
}

#[test]
fn assignment_trims_and_unfolds_without_decoding_literal_words() {
    for (value, expected) in [
        (" \t\r\nHello\r\n world \u{3000}", "Hello world"),
        ("a\r\nb\nc\rd", "abcd"),
        ("\u{a0} spaced  text \u{a0}", "spaced  text"),
        ("=?utf-8?q?hello?=", "=?utf-8?q?hello?="),
        ("literal =?broken?= text", "literal =?broken?= text"),
        ("a\0b\u{7f}c", "a\0b\u{7f}c"),
        ("a\tb  c", "a\tb  c"),
    ] {
        let header = round_trip("Subject", value, &Options::default());
        assert_eq!(header.decode_value().text, expected);
        assert!(!header.raw_value().contains(&0));
        assert!(!header.raw_value().contains(&0x7f));
    }
}

#[test]
fn empty_normalized_values_create_empty_headers() {
    for value in ["", " \t", "\r\n", "\u{3000}\u{a0}"] {
        let header = round_trip("Comments", value, &Options::default());
        assert_eq!(header.raw(), b"Comments:\r\n");
        assert_eq!(header.decode_value().text, "");
    }
}

#[test]
fn first_line_exact_limit_and_continuation_boundary() {
    let fits = "a".repeat(69);
    let header = round_trip("Subject", &fits, &Options::default());
    assert_eq!(header.raw(), format!("Subject: {fits}\r\n").as_bytes());
    let folds = "a".repeat(70);
    let header = round_trip("Subject", &folds, &Options::default());
    assert_eq!(header.raw(), format!("Subject:\r\n {folds}\r\n").as_bytes());

    let options = Options {
        max_line_length: 60,
        ..Default::default()
    };
    let fits = "a".repeat(59);
    let header = round_trip("Subject", &fits, &options);
    assert_eq!(header.raw(), format!("Subject:\r\n {fits}\r\n").as_bytes());
    let longer = round_trip("Subject", &"a".repeat(60), &options);
    assert!(std::str::from_utf8(longer.raw()).unwrap().contains("=?"));
}

#[test]
fn folding_preserves_repeated_spaces_even_across_whitespace_only_lines() {
    for spaces in [1, 2, 59, 60, 78, 79, 500] {
        for word_length in [1, 51, 59, 75] {
            let value = format!("{}{}last", "a".repeat(word_length), " ".repeat(spaces));
            for limit in [60, 78, 998] {
                round_trip(
                    "Subject",
                    &value,
                    &Options {
                        max_line_length: limit,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[test]
fn long_ascii_and_unicode_words_remain_complete_encoded_words() {
    for value in ["x".repeat(1000), "caf\u{e9} \u{1f600} ".repeat(50)] {
        for limit in [60, 78, 998] {
            let options = Options {
                max_line_length: limit,
                ..Default::default()
            };
            let header = round_trip("Subject", &value, &options);
            let mut decoded = String::new();
            for word in std::str::from_utf8(header.raw_value())
                .unwrap()
                .split_ascii_whitespace()
            {
                assert!(word.starts_with("=?"));
                assert!(word.ends_with("?="));
                assert!(word.len() <= 75.min(limit - 1));
                // Decode independently so a whole-run decoder cannot hide
                // splits inside a UTF-8 character or an encoded-word payload.
                let result = decode_text(word.as_bytes());
                assert!(result.issues.is_empty());
                decoded.push_str(&result.text);
            }
            assert_eq!(decoded, unfold(&value));
        }
    }
}

#[test]
fn unix_folding_and_explicit_charset_are_honored() {
    let options = Options {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        encoding: EncodeOptions {
            charset: Charset::LATIN1,
            max_word_length: 40,
        },
        ..Default::default()
    };
    let header = round_trip("Subject", &"caf\u{e9} ".repeat(40), &options);
    assert!(!header.raw().contains(&b'\r'));
    assert!(
        std::str::from_utf8(header.raw())
            .unwrap()
            .contains("=?iso-8859-1?")
    );
    for word in std::str::from_utf8(header.raw_value())
        .unwrap()
        .split_ascii_whitespace()
    {
        assert!(word.len() <= 40);
    }
}

#[test]
fn line_breaks_cannot_inject_headers_or_body_boundaries() {
    for value in [
        "hello\r\nBcc: injected@example.com",
        "hello\n\nbody",
        "hello\r\n\tfolded\rX-Injected: value",
        "=?utf-8?q?hello=0D=0ABcc:_injected?=",
    ] {
        let header = round_trip("Subject", value, &Options::default());
        let mut bytes = header.raw();
        let parsed = HeaderList::load(&mut bytes).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(!parsed.contains("Bcc"));
        assert!(!parsed.contains("X-Injected"));
    }
}

#[test]
fn invalid_fields_report_original_byte_offsets() {
    for (field, offset) in [
        ("", 0),
        ("Bad Name", 3),
        ("Bad:Name", 3),
        ("Bad\tName", 3),
        ("Bad\r\nName", 3),
        ("Bad\0Name", 3),
        ("X-\u{e9}", 2),
    ] {
        assert_eq!(
            Header::from_unstructured(field, "value"),
            Err(ErrorKind::InvalidFieldName { offset })
        );
    }
}

#[test]
fn field_prefix_must_fit_even_for_empty_text() {
    let options = Options {
        max_line_length: 60,
        ..Default::default()
    };
    let field = "X".repeat(59);
    round_trip(&field, "", &options);
    round_trip(&field, "value", &options);
    let error = Header::from_unstructured_with_options(&"X".repeat(60), "", &options).unwrap_err();
    assert_eq!(
        error,
        ErrorKind::LineTooLong {
            required: 61,
            limit: 60
        }
    );
    assert!(error.source().is_none());
    assert!(error.to_string().contains("61"));
}

#[test]
fn invalid_options_are_checked_before_capping_or_empty_output() {
    for max_line_length in [0, 59, 999, usize::MAX] {
        let options = Options {
            max_line_length,
            ..Default::default()
        };
        let error = Header::from_unstructured_with_options("Subject", "", &options).unwrap_err();
        assert!(matches!(error, ErrorKind::InvalidLineLength(_)));
        assert!(error.source().is_some());
    }
    let options = Options {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert_eq!(
        Header::from_unstructured_with_options("Subject", "", &options),
        Err(ErrorKind::InvalidNewLineFormat(NewLineFormat::Mixed))
    );
    for max_word_length in [0, 76, usize::MAX] {
        let options = Options {
            max_line_length: 60,
            encoding: EncodeOptions {
                max_word_length,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            Header::from_unstructured_with_options("Subject", "", &options),
            Err(ErrorKind::Encoding(EncodeError::InvalidWordLength {
                requested: max_word_length
            }))
        );
    }
}

#[test]
fn encoding_failures_and_offsets_follow_normalized_text() {
    let options = Options {
        encoding: EncodeOptions {
            charset: Charset::ASCII,
            ..Default::default()
        },
        ..Default::default()
    };
    let error =
        Header::from_unstructured_with_options("Subject", "caf\u{e9}", &options).unwrap_err();
    assert!(matches!(
        error,
        ErrorKind::Encoding(EncodeError::Charset(_))
    ));
    assert!(error.source().is_some());

    let options = Options {
        encoding: EncodeOptions {
            max_word_length: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let error =
        Header::from_unstructured_with_options("Subject", " \r\n\u{e9} ", &options).unwrap_err();
    assert!(matches!(
        error,
        ErrorKind::Encoding(EncodeError::WordTooLong {
            offset: 0,
            limit: 1,
            ..
        })
    ));
}

#[test]
fn international_short_values_are_utf8_even_with_a_legacy_charset() {
    for value in [
        "caf\u{e9}",
        "\u{6d4b}\u{8bd5}",
        "\u{1f600}",
        "a\u{301}",
        "a\u{a0}b",
    ] {
        for charset in [Charset::ASCII, Charset::LATIN1, Charset::UTF8] {
            for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
                let options = Options {
                    international: true,
                    new_line_format,
                    encoding: EncodeOptions {
                        charset,
                        max_word_length: 1,
                    },
                    ..Default::default()
                };
                let header = round_trip("sUbJeCt", value, &options);
                let newline = if new_line_format == NewLineFormat::Dos {
                    "\r\n"
                } else {
                    "\n"
                };
                assert_eq!(
                    header.raw(),
                    format!("sUbJeCt: {value}{newline}").as_bytes()
                );
            }
        }
    }
}

#[test]
fn international_folding_uses_byte_limits_not_scalar_counts() {
    for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let options = Options {
            international: true,
            new_line_format,
            ..Default::default()
        };
        let newline = if new_line_format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        let first_line = format!("{}abcde", "\u{1f600}".repeat(16));
        let header = round_trip("Subject", &first_line, &options);
        assert_eq!(
            header.raw(),
            format!("Subject: {first_line}{newline}").as_bytes()
        );
        assert_eq!(header.raw().len() - newline.len(), 78);
        let folded = format!("{first_line}x");
        let header = round_trip("Subject", &folded, &options);
        assert_eq!(
            header.raw(),
            format!("Subject:{newline} {folded}{newline}").as_bytes()
        );

        for limit in [60, 78, 998] {
            let options = Options {
                max_line_length: limit,
                ..options
            };
            let token = format!("{}\u{1f600}", "x".repeat(limit - 5));
            let header = round_trip("Subject", &token, &options);
            assert_eq!(
                header.raw(),
                format!("Subject:{newline} {token}{newline}").as_bytes()
            );
            let header = round_trip("Subject", &format!("{token}x"), &options);
            assert!(header.raw().is_ascii());
            assert!(
                std::str::from_utf8(header.raw())
                    .unwrap()
                    .contains("=?utf-8?")
            );
        }
    }
}

#[test]
fn international_folding_preserves_whitespace_and_upstream_values() {
    let options = Options {
        international: true,
        ..Default::default()
    };
    let original = "This is a subject value that should be long enough to force line wrapping to keep the line length under the 78 character limit.";
    let header = round_trip("Subject", original, &options);
    assert_eq!(
        unfold(std::str::from_utf8(header.raw_value()).unwrap()),
        original
    );
    for spaces in [1, 2, 59, 60, 78, 79, 500] {
        let value = format!(
            "{}{}\u{1f600} end",
            "caf\u{e9}".repeat(8),
            " ".repeat(spaces)
        );
        for limit in [60, 78, 998] {
            for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
                let header = round_trip(
                    "Subject",
                    &value,
                    &Options {
                        max_line_length: limit,
                        new_line_format,
                        ..options
                    },
                );
                assert!(!header.raw().is_ascii());
                assert_eq!(
                    unfold(std::str::from_utf8(header.raw_value()).unwrap()),
                    value
                );
            }
        }
    }

    // Match MimeKit's lossless Japanese folding regression: compare the decoded
    // value exactly, without removing spaces after unfolding.
    let japanese = concat!(
        "\u{72c2}\u{3063}\u{305f}\u{3053}\u{306e}\u{4e16}\u{3067}\u{72c2}\u{3046}",
        "\u{306a}\u{3089}\u{6c17}\u{306f}\u{78ba}\u{304b}\u{3060}\u{3002}"
    )
    .repeat(4);
    let header = round_trip("Subject", &japanese, &options);
    assert!(header.raw().is_ascii());
    let header = round_trip(
        "Subject",
        &japanese,
        &Options {
            max_line_length: 998,
            ..options
        },
    );
    assert!(!header.raw().is_ascii());
}

#[test]
fn upstream_internationalized_unstructured_header_round_trips() {
    let japanese = "\u{65e5}\u{672c}\u{8a9e}";
    let values = [
        "a".repeat(120),
        "a".repeat(60),
        "a".repeat(59),
        japanese.repeat(30),
        "\u{1f600}".repeat(40),
        format!("{}\u{1f600}{}", "a".repeat(50), "b".repeat(20)),
        format!(
            "short  {}   next\t\t{}  end",
            "a".repeat(80),
            "b".repeat(80)
        ),
        format!("{}   {japanese}   {}", "a".repeat(80), "b".repeat(80)),
        format!("{}{}{}", "a".repeat(80), " ".repeat(80), "b".repeat(80)),
        format!(
            "{}{}{}",
            "a".repeat(80),
            " ".repeat(80),
            "b\u{1f600}".repeat(40)
        ),
        format!("before{}after", " ".repeat(80)),
        "with_a_really_really_really_long_word_that_will_need_to_be_broken_up_in_order_to_fold"
            .to_owned(),
        format!("short  {japanese}  \u{1f600}"),
    ];
    for value in values {
        for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            for max_line_length in [60, 78] {
                let options = Options {
                    international: true,
                    new_line_format,
                    max_line_length,
                    ..Default::default()
                };
                let header = round_trip("Subject", &value, &options);
                // Reparse serialized bytes rather than trusting an assigned or
                // cached value; do not repair whitespace after decoding.
                let reparsed = Header::parse(header.raw()).unwrap();
                assert_eq!(reparsed.decode_value().text, value);
                for word in std::str::from_utf8(header.raw_value())
                    .unwrap()
                    .split_ascii_whitespace()
                {
                    if word.starts_with("=?") {
                        assert!(word.len() <= 75.min(max_line_length - 1));
                    }
                }
                if value.len() < 30 {
                    let newline = if new_line_format == NewLineFormat::Dos {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    assert_eq!(header.raw_value(), format!(" {value}{newline}").as_bytes());
                }
            }
        }
    }
}

#[test]
fn international_controls_and_literal_words_stay_encoded() {
    let options = Options {
        international: true,
        max_line_length: 60,
        ..Default::default()
    };
    for byte in (0u8..=31).chain([127]) {
        let value = format!("caf\u{e9}{}end", char::from(byte));
        let header = round_trip("Subject", &value, &options);
        assert_eq!(header.raw().is_ascii(), !matches!(byte, b'\r' | b'\n'));
    }
    for value in [
        "caf\u{e9} =?utf-8?q?literal?=",
        "caf\u{e9} =?broken?= text",
        "caf\u{e9}\r\nBcc: injected@example.com",
        "caf\u{e9}\r\n\r\nbody",
        "caf\u{e9}\r\n\tfolded\rX-Injected: no",
    ] {
        round_trip("Subject", value, &options);
    }
}

#[test]
fn international_fallback_keeps_characters_and_encoding_options() {
    for value in [
        format!("{}\u{1f600}{}", "a".repeat(50), "b".repeat(20)),
        "caf\u{e9}\0literal =?utf-8?q?text?=".to_owned(),
    ] {
        let header = round_trip(
            "Subject",
            &value,
            &Options {
                international: true,
                max_line_length: 60,
                encoding: EncodeOptions {
                    max_word_length: 40,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let mut decoded = String::new();
        for word in std::str::from_utf8(header.raw_value())
            .unwrap()
            .split_ascii_whitespace()
        {
            assert!(word.starts_with("=?utf-8?"));
            assert!(word.len() <= 40);
            // A supplementary scalar must not be split between encoded words.
            let result = decode_text(word.as_bytes());
            assert!(result.issues.is_empty());
            decoded.push_str(&result.text);
        }
        assert_eq!(decoded, value);
    }
    let options = Options {
        international: true,
        encoding: EncodeOptions {
            charset: Charset::LATIN1,
            ..Default::default()
        },
        ..Default::default()
    };
    let header = round_trip("Subject", "caf\u{e9}\0", &options);
    assert!(
        std::str::from_utf8(header.raw())
            .unwrap()
            .contains("=?iso-8859-1?")
    );
    assert!(matches!(
        Header::from_unstructured_with_options("Subject", "\u{1f600}\0", &options),
        Err(ErrorKind::Encoding(EncodeError::Charset(_)))
    ));
}

#[test]
fn international_mode_still_validates_options_and_empty_headers() {
    for value in ["", " \t\r\n\u{3000}", "caf\u{e9}"] {
        for options in [
            Options {
                max_line_length: 59,
                ..Default::default()
            },
            Options {
                max_line_length: 999,
                ..Default::default()
            },
            Options {
                new_line_format: NewLineFormat::Mixed,
                ..Default::default()
            },
            Options {
                encoding: EncodeOptions {
                    max_word_length: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            Options {
                encoding: EncodeOptions {
                    max_word_length: 76,
                    ..Default::default()
                },
                ..Default::default()
            },
        ] {
            let expected =
                Header::from_unstructured_with_options("Subject", value, &options).unwrap_err();
            assert_eq!(
                Header::from_unstructured_with_options(
                    "Subject",
                    value,
                    &Options {
                        international: true,
                        ..options
                    }
                ),
                Err(expected)
            );
        }
    }
    let options = Options {
        international: true,
        max_line_length: 60,
        ..Default::default()
    };
    for value in ["", " \t\r\n\u{3000}"] {
        assert_eq!(
            round_trip("Subject", value, &options).raw(),
            b"Subject:\r\n"
        );
    }
    round_trip(&"X".repeat(59), "caf\u{e9}", &options);
    assert_eq!(
        Header::from_unstructured_with_options(&"X".repeat(60), "", &options),
        Err(ErrorKind::LineTooLong {
            required: 61,
            limit: 60
        })
    );
    for field in ["", "X:Bad", "X\r\nBad", "X-\u{e9}"] {
        assert!(matches!(
            Header::from_unstructured_with_options(field, "caf\u{e9}", &options),
            Err(ErrorKind::InvalidFieldName { .. })
        ));
    }
    let too_small = Options {
        encoding: EncodeOptions {
            max_word_length: 1,
            ..Default::default()
        },
        ..options
    };
    assert!(matches!(
        Header::from_unstructured_with_options("Subject", " \r\n\u{e9}\0 ", &too_small),
        Err(ErrorKind::Encoding(EncodeError::WordTooLong {
            offset: 0,
            limit: 1,
            ..
        }))
    ));
}

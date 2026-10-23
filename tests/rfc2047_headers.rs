// Includes decoding cases adapted from MimeKit's Utils/Rfc2047Tests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    HeaderParser,
    charsets::Charset,
    encodings::{QEncodeMode, Rfc2047Base64Encoder, Rfc2047QuotedPrintableEncoder},
    rfc2047::{
        ComplianceMode, DecodeIssue, DecodeOptions, DecodedHeader, decode_phrase,
        decode_phrase_with_options, decode_text, decode_text_with_options,
    },
};

type Decoder = fn(&[u8], &DecodeOptions) -> DecodedHeader;
const DECODERS: [Decoder; 2] = [decode_text_with_options, decode_phrase_with_options];

fn assert_both(input: &[u8], expected: &str) {
    for decode in DECODERS {
        assert_eq!(
            decode(input, &DecodeOptions::default()).text,
            expected,
            "{input:?}"
        );
    }
}

#[test]
fn canonical_encoded_word_vectors() {
    for (input, expected) in [
        ("=?utf-8?q?Ren=C3=A9e?=", "Ren\u{e9}e"),
        ("=?utf-8?q?=E6=9D=8E=F0=9F=98=80?=", "\u{674e}\u{1f600}"),
        ("=?utf-8?q?=C3?= =?utf-8?q?=A9?=", "\u{e9}"),
        ("=?utf-8?b?ww==?= =?utf-8?b?qQ==?=", "\u{e9}"),
        ("=?utf-8?q?=C3=A9=FF?=", "\u{e9}\u{fffd}"),
        ("=?utf-8?q?=E2=82?=", "\u{fffd}"),
        ("=?unknown?q?=C3=A9?=", "\u{e9}"),
        ("=?unknown?q?=C3=A9=FF?=", "\u{c3}\u{a9}\u{ff}"),
        ("=?iso-8859-1?q?=C3=A9?=", "\u{c3}\u{a9}"),
        ("=?utf-16?b?QQBCAGkA?=", "ABi"),
        (
            "=?utf-8?q?=C3=A9?= =?iso-8859-1?q?=E9?= =?utf-8?q?=F0=9F=98=80?=",
            "\u{e9}\u{e9}\u{1f600}",
        ),
        ("=?iso-8859-1*en?q?this_is_english?=", "this is english"),
        (
            "=?iso-8859-1?q?latin1_?= =?utf-8?q?unicode_?= =?iso-8859-1?q?and_latin1_again?=",
            "latin1 unicode and latin1 again",
        ),
        (
            "=?iso-8859-1?q?latin1_?= =?iso-8859-2?q?latin2_?= =?iso-8859-3?q?latin3?=",
            "latin1 latin2 latin3",
        ),
        (
            "=?us-ascii?q?0?= =?iso-8859-1?q?1?= =?iso-8859-2?q?2?= =?iso-8859-3?q?3?= =?iso-8859-4?q?4?= =?iso-8859-5?q?5?= =?iso-8859-6?q?6?= =?iso-8859-7?q?7?= =?iso-8859-8?q?8?= =?iso-8859-9?q?9?= =?koi8-r?q?a?= =?koi8-u?q?b?= =?big5?q?c?= =?euc-cn?q?d?= =?euc-kr?q?e?= =?utf-8?q?f?= =?gb2312?q?g?=",
            "0123456789abcdefg",
        ),
    ] {
        let wrapped = format!("!prefix {input} suffix!");
        let bytes = &wrapped.as_bytes()[1..wrapped.len() - 1];
        for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
            let options = DecodeOptions {
                compliance_mode: mode,
                fallback_charset: Some(Charset::LATIN1),
            };
            for decode in DECODERS {
                assert_eq!(
                    decode(bytes, &options).text,
                    format!("prefix {expected} suffix"),
                    "{input}"
                );
            }
        }
    }
}

#[test]
fn malformed_wrappers_remain_literal() {
    for word in [
        "=??q?no_charset?=",
        "=?*en?q?no_charset?=",
        "=?iso-8859-1?x?invalid_encoding?=",
        "=?iso-8859-1?qb?invalid_encoding?=",
        "=?iso-8859-1?q?unfinished",
        "=?iso-8859-1",
        "=?is\u{f6}-8859-1?q?invalid_charset?=",
        "=?iso-8859-1*e\u{f1}-US?q?invalid_language?=",
        "=?utf-8?q?=",
        "=?",
        "=?a",
        "=?a?=",
        "=?a*q?=",
        "=?a?q?=",
        "=?utf-8:q?bad?=",
        "=?utf-8?Qx?bad?=",
    ] {
        let input = format!("blurdy bloop {word} beep boop");
        for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
            for decode in DECODERS {
                let result = decode(
                    input.as_bytes(),
                    &DecodeOptions {
                        compliance_mode: mode,
                        ..Default::default()
                    },
                );
                assert_eq!(result.text, input);
                assert!(result.issues.is_empty(), "{word}: {:?}", result.issues);
            }
        }
    }
}

#[test]
fn loose_recovery_keeps_later_words() {
    for (input, expected) in [
        ("abc=? =?utf-8?q?caf=C3=A9?=", "abc=? caf\u{e9}"),
        ("=? =?utf-8?q?caf=C3=A9?=", "=? caf\u{e9}"),
        (
            "x =?foo bar =?utf-8?q?caf=C3=A9?= end",
            "x =?foo bar caf\u{e9} end",
        ),
        ("=?utf-8?x? =?utf-8?q?caf=C3=A9?=", "=?utf-8?x? caf\u{e9}"),
        (
            "=?utf-8?q?foo, bar =?utf-8?q?caf=C3=A9?=",
            "=?utf-8?q?foo, bar caf\u{e9}",
        ),
        (
            "=?utf-8?q?foo=?utf-8?q?caf=C3=A9?=",
            "=?utf-8?q?foocaf\u{e9}",
        ),
        ("=?iso-8859-1?q?caf=E9 au lait?=", "caf\u{e9} au lait"),
        (
            "blurdy bloop=?iso-8859-1?q?_encoded_word_?=beep boop",
            "blurdy bloop encoded word beep boop",
        ),
        ("=?utf-8?q?one?==?utf-8?q?two?=", "onetwo"),
    ] {
        assert_both(input.as_bytes(), expected);
    }
}

#[test]
fn strict_boundaries_differ_for_text_and_phrases() {
    let options = DecodeOptions {
        compliance_mode: ComplianceMode::Strict,
        ..Default::default()
    };
    for input in ["prefix=?utf-8?q?word?=suffix", "=?utf-8?q?two words?="] {
        assert_eq!(
            decode_text_with_options(input.as_bytes(), &options).text,
            input
        );
        assert_eq!(
            decode_phrase_with_options(input.as_bytes(), &options).text,
            input
        );
    }
    let input = b"(=?utf-8?q?word?=), \"=?utf-8?q?name?=\"";
    assert_eq!(
        decode_text_with_options(input, &options).text.as_bytes(),
        input
    );
    assert_eq!(
        decode_phrase_with_options(input, &options).text,
        "(word), \"name\""
    );
    assert_eq!(decode_phrase(input).text, "(word), \"name\"");

    // Strict source tokenization can treat adjacent wrappers as one outer word.
    let input = b"=?utf-8?q?one?==?utf-8?q?two?=";
    for decode in DECODERS {
        assert_eq!(decode(input, &options).text, "one?==?utf-8?q?two");
    }
}

#[test]
fn whitespace_is_only_removed_between_encoded_words() {
    for separator in ["", " ", "\t", "\r\n ", "\r", "\n", " \t\r\n\t "] {
        let input = format!(" \t=?utf-8?q?one_?={separator}=?latin1?b?dHdv?=\r\n ");
        assert_both(input.as_bytes(), " \tone two\r\n ");
    }
    for input in [
        "",
        " \t\r\n ",
        "ordinary\r\n folded",
        "\"quoted\\\" name\" (comment)",
        "a\u{b}b\u{c}c\0",
    ] {
        assert_both(input.as_bytes(), input);
    }
    assert_both(b"=?utf-8?q?one?= \t, \t=?utf-8?q?two?=", "one \t, \ttwo");
    assert_both(
        b"=?utf-8?q?one?= =??q?bad?= =?utf-8?q?two?=",
        "one =??q?bad?= two",
    );
}

#[test]
fn empty_payloads_languages_and_case_aliases() {
    for input in ["=?utf-8?q??=", "=?utf-8?B??=", "=?utf-8?q??= \t=?UTF8?B??="] {
        for decode in DECODERS {
            let decoded = decode(input.as_bytes(), &DecodeOptions::default());
            assert_eq!(decoded.text, "");
            assert!(decoded.issues.is_empty());
        }
    }
    assert_both(b"=?UTF-8*?Q?=C3?= =?unicode*en*US?q?=A9?=", "\u{e9}");
    assert_both(b"=?utf8*en?b?ww==?= =?UTF-8*fr?B?qQ==?=", "\u{e9}");
    assert_both(b"a =?utf-8?q??= b", "a  b");
}

#[test]
fn payload_and_character_state_cross_word_boundaries() {
    assert_both(b"=?utf-8?q?=C?= =?utf8?Q?3=A?= =?unicode?q?9?=", "\u{e9}");
    assert_both(b"=?utf-8?b?w?= =?utf8?B?6?= =?unicode?b?k=?=", "\u{e9}");
    assert_both(
        b"=?iso-2022-jp?q?=1B$?= =?iso-2022-jp?q?B$\"=1B(B?=",
        "\u{3042}",
    );
    assert_both(b"=?utf-16?q?=3D=D8?= =?utf-16le?q?=00=DE?=", "\u{1f600}");
}

#[test]
fn run_boundaries_reset_payload_and_charset_state() {
    for input in [
        b"=?utf-8?q?=C3?= =?utf-8?b?qQ==?=".as_slice(),
        b"=?utf-8?b?ww==?= =?us-ascii?q?=A9?=",
    ] {
        for decode in DECODERS {
            let result = decode(input, &DecodeOptions::default());
            assert_eq!(result.text, "\u{fffd}\u{fffd}");
            assert_eq!(result.issues.len(), 2);
        }
    }
    assert_both(b"=?utf-8?q?=C3?=x=?utf-8?q?=A9?=", "\u{fffd}x\u{fffd}");
    assert_both(b"=?utf-8?q?=C?= x =?utf-8?q?3?=", " x 3");
    assert_both(b"=?utf-8?b?w?= x =?utf-8?b?6k=?=", " x ");
    assert_both(b"=?utf-8?q?=C?= =?latin1?q?3?=", "3");
    assert_both(b"=?utf-8?q?=C?= =?utf-8?b?QQ==?= =?utf-8?q?3?=", "A3");
}

#[test]
fn unsupported_labels_join_and_report_selected_charset() {
    for (input, expected, selected) in [
        ("=?x-one?q?=C3?= \t=?x-two?Q?=A9?=", "\u{e9}", Charset::UTF8),
        (
            "=?unknown?q?=C3=A9=FF?=",
            "\u{c3}\u{a9}\u{ff}",
            Charset::LATIN1,
        ),
        ("=?unknown?q??=", "", Charset::UTF8),
    ] {
        for decode in DECODERS {
            let result = decode(input.as_bytes(), &DecodeOptions::default());
            assert_eq!(result.text, expected);
            assert_eq!(
                result.issues,
                [DecodeIssue::UnsupportedCharset {
                    range: 0..input.len(),
                    selected
                }]
            );
        }
    }
    let windows = Charset::from_label("windows-1252").unwrap();
    let options = DecodeOptions {
        fallback_charset: Some(windows),
        ..Default::default()
    };
    let input = b"=?unknown?q?=80?=";
    let result = decode_text_with_options(input, &options);
    assert_eq!(result.text, "\u{20ac}");
    assert_eq!(
        result.issues,
        [DecodeIssue::UnsupportedCharset {
            range: 0..input.len(),
            selected: windows
        }]
    );
    assert_eq!(
        decode_text_with_options(b"=?latin1?q?=80?=", &options).text,
        "\u{80}"
    );
    assert_eq!(
        decode_text_with_options(b"=?unknown?q?=C3=A9?=", &options).text,
        "\u{e9}"
    );
}

#[test]
fn replacement_diagnostics_cover_whole_source_runs() {
    let word = "=?utf-8?q?=C3?= \r\n =?UTF8?q?=A9=FF?=";
    let input = format!("prefix {word} suffix");
    for decode in DECODERS {
        let result = decode(input.as_bytes(), &DecodeOptions::default());
        assert_eq!(result.text, "prefix \u{e9}\u{fffd} suffix");
        assert_eq!(
            result.issues,
            [DecodeIssue::InvalidBytes {
                range: 7..7 + word.len(),
                charset: Charset::UTF8
            }]
        );
    }
    let input = b"=?utf-8?q?=EF=BF=BD?=";
    let result = decode_text(input);
    assert_eq!(result.text, "\u{fffd}");
    assert!(result.issues.is_empty());

    let first = b"=?unknown?q?ok?=";
    let last = b"=?us-ascii?q?=FF?=";
    let input = [first.as_slice(), b" \xff ", last].concat();
    let result = decode_text(&input);
    assert_eq!(result.text, "ok \u{ff} \u{fffd}");
    assert_eq!(
        result.issues,
        [
            DecodeIssue::UnsupportedCharset {
                range: 0..first.len(),
                selected: Charset::UTF8
            },
            DecodeIssue::UnlabelledText {
                range: first.len() + 1..first.len() + 2,
                selected: Charset::LATIN1
            },
            DecodeIssue::InvalidBytes {
                range: first.len() + 3..input.len(),
                charset: Charset::ASCII
            },
        ]
    );
}

#[test]
fn canonical_raw_byte_fallback_vectors() {
    let windows = Charset::from_label("windows-1252").unwrap();
    for (bytes, preferred, expected, selected) in [
        (
            b"\xc3\xa9".as_slice(),
            Charset::LATIN1,
            "\u{e9}",
            Charset::UTF8,
        ),
        (
            b"\xc3\xa9\xff",
            Charset::LATIN1,
            "\u{c3}\u{a9}\u{ff}",
            Charset::LATIN1,
        ),
        (
            b"\xe2\x82",
            Charset::LATIN1,
            "\u{e2}\u{82}",
            Charset::LATIN1,
        ),
        (b"\x80", windows, "\u{20ac}", windows),
    ] {
        let input = [b"!prefix ".as_slice(), bytes, b" suffix!"].concat();
        let input = &input[1..input.len() - 1];
        for decode in DECODERS {
            let result = decode(
                input,
                &DecodeOptions {
                    fallback_charset: Some(preferred),
                    ..Default::default()
                },
            );
            assert_eq!(result.text, format!("prefix {expected} suffix"));
            if selected == Charset::UTF8 {
                assert!(result.issues.is_empty());
            } else {
                assert_eq!(
                    result.issues,
                    [DecodeIssue::UnlabelledText {
                        range: 7..7 + bytes.len(),
                        selected
                    }]
                );
            }
        }
    }
}

#[test]
fn phrase_tokenization_affects_raw_fallback_boundaries() {
    let input = b"\xc3\xa9(\xff)";
    assert_eq!(decode_text(input).text, "\u{c3}\u{a9}(\u{ff})");
    assert_eq!(decode_phrase(input).text, "\u{c3}\u{a9}(\u{ff})");
    let input = b"a\xc3\xa9b\xff";
    assert_eq!(decode_phrase(input).text, "a\u{c3}\u{a9}b\u{ff}");
    let options = DecodeOptions {
        compliance_mode: ComplianceMode::Strict,
        ..Default::default()
    };
    assert_eq!(
        decode_phrase_with_options(input, &options).text,
        "a\u{e9}b\u{ff}"
    );
    assert_eq!(
        decode_text_with_options(input, &options).text,
        "a\u{c3}\u{a9}b\u{ff}"
    );
}

#[test]
fn tolerant_decoding_is_not_validation_or_sanitization() {
    assert_both(b"=?utf-8?b?S!G@V#s$bG8=?=", "Hello");
    assert_both(b"=?utf-8?q?bad=XY_escapes?=", "bad=XY escapes");
    assert_both(b"=?utf-8?q?pending=?=", "pending");
    assert_both(b"=?utf-8?b?YQ?=", "");
    assert_both(b"=?utf-8?q?=00=0D=0A?=", "\0\r\n");
    let long = "a".repeat(1000);
    assert_both(format!("=?utf-8?q?{long}?=").as_bytes(), &long);
    assert_both(b"=?utf-8?q?=EF=BB=BFhello?=", "\u{feff}hello");
}

#[test]
fn all_payload_splits_round_trip_with_existing_encoders() {
    let text = "Hello =_?\0\r\n Ren\u{e9}e \u{674e}\u{1f600}";
    let mut base64 = Vec::new();
    Rfc2047Base64Encoder::new().encode(text.as_bytes(), &mut base64);
    let mut quoted = Vec::new();
    Rfc2047QuotedPrintableEncoder::new(QEncodeMode::Phrase).encode(text.as_bytes(), &mut quoted);
    for (encoding, payload) in [('B', base64), ('Q', quoted)] {
        let payload = std::str::from_utf8(&payload).unwrap();
        for split in 0..=payload.len() {
            let input = format!(
                "=?utf-8?{encoding}?{}?=\r\n =?UTF8?{encoding}?{}?=",
                &payload[..split],
                &payload[split..]
            );
            for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
                for decode in DECODERS {
                    let result = decode(
                        input.as_bytes(),
                        &DecodeOptions {
                            compliance_mode: mode,
                            ..Default::default()
                        },
                    );
                    assert_eq!(result.text, text, "{input}");
                    assert!(result.issues.is_empty());
                }
            }
        }
    }
}

#[test]
fn raw_header_integration_preserves_original_records() {
    let input = b"Subject: =?utf-8?q?=C3?=\r\n\t=?utf8?q?=A9?=\r\n\r\nbody";
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    let consumed = parser.parse(input, &mut headers).unwrap();
    assert_eq!(&input[consumed..], b"body");
    let original = headers[0].raw().to_vec();
    let result = decode_text(headers[0].raw_value());
    assert_eq!(result.text, " \u{e9}\r\n");
    assert!(result.issues.is_empty());
    assert_eq!(headers[0].raw(), original);
}

fn assert_ranges(result: &DecodedHeader, length: usize) {
    let mut previous_end = 0;
    for issue in &result.issues {
        let range = match issue {
            DecodeIssue::UnsupportedCharset { range, .. }
            | DecodeIssue::InvalidBytes { range, .. }
            | DecodeIssue::UnlabelledText { range, .. } => range,
        };
        assert!(previous_end <= range.start && range.start < range.end && range.end <= length);
        previous_end = range.end;
    }
}

#[test]
fn arbitrary_bytes_truncated_wrappers_and_long_runs_are_bounded() {
    let all_bytes: Vec<u8> = (0..=255).collect();
    let expected: String = all_bytes.iter().copied().map(char::from).collect();
    assert_both(&all_bytes, &expected);
    let examples: &[&[u8]] = &[
        b"=?utf-8*q?bad?= =?utf-8?q?=C3=A9?=",
        b"=?a*?B?YQ==?=",
        b"=?utf-8?q?broken=?unknown?q?=FF?=",
        b" \r\n=?utf8?q??=!?=",
    ];
    for input in examples {
        for start in 0..=input.len() {
            for end in start..=input.len() {
                for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
                    for decode in DECODERS {
                        let result = decode(
                            &input[start..end],
                            &DecodeOptions {
                                compliance_mode: mode,
                                ..Default::default()
                            },
                        );
                        assert_ranges(&result, end - start);
                    }
                }
            }
        }
    }
    let mut seed = 0x1234_5678_u32;
    for length in 0..512 {
        let input: Vec<u8> = (0..length)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for mode in [ComplianceMode::Loose, ComplianceMode::Strict] {
            for decode in DECODERS {
                assert_ranges(
                    &decode(
                        &input,
                        &DecodeOptions {
                            compliance_mode: mode,
                            ..Default::default()
                        },
                    ),
                    length,
                );
            }
        }
    }
    let input = "=?utf-8?q?x?= \r\n ".repeat(10_000);
    assert_both(input.as_bytes(), &format!("{} \r\n ", "x".repeat(10_000)));
}

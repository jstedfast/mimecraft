// Encoding cases adapted from MimeKit's UnitTests/Utils/Rfc2047Tests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    charsets::{Charset, CharsetError},
    encodings::{Base64Decoder, QuotedPrintableDecoder},
    rfc2047::{
        ComplianceMode, DecodeOptions, EncodeError, EncodeOptions, decode_phrase_with_options,
        decode_text, decode_text_with_options, encode_phrase, encode_phrase_with_options,
        encode_text, encode_text_with_options,
    },
    utils::unquote,
};

fn check_words(encoded: &str, expected: &str, options: &EncodeOptions, phrase: bool) {
    let mut decoded = String::new();
    for word in encoded.split(' ') {
        assert!(word.len() <= options.max_word_length, "{word}");
        let fields: Vec<_> = word.split('?').collect();
        assert_eq!(fields.len(), 5, "{word}");
        assert_eq!(fields[0], "=");
        assert_eq!(fields[1], options.charset.name());
        assert_eq!(fields[4], "=");
        let payload = fields[3].as_bytes();
        assert!(!payload.is_empty());
        assert!(payload.iter().all(|byte| (33..=126).contains(byte)));
        let mut bytes = Vec::new();
        match fields[2] {
            "b" => {
                assert_eq!(payload.len() % 4, 0);
                Base64Decoder::new().decode(payload, &mut bytes);
            }
            "q" => {
                let mut offset = 0;
                while offset < payload.len() {
                    if payload[offset] == b'=' {
                        assert!(offset + 2 < payload.len());
                        assert!(
                            payload[offset + 1..offset + 3]
                                .iter()
                                .all(u8::is_ascii_hexdigit)
                        );
                        offset += 3;
                    } else {
                        if phrase {
                            assert!(
                                payload[offset].is_ascii_alphanumeric()
                                    || b"!*+-/_".contains(&payload[offset])
                            );
                        }
                        offset += 1;
                    }
                }
                QuotedPrintableDecoder::for_encoded_words().decode(payload, &mut bytes);
            }
            method => panic!("unexpected encoding: {method}"),
        }
        // Decode each word separately: a successful decode of the whole run
        // could conceal split UTF-8, surrogate pairs, or charset shift state.
        decoded.push_str(&options.charset.decode(&bytes).unwrap());
    }
    assert_eq!(decoded, expected);
}

fn round_trip(text: &str, options: &EncodeOptions, phrase: bool) -> String {
    let encoded = if phrase {
        encode_phrase_with_options(text, options).unwrap()
    } else {
        encode_text_with_options(text, options).unwrap()
    };
    assert!(encoded.is_ascii());
    assert!(encoded.bytes().all(|byte| !byte.is_ascii_control()));
    let syntax = if phrase {
        unquote(&encoded, false)
    } else {
        (&encoded).into()
    };
    for compliance_mode in [ComplianceMode::Strict, ComplianceMode::Loose] {
        let options = DecodeOptions {
            compliance_mode,
            ..Default::default()
        };
        let decoded = if phrase {
            decode_phrase_with_options(syntax.as_bytes(), &options)
        } else {
            decode_text_with_options(syntax.as_bytes(), &options)
        };
        assert_eq!(decoded.text, text, "{encoded}");
        assert!(decoded.issues.is_empty(), "{encoded}");
    }
    if encoded.starts_with("=?") {
        check_words(&encoded, text, options, phrase);
        // Folding is the caller's responsibility. Separators may be folded
        // without changing the original whitespace stored inside the payloads.
        assert_eq!(
            decode_text(encoded.replace(' ', "\r\n\t").as_bytes()).text,
            text
        );
    }
    encoded
}

#[test]
fn defaults_and_plain_text_are_literal() {
    let options = EncodeOptions::default();
    assert_eq!(options.charset, Charset::UTF8);
    assert_eq!(options.max_word_length, 75);
    for text in [
        "",
        "Hello world",
        "  leading and trailing  ",
        "a  b",
        "a:b;c(d)\"e\\f",
    ] {
        assert_eq!(round_trip(text, &options, false), text);
    }
    assert_eq!(encode_text(&"a".repeat(75)).unwrap(), "a".repeat(75));
    assert!(
        encode_text(&"a".repeat(76))
            .unwrap()
            .starts_with("=?utf-8?")
    );
    let text = "abc ".repeat(1000);
    assert_eq!(encode_text(&text).unwrap(), text);
}

#[test]
fn canonical_ascii_phrase_quoting() {
    let options = EncodeOptions::default();
    for (text, expected) in [
        ("", "\"\""),
        ("Jane Doe", "Jane Doe"),
        ("Jane.Doe", "\"Jane.Doe\""),
        ("Doe, Jane", "\"Doe, Jane\""),
        (
            "John \"Jacob Jingle Heimer\" Schmidt",
            "\"John \\\"Jacob Jingle Heimer\\\" Schmidt\"",
        ),
        (
            "John (Jacob Jingle Heimer) Schmidt",
            "\"John (Jacob Jingle Heimer) Schmidt\"",
        ),
        ("a\\b", "\"a\\\\b\""),
        (" leading ", "\" leading \""),
        ("a  b", "\"a  b\""),
        (" ", "\" \""),
    ] {
        assert_eq!(round_trip(text, &options, true), expected);
    }
    let text = "Once upon a time, back when things that are old now were new, there lived a man with a very particular set of skills.";
    assert!(round_trip(text, &options, true).starts_with("=?"));
}

#[test]
fn exact_b_and_q_output() {
    assert_eq!(
        encode_text("Hello Ren\u{e9}e").unwrap(),
        "=?utf-8?q?Hello_Ren=C3=A9e?="
    );
    assert_eq!(encode_text("\u{1f600}").unwrap(), "=?utf-8?b?8J+YgA==?=");
    assert_eq!(encode_phrase("\u{1f600}").unwrap(), "=?utf-8?b?8J+YgA==?=");
    assert_eq!(encode_text("\u{7}").unwrap(), "=?utf-8?q?=07?=");
    assert_eq!(encode_text("=?").unwrap(), "=?utf-8?b?PT8=?=");
    let text = "a".repeat(20) + "'\u{1}";
    assert_eq!(
        encode_text(&text).unwrap(),
        format!("=?utf-8?q?{}'=01?=", "a".repeat(20))
    );
    assert_eq!(
        encode_phrase(&text).unwrap(),
        format!("=?utf-8?q?{}=27=01?=", "a".repeat(20))
    );
}

#[test]
fn canonical_unicode_control_and_wrapper_cases() {
    for phrase in [false, true] {
        for text in [
            "I'm so happy! \u{7} I love MIME so much \u{7}\u{7}! Isn't it great?",
            "I'm so happy! \u{1f600} I love MIME so much \u{2764}\u{fe0f}\u{200d}\u{1f525}! Isn't it great?",
            "Hello =?utf-8?b?4oI=?= =?utf-8?b?rA==?= and foo=?bar",
            "Hello \"=?utf-8?b?4oI=?= quoted\" world",
            "Hello \"unterminated =?utf-8?b?4oI=?= quote",
            "John \"Jacob Jingle \u{9ede}\u{770b}@\u{540d}\u{304c}\u{30c9}\u{30e1}\u{30a4}\u{30f3} Heimer\" Schmidt",
            "John (\u{9ede}\u{770b} Jacob Jingle Heimer) Schmidt",
            "\r\nBcc: not-an-injected-header\0",
            "\u{feff} \u{10ffff} \u{d7ff} \u{e000}",
            " \t\r\n leading \u{e9} and trailing \n\t ",
        ] {
            round_trip(text, &EncodeOptions::default(), phrase);
        }
    }
}

#[test]
fn every_ascii_character_is_safe_and_preserved() {
    for code in 0..=127u8 {
        let text = format!("before{}after", char::from(code));
        for phrase in [false, true] {
            round_trip(&text, &EncodeOptions::default(), phrase);
        }
    }
    let text: String = (0..=255u8).map(char::from).collect();
    for charset in [Charset::UTF8, Charset::LATIN1] {
        let options = EncodeOptions {
            charset,
            ..Default::default()
        };
        for phrase in [false, true] {
            round_trip(&text.repeat(10), &options, phrase);
        }
    }
}

#[test]
fn word_lengths_and_scalar_boundaries_are_exact() {
    for max_word_length in 20..=75 {
        let options = EncodeOptions {
            max_word_length,
            ..Default::default()
        };
        for phrase in [false, true] {
            for text in [
                "a".repeat(200),
                "\u{1f600}".repeat(100),
                "\u{e9}".repeat(100),
                " ?=_' \u{1}\u{e9}\u{2603}\u{1f680} ".repeat(40),
            ] {
                round_trip(&text, &options, phrase);
            }
        }
    }
    // Q payloads can use all 63 bytes after the UTF-8 wrapper. No extra
    // whitespace may be introduced when this unbroken token is split.
    let output = encode_text(&"a".repeat(126)).unwrap();
    let expected = format!(
        "=?utf-8?q?{}?= =?utf-8?q?{}?=",
        "a".repeat(63),
        "a".repeat(63)
    );
    assert_eq!(output, expected);
    for prefix in 0..160 {
        let text = "?".repeat(prefix) + "\u{21861}\u{1f600}" + &"x".repeat(80);
        round_trip(&text, &EncodeOptions::default(), true);
    }
}

#[test]
fn original_whitespace_survives_word_boundaries() {
    let options = EncodeOptions {
        max_word_length: 25,
        ..Default::default()
    };
    for phrase in [false, true] {
        for text in [
            " \u{e9}  \u{e9}   \u{e9}    ",
            " \t\r\n\u{e9} ",
            " =?utf-8?q?hello?= ",
            "\u{1}                                                              ",
        ] {
            round_trip(text, &options, phrase);
        }
    }
}

#[test]
fn explicit_charsets_and_stateful_words() {
    for (label, text) in [
        ("us-ascii", "literal =?word?= with \u{7}"),
        ("iso-8859-1", "Ren\u{e9}e \u{80}\u{ff}"),
        ("windows-1252", "price \u{20ac} \u{201c}quoted\u{201d}"),
        ("shift_jis", "\u{65e5}\u{672c}\u{8a9e} ABC "),
        ("iso-2022-jp", "\u{65e5}\u{672c}\u{8a9e} ABC "),
        ("utf-16", "\u{65e5} \u{1f600} "),
        ("utf-16be", "\u{65e5} \u{1f600} "),
    ] {
        let options = EncodeOptions {
            charset: Charset::from_label(label).unwrap(),
            max_word_length: 50,
        };
        for phrase in [false, true] {
            let output = round_trip(&text.repeat(30), &options, phrase);
            if label == "iso-2022-jp" {
                assert!(output.split(' ').all(|word| word.contains("?b?")));
            }
        }
    }
}

#[test]
fn unmappable_text_is_an_error_without_replacement_or_fallback() {
    use std::error::Error;

    for charset in [Charset::ASCII, Charset::LATIN1] {
        let options = EncodeOptions {
            charset,
            ..Default::default()
        };
        for text in ["\u{1f600}", "prefix \u{e9} \u{65e5}\u{672c}\u{8a9e}"] {
            for encode in [encode_text_with_options, encode_phrase_with_options] {
                let error = encode(text, &options).unwrap_err();
                assert_eq!(
                    error,
                    EncodeError::Charset(CharsetError::UnmappableText(charset))
                );
                assert!(error.source().is_some());
                assert!(!error.to_string().is_empty());
            }
        }
        let text = "a".repeat(200) + "\u{1f600}";
        assert!(encode_text_with_options(&text, &options).is_err());
    }
}

#[test]
fn option_errors_and_unfittable_characters_are_explicit() {
    for max_word_length in [0, 76, 998, usize::MAX] {
        let options = EncodeOptions {
            max_word_length,
            ..Default::default()
        };
        for text in ["", "plain", "\u{1f600}"] {
            for encode in [encode_text_with_options, encode_phrase_with_options] {
                assert_eq!(
                    encode(text, &options),
                    Err(EncodeError::InvalidWordLength {
                        requested: max_word_length
                    })
                );
            }
        }
    }
    let options = EncodeOptions {
        max_word_length: 13,
        ..Default::default()
    };
    for encode in [encode_text_with_options, encode_phrase_with_options] {
        assert_eq!(
            encode("a\u{1f600}", &options),
            Err(EncodeError::WordTooLong {
                offset: 1,
                required: 20,
                limit: 13
            })
        );
    }
    let options = EncodeOptions {
        max_word_length: 1,
        ..Default::default()
    };
    assert_eq!(
        encode_text_with_options("abc", &options),
        Err(EncodeError::WordTooLong {
            offset: 0,
            required: 13,
            limit: 1
        })
    );
    let options = EncodeOptions {
        max_word_length: 16,
        ..Default::default()
    };
    assert_eq!(
        encode_text_with_options("\u{e9}\u{1f600}", &options),
        Err(EncodeError::WordTooLong {
            offset: 2,
            required: 20,
            limit: 16
        })
    );
}

#[test]
fn syntax_is_unquoted_before_encoded_words_are_decoded() {
    for text in ["a\"b\\c", "a\"b\\c \u{e9}", "literal \"=?utf-8?q?text?=\""] {
        let encoded = encode_phrase(text).unwrap();
        let unquoted = unquote(&encoded, false);
        assert_eq!(decode_text(unquoted.as_bytes()).text, text);
    }
}

#[test]
fn large_inputs_do_not_limit_number_of_words() {
    let text = "text \u{e9}\u{1f600} \t".repeat(5_000);
    let output = round_trip(&text, &EncodeOptions::default(), false);
    assert!(output.split(' ').count() > 1000);
    assert!(!output.ends_with(' '));
    assert!(!output.contains('\n'));
}

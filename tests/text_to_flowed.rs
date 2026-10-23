// Adapted from MimeKit's UnitTests/Text/TextToFlowedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeMessage, TextFormat, TextPart,
    encodings::{EncoderOptionsError, NewLineFormat},
    text::{FlowedToText, TextToFlowed},
};

fn unix() -> TextToFlowed {
    TextToFlowed::with_new_line_format(NewLineFormat::Unix).unwrap()
}

fn unflow(text: &str) -> String {
    let mut decoder = FlowedToText::with_new_line_format(NewLineFormat::Unix).unwrap();
    decoder.delete_space = true;
    decoder.convert(text)
}

#[test]
fn default_and_explicit_newlines_are_platform_independent() {
    let converter = TextToFlowed::new();
    assert_eq!(converter, TextToFlowed::default());
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    assert_eq!(unix().new_line_format(), NewLineFormat::Unix);
    assert_eq!(converter.convert("one\ntwo"), "one\r\ntwo\r\n");
    assert_eq!(unix().convert("one\r\ntwo\rthree\n"), "one\ntwo\nthree\n");
    assert_eq!(
        TextToFlowed::with_new_line_format(NewLineFormat::Mixed),
        Err(EncoderOptionsError::InvalidNewLineFormat(
            NewLineFormat::Mixed
        ))
    );
}

#[test]
fn line_endings_trailing_spaces_and_empty_input_follow_textreader_rules() {
    for (input, expected) in [
        ("", ""),
        ("\r\n", "\n"),
        ("\n\n", "\n\n"),
        ("a", "a\n"),
        ("a\r\n", "a\n"),
        ("a\rb\r\nc\n", "a\nb\nc\n"),
        ("a  \r\nb ", "a\nb\n"),
        ("   ", "\n"),
        ("a\t \nb\u{a0} ", "a\t\nb\u{a0}\n"),
        ("\u{feff}a\0b", "\u{feff}a\0b\n"),
        ("-- \nSignature\n", "--\nSignature\n"),
    ] {
        assert_eq!(unix().convert(input), expected, "{input:?}");
    }
}

#[test]
fn stuffing_and_quote_normalization_match_source_behavior() {
    for (input, expected) in [
        ("From Russia with love.", " From Russia with love.\n"),
        ("from someone", "from someone\n"),
        ("From\tsomeone", "From\tsomeone\n"),
        (" leading", "  leading\n"),
        ("\tleading", "\tleading\n"),
        ("> quote", "> quote\n"),
        (">quote", "> quote\n"),
        ("> > >> quote", ">>>> quote\n"),
        (">  >literal", ">  >literal\n"),
        (">", "> \n"),
        ("> > ", ">> \n"),
        (" >literal", "  >literal\n"),
    ] {
        assert_eq!(unix().convert(input), expected, "{input:?}");
    }
}

#[test]
fn soft_broken_lines_are_stuffed_without_changing_quote_depth() {
    for suffix in [">this is not a quote", "From someone with love"] {
        let text = format!("{} {suffix}\n", "a".repeat(75));
        let expected = format!("{}  \n {suffix}\n", "a".repeat(75));
        let flowed = unix().convert(&text);
        assert_eq!(flowed, expected);
        assert_eq!(unflow(&flowed), text);
    }
    let text = format!("{}  extra spaces\n", "a".repeat(76));
    let expected = format!("{}  \n  extra spaces\n", "a".repeat(76));
    assert_eq!(unix().convert(&text), expected);
    assert_eq!(unflow(&expected), text);
}

#[test]
fn words_wrap_at_the_soft_target_without_losing_original_spaces() {
    let text = format!("{}word\n", "word ".repeat(39));
    let expected = format!(
        "{} \n{} \n{}word\n",
        "word ".repeat(15),
        "word ".repeat(15),
        "word ".repeat(9)
    );
    let flowed = unix().convert(&text);
    assert_eq!(flowed, expected);
    assert!(flowed.lines().all(|line| line.len() <= 78));
    assert_eq!(unflow(&flowed), text);

    let text = format!("{}  end\n", "a".repeat(76));
    let flowed = unix().convert(&text);
    assert!(flowed.lines().all(|line| line.len() <= 78));
    assert_eq!(unflow(&flowed), text);
}

#[test]
fn quoted_lines_include_prefix_and_soft_break_in_the_soft_target() {
    for depth in 1..=6 {
        let prefix = format!("{} ", ">".repeat(depth));
        let text = format!("{prefix}{}word\n", "word ".repeat(39));
        let words_per_line = (78 - prefix.len() - 1) / 5;
        let expected = format!(
            "{prefix}{} \n{prefix}{} \n{prefix}{}word\n",
            "word ".repeat(words_per_line),
            "word ".repeat(words_per_line),
            "word ".repeat(39 - words_per_line * 2)
        );
        let flowed = unix().convert(&text);
        assert_eq!(flowed, expected, "{depth}");
        assert!(flowed.lines().all(|line| line.len() <= 78));
        assert_eq!(unflow(&flowed), text);
    }
}

#[test]
fn long_words_exceed_soft_target_but_split_at_hard_limit() {
    let text = format!("Hello {} world\n", "x".repeat(100));
    let expected = format!("Hello {}  \nworld\n", "x".repeat(100));
    assert_eq!(unix().convert(&text), expected);
    assert_eq!(unflow(&expected), text);

    let text = format!("{}\n", "x".repeat(2000));
    let expected = format!(
        "{} \n{} \n{}\n",
        "x".repeat(997),
        "x".repeat(997),
        "x".repeat(6)
    );
    assert_eq!(unix().convert(&text), expected);
    assert_eq!(unflow(&expected), text);

    for length in [77, 78, 79, 996, 997, 998] {
        let text = format!("{}\n", "x".repeat(length));
        assert_eq!(unix().convert(&text), text, "{length}");
    }
    for length in [999, 1000] {
        let text = format!("{}\n", "x".repeat(length));
        let expected = format!("{} \n{}\n", "x".repeat(997), "x".repeat(length - 997));
        assert_eq!(unix().convert(&text), expected);
        assert_eq!(unflow(&expected), text);
    }
}

#[test]
fn long_word_that_fits_on_its_own_line_is_not_split() {
    let text = format!("Hello {} world\n", "x".repeat(991));
    let flowed = unix().convert(&text);
    assert_eq!(flowed, format!("Hello  \n{}  \nworld\n", "x".repeat(991)));
    assert_eq!(unflow(&flowed), text);
    assert!(flowed.lines().all(|line| line.len() <= 998));
}

#[test]
fn deep_quote_prefixes_expand_limits_without_losing_text() {
    for depth in [70, 75, 76, 77, 90, 996, 997, 2000] {
        let prefix = format!("{} ", ">".repeat(depth));
        let mut text = prefix.clone();
        for i in 0..300 {
            text.push_str("word ");
            text.push_str(&"x".repeat(i % 10));
            text.push(' ');
        }
        text.push_str(&"y".repeat(1500));
        text.push('\n');
        let flowed = unix().convert(&text);
        for line in flowed.lines() {
            assert!(line.starts_with(&prefix));
            // Deep quotes may exceed 998 bytes only when the prefix takes at
            // least half that space, just as in MimeKit's regression tests.
            assert!(line.len() <= 998.max(prefix.len() * 2), "{depth}");
        }
        assert_eq!(unflow(&flowed), text, "{depth}");
    }
}

#[test]
fn deep_quote_prefixes_do_not_amplify_output_per_word() {
    for depth in [2, 38, 39, 77, 200, 900, 5000] {
        let text = format!("{}{}\n", ">".repeat(depth), " a".repeat(5000));
        let prefix = format!("{} ", ">".repeat(depth));
        let flowed = unix().convert(&text);
        assert!(flowed.len() <= text.len() * 3, "{depth}");
        for line in flowed.lines() {
            assert!(line.starts_with(&prefix));
            assert!(line.len() <= 78.max(prefix.len() * 2), "{depth}");
        }
        assert_eq!(unflow(&flowed), text, "{depth}");
    }
}

#[test]
fn utf8_words_use_byte_limits_and_never_split_a_character() {
    for word in ["\u{e9}", "\u{65e5}", "\u{1f408}", "e\u{301}"] {
        for count in [20, 25, 26, 77, 249, 250, 332, 333, 499, 500, 2000] {
            let text = format!("{}\n", word.repeat(count));
            let flowed = unix().convert(&text);
            assert!(flowed.lines().all(|line| line.len() <= 998));
            assert_eq!(unflow(&flowed), text);
        }
    }
    let text = format!("{}\n", "\u{1f408}".repeat(250));
    assert_eq!(
        unix().convert(&text),
        format!("{} \n\u{1f408}\n", "\u{1f408}".repeat(249))
    );
    // UTF-16 would count fewer characters; UTF-8 still counts all four bytes.
    let text = format!("{} end\n", "\u{1f408}".repeat(19));
    assert_eq!(
        unix().convert(&text),
        format!("{}  \nend\n", "\u{1f408}".repeat(19))
    );
    assert_eq!(unflow(&unix().convert(&text)), text);
}

#[test]
fn varied_spacing_and_unicode_round_trip_after_soft_wrapping() {
    for depth in [0, 1, 38, 400, 1000] {
        let prefix = if depth == 0 {
            String::new()
        } else {
            format!("{} ", ">".repeat(depth))
        };
        for word in [
            "word",
            ">literal",
            "From someone",
            "\u{1f408}\u{65e5}",
            "\tword",
        ] {
            for spaces in [1, 2, 75, 78, 998, 1100] {
                let text = format!(
                    "{prefix}start {}end\n",
                    format!("{word}{}", " ".repeat(spaces)).repeat(8)
                );
                let flowed = unix().convert(&text);
                assert!(
                    flowed
                        .lines()
                        .all(|line| line.len() <= 998.max(prefix.len() * 2))
                );
                assert_eq!(
                    unflow(&flowed),
                    text,
                    "depth={depth} spaces={spaces} word={word:?}"
                );
            }
        }
    }
}

#[test]
fn repeated_calls_and_a_large_unbroken_word_preserve_progress() {
    let converter = unix();
    let text = format!("{}\n", "x".repeat(1_000_000));
    let flowed = converter.convert(&text);
    assert!(flowed.lines().all(|line| line.len() <= 998));
    assert_eq!(unflow(&flowed), text);
    assert_eq!(converter.convert("next"), "next\n");
    assert_eq!(converter.convert(""), "");
}

#[test]
fn flowed_output_composes_with_text_parts_and_message_body_decoding() {
    let plain = format!("From someone\r\n> {}end\r\n", "words ".repeat(40));
    let flowed = TextToFlowed::new().convert(&plain);
    let mut part = TextPart::new("plain", &flowed).unwrap();
    let mut content_type = part.part().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("format", "flowed")
        .unwrap();
    content_type.parameters_mut().set("delsp", "yes").unwrap();
    part.part_mut().set_content_type(&content_type).unwrap();
    let mut message = MimeMessage::with_body(part.into());
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap(),
        Some(plain.clone())
    );
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let mut loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(loaded.text_body(TextFormat::Flowed).unwrap(), Some(plain));
}

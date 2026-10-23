// Adapted from MimeKit's Text/PlainTextPreviewer.cs and its unit tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeMessage, TextFormat, TextPart,
    text::{PlainTextPreviewer, TextToFlowed},
};

fn preview(limit: usize, text: &str) -> String {
    PlainTextPreviewer::with_maximum_preview_length(limit)
        .unwrap()
        .preview(text)
}

#[test]
fn defaults_and_length_bounds() {
    let previewer = PlainTextPreviewer::new();
    assert_eq!(previewer, PlainTextPreviewer::default());
    assert_eq!(previewer.maximum_preview_length(), 230);
    for limit in [1, 230, 1024] {
        let previewer = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap();
        assert_eq!(previewer.maximum_preview_length(), limit);
    }
    for limit in [0, 1025, usize::MAX] {
        let error = PlainTextPreviewer::with_maximum_preview_length(limit).unwrap_err();
        assert_eq!(error.requested(), limit);
        assert_eq!(
            error.to_string(),
            format!("preview length must be between 1 and 1024 characters, got {limit}")
        );
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn empty_and_whitespace_only_input() {
    for limit in [1, 2, 230, 1024] {
        for text in [
            "",
            " ",
            "\t\r\n\u{b}\u{c}",
            "\u{a0}\u{200b}\u{200c}\u{200d}",
        ] {
            assert_eq!(preview(limit, text), "");
        }
    }
}

#[test]
fn collapses_ascii_whitespace_without_a_final_newline() {
    assert_eq!(
        preview(230, "\t First \r\n\r\nsecond \t third\rfourth\n "),
        "First second third fourth"
    );
    assert_eq!(preview(230, "word"), "word");
    assert_eq!(preview(230, "word\r\n"), "word");
}

#[test]
fn recognizes_unicode_whitespace_and_source_zero_width_characters() {
    let whitespace = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ];
    for character in whitespace {
        let text = format!("{character}a{character}{character}b{character}");
        assert_eq!(preview(230, &text), "a b", "{character:?}");
    }
    for character in [
        '\0', '\u{1c}', '\u{180e}', '\u{200e}', '\u{2060}', '\u{feff}',
    ] {
        let text = format!("{character}a{character}");
        assert_eq!(preview(230, &text), text, "{character:?}");
    }
}

#[test]
fn truncation_matches_the_source_string_overload() {
    for (limit, input, expected) in [
        (1, "a", "a"),
        (1, "ab", "\u{2026}"),
        (1, "a ", "\u{2026}"),
        (1, "  a", "a"),
        (2, "abc", "a\u{2026}"),
        (2, "a b", "\u{2026}"),
        (3, "abc", "abc"),
        (3, "abcd", "ab\u{2026}"),
        (3, "abc ", "ab\u{2026}"),
        (3, "ab ", "ab"),
        (3, "ab  ", "a\u{2026}"),
        (3, "ab cd", "a\u{2026}"),
        (3, "a bc", "a \u{2026}"),
        (3, "a  b", "a b"),
        (4, "abc def", "ab\u{2026}"),
    ] {
        assert_eq!(preview(limit, input), expected, "{limit}: {input:?}");
    }
}

#[test]
fn default_and_maximum_limits_include_the_ellipsis() {
    for limit in [230, 1024] {
        let exact = "a".repeat(limit);
        assert_eq!(preview(limit, &exact), exact);
        let expected = format!("{}\u{2026}", "a".repeat(limit - 1));
        assert_eq!(preview(limit, &(exact + "b")), expected);
        assert_eq!(expected.chars().count(), limit);
    }
}

#[test]
fn limits_count_unicode_scalars_not_bytes_or_utf16_units() {
    let text = "\u{1f408}\u{65e5}\u{e9}";
    assert_eq!(preview(3, text), text);
    assert_eq!(preview(2, text), "\u{1f408}\u{2026}");
    assert_eq!(preview(1, text), "\u{2026}");
    assert_eq!(preview(4, "\u{1f408}ab\u{1f408}z"), "\u{1f408}ab\u{2026}");
    assert_eq!(preview(2, "e\u{301}x"), "e\u{2026}");
    assert_eq!(
        preview(230, "\u{1f469}\u{200d}\u{1f4bb}"),
        "\u{1f469} \u{1f4bb}"
    );
}

#[test]
fn plain_text_is_not_interpreted_as_html_or_flowed() {
    assert_eq!(preview(230, "<b>Hello</b> &amp;"), "<b>Hello</b> &amp;");
    assert_eq!(preview(230, "> one \n> two\n-- \n"), "> one > two --");
}

#[test]
fn repeated_calls_are_independent_and_large_input_is_bounded() {
    let previewer = PlainTextPreviewer::with_maximum_preview_length(4).unwrap();
    let text = format!("{}first{}", " ".repeat(100_000), "x".repeat(1_000_000));
    assert_eq!(previewer.preview(&text), "fir\u{2026}");
    assert_eq!(previewer.preview("next"), "next");
    assert_eq!(previewer.preview(""), "");
    assert_eq!(previewer.preview(&" ".repeat(100_000)), "");
    let copy = previewer;
    assert_eq!(copy.preview("other"), "oth\u{2026}");
}

#[test]
fn varied_unicode_and_spacing_always_respect_scalar_limit() {
    let tokens = ["", " ", "\r\n", "a", "\u{1f408}", "\u{301}", "\u{200b}"];
    for limit in 1..=8 {
        for first in tokens {
            for second in tokens {
                for third in tokens {
                    let text = format!("{first}{second}{third}end");
                    let result = preview(limit, &text);
                    assert!(result.chars().count() <= limit, "{limit}: {text:?}");
                    assert!(!result.starts_with(' '));
                    assert!(!result.ends_with(' '));
                    assert!(!result.contains("  "));
                }
            }
        }
    }
}

#[test]
fn previews_compose_with_decoded_flowed_message_bodies() {
    let original = format!("From someone\r\n> {}end\r\n", "words ".repeat(40));
    let flowed = TextToFlowed::new().convert(&original);
    let mut part = TextPart::new("plain", &flowed).unwrap();
    let mut content_type = part.part().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("format", "flowed")
        .unwrap();
    content_type.parameters_mut().set("delsp", "yes").unwrap();
    part.part_mut().set_content_type(&content_type).unwrap();
    let mut message = MimeMessage::with_body(part.into());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let mut loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    let text = loaded.text_body(TextFormat::Plain).unwrap().unwrap();
    assert_eq!(text, original);
    assert_eq!(preview(24, &text), "From someone > words wo\u{2026}");
}

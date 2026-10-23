// Adapted from MimeKit's UnitTests/Text/TextToTextTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::TextToText;

#[test]
fn defaults_preserve_input_verbatim() {
    let converter = TextToText::new();
    assert_eq!(converter, TextToText::default());
    assert_eq!(converter.header, None);
    assert_eq!(converter.footer, None);
    for text in [
        "",
        " ",
        "\t  \r\n\r\n",
        "unterminated",
        "CRLF\r\nLF\nCR\r",
        "> quoted\n From someone\n-- \n",
        "<b>&amp; \"literal\"</b>",
        "\0\u{b}\u{c}\u{7f}\u{85}\u{feff}",
        "caf\u{e9} e\u{301} \u{1f631}\u{200d}\u{1f600}\u{2028}",
    ] {
        assert_eq!(converter.convert(text), text, "{text:?}");
    }
}

#[test]
fn source_header_and_footer_case() {
    let converter = TextToText {
        header: Some("Header".into()),
        footer: Some("Footer".into()),
    };
    assert_eq!(converter.convert(","), "Header,Footer");
}

#[test]
fn source_multiline_case_preserves_each_newline_style() {
    for newline in ["\r\n", "\n", "\r"] {
        let text = [
            "This is some sample text. This is line #1.",
            "This is line #2.",
            "And this is line #3.",
            "",
        ]
        .join(newline);
        assert_eq!(TextToText::new().convert(&text), text);
    }
}

#[test]
fn absent_empty_and_nonempty_decorations_with_empty_body() {
    for header in [None, Some(""), Some("Header")] {
        for footer in [None, Some(""), Some("Footer")] {
            let converter = TextToText {
                header: header.map(str::to_owned),
                footer: footer.map(str::to_owned),
            };
            for body in ["", "body"] {
                assert_eq!(
                    converter.convert(body),
                    format!("{}{body}{}", header.unwrap_or(""), footer.unwrap_or("")),
                );
            }
        }
    }
}

#[test]
fn decoration_boundaries_do_not_add_or_normalize_separators() {
    let converter = TextToText {
        header: Some("Header\r".into()),
        footer: Some("\rFooter".into()),
    };
    assert_eq!(converter.convert("\nbody\n"), "Header\r\nbody\n\rFooter");
    assert_eq!(converter.convert(""), "Header\r\rFooter");
}

#[test]
fn decorations_preserve_unicode_controls_and_markup() {
    let converter = TextToText {
        header: Some("\u{feff}<header>&\0\u{1f631}".into()),
        footer: Some("\u{2028}\u{200b}</footer>\u{7f}".into()),
    };
    assert_eq!(
        converter.convert("e\u{301}"),
        "\u{feff}<header>&\0\u{1f631}e\u{301}\u{2028}\u{200b}</footer>\u{7f}",
    );
}

#[test]
fn conversion_is_repeatable_and_clones_have_independent_decorations() {
    let converter = TextToText {
        header: Some("[".into()),
        footer: Some("]".into()),
    };
    let mut clone = converter.clone();
    for body in ["one", "", "two", "one"] {
        assert_eq!(converter.convert(body), format!("[{body}]"));
        assert_eq!(clone, converter);
    }
    clone.header.as_mut().unwrap().push('>');
    clone.footer = None;
    assert_eq!(clone.convert("body"), "[>body");
    assert_eq!(converter.convert("body"), "[body]");
    clone.header = None;
    assert_eq!(clone.convert("body"), "body");
}

#[test]
fn long_text_and_decorations_are_not_truncated() {
    // Cross the source converter's 4096-character copy-buffer boundary,
    // including multi-byte scalars and mixed newline sequences.
    let header = "h\u{1f631}\r".repeat(4097);
    let body = "\ne\u{301}\0\r\n".repeat(4097);
    let footer = "\u{feff}\n".repeat(4097);
    let expected = format!("{header}{body}{footer}");
    let converter = TextToText {
        header: Some(header),
        footer: Some(footer),
    };
    assert_eq!(converter.convert(&body), expected);
}

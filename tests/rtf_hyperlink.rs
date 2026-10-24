// Adapted from MimeKit's UnitTests/Text/RtfInterpreterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{HtmlAttributeQuote, html_attribute_encode, parse_rtf_hyperlink};

#[test]
fn canonical_field_instructions() {
    let cases = [
        (
            r#"HYPERLINK "http://www.example.com/""#,
            Some("http://www.example.com/"),
        ),
        (
            "  HYPERLINK  http://www.example.com/ ",
            Some("http://www.example.com/"),
        ),
        (r#"HYPERLINK \l "bookmark""#, None),
        (
            r#"HYPERLINK "mailto:user@example.com" \o "tooltip""#,
            Some("mailto:user@example.com"),
        ),
        ("PAGE", None),
        ("HYPERLINK", None),
        (
            r#"HYPERLINK \o "tip" "http://example.com/""#,
            Some("http://example.com/"),
        ),
        (
            r#"HYPERLINK \o tip http://example.com/"#,
            Some("http://example.com/"),
        ),
        (
            r#"HYPERLINK \t "_blank" "https://example.com/""#,
            Some("https://example.com/"),
        ),
        (
            r#"HYPERLINK \n "https://example.com/""#,
            Some("https://example.com/"),
        ),
        (
            r#"HYPERLINK \m "https://example.com/""#,
            Some("https://example.com/"),
        ),
        (r#"HYPERLINK \o "unterminated"#, None),
        (r"HYPERLINK \o", None),
        (
            r#"HYPERLINK "http://example.com/"#,
            Some("http://example.com/"),
        ),
        (
            r#"HYPERLINK " http://example.com/ ""#,
            Some("http://example.com/"),
        ),
        ("HYPERLINK \"java\tscript:alert(1)\"", None),
        ("HYPERLINK \"http://exa\u{1}mple.com/\"", None),
        ("HYPERLINK \"\"", None),
        (r#"HYPERLINK ":foo""#, None),
        (r#"HYPERLINK "example.com""#, None),
        (
            r#"HYPERLINK "FTP://example.com/""#,
            Some("FTP://example.com/"),
        ),
        (r#"HYPERLINK "tel:+15555551234""#, Some("tel:+15555551234")),
        (
            r#"hyperlink "https://example.com/""#,
            Some("https://example.com/"),
        ),
        ("HYPERLINKX", None),
        ("HYPER", None),
        ("", None),
    ];
    for (input, expected) in cases {
        assert_eq!(parse_rtf_hyperlink(input), expected, "{input:?}");
    }
}

#[test]
fn permitted_schemes_preserve_original_spelling() {
    for scheme in ["http", "https", "mailto", "ftp", "tel"] {
        for spelling in [scheme.to_owned(), scheme.to_ascii_uppercase()] {
            let url = format!("{spelling}:target");
            let instruction = format!("hYpErLiNk \"{url}\"");
            assert_eq!(parse_rtf_hyperlink(&instruction), Some(url.as_str()));
        }
    }
}

#[test]
fn disallowed_schemes_and_non_external_targets_are_rejected() {
    for url in [
        "javascript:alert(1)",
        "JAVASCRIPT:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/plain,hello",
        "file:///tmp/document",
        "sftp://example.com/",
        "news:example",
        "sip:user@example.com",
        "httpsx://example.com/",
        "://example.com/",
        "//example.com/",
        "/relative",
        "#bookmark",
        "example.com",
        "http ://example.com/",
        "http\u{200b}://example.com/",
    ] {
        let instruction = format!("HYPERLINK \"{url}\"");
        assert_eq!(parse_rtf_hyperlink(&instruction), None, "{url:?}");
    }
}

#[test]
fn every_c0_and_c1_control_inside_a_quoted_target_is_rejected() {
    for control in (0..=0x9f)
        .filter_map(char::from_u32)
        .filter(|c| c.is_control())
    {
        let instruction = format!("HYPERLINK \"https://example.com/a{control}b\"");
        assert_eq!(parse_rtf_hyperlink(&instruction), None, "{control:?}");
    }
}

#[test]
fn unicode_whitespace_separates_and_trims_arguments() {
    let whitespace = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}',
        '\u{3000}',
    ];
    for w in whitespace {
        let instruction = format!("{w}HYPERLINK{w}\\o{w}\"tip\"{w}\"{w}https://example.com/{w}\"");
        assert_eq!(
            parse_rtf_hyperlink(&instruction),
            Some("https://example.com/"),
            "{w:?}"
        );
        let instruction = format!("HYPERLINK https://example.com/{w}ignored");
        assert_eq!(
            parse_rtf_hyperlink(&instruction),
            Some("https://example.com/"),
            "{w:?}"
        );
    }
}

#[test]
fn non_whitespace_controls_are_not_trimmed_from_targets() {
    for c in ['\0', '\u{1}', '\u{1f}', '\u{7f}', '\u{9f}'] {
        for url in [
            format!("{c}https://example.com/"),
            format!("https://example.com/{c}"),
        ] {
            assert_eq!(parse_rtf_hyperlink(&format!("HYPERLINK \"{url}\"")), None);
        }
    }
}

#[test]
fn switches_with_arguments_consume_exactly_one_argument() {
    for switches in [
        r#"\o "a tooltip" \t "_blank" \m \n"#,
        r"\t frame \o tooltip",
        r#"\o "" \t """#,
        r"\unknown \m \n",
    ] {
        let instruction = format!("HYPERLINK {switches} https://example.com/");
        assert_eq!(
            parse_rtf_hyperlink(&instruction),
            Some("https://example.com/")
        );
    }
    for instruction in [
        r"HYPERLINK \o https://example.com/",
        r"HYPERLINK \t https://example.com/",
        r#"HYPERLINK \t "https://example.com/""#,
        r#"HYPERLINK \o "unterminated https://example.com/"#,
        r"HYPERLINK \",
    ] {
        assert_eq!(parse_rtf_hyperlink(instruction), None, "{instruction}");
    }
}

#[test]
fn switches_are_case_sensitive_and_match_the_first_letter() {
    for switches in [r"\L", r"\O", r"\T", r"\optional tip", r"\target frame"] {
        let instruction = format!("HYPERLINK {switches} https://example.com/");
        assert_eq!(
            parse_rtf_hyperlink(&instruction),
            Some("https://example.com/")
        );
    }
    for switches in [r"\l", r"\local", r#"\o "tip" \l"#] {
        let instruction = format!("HYPERLINK {switches} https://example.com/");
        assert_eq!(parse_rtf_hyperlink(&instruction), None);
    }
    // Uppercase \O is not an argument-taking switch; the tooltip is the target.
    assert_eq!(
        parse_rtf_hyperlink(r#"HYPERLINK \O "tip" https://example.com/"#),
        None
    );
}

#[test]
fn first_target_ends_parsing_even_when_switches_follow() {
    for suffix in [
        r"\l bookmark",
        r"\o",
        "\"unterminated",
        "javascript:bad",
        "\0",
    ] {
        let instruction = format!("HYPERLINK \"https://example.com/\" {suffix}");
        assert_eq!(
            parse_rtf_hyperlink(&instruction),
            Some("https://example.com/")
        );
    }
    assert_eq!(
        parse_rtf_hyperlink(r#"HYPERLINK "file:///bad" "https://example.com/""#),
        None,
    );
}

#[test]
fn keyword_prefix_does_not_require_a_separator() {
    for instruction in [
        r#"HYPERLINK"https://example.com/""#,
        "HYPERLINKhttps://example.com/",
        r"HYPERLINK\n https://example.com/",
    ] {
        assert_eq!(
            parse_rtf_hyperlink(instruction),
            Some("https://example.com/")
        );
    }
    assert_eq!(parse_rtf_hyperlink("HYPERLINKX https://example.com/"), None);
}

#[test]
fn quotes_terminate_at_the_first_quote_without_backslash_unescaping() {
    assert_eq!(
        parse_rtf_hyperlink(r#"HYPERLINK "https://example.com/a\"b""#),
        Some("https://example.com/a\\"),
    );
    assert_eq!(
        parse_rtf_hyperlink(r#"HYPERLINK "https://example.com/"ignored"#),
        Some("https://example.com/"),
    );
    assert_eq!(
        parse_rtf_hyperlink("HYPERLINK 'https://example.com/'"),
        None
    );
}

#[test]
fn result_borrows_unicode_target_from_the_original_instruction() {
    let instruction = String::from(
        " \u{3000}HYPERLINK \\o \"\u{8aac}\u{660e}\" \" https://\u{4f8b}.example/\u{1f3a8}?a=1&b=2 \"",
    );
    let expected = "https://\u{4f8b}.example/\u{1f3a8}?a=1&b=2";
    let url = parse_rtf_hyperlink(&instruction).unwrap();
    assert_eq!(url, expected);
    let offset = instruction.find(expected).unwrap();
    assert_eq!(url.as_ptr(), instruction[offset..].as_ptr());
}

#[test]
fn malformed_unicode_keyword_prefixes_do_not_split_utf8() {
    for input in [
        "HYPERL\u{1f4a1}",
        "HYPERLIN\u{e9}",
        "\u{1f4a1}HYPERLINK",
        "HYPERL\u{130}NK",
        "HYPERLINK\u{1f4a1}",
    ] {
        for end in input.char_indices().map(|(i, _)| i).chain([input.len()]) {
            assert_eq!(parse_rtf_hyperlink(&input[..end]), None);
        }
    }
}

#[test]
fn parsing_does_not_validate_normalize_or_encode_urls() {
    for url in [
        "https:",
        "HTTP:relative",
        "https://",
        "https://example.com/a b",
        "https://example.com/a\u{a0}b",
        "https://example.com/a\u{200b}b",
        "https://example.com/%0A",
    ] {
        assert_eq!(
            parse_rtf_hyperlink(&format!("HYPERLINK \"{url}\"")),
            Some(url)
        );
    }
    let url = parse_rtf_hyperlink(r#"HYPERLINK "https://example.com/?a=1&b=<value>""#).unwrap();
    assert_eq!(
        html_attribute_encode(url, HtmlAttributeQuote::Double),
        "\"https://example.com/?a=1&amp;b=&lt;value&gt;\"",
    );
}

#[test]
fn input_is_a_decoded_instruction_not_an_rtf_document() {
    assert_eq!(
        parse_rtf_hyperlink(
            r#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/"}{\fldrslt link}}}"#
        ),
        None,
    );
}

#[test]
fn long_targets_and_many_switches_are_not_truncated() {
    let url = format!("https://example.com/{}", "x".repeat(100_000));
    let instruction = format!("HYPERLINK {}\"{url}\"", "\\n ".repeat(100_000));
    let parsed = parse_rtf_hyperlink(&instruction).unwrap();
    assert_eq!(parsed, url);
    let offset = instruction.find("https:").unwrap();
    assert_eq!(parsed.as_ptr(), instruction[offset..].as_ptr());
}

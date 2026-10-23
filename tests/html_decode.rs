// Adapted from MimeKit's UnitTests/Text/HtmlEntityDecoderTests.cs and HtmlUtilsTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{html_decode, html_encode};

#[test]
fn literal_text_empty_input_and_whitespace_are_unchanged() {
    for text in [
        "",
        "plain text",
        "\0\t\r\n\u{c}",
        "\u{65e5}\u{1f408}",
        "<b>text</b>",
    ] {
        assert_eq!(html_decode(text), text);
    }
}

#[test]
fn canonical_named_and_numeric_vectors() {
    assert_eq!(
        html_decode("&lt;&pound;&euro;&cent;&yen;&nbsp;&copy;&reg;&gt;"),
        "<\u{a3}\u{20ac}\u{a2}\u{a5}\u{a0}\u{a9}\u{ae}>"
    );
    assert_eq!(
        html_decode("&#65 &#x42;&#x110000;&#xD800;&#x96;&#1;&#;"),
        "A B\u{fffd}\u{fffd}\u{2013}\u{1}&#;"
    );
}

#[test]
fn longest_named_matches_and_legacy_prefixes() {
    for (text, expected) in [
        ("&notin;", "\u{2209}"),
        ("&notit;", "\u{ac}it;"),
        ("&notin", "\u{ac}in"),
        ("&not", "\u{ac}"),
        ("&ampere", "&ere"),
        ("&amp=", "&="),
        ("&copycat", "\u{a9}cat"),
        ("&CounterClockwiseContourIntegral;", "\u{2233}"),
        (
            "&CounterClockwiseContourIntegral",
            "&CounterClockwiseContourIntegral",
        ),
        ("&NotEqualTilde;", "\u{2242}\u{338}"),
        ("&fjlig;", "fj"),
        ("&Afr;", "\u{1d504}"),
        ("&apos;", "'"),
        ("&apos", "&apos"),
        ("&AMP;&amp;&Amp;", "&&&Amp;"),
    ] {
        assert_eq!(html_decode(text), expected, "{text}");
    }
}

#[test]
fn unknown_and_incomplete_references_remain_literal() {
    for text in [
        "&",
        "&&",
        "&unknown;",
        "&NotAnEntity;",
        "&;",
        "&#",
        "&#x",
        "&#X",
        "&#;",
        "&#x;",
        "&#a",
        "&#/",
        "&#x@",
        "&#xG",
        "&#xg",
        "&#-1;",
        "&#+1;",
        "&# 65;",
        "&\u{1f408};",
    ] {
        assert_eq!(html_decode(text), text, "{text}");
    }
    assert_eq!(html_decode("&&amp;&#x;&lt;"), "&&&#x;<");
    assert_eq!(html_decode("&unknown;&amp;"), "&unknown;&");
}

#[test]
fn numeric_radices_terminators_and_optional_semicolons() {
    for (text, expected) in [
        ("&#65", "A"),
        ("&#x41", "A"),
        ("&#X41;", "A"),
        ("&#x7a;", "z"),
        ("&#x7A;", "z"),
        ("&#32;", " "),
        ("&#65rest", "Arest"),
        ("&#65=rest", "A=rest"),
        ("&#x41G", "AG"),
        ("&#x41g;", "Ag;"),
        ("&#65;;", "A;"),
        ("&#0x41;", "\u{fffd}x41;"),
        ("&#65\u{65e5}", "A\u{65e5}"),
        ("&#x41&#66;", "AB"),
    ] {
        assert_eq!(html_decode(text), expected, "{text}");
    }
}

#[test]
fn invalid_unicode_values_are_replaced_but_noncharacters_are_preserved() {
    for value in [0, 0xd800, 0xdbff, 0xdc00, 0xdfff, 0x110000, u32::MAX] {
        assert_eq!(html_decode(&format!("&#{value};")), "\u{fffd}");
        assert_eq!(html_decode(&format!("&#x{value:X};")), "\u{fffd}");
    }
    for value in [
        0xd7ff, 0xe000, 0xfdd0, 0xfdef, 0xfffe, 0xffff, 0x10000, 0x10ffff,
    ] {
        let expected = char::from_u32(value).unwrap().to_string();
        assert_eq!(html_decode(&format!("&#{value};")), expected);
        assert_eq!(html_decode(&format!("&#x{value:X};")), expected);
    }
}

#[test]
fn windows_1252_compatibility_mapping_covers_every_c1_value() {
    let expected = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}',
        '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}',
        '\u{178}',
    ];
    for (offset, character) in expected.into_iter().enumerate() {
        let value = 0x80 + offset;
        assert_eq!(html_decode(&format!("&#{value};")), character.to_string());
        assert_eq!(
            html_decode(&format!("&#x{value:X};")),
            character.to_string()
        );
    }
}

#[test]
fn other_controls_are_emitted_without_normalization() {
    for value in (1..32).chain([127]) {
        let expected = char::from_u32(value).unwrap().to_string();
        assert_eq!(html_decode(&format!("&#{value};")), expected);
    }
    assert_eq!(html_decode("&#13;&#10;"), "\r\n");
}

#[test]
fn arbitrarily_long_numeric_runs_consume_every_digit() {
    let zeros = "0".repeat(100_000);
    let nines = "9".repeat(100_000);
    assert_eq!(html_decode(&format!("&#{zeros}65;end")), "Aend");
    assert_eq!(html_decode(&format!("&#x{zeros}41;end")), "Aend");
    assert_eq!(html_decode(&format!("&#{nines};end")), "\u{fffd}end");
    assert_eq!(html_decode(&format!("&#x{nines}end")), "\u{fffd}nd");
    assert_eq!(html_decode("&#2147483648"), "\u{fffd}");
    assert_eq!(html_decode("&#x7FFFFFFF0"), "\u{fffd}");
}

#[test]
fn decoding_is_single_pass_and_does_not_parse_attributes_or_markup() {
    assert_eq!(html_decode("&amp;lt;"), "&lt;");
    assert_eq!(html_decode("&#38;lt;"), "&lt;");
    assert_eq!(html_decode("&#x26;#65;"), "&#65;");
    assert_eq!(
        html_decode("<a title=\"&notit;\">&lt;b&gt;</a>"),
        "<a title=\"\u{ac}it;\"><b></a>"
    );
}

#[test]
fn encoding_round_trips_supported_unicode_scalars() {
    let text = "<'\"&>\t\r\n\u{c} \u{a0}\u{ff}\u{100}\u{2028}\u{1f408}\u{10ffff}";
    assert_eq!(html_decode(&html_encode(text)), text);
    // Encoding deliberately drops these controls; decoding cannot restore them.
    assert_eq!(html_decode(&html_encode("a\0\u{7f}\u{80}b")), "ab");
}

#[test]
fn repeated_unknown_references_and_long_names_preserve_progress() {
    let text = format!("&{};&amp;", "z".repeat(100_000));
    assert_eq!(html_decode(&text), format!("&{};&", "z".repeat(100_000)));
    let text = "&bogus;&notit;&#65;&\u{1f408}".repeat(10_000);
    let expected = "&bogus;\u{ac}it;A&\u{1f408}".repeat(10_000);
    assert_eq!(html_decode(&text), expected);
}

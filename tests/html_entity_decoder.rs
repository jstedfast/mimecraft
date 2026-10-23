// Adapted from MimeKit's UnitTests/Text/HtmlEntityDecoderTests.cs
// and HtmlTokenizerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HtmlEntityDecoder, HtmlEntityDecoderError, html_attribute_decode, html_decode,
};

fn incremental_decode(text: &str, attribute: bool) -> String {
    let mut decoder = HtmlEntityDecoder::new();
    let mut characters = text.chars().peekable();
    let mut output = String::new();
    while let Some(character) = characters.next() {
        if character != '&' {
            output.push(character);
            continue;
        }
        decoder.reset();
        assert!(decoder.push('&').unwrap());
        while let Some(&character) = characters.peek() {
            if !decoder.push(character).unwrap() {
                break;
            }
            characters.next();
        }
        output.push_str(&if attribute {
            decoder.attribute_value(characters.peek().copied())
        } else {
            decoder.value()
        });
    }
    output
}

fn push_all(decoder: &mut HtmlEntityDecoder, text: &str) {
    for character in text.chars() {
        assert!(decoder.push(character).unwrap(), "{text:?}: {character:?}");
    }
}

#[test]
fn defaults_reset_and_invalid_first_character() {
    let mut decoder = HtmlEntityDecoder::new();
    assert_eq!(decoder, HtmlEntityDecoder::default());
    assert_eq!(decoder.value(), "");
    assert_eq!(decoder.attribute_value(None), "");
    for character in ['a', '#', ';', '\0', '\u{1f408}'] {
        let original = decoder.clone();
        assert_eq!(
            decoder.push(character),
            Err(HtmlEntityDecoderError::ExpectedAmpersand)
        );
        assert_eq!(decoder, original);
    }
    for text in ["&", "&noti", "&amp;", "&#", "&#x", "&#65", "&#x80;"] {
        push_all(&mut decoder, text);
        decoder.reset();
        assert_eq!(decoder, HtmlEntityDecoder::new());
        decoder.reset();
        assert_eq!(decoder.value(), "");
    }
    let error: &dyn std::error::Error = &HtmlEntityDecoderError::ExpectedAmpersand;
    assert!(!error.to_string().is_empty());
    assert!(error.source().is_none());
}

#[test]
fn named_longest_matches_retain_pushed_suffixes() {
    for (text, expected) in [
        ("&", "&"),
        ("&n", "&n"),
        ("&not", "\u{ac}"),
        ("&noti", "\u{ac}i"),
        ("&notin", "\u{ac}in"),
        ("&notin;", "\u{2209}"),
        (
            "&CounterClockwiseContourIntegral",
            "&CounterClockwiseContourIntegral",
        ),
        ("&CounterClockwiseContourIntegral;", "\u{2233}"),
        ("&NotEqualTilde;", "\u{2242}\u{338}"),
        ("&fjlig;", "fj"),
        ("&Afr;", "\u{1d504}"),
        ("&AMP;", "&"),
        ("&apos", "&apos"),
        ("&apos;", "'"),
    ] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, text);
        let original = decoder.clone();
        assert_eq!(decoder.value(), expected, "{text}");
        assert_eq!(decoder.value(), expected, "{text}");
        assert_eq!(decoder, original);
    }
}

#[test]
fn refused_characters_are_not_consumed_or_included_in_values() {
    for (prefix, refused, expected) in [
        ("&noti", 't', "\u{ac}i"),
        ("&", '\u{1f408}', "&"),
        ("&", ';', "&"),
        ("&amp;", '&', "&"),
        ("&#65", '=', "A"),
        ("&#x41", 'G', "A"),
        ("&#x41;", ';', "A"),
        ("&AMP", '\0', "&"),
        ("&CounterClockwiseContourIntegral;", 'x', "\u{2233}"),
    ] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, prefix);
        assert!(!decoder.push(refused).unwrap(), "{prefix}, {refused:?}");
        assert_eq!(decoder.value(), expected);
    }
}

#[test]
fn accepted_semicolon_ends_the_reference_until_reset() {
    for text in ["&amp;", "&#65;", "&#x41;"] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, text);
        let original = decoder.clone();
        for character in ['&', 'a', '#', ';', '0', '\u{e9}'] {
            assert!(!decoder.push(character).unwrap());
            assert_eq!(decoder, original);
        }
        decoder.reset();
        push_all(&mut decoder, "&lt;");
        assert_eq!(decoder.value(), "<");
    }
}

#[test]
fn chunk_boundaries_do_not_finalize_or_reset_a_reference() {
    for text in [
        "&amp;",
        "&notin;",
        "&NotEqualTilde;",
        "&CounterClockwiseContourIntegral;",
        "&#x000000000000000000000000000000000000000041;",
        "&#65",
        "&#X10FFFF;",
    ] {
        for split in 0..=text.len() {
            let mut decoder = HtmlEntityDecoder::new();
            push_all(&mut decoder, &text[..split]);
            assert_eq!(decoder.value(), html_decode(&text[..split]));
            push_all(&mut decoder, &text[split..]);
            assert_eq!(decoder.value(), html_decode(text), "{text}, {split}");
            assert_eq!(decoder.attribute_value(None), html_attribute_decode(text));
        }
    }
}

#[test]
fn a_cloned_partial_decoder_can_follow_a_different_suffix() {
    let mut first = HtmlEntityDecoder::new();
    push_all(&mut first, "&not");
    let mut second = first.clone();
    push_all(&mut first, "in;");
    push_all(&mut second, ";");
    assert_eq!(first.value(), "\u{2209}");
    assert_eq!(second.value(), "\u{ac}");
    first.reset();
    assert_eq!(second.value(), "\u{ac}");
}

#[test]
fn source_numeric_vectors_preserve_recovery_rules() {
    for (text, expected) in [
        ("&#x00;", "\u{fffd}"),
        ("&#x80;", "\u{20ac}"),
        ("&#x95", "\u{2022}"),
        ("&#x9F;", "\u{178}"),
        ("&#X10FFFF;", "\u{10ffff}"),
        ("&#xFDD0;", "\u{fdd0}"),
        ("&#xD800;", "\u{fffd}"),
        ("&#1;", "\u{1}"),
        ("&#13;", "\r"),
        ("&#x7F;", "\u{7f}"),
        ("&#x110000;", "\u{fffd}"),
        ("&#xFFFFFFFF;", "\u{fffd}"),
        ("&#x7FFFFFFF0", "\u{fffd}"),
        ("&#2147483648", "\u{fffd}"),
        (
            "&#99999999999999999999999999999999999999999999;",
            "\u{fffd}",
        ),
        ("&#x00000000000000000000000000000000000000000041;", "A"),
        ("&#65", "A"),
        ("&#X41", "A"),
        ("&#32;", " "),
        ("&#x7a;", "z"),
    ] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, text);
        assert_eq!(decoder.value(), expected, "{text}");
        assert_eq!(decoder.attribute_value(Some('=')), expected, "{text}");
        assert_eq!(html_attribute_decode(text), expected, "{text}");
    }
}

#[test]
fn all_byte_sized_numeric_values_match_existing_character_data_recovery() {
    for value in 0..=255 {
        for text in [format!("&#{value};"), format!("&#x{value:X};")] {
            let expected = html_decode(&text);
            assert_eq!(incremental_decode(&text, false), expected);
            assert_eq!(incremental_decode(&text, true), expected);
            assert_eq!(html_attribute_decode(&text), expected);
        }
    }
}

#[test]
fn invalid_numeric_characters_are_refused_and_digitless_prefixes_stay_literal() {
    for (prefix, refused) in [
        ("&#", 'a'),
        ("&#", '/'),
        ("&#x", '@'),
        ("&#x", 'G'),
        ("&#x", 'g'),
        ("&#", ';'),
        ("&#x", ';'),
        ("&#", '-'),
        ("&#", '+'),
        ("&#", '\u{ff11}'),
        ("&#x", '\u{ff21}'),
        ("&#", '\u{661}'),
    ] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, prefix);
        assert!(!decoder.push(refused).unwrap());
        assert_eq!(decoder.value(), prefix);
        assert_eq!(decoder.attribute_value(Some(refused)), prefix);
        decoder.reset();
        push_all(&mut decoder, "&#65;");
        assert_eq!(decoder.value(), "A");
    }
    for text in ["&", "&#", "&#x", "&#X"] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, text);
        assert_eq!(decoder.value(), text);
    }
}

#[test]
fn attribute_lookahead_uses_the_character_after_the_longest_match() {
    for (prefix, next, expected) in [
        ("&not", Some('='), "&not"),
        ("&not", Some('0'), "&not"),
        ("&not", Some('A'), "&not"),
        ("&not", Some('z'), "&not"),
        ("&not", Some('!'), "\u{ac}"),
        ("&not", None, "\u{ac}"),
        ("&not", Some('\u{e9}'), "\u{ac}"),
        ("&not", Some('\0'), "\u{ac}"),
        ("&not;", Some('='), "\u{ac}"),
        ("&not;", Some('i'), "\u{ac}"),
        ("&noti", Some(';'), "&noti"),
        ("&notin", None, "&notin"),
        ("&notin;", Some('a'), "\u{2209}"),
        ("&apos", Some('!'), "&apos"),
    ] {
        let mut decoder = HtmlEntityDecoder::new();
        push_all(&mut decoder, prefix);
        assert_eq!(
            decoder.attribute_value(next),
            expected,
            "{prefix}, {next:?}"
        );
    }
}

#[test]
fn upstream_attribute_value_fixture_preserves_ambiguous_ampersands() {
    // Historical attribute references without ';' are left as-is before '='
    // or an ASCII alphanumeric, unlike ordinary character data.
    let text = "?a=1&not=2&noti;&notin;&not;&not!";
    let expected = "?a=1&not=2&noti;\u{2209}\u{ac}\u{ac}!";
    assert_eq!(html_attribute_decode(text), expected);
    assert_eq!(incremental_decode(text, true), expected);
    assert_eq!(html_decode("&noti;&notin"), "\u{ac}i;\u{ac}in");
    assert_eq!(
        incremental_decode("&noti;&notin", false),
        "\u{ac}i;\u{ac}in"
    );
}

#[test]
fn attribute_decoding_is_context_specific_not_quote_specific() {
    for (text, expected) in [
        ("&notit;", "&notit;"),
        ("&notin", "&notin"),
        ("&ampere", "&ampere"),
        ("&copycat", "&copycat"),
        ("&amp=", "&amp="),
        ("&amp;=", "&="),
        ("&copy!", "\u{a9}!"),
        ("&copy\u{e9}", "\u{a9}\u{e9}"),
        ("&copy\u{ff11}", "\u{a9}\u{ff11}"),
        ("&copy\u{661}", "\u{a9}\u{661}"),
        ("'&copy' \"&copy\"", "'\u{a9}' \"\u{a9}\""),
        ("&#65x &#65= &#x41G", "Ax A= AG"),
        ("&unknown;&amp;", "&unknown;&"),
        ("&AMP;&amp;&Amp;", "&&&Amp;"),
    ] {
        assert_eq!(html_attribute_decode(text), expected, "{text}");
        assert_eq!(incremental_decode(text, true), expected, "{text}");
    }
}

#[test]
fn literal_text_and_single_pass_decoding_are_unchanged() {
    for text in [
        "",
        "plain",
        "\0\t\r\n\u{c}",
        "<b>x</b>",
        "&unknown;",
        "&\u{1f408};",
    ] {
        assert_eq!(html_attribute_decode(text), text);
        assert_eq!(incremental_decode(text, true), text);
        assert_eq!(incremental_decode(text, false), text);
    }
    for (text, expected) in [
        ("&amp;lt;", "&lt;"),
        ("&#38;lt;", "&lt;"),
        ("&#x26;#65;", "&#65;"),
        ("&&amp;&#x;&lt;", "&&&#x;<"),
    ] {
        assert_eq!(html_attribute_decode(text), expected);
        assert_eq!(incremental_decode(text, true), expected);
        assert_eq!(incremental_decode(text, false), expected);
    }
}

#[test]
fn long_unknown_names_and_numeric_runs_preserve_progress() {
    let zeros = "0".repeat(100_000);
    let nines = "9".repeat(100_000);
    for (text, expected) in [
        (format!("&#{zeros}65;end"), "Aend".to_owned()),
        (format!("&#x{zeros}41;end"), "Aend".to_owned()),
        (format!("&#{nines};end"), "\u{fffd}end".to_owned()),
        (format!("&#x{nines}end"), "\u{fffd}nd".to_owned()),
        (
            format!("&{};&amp;", "z".repeat(100_000)),
            format!("&{};&", "z".repeat(100_000)),
        ),
    ] {
        assert_eq!(html_attribute_decode(&text), expected);
        assert_eq!(incremental_decode(&text, false), expected);
        assert_eq!(incremental_decode(&text, true), expected);
    }
    let text = "&bogus;&notit;&#65;&\u{1f408}".repeat(10_000);
    let expected = "&bogus;&notit;A&\u{1f408}".repeat(10_000);
    assert_eq!(html_attribute_decode(&text), expected);
    assert_eq!(incremental_decode(&text, true), expected);
}

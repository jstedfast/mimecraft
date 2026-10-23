// Adapted from MimeKit's UnitTests/Text/HtmlUtilsTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{HtmlAttributeQuote, html_attribute_encode, html_encode};

#[test]
fn empty_input_and_default_quoting() {
    assert_eq!(HtmlAttributeQuote::default(), HtmlAttributeQuote::Double);
    assert_eq!(html_encode(""), "");
    assert_eq!(
        html_attribute_encode("", HtmlAttributeQuote::Double),
        "\"\""
    );
    assert_eq!(html_attribute_encode("", HtmlAttributeQuote::Single), "''");
}

#[test]
fn text_escapes_both_quotes_and_html_delimiters() {
    assert_eq!(html_encode("'\"&<>"), "&#39;&quot;&amp;&lt;&gt;");
    assert_eq!(
        html_encode("plain ASCII 123 /:=!?"),
        "plain ASCII 123 /:=!?"
    );
}

#[test]
fn attributes_escape_only_the_selected_quote() {
    assert_eq!(
        html_attribute_encode("'\"&<>", HtmlAttributeQuote::Double),
        "\"'&quot;&amp;&lt;&gt;\""
    );
    assert_eq!(
        html_attribute_encode("'\"&<>", HtmlAttributeQuote::Single),
        "'&#39;\"&amp;&lt;&gt;'"
    );
    // Exercise the opposite quote both before and after the first escape.
    assert_eq!(
        html_attribute_encode("'before & after'", HtmlAttributeQuote::Double),
        "\"'before &amp; after'\""
    );
    assert_eq!(
        html_attribute_encode("\"before & after\"", HtmlAttributeQuote::Single),
        "'\"before &amp; after\"'"
    );
}

#[test]
fn canonical_japanese_text_and_attribute_vector() {
    let text = concat!(
        "if (showJapaneseText && x > 0 && x <= 1)\ttext = '",
        "\u{72c2}\u{3063}\u{305f}\u{3053}\u{306e}\u{4e16}\u{3067}",
        "\u{72c2}\u{3046}\u{306a}\u{3089}\u{6c17}\u{306f}\u{78ba}",
        "\u{304b}\u{3060}\u{3002}';"
    );
    let references = concat!(
        "&#29378;&#12387;&#12383;&#12371;&#12398;&#19990;&#12391;",
        "&#29378;&#12358;&#12394;&#12425;&#27671;&#12399;&#30906;",
        "&#12363;&#12384;&#12290;"
    );
    let prefix = "if (showJapaneseText &amp;&amp; x &gt; 0 &amp;&amp; x &lt;= 1)\ttext = ";
    assert_eq!(
        html_encode(text),
        format!("{prefix}&#39;{references}&#39;;")
    );
    assert_eq!(
        html_attribute_encode(text, HtmlAttributeQuote::Double),
        format!("\"{prefix}'{references}';\"")
    );
}

#[test]
fn canonical_numeric_reference_boundaries() {
    let text = "\u{a0}\u{ff}\u{100}\u{ffff}\u{10000}\u{10ffff}";
    let expected = "&#160;&#255;&#256;&#65535;&#65536;&#1114111;";
    assert_eq!(html_encode(text), expected);
    for (quote, delimiter) in [
        (HtmlAttributeQuote::Single, '\''),
        (HtmlAttributeQuote::Double, '"'),
    ] {
        assert_eq!(
            html_attribute_encode(text, quote),
            format!("{delimiter}{expected}{delimiter}")
        );
    }
}

#[test]
fn supplementary_characters_are_encoded_as_single_references() {
    assert_eq!(
        html_encode("This emoji (\u{1f631}) contains a surrogate pair."),
        "This emoji (&#128561;) contains a surrogate pair."
    );
    let text = "\u{1f469}\u{200d}\u{1f4bb}e\u{301}\u{fffd}";
    assert_eq!(
        html_encode(text),
        "&#128105;&#8205;&#128187;e&#769;&#65533;"
    );
}

#[test]
fn numeric_references_cross_each_decimal_digit_boundary() {
    for value in [
        160, 999, 1000, 9999, 10000, 0xd7ff, 0xe000, 99999, 100000, 999999, 1000000, 0x10ffff,
    ] {
        let character = char::from_u32(value).unwrap();
        let text = format!("before{character}after");
        let expected = format!("before&#{value};after");
        assert_eq!(html_encode(&text), expected);
        assert_eq!(
            html_attribute_encode(&text, HtmlAttributeQuote::Double),
            format!("\"{expected}\"")
        );
    }
}

#[test]
fn every_c0_and_c1_control_follows_source_rules() {
    for value in (0..=31).chain(127..=159) {
        let character = char::from_u32(value).unwrap();
        let retained = matches!(character, '\t' | '\r' | '\n' | '\u{c}');
        let expected = if retained {
            format!("before{character}after")
        } else {
            "beforeafter".to_owned()
        };
        let text = format!("before{character}after");
        assert_eq!(html_encode(&text), expected, "{value}");
        for (quote, delimiter) in [
            (HtmlAttributeQuote::Single, '\''),
            (HtmlAttributeQuote::Double, '"'),
        ] {
            assert_eq!(
                html_attribute_encode(&text, quote),
                format!("{delimiter}{expected}{delimiter}"),
                "{value}"
            );
        }
    }
    assert_eq!(
        html_encode("This contains some embedded control sequences (\u{19}\u{80}\u{9f})"),
        "This contains some embedded control sequences ()"
    );
    assert_eq!(html_encode("\0\u{7f}\u{9f}"), "");
}

#[test]
fn permitted_whitespace_is_not_normalized_or_converted_to_markup() {
    let text = " \t\r\n\n\r\u{c}  ";
    assert_eq!(html_encode(text), text);
    assert_eq!(
        html_attribute_encode(text, HtmlAttributeQuote::Double),
        format!("\"{text}\"")
    );
    assert_eq!(html_encode("\u{2028}\u{2029}"), "&#8232;&#8233;");
}

#[test]
fn entities_and_markup_are_data_not_parsed_html() {
    assert_eq!(
        html_encode("<b>&amp; &#160; &#xA0;</b>"),
        "&lt;b&gt;&amp;amp; &amp;#160; &amp;#xA0;&lt;/b&gt;"
    );
    assert_eq!(
        html_attribute_encode("&quot; onclick=\"example()\"", HtmlAttributeQuote::Double),
        "\"&amp;quot; onclick=&quot;example()&quot;\""
    );
    assert_eq!(html_encode(&html_encode("&")), "&amp;amp;");
}

#[test]
fn attribute_encoding_does_not_validate_value_semantics() {
    assert_eq!(
        html_attribute_encode("javascript:example()", HtmlAttributeQuote::Double),
        "\"javascript:example()\""
    );
}

#[test]
fn safe_prefixes_and_repeated_unicode_preserve_all_content() {
    let prefix = "plain text ".repeat(10_000);
    let text = format!("{prefix}\u{10ffff}end");
    assert_eq!(html_encode(&text), format!("{prefix}&#1114111;end"));
    let text = "\u{1f631}".repeat(10_000);
    let expected = "&#128561;".repeat(10_000);
    assert_eq!(html_encode(&text), expected);
    assert_eq!(
        html_attribute_encode(&text, HtmlAttributeQuote::Single),
        format!("'{expected}'")
    );
}

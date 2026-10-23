// Adapted from MimeKit's UnitTests/Text/HtmlWriterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HtmlAttributeQuote, HtmlWriter, HtmlWriterError, HtmlWriterState, html_attribute_encode,
    html_encode,
};

#[test]
fn defaults_and_finish_complete_only_the_pending_tag() {
    let mut html = HtmlWriter::new();
    assert_eq!(html, HtmlWriter::default());
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(HtmlWriterState::default(), HtmlWriterState::Content);
    assert_eq!(html.as_str(), "");
    assert_eq!(html.clone().finish(), "");
    html.write_start_tag("html").unwrap();
    html.write_start_tag("body").unwrap();
    html.write_start_tag("p").unwrap();
    assert_eq!(html.as_str(), "<html><body><p");
    assert_eq!(html.finish(), "<html><body><p>");
}

#[test]
fn source_writer_sequence() {
    let expected = concat!(
        "<html ltr=\"true\"><head/><body>",
        "<p class=\"paragraph\" style=\"font: arial; color: red\" align=\"left\">",
        "special characters in this text should get encoded: &lt;&gt;&#39;&amp;\n",
        "and this is a formatted string with a few args: 1 apple<br/><br/></p>",
        "<p class=\"paragraph\" style=\"font: arial; color: red\" align=\"left\">",
        "special characters should not get encoded: &lt;&gt;</p><p></p>",
        "<p class=\"paragraph\" style=\"font: arial; color: red\" align=\"left\">",
        "special characters in this text should get encoded: &lt;&gt;&#39;&amp;\n<br/><br/></p>",
        "<p class=\"paragraph\" style=\"font: arial; color: red\" align=\"left\">",
        "special characters should not get encoded: &lt;&gt;</p></body></html>"
    );
    let text = "special characters in this text should get encoded: <>'&\n";
    let markup = "special characters should not get encoded: &lt;&gt;";
    let style = "font: arial; color: red";
    let mut html = HtmlWriter::new();

    // Attributes cannot be written until a start tag is pending.
    assert_eq!(
        html.write_attribute("action", "invalid state"),
        Err(HtmlWriterError::AttributeOutsideTag)
    );
    html.write_start_tag("html").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Tag);
    html.write_attribute("ltr", "true").unwrap();
    html.write_empty_element_tag("head").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Tag);
    html.write_start_tag("body").unwrap();
    html.write_start_tag("p").unwrap();

    // A value needs its own preceding attribute name.
    assert_eq!(
        html.write_attribute_value("attrValue"),
        Err(HtmlWriterError::ValueWithoutAttribute)
    );
    html.write_attribute_name("class").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Attribute);
    html.write_attribute_value("paragraph").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Tag);
    html.write_attribute_name("style").unwrap();
    html.write_attribute_value(style).unwrap();
    html.write_attribute("align", "left").unwrap();
    html.write_text(text);
    assert_eq!(html.state(), HtmlWriterState::Content);
    html.write_text(&format!(
        "and this is a formatted string with a few args: {} {}",
        1, "apple"
    ));
    html.write_empty_element_tag("br").unwrap();
    html.write_empty_element_tag("br").unwrap();
    html.write_end_tag("p").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(
        html.write_attribute_name("style"),
        Err(HtmlWriterError::AttributeOutsideTag)
    );

    html.write_start_tag("p").unwrap();
    html.write_attribute("class", "paragraph").unwrap();
    html.write_attribute("style", style).unwrap();
    html.write_attribute("align", "left").unwrap();
    html.write_markup_text(markup);
    html.write_end_tag("p").unwrap();
    html.write_start_tag("p").unwrap();
    html.write_end_tag("p").unwrap();

    html.write_start_tag("p").unwrap();
    html.write_attribute("class", "paragraph").unwrap();
    html.write_attribute("style", style).unwrap();
    html.write_attribute("align", "left").unwrap();
    html.write_text(text);
    html.write_empty_element_tag("br").unwrap();
    html.write_empty_element_tag("br").unwrap();
    html.write_end_tag("p").unwrap();

    html.write_start_tag("p").unwrap();
    html.write_attribute("class", "paragraph").unwrap();
    html.write_attribute("style", style).unwrap();
    html.write_attribute("align", "left").unwrap();
    html.write_markup_text(markup);
    html.write_end_tag("p").unwrap();
    html.write_end_tag("body").unwrap();
    html.write_end_tag("html").unwrap();
    html.flush();
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(html.as_str(), expected);
    assert_eq!(html.finish(), expected);
}

#[test]
fn values_are_complete_and_invalid_states_are_atomic() {
    let mut html = HtmlWriter::new();
    for state in [HtmlWriterState::Content, HtmlWriterState::Tag] {
        if state == HtmlWriterState::Tag {
            html.write_start_tag("input").unwrap();
        }
        let before = html.clone();
        assert_eq!(
            html.write_attribute_value("value"),
            Err(HtmlWriterError::ValueWithoutAttribute)
        );
        assert_eq!(html, before);
    }
    html.write_attribute_name("title").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Attribute);
    html.write_attribute_value("first").unwrap();
    let before = html.clone();
    assert_eq!(
        html.write_attribute_value("second"),
        Err(HtmlWriterError::ValueWithoutAttribute)
    );
    assert_eq!(html, before);
    html.flush();
    let before = html.clone();
    assert_eq!(
        html.write_attribute_name("extra"),
        Err(HtmlWriterError::AttributeOutsideTag)
    );
    assert_eq!(
        html.write_attribute("extra", "value"),
        Err(HtmlWriterError::AttributeOutsideTag)
    );
    assert_eq!(html, before);
    assert_eq!(html.finish(), "<input title=\"first\">");
}

#[test]
fn valueless_empty_duplicate_and_mixed_case_attributes() {
    let mut html = HtmlWriter::new();
    html.write_empty_element_tag("INPUT").unwrap();
    html.write_attribute_name("disabled").unwrap();
    html.write_attribute_name("required").unwrap();
    html.write_attribute("title", "").unwrap();
    html.write_attribute("TITLE", "last").unwrap();
    html.write_attribute_name("checked").unwrap();
    assert_eq!(html.state(), HtmlWriterState::Attribute);
    assert_eq!(
        html.finish(),
        "<INPUT disabled required title=\"\" TITLE=\"last\" checked/>"
    );
}

#[test]
fn all_content_operations_close_pending_tags() {
    type Operation = fn(&mut HtmlWriter);
    let operations: &[(&str, Operation)] = &[
        ("", |html| html.flush()),
        ("", |html| html.write_text("")),
        ("", |html| html.write_markup_text("")),
        ("text", |html| html.write_text("text")),
        ("<b>raw</b>", |html| html.write_markup_text("<b>raw</b>")),
        ("<next>", |html| html.write_start_tag("next").unwrap()),
        ("<next/>", |html| {
            html.write_empty_element_tag("next").unwrap()
        }),
        ("</other>", |html| html.write_end_tag("other").unwrap()),
    ];
    for empty in [false, true] {
        for attribute in [false, true] {
            for &(suffix, operation) in operations {
                let mut html = HtmlWriter::new();
                if empty {
                    html.write_empty_element_tag("tag").unwrap();
                } else {
                    html.write_start_tag("tag").unwrap();
                }
                if attribute {
                    html.write_attribute_name("flag").unwrap();
                }
                operation(&mut html);
                assert_eq!(
                    html.finish(),
                    format!(
                        "<tag{}{}{}",
                        if attribute { " flag" } else { "" },
                        if empty { "/>" } else { ">" },
                        suffix
                    ),
                    "empty={empty}, attribute={attribute}, suffix={suffix:?}"
                );
            }
        }
    }
}

#[test]
fn flush_is_idempotent_and_resets_empty_element_state() {
    let mut html = HtmlWriter::new();
    html.flush();
    html.flush();
    assert_eq!(html.as_str(), "");
    html.write_empty_element_tag("br").unwrap();
    html.flush();
    html.flush();
    html.write_start_tag("br").unwrap();
    html.flush();
    html.write_empty_element_tag("div").unwrap();
    html.write_end_tag("unmatched").unwrap();
    html.write_start_tag("p").unwrap();
    html.write_attribute_name("flag").unwrap();
    html.flush();
    html.flush();
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(html.finish(), "<br/><br><div/></unmatched><p flag>");
}

#[test]
fn invalid_names_do_not_close_or_modify_pending_tags() {
    for empty in [false, true] {
        for attribute in [false, true] {
            let mut html = HtmlWriter::new();
            if empty {
                html.write_empty_element_tag("x").unwrap();
            } else {
                html.write_start_tag("x").unwrap();
            }
            if attribute {
                html.write_attribute_name("flag").unwrap();
            }
            let before = html.clone();
            for name in [
                "", "a b", "a/b", "a>b", "a\0b", "a\tb", "a\rb", "a\nb", "a\u{c}b",
            ] {
                assert_eq!(
                    html.write_start_tag(name),
                    Err(HtmlWriterError::InvalidTagName)
                );
                assert_eq!(
                    html.write_empty_element_tag(name),
                    Err(HtmlWriterError::InvalidTagName)
                );
                assert_eq!(
                    html.write_end_tag(name),
                    Err(HtmlWriterError::InvalidTagName)
                );
                assert_eq!(html, before);
            }
            for name in [
                "", "a b", "=value", "a=b", "a\"b", "a'b", "a<b", "a/b", "a>b", "a\0b",
            ] {
                assert_eq!(
                    html.write_attribute_name(name),
                    Err(HtmlWriterError::InvalidAttributeName)
                );
                assert_eq!(
                    html.write_attribute(name, "value"),
                    Err(HtmlWriterError::InvalidAttributeName)
                );
                assert_eq!(html, before);
            }
            if attribute {
                html.write_attribute_value("kept").unwrap();
            }
            assert_eq!(
                html.finish(),
                format!(
                    "<x{}{}",
                    if attribute { " flag=\"kept\"" } else { "" },
                    if empty { "/>" } else { ">" }
                )
            );
        }
    }
    let mut html = HtmlWriter::new();
    assert_eq!(
        html.write_attribute("", ""),
        Err(HtmlWriterError::InvalidAttributeName)
    );
    assert_eq!(
        html.write_attribute_name("a=b"),
        Err(HtmlWriterError::InvalidAttributeName)
    );
    assert_eq!(html.finish(), "");
}

#[test]
fn leading_equals_cannot_reinterpret_a_valueless_attribute() {
    let mut html = HtmlWriter::new();
    html.write_start_tag("a").unwrap();
    html.write_attribute_name("href").unwrap();
    let before = html.clone();
    // Tokenizer-originated names need a serialization workaround upstream;
    // this checked string API rejects them without changing the prior attribute.
    assert_eq!(
        html.write_attribute_name("=example"),
        Err(HtmlWriterError::InvalidAttributeName)
    );
    assert_eq!(html, before);
    html.write_attribute_value("https://example.com").unwrap();
    assert_eq!(html.finish(), "<a href=\"https://example.com\">");
}

#[test]
fn exact_ascii_name_delimiters_match_source() {
    for byte in 0..=127 {
        let name = format!("x{}y", char::from(byte));
        let mut html = HtmlWriter::new();
        let result = html.write_start_tag(&name);
        if b"\0\t\r\n\x0c />".contains(&byte) {
            assert_eq!(result, Err(HtmlWriterError::InvalidTagName), "{byte}");
        } else {
            assert_eq!(result, Ok(()), "{byte}");
            assert_eq!(html.finish(), format!("<{name}>"));
        }
        let mut html = HtmlWriter::new();
        html.write_start_tag("x").unwrap();
        let result = html.write_attribute(&name, "");
        if b"\0\t\r\n\x0c /=>\"'<".contains(&byte) {
            assert_eq!(result, Err(HtmlWriterError::InvalidAttributeName), "{byte}");
        } else {
            assert_eq!(result, Ok(()), "{byte}");
            assert_eq!(html.finish(), format!("<x {name}=\"\">"));
        }
    }
}

#[test]
fn names_are_not_normalized_or_checked_against_an_element_registry() {
    for name in [
        "Custom-Element",
        "svg:path",
        "data_\u{e9}",
        "\u{4e2d}\u{6587}",
        "x\u{a0}y",
        "1",
    ] {
        let mut html = HtmlWriter::new();
        html.write_start_tag(name).unwrap();
        html.write_attribute(name, "value").unwrap();
        html.write_end_tag(name).unwrap();
        assert_eq!(html.finish(), format!("<{name} {name}=\"value\"></{name}>"));
    }
}

#[test]
fn text_and_attribute_values_use_distinct_escaping() {
    let value = "<>'\"&\r\n\t\u{c}\0\u{b}\u{7f}\u{80}\u{9f}\u{a0}\u{e9}\u{1f631}";
    let mut html = HtmlWriter::new();
    html.write_start_tag("x").unwrap();
    html.write_attribute("data-value", value).unwrap();
    html.write_text(value);
    html.write_end_tag("x").unwrap();
    assert_eq!(
        html.finish(),
        concat!(
            "<x data-value=\"&lt;&gt;'&quot;&amp;\r\n\t\u{c}&#160;&#233;&#128561;\">",
            "&lt;&gt;&#39;&quot;&amp;\r\n\t\u{c}&#160;&#233;&#128561;</x>"
        )
    );
}

#[test]
fn text_does_not_auto_link_or_collapse_whitespace() {
    let mut html = HtmlWriter::new();
    html.write_text("Visit www.example.com &amp;  \t now\r\nnext");
    assert_eq!(
        html.finish(),
        "Visit www.example.com &amp;amp;  \t now\r\nnext"
    );
}

#[test]
fn raw_markup_is_verbatim_and_does_not_change_the_writer_state() {
    let markup = "<b>\0\u{e9}&amp;</b>\r\n<p";
    let mut html = HtmlWriter::new();
    html.write_empty_element_tag("br").unwrap();
    html.write_markup_text(markup);
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(
        html.write_attribute("title", "x"),
        Err(HtmlWriterError::AttributeOutsideTag)
    );
    assert_eq!(html.finish(), format!("<br/>{markup}"));
}

#[test]
fn unicode_slices_and_long_values_do_not_require_character_buffers() {
    let long = format!("{}&\u{1f631}", "\u{e9}".repeat(10_000));
    let surrounded = format!("skip{long}outside");
    let text = &surrounded[4..4 + long.len()];
    let mut html = HtmlWriter::new();
    html.write_start_tag("p").unwrap();
    html.write_attribute_name("title").unwrap();
    html.write_attribute_value(text).unwrap();
    html.write_text(text);
    html.write_end_tag("p").unwrap();
    assert_eq!(
        html.finish(),
        format!(
            "<p title={}>{}</p>",
            html_attribute_encode(text, HtmlAttributeQuote::Double),
            html_encode(text)
        )
    );
}

#[test]
fn cloned_writers_own_independent_pending_state_and_output() {
    let mut html = HtmlWriter::new();
    html.write_empty_element_tag("input").unwrap();
    html.write_attribute_name("title").unwrap();
    let mut clone = html.clone();
    assert_eq!(html, clone);
    clone.write_attribute_value("copy").unwrap();
    html.write_attribute_value("original").unwrap();
    assert_eq!(clone.finish(), "<input title=\"copy\"/>");
    assert_eq!(html.finish(), "<input title=\"original\"/>");
}

#[test]
fn writer_errors_have_distinct_display_messages() {
    let cases = [
        (HtmlWriterError::InvalidTagName, "invalid HTML tag name"),
        (
            HtmlWriterError::InvalidAttributeName,
            "invalid HTML attribute name",
        ),
        (
            HtmlWriterError::AttributeOutsideTag,
            "cannot write an HTML attribute outside a start tag",
        ),
        (
            HtmlWriterError::ValueWithoutAttribute,
            "cannot write an HTML attribute value without a name",
        ),
    ];
    for (error, message) in cases {
        let error: &dyn std::error::Error = &error;
        assert_eq!(error.to_string(), message);
        assert!(error.source().is_none());
    }
}

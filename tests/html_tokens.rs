// Adapted from MimeKit's UnitTests/Text/HtmlTokenTests.cs,
// HtmlTokenizerTests.cs, and HtmlWriterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HtmlAttribute, HtmlAttributeCollection, HtmlAttributeId, HtmlCommentToken, HtmlDataToken,
    HtmlDocTypeError, HtmlDocTypeToken, HtmlTagId, HtmlTagToken, HtmlToken, HtmlTokenKind,
    HtmlWriter, HtmlWriterError, HtmlWriterState,
};

fn render_doctype(doctype: &HtmlDocTypeToken) -> String {
    HtmlToken::DocType(doctype.clone()).to_html()
}

fn named_doctype() -> HtmlDocTypeToken {
    let mut doctype = HtmlDocTypeToken::new();
    doctype.set_name(Some("html".into())).unwrap();
    doctype
}

#[test]
fn token_variants_report_their_kind_and_append_complete_output() {
    let cases = [
        (
            HtmlToken::CData("data".into()),
            HtmlTokenKind::CData,
            "<![CDATA[data]]>",
        ),
        (
            HtmlToken::Comment(HtmlCommentToken::new("comment")),
            HtmlTokenKind::Comment,
            "<!--comment-->",
        ),
        (
            HtmlToken::Data(HtmlDataToken::new("data")),
            HtmlTokenKind::Data,
            "data",
        ),
        (
            HtmlToken::DocType(named_doctype()),
            HtmlTokenKind::DocType,
            "<!DOCTYPE html>",
        ),
        (
            HtmlToken::ScriptData("script".into()),
            HtmlTokenKind::ScriptData,
            "script",
        ),
        (
            HtmlToken::Tag(HtmlTagToken::end("p")),
            HtmlTokenKind::Tag,
            "</p>",
        ),
    ];
    for (token, kind, expected) in cases {
        assert_eq!(token.kind(), kind);
        assert_eq!(token.to_html(), expected);
        let mut output = String::from("prefix:");
        token.write_to(&mut output);
        token.write_to(&mut output);
        assert_eq!(output, format!("prefix:{expected}{expected}"));
        assert_eq!(token.clone(), token);
    }
}

#[test]
fn normal_comments_are_not_escaped_or_normalized() {
    for (comment, expected) in [
        ("", "<!---->"),
        ("a<&\"\u{e9}", "<!--a<&\"\u{e9}-->"),
        ("a--><p>", "<!--a--><p>-->"),
        ("a\r\nb\0", "<!--a\r\nb\0-->"),
    ] {
        let token = HtmlCommentToken::new(comment);
        assert_eq!(token.comment(), comment);
        assert!(!token.is_bogus_comment());
        assert!(!token.is_bang_comment());
        assert_eq!(HtmlToken::Comment(token).to_html(), expected);
    }
}

#[test]
fn bogus_comments_restore_their_opening_delimiter() {
    // The slash is not part of malformed-end-tag comment text. Restore it so
    // "</<o" cannot become "<<o>" and acquire a different tokenization.
    for (comment, expected) in [
        ("", "</>"),
        (" ", "</ >"),
        ("<o", "</<o>"),
        ("?", "<?>"),
        ("?xml version=\"1.0\"?", "<?xml version=\"1.0\"?>"),
        ("\u{fffd}", "</\u{fffd}>"),
    ] {
        let token = HtmlCommentToken::bogus(comment);
        assert_eq!(token.comment(), comment);
        assert!(token.is_bogus_comment());
        assert!(!token.is_bang_comment());
        assert_eq!(HtmlToken::Comment(token).to_html(), expected);
    }
    for (comment, expected) in [
        ("", "<!>"),
        ("DOC", "<!DOC>"),
        ("[CDAT[", "<![CDAT[>"),
        ("?xml?", "<!?xml?>"),
    ] {
        let token = HtmlCommentToken::bang(comment);
        assert!(token.is_bogus_comment());
        assert!(token.is_bang_comment());
        assert_eq!(HtmlToken::Comment(token).to_html(), expected);
    }
}

#[test]
fn character_data_encoding_is_explicit() {
    let text = "a&<>'\"\u{e9}\0";
    let raw = HtmlDataToken::new(text);
    let encoded = HtmlDataToken::from_text(text);
    assert_eq!(raw.data(), text);
    assert_eq!(encoded.data(), text);
    assert_eq!(HtmlToken::Data(raw).to_html(), text);
    assert_eq!(
        HtmlToken::Data(encoded).to_html(),
        "a&amp;&lt;&gt;&#39;&quot;&#233;"
    );
    assert_eq!(
        HtmlToken::Data(HtmlDataToken::new("&amp;")).to_html(),
        "&amp;"
    );
    assert_eq!(
        HtmlToken::Data(HtmlDataToken::from_text("&amp;")).to_html(),
        "&amp;amp;"
    );
}

#[test]
fn data_state_protects_only_a_final_literal_less_than() {
    for (data, expected) in [
        ("", ""),
        ("<", "&lt;"),
        ("<<", "<&lt;"),
        ("a<b", "a<b"),
        ("a<&amp;<", "a<&amp;&lt;"),
        ("\u{1f600}<", "\u{1f600}&lt;"),
        ("&lt;", "&lt;"),
        ("< ", "< "),
        ("\0<", "\0&lt;"),
    ] {
        let token = HtmlDataToken::from_data_state(data);
        assert_eq!(token.data(), data);
        assert_eq!(HtmlToken::Data(token).to_html(), expected);
        assert_eq!(HtmlToken::Data(HtmlDataToken::new(data)).to_html(), data);
    }
}

#[test]
fn dropping_following_markup_cannot_turn_a_protected_less_than_into_a_tag() {
    // Model dropping the "</>" between the first '<' and "c>" in "<</>c>".
    let mut output = String::new();
    HtmlToken::Data(HtmlDataToken::from_data_state("<")).write_to(&mut output);
    HtmlToken::Data(HtmlDataToken::from_data_state("c>")).write_to(&mut output);
    assert_eq!(output, "&lt;c>");
}

#[test]
fn script_and_cdata_contents_remain_raw_including_terminators() {
    for data in [
        "",
        "a < b && c",
        "&amp;",
        "\u{e9}\0\r\n",
        "x<",
        "]]>",
        "</script>",
    ] {
        assert_eq!(HtmlToken::ScriptData(data.into()).to_html(), data);
        assert_eq!(
            HtmlToken::CData(data.into()).to_html(),
            format!("<![CDATA[{data}]]>")
        );
    }
}

#[test]
fn source_image_tag_preserves_attributes_and_explicit_empty_spelling() {
    let attributes = vec![
        HtmlAttribute::new("src", Some("image.png".into())).unwrap(),
        HtmlAttribute::new("alt", Some("[image]".into())).unwrap(),
    ];
    let tag = HtmlTagToken::empty("img", attributes.clone());
    assert_eq!(tag.id(), HtmlTagId::Image);
    assert_eq!(tag.name(), "img");
    assert!(tag.is_empty_element());
    assert!(!tag.is_end_tag());
    assert_eq!(tag.attributes().as_slice(), attributes);
    assert_eq!(
        HtmlToken::Tag(tag).to_html(),
        "<img src=\"image.png\" alt=\"[image]\"/>"
    );

    let tag = HtmlTagToken::start("IMG", HtmlAttributeCollection::new());
    assert_eq!(tag.id(), HtmlTagId::Image);
    assert!(!tag.is_empty_element());
    assert_eq!(HtmlToken::Tag(tag).to_html(), "<IMG>");
    assert_eq!(
        HtmlToken::Tag(HtmlTagToken::empty("div", HtmlAttributeCollection::new())).to_html(),
        "<div/>"
    );
}

#[test]
fn tag_attributes_preserve_duplicates_and_optional_values() {
    let tag = HtmlTagToken::start(
        "Input",
        vec![
            HtmlAttribute::from_id(HtmlAttributeId::Disabled, None).unwrap(),
            HtmlAttribute::new("ID", Some(String::new())).unwrap(),
            HtmlAttribute::new("id", Some("a&\"<\u{e9}'".into())).unwrap(),
            HtmlAttribute::new("data-extra", None).unwrap(),
        ],
    );
    assert_eq!(tag.attributes().len(), 4);
    assert_eq!(
        tag.attributes()
            .get_by_id(HtmlAttributeId::Id)
            .unwrap()
            .name(),
        "ID"
    );
    assert_eq!(
        HtmlToken::Tag(tag).to_html(),
        "<Input disabled ID=\"\" id=\"a&amp;&quot;&lt;&#233;'\" data-extra>"
    );
}

#[test]
fn end_tags_and_unknown_tag_names_are_preserved() {
    for name in ["p", "Custom-Tag", "!comment"] {
        let tag = HtmlTagToken::end(name);
        assert_eq!(tag.name(), name);
        assert_eq!(tag.id(), HtmlTagId::from_name(name));
        assert!(tag.is_end_tag());
        assert!(!tag.is_empty_element());
        assert!(tag.attributes().is_empty());
        assert_eq!(HtmlToken::Tag(tag).to_html(), format!("</{name}>"));
    }
}

#[test]
fn token_names_are_raw_unlike_checked_writer_names() {
    for name in ["", "a b", "x>y", "a/b", "\0", "\u{e9}"] {
        let tag = HtmlTagToken::start(name, HtmlAttributeCollection::new());
        assert_eq!(tag.name(), name);
        assert_eq!(HtmlToken::Tag(tag).to_html(), format!("<{name}>"));
    }
}

#[test]
fn default_doctype_and_quirks_metadata_do_not_invent_a_name() {
    let mut doctype = HtmlDocTypeToken::new();
    assert_eq!(doctype, HtmlDocTypeToken::default());
    assert_eq!(doctype.name(), None);
    assert_eq!(doctype.public_identifier(), None);
    assert_eq!(doctype.system_identifier(), None);
    assert_eq!(doctype.public_keyword(), None);
    assert_eq!(doctype.system_keyword(), None);
    assert!(!doctype.force_quirks_mode);
    assert_eq!(render_doctype(&doctype), "<!DOCTYPE>");
    doctype.force_quirks_mode = true;
    assert_eq!(render_doctype(&doctype), "<!DOCTYPE>");
}

#[test]
fn public_identifier_keyword_transitions_match_mimekit() {
    let mut doctype = HtmlDocTypeToken::new();
    doctype
        .set_public_identifier(Some("public-identifier".into()))
        .unwrap();
    assert_eq!(doctype.public_identifier(), Some("public-identifier"));
    assert_eq!(doctype.public_keyword(), Some("PUBLIC"));
    assert_eq!(doctype.system_keyword(), None);
    doctype.set_public_identifier(None).unwrap();
    assert_eq!(doctype.public_identifier(), None);
    assert_eq!(doctype.public_keyword(), Some("PUBLIC"));
    assert_eq!(doctype.system_keyword(), None);

    doctype
        .set_public_identifier(Some("public-identifier".into()))
        .unwrap();
    doctype
        .set_system_identifier(Some("system-identifier".into()))
        .unwrap();
    assert_eq!(doctype.system_keyword(), None);
    doctype.set_public_identifier(None).unwrap();
    assert_eq!(doctype.public_keyword(), Some("PUBLIC"));
    assert_eq!(doctype.system_keyword(), Some("SYSTEM"));
}

#[test]
fn system_identifier_keyword_transitions_match_mimekit() {
    let mut doctype = named_doctype();
    doctype
        .set_system_identifier(Some("system-identifier".into()))
        .unwrap();
    assert_eq!(doctype.system_identifier(), Some("system-identifier"));
    assert_eq!(doctype.system_keyword(), Some("SYSTEM"));
    doctype
        .set_public_identifier(Some("public-identifier".into()))
        .unwrap();
    assert_eq!(doctype.system_keyword(), Some("SYSTEM"));
    assert_eq!(
        render_doctype(&doctype),
        "<!DOCTYPE html PUBLIC \"public-identifier\" \"system-identifier\">"
    );
    doctype.set_system_identifier(None).unwrap();
    assert_eq!(doctype.system_identifier(), None);
    assert_eq!(doctype.system_keyword(), None);
    assert_eq!(
        render_doctype(&doctype),
        "<!DOCTYPE html PUBLIC \"public-identifier\">"
    );
    doctype.set_public_identifier(None).unwrap();
    assert_eq!(render_doctype(&doctype), "<!DOCTYPE html>");
}

#[test]
fn invalid_doctype_names_leave_all_fields_unchanged() {
    // Names that cannot be written as a DOCTYPE used to permit markup to be
    // injected into or after it, for example a name containing "html><script>".
    let mut doctype = named_doctype();
    doctype.set_public_identifier(Some("p".into())).unwrap();
    doctype.set_system_identifier(Some("s".into())).unwrap();
    let original = doctype.clone();
    for name in [
        "",
        "html><script>",
        "html PUBLIC",
        "a\tb",
        "a\nb",
        "a\u{c}b",
        "a\rb",
        "a\0b",
    ] {
        assert_eq!(
            doctype.set_name(Some(name.into())),
            Err(HtmlDocTypeError::InvalidName)
        );
        assert_eq!(doctype, original);
    }
}

#[test]
fn doctype_name_checks_use_the_exact_ascii_delimiter_set() {
    for byte in 0..=127u8 {
        let name = format!("x{}y", char::from(byte));
        let mut doctype = named_doctype();
        let original = doctype.clone();
        let result = doctype.set_name(Some(name.clone()));
        if b"\0\t\n\x0c\r >".contains(&byte) {
            assert_eq!(result, Err(HtmlDocTypeError::InvalidName), "{byte}");
            assert_eq!(doctype, original);
        } else {
            result.unwrap();
            assert_eq!(doctype.name(), Some(name.as_str()));
        }
    }
    for name in ["\u{a0}", "\u{2003}", "<x/\"'", "\u{1f600}"] {
        let mut doctype = named_doctype();
        doctype.set_name(Some(name.into())).unwrap();
        assert_eq!(render_doctype(&doctype), format!("<!DOCTYPE {name}>"));
    }
}

#[test]
fn invalid_doctype_identifiers_leave_values_and_keywords_unchanged() {
    let mut doctype = named_doctype();
    doctype.set_public_identifier(Some("p".into())).unwrap();
    doctype.set_system_identifier(Some("s".into())).unwrap();
    let original = doctype.clone();
    for identifier in ["x\"><p>", "a>b", "a\0b", "a'b\"c"] {
        assert_eq!(
            doctype.set_public_identifier(Some(identifier.into())),
            Err(HtmlDocTypeError::InvalidPublicIdentifier)
        );
        assert_eq!(doctype, original);
        assert_eq!(
            doctype.set_system_identifier(Some(identifier.into())),
            Err(HtmlDocTypeError::InvalidSystemIdentifier)
        );
        assert_eq!(doctype, original);
    }
}

#[test]
fn doctype_identifier_checks_allow_either_quote_but_not_both() {
    for byte in 0..=127u8 {
        let ch = char::from(byte);
        for id in [format!("x{ch}y"), format!("'{ch}"), format!("\"{ch}")] {
            let valid = byte != 0 && byte != b'>' && !(id.contains('\'') && id.contains('"'));
            let mut public = named_doctype();
            let mut system = public.clone();
            let original = public.clone();
            let public_result = public.set_public_identifier(Some(id.clone()));
            let system_result = system.set_system_identifier(Some(id.clone()));
            if valid {
                public_result.unwrap();
                system_result.unwrap();
                assert_eq!(public.public_identifier(), Some(id.as_str()));
                assert_eq!(system.system_identifier(), Some(id.as_str()));
            } else {
                assert_eq!(
                    public_result,
                    Err(HtmlDocTypeError::InvalidPublicIdentifier)
                );
                assert_eq!(
                    system_result,
                    Err(HtmlDocTypeError::InvalidSystemIdentifier)
                );
                assert_eq!(public, original);
                assert_eq!(system, original);
            }
        }
    }
}

#[test]
fn source_doctype_serialization_uses_literal_identifiers_and_safe_delimiters() {
    for (name, public, system, expected) in [
        (
            "html",
            Some("-//W3C//DTD HTML 4.01//EN"),
            Some("http://www.w3.org/TR/html4/strict.dtd"),
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\" \"http://www.w3.org/TR/html4/strict.dtd\">",
        ),
        (
            "<x/\"'",
            Some("it's"),
            Some("say \"hi\""),
            "<!DOCTYPE <x/\"' PUBLIC \"it's\" 'say \"hi\"'>",
        ),
        (
            "html",
            Some(""),
            Some(""),
            "<!DOCTYPE html PUBLIC \"\" \"\">",
        ),
        ("html", None, Some("s"), "<!DOCTYPE html SYSTEM \"s\">"),
        ("html", Some("p"), None, "<!DOCTYPE html PUBLIC \"p\">"),
        ("html", None, None, "<!DOCTYPE html>"),
        (
            "html",
            Some("a<&\u{e9}\r\nb"),
            Some(""),
            "<!DOCTYPE html PUBLIC \"a<&\u{e9}\r\nb\" \"\">",
        ),
        (
            "html",
            None,
            Some("say \"hi\""),
            "<!DOCTYPE html SYSTEM 'say \"hi\"'>",
        ),
    ] {
        let mut doctype = HtmlDocTypeToken::new();
        doctype.set_name(Some(name.into())).unwrap();
        doctype
            .set_public_identifier(public.map(str::to_owned))
            .unwrap();
        doctype
            .set_system_identifier(system.map(str::to_owned))
            .unwrap();
        assert_eq!(doctype.name(), Some(name));
        assert_eq!(doctype.public_identifier(), public);
        assert_eq!(doctype.system_identifier(), system);
        assert_eq!(render_doctype(&doctype), expected);
    }
}

#[test]
fn missing_doctype_name_suppresses_but_does_not_clear_identifiers() {
    for public in [None, Some("p")] {
        let mut doctype = HtmlDocTypeToken::new();
        doctype
            .set_public_identifier(public.map(str::to_owned))
            .unwrap();
        doctype.set_system_identifier(Some("s".into())).unwrap();
        // Without a name, PUBLIC or SYSTEM would be parsed as the name.
        assert_eq!(render_doctype(&doctype), "<!DOCTYPE>");
        doctype.set_name(Some("html".into())).unwrap();
        let with_name = render_doctype(&doctype);
        assert_eq!(
            with_name,
            if public.is_some() {
                "<!DOCTYPE html PUBLIC \"p\" \"s\">"
            } else {
                "<!DOCTYPE html SYSTEM \"s\">"
            }
        );
        doctype.set_name(None).unwrap();
        assert_eq!(render_doctype(&doctype), "<!DOCTYPE>");
        assert_eq!(doctype.public_identifier(), public);
        assert_eq!(doctype.system_identifier(), Some("s"));
        doctype.set_name(Some("html".into())).unwrap();
        assert_eq!(render_doctype(&doctype), with_name);
    }
}

#[test]
fn doctype_errors_implement_standard_error_reporting() {
    for error in [
        HtmlDocTypeError::InvalidName,
        HtmlDocTypeError::InvalidPublicIdentifier,
        HtmlDocTypeError::InvalidSystemIdentifier,
    ] {
        let error: &dyn std::error::Error = &error;
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
}

#[test]
fn writer_flushes_pending_syntax_before_every_token_kind() {
    let cases = [
        (HtmlToken::CData("".into()), "<![CDATA[]]>"),
        (HtmlToken::Comment(HtmlCommentToken::new("")), "<!---->"),
        (HtmlToken::Data(HtmlDataToken::new("")), ""),
        (HtmlToken::Data(HtmlDataToken::from_text("")), ""),
        (HtmlToken::Data(HtmlDataToken::from_data_state("")), ""),
        (HtmlToken::DocType(named_doctype()), "<!DOCTYPE html>"),
        (HtmlToken::ScriptData("".into()), ""),
        (
            HtmlToken::Tag(HtmlTagToken::start("p", HtmlAttributeCollection::new())),
            "<p>",
        ),
    ];
    for (token, expected) in cases {
        for state in 0..5 {
            let mut html = HtmlWriter::new();
            let prefix = match state {
                0 => "",
                1 => {
                    html.write_start_tag("div").unwrap();
                    "<div>"
                }
                2 => {
                    html.write_start_tag("div").unwrap();
                    html.write_attribute_name("hidden").unwrap();
                    "<div hidden>"
                }
                3 => {
                    html.write_empty_element_tag("img").unwrap();
                    "<img/>"
                }
                _ => {
                    html.write_empty_element_tag("img").unwrap();
                    html.write_attribute_name("hidden").unwrap();
                    "<img hidden/>"
                }
            };
            html.write_token(&token);
            assert_eq!(html.state(), HtmlWriterState::Content);
            assert_eq!(html.as_str(), format!("{prefix}{expected}"));
            assert_eq!(
                html.write_attribute("extra", "value"),
                Err(HtmlWriterError::AttributeOutsideTag)
            );
            html.write_text("tail");
            assert_eq!(html.finish(), format!("{prefix}{expected}tail"));
        }
    }
}

#[test]
fn writer_uses_token_encoding_and_does_not_parse_raw_contents() {
    let mut html = HtmlWriter::new();
    html.write_token(&HtmlToken::Data(HtmlDataToken::new("&amp;<")));
    html.write_token(&HtmlToken::Data(HtmlDataToken::from_text("&amp;<")));
    html.write_token(&HtmlToken::Data(HtmlDataToken::from_data_state("&amp;<")));
    html.write_token(&HtmlToken::ScriptData("<tag unfinished".into()));
    assert_eq!(html.state(), HtmlWriterState::Content);
    assert_eq!(html.finish(), "&amp;<&amp;amp;&lt;&amp;&lt;<tag unfinished");
}

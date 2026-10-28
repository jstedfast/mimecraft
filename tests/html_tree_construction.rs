// Adapted from MimeKit's UnitTests/Text/HtmlTreeConstructionTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HtmlAttribute, HtmlClosedElement, HtmlContentMode as Mode, HtmlNamespace,
    HtmlTagDisposition as Disposition, HtmlTagResult, HtmlTagToken, HtmlTreeConstruction,
    InvalidHtmlElementDepth,
};

fn start(tree: &mut HtmlTreeConstruction, name: &str) -> HtmlTagResult {
    tree.process_tag(&HtmlTagToken::start(name, vec![]))
}

fn end(tree: &mut HtmlTreeConstruction, name: &str) -> HtmlTagResult {
    tree.process_tag(&HtmlTagToken::end(name))
}

fn attributes(name: &str, values: &[(&str, Option<&str>)]) -> HtmlTagToken {
    HtmlTagToken::start(
        name,
        values
            .iter()
            .map(|(key, value)| HtmlAttribute::new(*key, value.map(str::to_owned)).unwrap())
            .collect::<Vec<_>>(),
    )
}

fn closed(tree: &HtmlTreeConstruction) -> Vec<&str> {
    tree.closed_elements()
        .iter()
        .map(|e| e.name.as_str())
        .collect()
}

#[test]
fn content_state() {
    for (name, mode) in [
        ("style", Mode::RawText),
        ("xmp", Mode::RawText),
        ("iframe", Mode::RawText),
        ("noembed", Mode::RawText),
        ("noframes", Mode::RawText),
        ("title", Mode::RcData),
        ("textarea", Mode::RcData),
        ("script", Mode::ScriptData),
        ("plaintext", Mode::PlainText),
        ("div", Mode::Data),
    ] {
        for empty in [false, true] {
            let mut tree = HtmlTreeConstruction::with_max_element_depth(32).unwrap();
            let tag = if empty {
                HtmlTagToken::empty(name, vec![])
            } else {
                HtmlTagToken::start(name, vec![])
            };
            let result = tree.process_tag(&tag);
            assert_eq!(result.content_mode, mode, "{name}, empty={empty}");
            assert_eq!(result.disposition, Disposition::Normal);
            assert!(!result.broke_out_of_foreign_content);
            assert_eq!(tree.last_tag(), Some(result));
        }
    }
}

#[test]
fn noscript_state() {
    for enabled in [false, true] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_scripting_enabled(enabled);
        assert_eq!(tree.scripting_enabled(), enabled);
        assert_eq!(
            start(&mut tree, "noscript").content_mode,
            if enabled { Mode::RawText } else { Mode::Data }
        );
        assert!(tree.is_noscript_open());
        assert_eq!(end(&mut tree, "noscript").content_mode, Mode::Data);
        assert!(!tree.is_noscript_open());
    }
}

#[test]
fn foreign_content_state() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "svg");
    let result = start(&mut tree, "style");
    assert!(tree.is_in_foreign_content());
    assert_eq!(result.content_mode, Mode::Data);
    assert_eq!(result.disposition, Disposition::Normal);
    end(&mut tree, "style");
    start(&mut tree, "foreignObject");
    assert_eq!(start(&mut tree, "style").content_mode, Mode::RawText);
    assert!(!tree.is_in_foreign_content());
}

#[test]
fn frameset_dispositions() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "frameset");
    let result = start(&mut tree, "style");
    assert!(tree.is_in_frameset());
    assert_eq!(result.disposition, Disposition::Ignored);
    assert_eq!(result.content_mode, Mode::Data);
    let result = start(&mut tree, "noframes");
    assert_eq!(result.disposition, Disposition::Normal);
    assert_eq!(result.content_mode, Mode::RawText);
}

#[test]
fn merged_attributes() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "body");
    let result = tree.process_tag(&attributes("body", &[("class", Some("example"))]));
    assert_eq!(result.disposition, Disposition::AttributesMerged);
    assert_eq!(result.content_mode, Mode::Data);
    assert_eq!(end(&mut tree, "body").disposition, Disposition::Normal);
}

#[test]
fn ignored_end_tag() {
    let mut tree = HtmlTreeConstruction::new();
    for name in ["math", "mi", "span"] {
        start(&mut tree, name);
    }
    let result = end(&mut tree, "mi");
    assert_eq!(result.disposition, Disposition::Ignored);
    assert_eq!(result.content_mode, Mode::Data);
    assert_eq!(end(&mut tree, "span").disposition, Disposition::Normal);
    assert!(tree.is_in_foreign_content());
}

#[test]
fn implicit_closures() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_track_closed_elements(true);
    for name in ["svg", "g", "g"] {
        start(&mut tree, name);
    }
    let result = start(&mut tree, "div");
    assert!(result.broke_out_of_foreign_content);
    assert!(!tree.is_in_foreign_content());
    assert_eq!(
        tree.closed_elements(),
        [
            HtmlClosedElement {
                name: "g".into(),
                count: 2,
                is_foreign: true
            },
            HtmlClosedElement {
                name: "svg".into(),
                count: 1,
                is_foreign: true
            },
        ]
    );
    assert!(!end(&mut tree, "div").broke_out_of_foreign_content);
    assert!(tree.closed_elements().is_empty());
}

#[test]
fn form_removal_preserves_foreign_elements() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_track_closed_elements(true);
    for name in ["form", "svg", "style"] {
        start(&mut tree, name);
    }
    assert_eq!(end(&mut tree, "form").disposition, Disposition::Normal);
    assert!(tree.is_in_foreign_content());
    assert!(tree.closed_elements().is_empty());
    assert_eq!(start(&mut tree, "style").content_mode, Mode::Data);
}

#[test]
fn popped_form_pointer_does_not_refer_to_reused_slot() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "div");
    start(&mut tree, "form");
    end(&mut tree, "div");
    start(&mut tree, "section");
    start(&mut tree, "svg");
    end(&mut tree, "form");
    assert!(tree.is_in_foreign_content());
    assert_eq!(start(&mut tree, "style").content_mode, Mode::Data);
}

#[test]
fn depth_limit_overrides_content_state() {
    let mut tree = HtmlTreeConstruction::with_max_element_depth(1).unwrap();
    start(&mut tree, "div");
    assert_eq!(start(&mut tree, "style").content_mode, Mode::PlainText);
    assert!(tree.depth_exceeded());
}

#[test]
fn defaults_and_checked_depth() {
    let mut tree = HtmlTreeConstruction::default();
    assert_eq!(tree.max_element_depth(), 4096);
    assert!(tree.scripting_enabled());
    assert!(!tree.depth_exceeded());
    assert!(!tree.is_in_foreign_content());
    assert!(!tree.is_in_frameset());
    assert!(!tree.is_noscript_open());
    assert!(tree.tracks_character_data());
    assert!(!tree.tracks_closed_elements());
    assert_eq!(tree.last_tag(), None);
    assert_eq!(tree.html_namespace(), HtmlNamespace::Html);
    assert!(matches!(
        HtmlTreeConstruction::with_max_element_depth(0),
        Err(InvalidHtmlElementDepth)
    ));
    assert_eq!(tree.set_max_element_depth(0), Err(InvalidHtmlElementDepth));
    assert_eq!(tree.max_element_depth(), 4096);
    tree.set_max_element_depth(1).unwrap();
    start(&mut tree, "span");
    start(&mut tree, "SPAN");
    assert!(!tree.depth_exceeded());
    assert_eq!(start(&mut tree, "div").content_mode, Mode::PlainText);
    tree.set_max_element_depth(100).unwrap();
    assert!(tree.depth_exceeded());
    let result = end(&mut tree, "span");
    assert_eq!(result.content_mode, Mode::PlainText);
    assert_eq!(result.disposition, Disposition::Ignored);
    assert_eq!(tree.last_tag(), Some(result));
}

#[test]
fn overflow_during_head_noscript_is_terminal() {
    let mut tree = HtmlTreeConstruction::with_max_element_depth(1).unwrap();
    tree.set_scripting_enabled(false);
    start(&mut tree, "title");
    // Supplying tokens directly can exceed the budget in head-noscript bookkeeping.
    assert_eq!(start(&mut tree, "noscript").content_mode, Mode::PlainText);
    tree.process_character_data("non-whitespace");
    for name in ["div", "head", "br", "noscript", "form"] {
        assert_eq!(start(&mut tree, name).content_mode, Mode::PlainText);
        assert_eq!(end(&mut tree, name).content_mode, Mode::PlainText);
    }
}

#[test]
fn namespace_urls() {
    for ns in [
        HtmlNamespace::Html,
        HtmlNamespace::MathMl,
        HtmlNamespace::Svg,
        HtmlNamespace::XLink,
        HtmlNamespace::Xml,
        HtmlNamespace::XmlNs,
    ] {
        assert_eq!(HtmlNamespace::from_url(ns.as_str()), Some(ns));
        assert_eq!(
            HtmlNamespace::from_url(&ns.as_str().to_ascii_uppercase()),
            Some(ns)
        );
        assert_eq!(HtmlNamespace::from_url(&format!(" {}", ns.as_str())), None);
        assert_eq!(HtmlNamespace::from_url(&format!("{}x", ns.as_str())), None);
    }
    assert_eq!(HtmlNamespace::from_url(""), None);
    assert_eq!(HtmlNamespace::from_url("urn:unknown"), None);
}

#[test]
fn declared_namespace_uses_last_valued_attribute_not_current_node() {
    let mut tree = HtmlTreeConstruction::new();
    let tag = attributes(
        "html",
        &[
            ("xmlns", Some(HtmlNamespace::MathMl.as_str())),
            ("XMLNS", Some(HtmlNamespace::Svg.as_str())),
            ("xmlns", None),
        ],
    );
    tree.process_tag(&tag);
    assert_eq!(tree.html_namespace(), HtmlNamespace::Svg);
    assert!(!tree.is_in_foreign_content());
    assert_eq!(start(&mut tree, "style").content_mode, Mode::RawText);
    end(&mut tree, "style");
    tree.process_tag(&HtmlTagToken::empty(
        "html",
        vec![HtmlAttribute::new("xmlns", Some(HtmlNamespace::MathMl.as_str().into())).unwrap()],
    ));
    assert_eq!(tree.html_namespace(), HtmlNamespace::Svg);
    tree.process_tag(&attributes("html", &[("xmlns", Some("unrecognized"))]));
    assert_eq!(tree.html_namespace(), HtmlNamespace::Html);
    start(&mut tree, "svg");
    tree.process_tag(&tag);
    assert_eq!(tree.html_namespace(), HtmlNamespace::Html);
    assert!(tree.is_in_foreign_content());
}

#[test]
fn svg_and_math_integration_points() {
    for (root, integration) in [
        ("svg", "foreignObject"),
        ("svg", "desc"),
        ("svg", "title"),
        ("math", "mi"),
        ("math", "mo"),
        ("math", "mn"),
        ("math", "ms"),
        ("math", "mtext"),
    ] {
        let mut tree = HtmlTreeConstruction::new();
        start(&mut tree, root);
        start(&mut tree, &integration.to_ascii_uppercase());
        assert!(tree.is_in_foreign_content(), "{integration}");
        let result = start(&mut tree, "style");
        assert_eq!(result.content_mode, Mode::RawText, "{integration}");
        assert!(!result.broke_out_of_foreign_content);
        end(&mut tree, "style");
        assert!(tree.is_in_foreign_content());
        end(&mut tree, integration);
        assert_eq!(start(&mut tree, "style").content_mode, Mode::Data);
    }
}

#[test]
fn math_text_integration_exclusions() {
    for name in ["mglyph", "malignmark"] {
        let mut tree = HtmlTreeConstruction::new();
        start(&mut tree, "math");
        start(&mut tree, "mi");
        start(&mut tree, name);
        assert_eq!(start(&mut tree, "style").content_mode, Mode::Data);
        end(&mut tree, "style");
        end(&mut tree, name);
        assert_eq!(start(&mut tree, "style").content_mode, Mode::RawText);
    }
}

#[test]
fn annotation_encoding_uses_first_attribute() {
    for (first, expected) in [
        (None, Mode::Data),
        (Some("text/plain"), Mode::Data),
        (Some("TEXT/HTML"), Mode::RawText),
        (Some("application/xhtml+xml"), Mode::RawText),
    ] {
        let mut tree = HtmlTreeConstruction::new();
        start(&mut tree, "math");
        tree.process_tag(&attributes(
            "annotation-xml",
            &[("encoding", first), ("ENCODING", Some("text/html"))],
        ));
        assert_eq!(start(&mut tree, "style").content_mode, expected);
    }
    let mut tree = HtmlTreeConstruction::new();
    for name in ["math", "annotation-xml", "svg", "foreignObject"] {
        start(&mut tree, name);
    }
    // svg is processed with HTML rules even without annotation-xml's HTML encoding.
    assert_eq!(start(&mut tree, "style").content_mode, Mode::RawText);
}

#[test]
fn foreign_font_breakout_requires_presentational_attribute() {
    for attribute in [
        None,
        Some("color"),
        Some("face"),
        Some("size"),
        Some("class"),
    ] {
        let mut tree = HtmlTreeConstruction::new();
        start(&mut tree, "svg");
        let tag = if let Some(name) = attribute {
            attributes("font", &[(name, None)])
        } else {
            HtmlTagToken::start("font", vec![])
        };
        let result = tree.process_tag(&tag);
        let breakout = matches!(attribute, Some("color" | "face" | "size"));
        assert_eq!(result.broke_out_of_foreign_content, breakout);
        assert_eq!(tree.is_in_foreign_content(), !breakout);
    }
}

#[test]
fn foreign_end_breakout_stops_at_integration_point() {
    for name in ["br", "p"] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_track_closed_elements(true);
        for tag in ["math", "mi", "mglyph", "style"] {
            start(&mut tree, tag);
        }
        assert!(end(&mut tree, name).broke_out_of_foreign_content);
        assert_eq!(closed(&tree), ["style", "mglyph"]);
        assert!(tree.is_in_foreign_content());
        assert!(!end(&mut tree, name).broke_out_of_foreign_content);
        assert_eq!(start(&mut tree, "style").content_mode, Mode::RawText);
    }
}

#[test]
fn foreign_self_closing_elements_are_not_pushed() {
    let mut tree = HtmlTreeConstruction::new();
    tree.process_tag(&HtmlTagToken::empty("svg", vec![]));
    assert!(!tree.is_in_foreign_content());
    start(&mut tree, "math");
    tree.process_tag(&HtmlTagToken::empty("mi", vec![]));
    assert_eq!(start(&mut tree, "style").content_mode, Mode::Data);
}

#[test]
fn closures_can_be_enabled_cleared_and_disabled_without_changing_state() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "svg");
    start(&mut tree, "g");
    tree.set_track_closed_elements(true);
    start(&mut tree, "p");
    assert_eq!(closed(&tree), ["g", "svg"]);
    tree.set_track_closed_elements(true);
    assert_eq!(closed(&tree), ["g", "svg"]);
    tree.clear_closed_elements();
    assert!(tree.closed_elements().is_empty());
    start(&mut tree, "svg");
    start(&mut tree, "g");
    tree.set_track_closed_elements(false);
    assert!(!tree.tracks_closed_elements());
    assert!(tree.is_in_foreign_content());
    assert!(start(&mut tree, "div").broke_out_of_foreign_content);
    assert!(tree.closed_elements().is_empty());
}

#[test]
fn character_data_closes_head_noscript_but_not_for_html_spaces() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_scripting_enabled(false);
    tree.set_track_closed_elements(true);
    start(&mut tree, "head");
    start(&mut tree, "noscript");
    tree.process_character_data(" \t\r\n\u{c}");
    assert!(tree.is_noscript_open());
    tree.process_character_data("\u{a0}");
    assert!(!tree.is_noscript_open());
    assert!(!tree.tracks_character_data());
    assert_eq!(
        start(&mut tree, "frameset").disposition,
        Disposition::Ignored
    );
    // Whitespace does not imply html, but NUL does.
    let mut tree = HtmlTreeConstruction::new();
    tree.process_character_data(" ");
    assert_eq!(
        tree.process_tag(&attributes("html", &[("lang", Some("en"))]))
            .disposition,
        Disposition::Normal
    );
    let mut tree = HtmlTreeConstruction::new();
    tree.process_character_data("\0");
    assert_eq!(
        tree.process_tag(&attributes("html", &[("lang", Some("en"))]))
            .disposition,
        Disposition::AttributesMerged
    );
    assert_eq!(
        start(&mut tree, "frameset").disposition,
        Disposition::Normal
    );
}

#[test]
fn head_noscript_whitelist_and_reprocessing() {
    for name in [
        "html", "basefont", "bgsound", "link", "meta", "noframes", "style",
    ] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_scripting_enabled(false);
        start(&mut tree, "noscript");
        start(&mut tree, name);
        assert!(tree.is_noscript_open(), "{name}");
    }
    for name in ["head", "noscript"] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_scripting_enabled(false);
        start(&mut tree, "noscript");
        assert_eq!(start(&mut tree, name).disposition, Disposition::Ignored);
        end(&mut tree, "noscript");
        assert!(!tree.is_noscript_open());
    }
    for name in ["body", "div", "title", "base"] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_scripting_enabled(false);
        start(&mut tree, "noscript");
        start(&mut tree, name);
        assert!(!tree.is_noscript_open(), "{name}");
    }
    for name in ["head", "body", "html"] {
        let mut tree = HtmlTreeConstruction::new();
        tree.set_scripting_enabled(false);
        start(&mut tree, "noscript");
        end(&mut tree, name);
        assert!(tree.is_noscript_open());
        end(&mut tree, "br");
        assert!(!tree.is_noscript_open());
    }
}

#[test]
fn template_preserves_head_and_implied_body_resets_frameset_flag() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "head");
    start(&mut tree, "template");
    tree.process_character_data("text");
    assert!(tree.tracks_character_data());
    for name in ["html", "body", "frameset"] {
        assert_eq!(
            tree.process_tag(&attributes(name, &[("class", Some("x"))]))
                .disposition,
            Disposition::Ignored
        );
    }
    end(&mut tree, "template");
    // A div implies body and resets frameset-ok despite earlier template text.
    start(&mut tree, "div");
    assert_eq!(
        start(&mut tree, "frameset").disposition,
        Disposition::Normal
    );
    assert!(tree.is_in_frameset());
}

#[test]
fn frameset_nesting_self_closing_and_terminal_modes() {
    let mut tree = HtmlTreeConstruction::new();
    tree.process_tag(&HtmlTagToken::empty("frameset", vec![]));
    tree.process_tag(&HtmlTagToken::empty("frameset", vec![]));
    end(&mut tree, "frameset");
    assert_eq!(start(&mut tree, "frame").disposition, Disposition::Normal);
    end(&mut tree, "frameset");
    for name in ["frame", "frameset", "svg", "style", "script", "plaintext"] {
        let result = start(&mut tree, name);
        assert_eq!(result.disposition, Disposition::Ignored, "{name}");
        assert_eq!(result.content_mode, Mode::Data);
    }
    tree.process_character_data("text");
    assert!(tree.is_in_frameset());
    assert!(!tree.tracks_character_data());
    assert_eq!(start(&mut tree, "noframes").content_mode, Mode::RawText);
}

#[test]
fn hidden_input_uses_first_ascii_type_only() {
    for (first, accepted) in [
        (None, false),
        (Some("hidden"), true),
        (Some("HIDDEN"), true),
        (Some("h\u{131}dden"), false),
        (Some(" hidden"), false),
        (Some("text"), false),
    ] {
        let mut tree = HtmlTreeConstruction::new();
        tree.process_tag(&attributes(
            "input",
            &[("type", first), ("TYPE", Some("hidden"))],
        ));
        start(&mut tree, "frameset");
        assert_eq!(tree.is_in_frameset(), accepted, "{first:?}");
    }
}

#[test]
fn table_structure_and_implied_closures() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_track_closed_elements(true);
    for name in ["td", "tr", "tbody", "caption", "colgroup", "col"] {
        assert_eq!(start(&mut tree, name).disposition, Disposition::Ignored);
    }
    start(&mut tree, "table");
    start(&mut tree, "td");
    start(&mut tree, "span");
    start(&mut tree, "th");
    assert_eq!(closed(&tree), ["span", "td"]);
    end(&mut tree, "table");
    assert_eq!(closed(&tree), ["th", "tr", "tbody"]);
    start(&mut tree, "table");
    start(&mut tree, "tr");
    start(&mut tree, "span");
    start(&mut tree, "table");
    assert_eq!(closed(&tree), ["span", "tr", "tbody"]);
}

#[test]
fn forms_in_table_keep_pointer_after_immediate_pop() {
    let mut tree = HtmlTreeConstruction::new();
    start(&mut tree, "table");
    start(&mut tree, "form");
    start(&mut tree, "td");
    assert_eq!(start(&mut tree, "form").disposition, Disposition::Ignored);
    end(&mut tree, "form");
    assert_eq!(start(&mut tree, "form").disposition, Disposition::Normal);
}

#[test]
fn form_end_generates_implied_closures_but_preserves_unrelated_nodes() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_track_closed_elements(true);
    for name in ["form", "div", "p", "rb", "rtc"] {
        start(&mut tree, name);
    }
    end(&mut tree, "form");
    assert_eq!(closed(&tree), ["rtc", "rb", "p"]);
    start(&mut tree, "svg");
    end(&mut tree, "div");
    assert_eq!(closed(&tree), ["svg"]);
    assert!(!tree.is_in_foreign_content());
}

#[test]
fn scope_boundaries_list_items_and_headings() {
    let mut tree = HtmlTreeConstruction::new();
    tree.set_track_closed_elements(true);
    for name in ["ul", "li", "span"] {
        start(&mut tree, name);
    }
    start(&mut tree, "li");
    assert_eq!(closed(&tree), ["span"]);
    start(&mut tree, "ul");
    assert_eq!(end(&mut tree, "li").disposition, Disposition::Ignored);
    end(&mut tree, "ul");
    end(&mut tree, "li");
    for name in ["h1", "span"] {
        start(&mut tree, name);
    }
    end(&mut tree, "h6");
    assert_eq!(closed(&tree), ["span"]);
    for name in ["p", "button", "span"] {
        start(&mut tree, name);
    }
    assert_eq!(end(&mut tree, "p").disposition, Disposition::Ignored);
    end(&mut tree, "button");
    assert_eq!(closed(&tree), ["span"]);
}

// These are token sequences, not HTML input: the character-scanning tokenizer
// is deliberately not approximated by the structural regression helper.
fn feed(tree: &mut HtmlTreeConstruction, sequence: &str) {
    for name in sequence.split_ascii_whitespace() {
        if let Some(name) = name.strip_prefix('/') {
            end(tree, name);
        } else {
            start(tree, name);
        }
    }
}

#[test]
fn formatting_end_tags_close_foreign_content() {
    // Adapted from HtmlTokenizerTests.TestFormattingEndTagClosesForeignContent.
    for name in [
        "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt",
        "u",
    ] {
        let mut tree = HtmlTreeConstruction::new();
        start(&mut tree, name);
        feed(&mut tree, "p svg");
        end(&mut tree, name);
        assert!(!tree.is_in_foreign_content(), "{name}");
        assert_eq!(start(&mut tree, "script").content_mode, Mode::ScriptData);
    }
}

#[test]
fn adoption_agency_stack_updates() {
    // Adapted from HtmlTokenizerTests.TestAdoptionAgencyStackUpdates.
    for (sequence, raw) in [
        ("b div svg /b svg /div", true),
        ("b div section svg /b svg /section", true),
        ("b div math /b", true),
        ("b div svg g /b", true),
        ("b b div svg /b svg /b", true),
        ("b div svg foreignObject /b svg", false),
        ("b table tr td svg /b", false),
        ("b object svg /b", false),
        ("b template svg /b", false),
        ("b span div /b /div svg /span", false),
        ("b i div /b /div svg /i", true),
        ("b i span span div /b /div svg /i", true),
        ("b i span span span div /b /div svg /i", false),
        ("b form div svg /b svg /form", false),
        ("form b div svg /b svg /form", false),
        ("form template b form div svg /b /template svg /form", false),
    ] {
        let mut tree = HtmlTreeConstruction::new();
        feed(&mut tree, sequence);
        assert_eq!(
            start(&mut tree, "style").content_mode,
            if raw { Mode::RawText } else { Mode::Data },
            "{sequence}"
        );
    }
}

#[test]
fn adoption_agency_outer_loop_limit() {
    // Adapted from HtmlTokenizerTests.TestAdoptionAgencyOuterLoopLimit.
    for blocks in [0, 1, 7, 8, 9] {
        for folded in [false, true] {
            let mut tree = HtmlTreeConstruction::new();
            start(&mut tree, "b");
            for _ in 0..blocks {
                start(&mut tree, "div");
                if !folded {
                    start(&mut tree, "span");
                }
            }
            feed(&mut tree, "svg /b");
            assert_eq!(
                start(&mut tree, "style").content_mode,
                if blocks < 8 {
                    Mode::RawText
                } else {
                    Mode::Data
                },
                "blocks={blocks}, folded={folded}"
            );
        }
    }
}

#[test]
fn adoption_replacement_can_exceed_distinct_entry_depth() {
    let mut tree = HtmlTreeConstruction::with_max_element_depth(3).unwrap();
    feed(&mut tree, "b b div div svg");
    assert!(!tree.depth_exceeded());
    assert_eq!(end(&mut tree, "b").content_mode, Mode::PlainText);
    assert!(tree.depth_exceeded());
}

#[test]
fn table_implied_entries_count_toward_depth_limit() {
    for maximum in [1, 2, 3, 4] {
        let mut tree = HtmlTreeConstruction::with_max_element_depth(maximum).unwrap();
        start(&mut tree, "table");
        // The implied tbody and tr count too.
        let result = start(&mut tree, "td");
        assert_eq!(tree.depth_exceeded(), maximum < 4);
        assert_eq!(
            result.content_mode,
            if maximum < 4 {
                Mode::PlainText
            } else {
                Mode::Data
            }
        );
    }
}

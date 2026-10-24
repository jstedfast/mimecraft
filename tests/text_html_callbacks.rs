// Adapted from MimeKit's UnitTests/Text/TextToHtmlTests.cs and FlowedToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    encodings::NewLineFormat,
    text::{
        FlowedToHtml, HeaderFooterFormat, HtmlAttributeId, HtmlTagContext, HtmlTagId, HtmlWriter,
        HtmlWriterState, TextToHtml,
    },
};

fn convert(
    flowed: bool,
    text: &str,
    callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
) -> String {
    if flowed {
        let mut converter = FlowedToHtml::new();
        converter.output_html_fragment = true;
        converter.convert_with_tag_callback(text, callback)
    } else {
        let converter = TextToHtml {
            output_html_fragment: true,
            ..TextToHtml::new()
        };
        converter.convert_with_tag_callback(text, callback)
    }
}

fn paragraph(flowed: bool, text: &str) -> String {
    if flowed {
        format!("<p>{text}</p>\r\n")
    } else {
        format!("{text}<br/>")
    }
}

#[test]
fn deleted_quote_end_tags_stay_deleted_at_eof() {
    for flowed in [false, true] {
        // EOF must not bypass the callback's earlier deletion decisions.
        let html = convert(flowed, "> a\n>> b", |context, writer| {
            if context.tag_id() == HtmlTagId::BlockQuote {
                context.delete_tag = true;
                context.delete_end_tag = true;
                context.invoke_callback_for_end_tag = true;
                assert!(!context.is_end_tag());
            } else {
                context.write_tag(writer, true).unwrap();
            }
        });
        assert_eq!(html, paragraph(flowed, "a") + &paragraph(flowed, "b"));
    }
}

#[test]
fn suppressed_quote_descendants_never_reach_the_callback() {
    for flowed in [false, true] {
        for close_quotes in [false, true] {
            let text = format!(
                "> a\n>> b\n>>> b\nc\n>> d\n>>> e{}",
                if close_quotes { "\nf" } else { "" }
            );
            let mut invoked = 0;
            let html = convert(flowed, &text, |context, writer| {
                if context.tag_id() == HtmlTagId::BlockQuote && !context.is_end_tag() {
                    // In the source regression, deeper quotes incorrectly
                    // reached the callback even though their parent hid them.
                    context.suppress_inner_content = true;
                    invoked += 1;
                }
                context.write_tag(writer, true).unwrap();
            });
            let mut expected = format!(
                "<blockquote></blockquote>{}<blockquote></blockquote>",
                paragraph(flowed, "c")
            );
            if close_quotes {
                expected.push_str(&paragraph(flowed, "f"));
            }
            assert_eq!(html, expected);
            assert_eq!(invoked, 2);
        }
    }
}

#[test]
fn suppression_ends_at_its_owner_without_losing_outer_contexts() {
    for flowed in [false, true] {
        let mut opened = 0;
        let mut closed = 0;
        let html = convert(
            flowed,
            "> a\n>> hidden\n>>>> hidden\n>>> hidden\n> b\nc",
            |context, writer| {
                if context.tag_id() == HtmlTagId::BlockQuote {
                    if context.is_end_tag() {
                        closed += 1;
                        assert_eq!(context.suppress_inner_content, closed == 1);
                    } else {
                        opened += 1;
                        context.invoke_callback_for_end_tag = true;
                        context.suppress_inner_content = opened == 2;
                    }
                }
                context.write_tag(writer, true).unwrap();
            },
        );
        assert_eq!((opened, closed), (2, 2));
        assert_eq!(
            html,
            format!(
                "<blockquote>{}<blockquote></blockquote>{}</blockquote>{}",
                paragraph(flowed, "a"),
                paragraph(flowed, "b"),
                paragraph(flowed, "c"),
            )
        );
    }
}

#[test]
fn replacement_end_tags_and_original_link_attributes_are_preserved() {
    for flowed in [false, true] {
        let mut link_calls = 0;
        let html = convert(flowed, "> www.example.com", |context, writer| {
            let replacement = match context.tag_id() {
                HtmlTagId::BlockQuote => "section",
                HtmlTagId::A => {
                    link_calls += 1;
                    assert_eq!(
                        context
                            .attributes()
                            .get_by_id(HtmlAttributeId::Href)
                            .unwrap()
                            .value(),
                        Some("http://www.example.com")
                    );
                    "strong"
                }
                _ => {
                    context.write_tag(writer, true).unwrap();
                    return;
                }
            };
            if context.is_end_tag() {
                writer.write_end_tag(replacement).unwrap();
            } else {
                context.invoke_callback_for_end_tag = true;
                writer.write_start_tag(replacement).unwrap();
            }
        });
        assert_eq!(link_calls, 2);
        assert_eq!(
            html,
            format!(
                "<section>{}</section>",
                paragraph(flowed, "<strong>www.example.com</strong>")
            )
        );
    }
}

#[test]
fn link_suppression_and_unwrapping_leave_surrounding_text_visible() {
    for flowed in [false, true] {
        let mut links = 0;
        let html = convert(
            flowed,
            "a www.one.com b www.two.com c",
            |context, writer| {
                if context.tag_id() == HtmlTagId::A {
                    links += 1;
                    context.delete_tag = true;
                    context.delete_end_tag = true;
                    context.suppress_inner_content = links == 1;
                } else {
                    context.write_tag(writer, true).unwrap();
                }
            },
        );
        assert_eq!(links, 2);
        assert_eq!(html, paragraph(flowed, "a  b www.two.com c"));
    }
}

#[test]
fn link_attribute_replacement_is_escaped_but_does_not_change_the_label() {
    for flowed in [false, true] {
        let html = convert(flowed, "www.example.com", |context, writer| {
            if context.tag_id() == HtmlTagId::A {
                context.write_tag(writer, false).unwrap();
                writer
                    .write_attribute("href", "https://other/?a=\"\u{e9}\"&b=2")
                    .unwrap();
                writer.write_attribute_name("data-flag").unwrap();
            } else {
                context.write_tag(writer, true).unwrap();
            }
        });
        assert_eq!(
            html,
            paragraph(
                flowed,
                concat!(
                    "<a href=\"https://other/?a=&quot;&#233;&quot;&amp;b=2\" data-flag>",
                    "www.example.com</a>",
                )
            )
        );
    }
}

#[test]
fn paragraph_suppression_skips_links_but_not_its_own_end_callback() {
    let mut paragraphs = 0;
    let mut ends = 0;
    let html = convert(true, "www.secret.com\n\npublic", |context, writer| {
        assert_ne!(context.tag_id(), HtmlTagId::A);
        if context.tag_id() == HtmlTagId::P {
            if context.is_end_tag() {
                ends += 1;
                assert_eq!(context.suppress_inner_content, ends == 1);
            } else {
                paragraphs += 1;
                context.suppress_inner_content = paragraphs == 1;
                context.invoke_callback_for_end_tag = true;
            }
        }
        context.write_tag(writer, true).unwrap();
    });
    assert_eq!((paragraphs, ends), (2, 2));
    assert_eq!(html, "<p></p>\r\n<br/>\r\n<p>public</p>\r\n");
}

#[test]
fn flowed_newlines_follow_delete_tag_not_delete_end_tag() {
    for (delete_tag, delete_end, expected) in [
        (false, false, "<p>x</p>\r\n<br/>\r\n"),
        (false, true, "<p>x\r\n<br/>\r\n"),
        (true, false, "<p>x</p><br/>"),
        (true, true, "<p>x<br/>"),
    ] {
        let mut end_calls = 0;
        let html = convert(true, "x\n\n", |context, writer| {
            if context.is_end_tag() {
                end_calls += 1;
                assert_eq!(context.delete_tag, delete_tag);
            } else {
                context.delete_tag = delete_tag;
                context.delete_end_tag = delete_end;
                context.invoke_callback_for_end_tag = true;
            }
            // DeleteTag does not undo what the callback chooses to write.
            context.write_tag(writer, true).unwrap();
        });
        assert_eq!(html, expected);
        assert_eq!(end_calls, usize::from(!delete_end));
    }
}

#[test]
fn flowed_end_callback_can_change_newline_decision() {
    let html = convert(true, "a\nb", |context, writer| {
        context.invoke_callback_for_end_tag = true;
        context.delete_tag = context.is_end_tag();
        context.write_tag(writer, true).unwrap();
    });
    assert_eq!(html, "<p>a</p><p>b</p>");
}

#[test]
fn removing_breaks_does_not_suppress_following_lines() {
    for flowed in [false, true] {
        let mut breaks = 0;
        let html = convert(flowed, "a\n\nb", |context, writer| {
            if context.tag_id() == HtmlTagId::Br {
                assert!(!context.is_end_tag());
                assert!(context.is_empty_element());
                breaks += 1;
                context.delete_tag = true;
                context.suppress_inner_content = true;
                context.invoke_callback_for_end_tag = true;
            } else {
                context.write_tag(writer, true).unwrap();
            }
        });
        assert_eq!(breaks, if flowed { 1 } else { 3 });
        assert_eq!(
            html,
            if flowed {
                "<p>a</p>\r\n<p>b</p>\r\n"
            } else {
                "ab"
            }
        );
    }
}

#[test]
fn flowed_callbacks_see_joined_links_and_configured_newlines() {
    let mut converter = FlowedToHtml::with_new_line_format(NewLineFormat::Unix).unwrap();
    converter.output_html_fragment = true;
    converter.delete_space = true;
    let mut hrefs = Vec::new();
    let html =
        converter.convert_with_tag_callback("> www.exam \n> ple.com \nplain", |context, writer| {
            if context.tag_id() == HtmlTagId::A {
                hrefs.push(
                    context
                        .attributes()
                        .get("href")
                        .unwrap()
                        .value()
                        .unwrap()
                        .to_owned(),
                );
            }
            context.write_tag(writer, true).unwrap();
        });
    assert_eq!(hrefs, ["http://www.example.com"]);
    assert_eq!(
        html,
        "<blockquote><p><a href=\"http://www.example.com\">www.example.com</a></p>\n</blockquote><p>plain</p>\n"
    );
}

#[test]
fn empty_flowed_continuations_do_not_generate_phantom_callbacks() {
    let mut converter = FlowedToHtml::new();
    converter.output_html_fragment = true;
    converter.delete_space = true;
    for input in ["", "  ", ">  \n>>  "] {
        assert_eq!(
            converter.convert_with_tag_callback(input, |_, _| {
                panic!("empty pending paragraphs have no tags");
            }),
            ""
        );
    }
}

#[test]
fn wrappers_decorations_and_custom_markup_bypass_callbacks() {
    for flowed in [false, true] {
        let mut calls = 0;
        let callback = |context: &mut HtmlTagContext, writer: &mut HtmlWriter| {
            calls += 1;
            assert_eq!(context.tag_id(), HtmlTagId::BlockQuote);
            context.delete_tag = true;
            context.delete_end_tag = true;
            context.suppress_inner_content = true;
            writer.write_markup_text("<aside>replacement</aside>");
        };
        let html = if flowed {
            let mut converter = FlowedToHtml::new();
            converter.header = Some("> header".into());
            converter.footer = Some("<footer>raw</footer>".into());
            converter.footer_format = HeaderFooterFormat::Html;
            converter.convert_with_tag_callback("> www.hidden.com", callback)
        } else {
            let converter = TextToHtml {
                header: Some("> header".into()),
                footer: Some("<footer>raw</footer>".into()),
                footer_format: HeaderFooterFormat::Html,
                ..TextToHtml::new()
            };
            converter.convert_with_tag_callback("> www.hidden.com", callback)
        };
        assert_eq!(calls, 1);
        assert_eq!(
            html,
            "<html><body><blockquote>header<br/></blockquote><aside>replacement</aside><footer>raw</footer></body></html>"
        );
    }
}

#[test]
fn callback_contexts_are_fresh_on_each_conversion() {
    let mut flowed = FlowedToHtml::new();
    flowed.output_html_fragment = true;
    let plain = TextToHtml {
        output_html_fragment: true,
        ..TextToHtml::new()
    };
    for _ in 0..2 {
        assert_eq!(
            flowed.convert_with_tag_callback("> hidden", |context, _| {
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            }),
            ""
        );
        assert_eq!(
            plain.convert_with_tag_callback("> hidden", |context, _| {
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            }),
            ""
        );
        assert_eq!(
            flowed.convert("> visible"),
            "<blockquote><p>visible</p>\r\n</blockquote>"
        );
        assert_eq!(
            plain.convert("> visible"),
            "<blockquote>visible<br/></blockquote>"
        );
    }
}

#[test]
fn very_deep_hidden_quotations_only_invoke_the_outer_callback() {
    let input = format!("{} hidden\nplain", ">".repeat(1_000_000));
    for flowed in [false, true] {
        let mut quotes = 0;
        let html = convert(flowed, &input, |context, writer| {
            if context.tag_id() == HtmlTagId::BlockQuote {
                quotes += 1;
                context.suppress_inner_content = true;
                context.delete_end_tag = true;
            } else {
                context.write_tag(writer, true).unwrap();
            }
        });
        assert_eq!(quotes, 1);
        assert_eq!(html, paragraph(flowed, "plain"));
    }
}

#[test]
fn link_at_boundary_does_not_flush_pending_callback_tags_early() {
    for flowed in [false, true] {
        let mut pending = false;
        let html = convert(flowed, "> www.example.com", |context, writer| {
            if pending {
                assert_eq!(writer.state(), HtmlWriterState::Tag);
                writer.write_attribute("data-following", "yes").unwrap();
                pending = false;
            }
            if context.tag_id() == HtmlTagId::A && !context.is_end_tag() {
                assert_eq!(writer.state(), HtmlWriterState::Tag);
            }
            context.invoke_callback_for_end_tag = true;
            context.write_tag(writer, true).unwrap();
            if context.tag_id() == HtmlTagId::A && context.is_end_tag() {
                writer.write_empty_element_tag("hr").unwrap();
                pending = true;
            }
        });
        assert!(!pending);
        assert!(html.contains("<hr data-following=\"yes\"/>"));
    }
}

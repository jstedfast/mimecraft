// Adapted from MimeKit's UnitTests/Text/HtmlToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{
    HeaderFooterFormat, HtmlNoScriptHandling, HtmlTagContext, HtmlTagId, HtmlToHtml, HtmlToken,
    HtmlTokenizerError, HtmlTokenizerReader, HtmlWriter,
};
use std::{
    io::{self, BufReader, Read, Write},
    num::NonZeroUsize,
};

#[path = "data/html_to_html/cases.rs"]
mod cases;

type Callback = fn(&mut HtmlTagContext, &mut HtmlWriter);

fn keep(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
    context.write_tag(writer, true).unwrap();
}

fn remove_image(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
    if context.tag_id() == HtmlTagId::Image {
        context.delete_tag = true;
        context.delete_end_tag = true;
    } else {
        keep(context, writer);
    }
}

fn suppress_x(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
    if context.tag_name().eq_ignore_ascii_case("x") {
        context.suppress_inner_content = true;
    }
    keep(context, writer);
}

fn suppress_raw(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
    if matches!(context.tag_id(), HtmlTagId::Script | HtmlTagId::Style) {
        context.suppress_inner_content = true;
        context.delete_tag = true;
        context.delete_end_tag = true;
    } else {
        keep(context, writer);
    }
}

fn check(converter: &HtmlToHtml, input: &str, expected: &str, callback: Callback) {
    assert_eq!(
        converter
            .convert_with_tag_callback(input, callback)
            .unwrap(),
        expected,
        "{input:?}"
    );
    for capacity in [1, 7, 4096] {
        let reader = BufReader::with_capacity(capacity, input.as_bytes());
        assert_eq!(
            converter
                .convert_reader_with_tag_callback(reader, callback)
                .unwrap(),
            expected,
            "capacity={capacity}, {input:?}"
        );
    }
    let mut output = Vec::new();
    converter
        .convert_to_with_tag_callback(input.as_bytes(), &mut output, callback)
        .unwrap();
    assert_eq!(output, expected.as_bytes());
}

#[test]
fn defaults_and_empty_input() {
    let converter = HtmlToHtml::new();
    assert_eq!(converter, HtmlToHtml::default());
    assert!(!converter.filter_comments);
    assert!(!converter.output_html_fragment);
    assert_eq!(converter.max_element_depth.get(), 4096);
    assert_eq!(converter.no_script_handling, HtmlNoScriptHandling::Unwrap);
    assert_eq!(converter.header, None);
    assert_eq!(converter.footer, None);
    assert_eq!(converter.header_format, HeaderFooterFormat::Text);
    assert_eq!(converter.footer_format, HeaderFooterFormat::Text);
    assert!(NonZeroUsize::new(0).is_none());
    check(&converter, "", "", keep);
}

#[test]
fn original_document_with_image_url_replacement() {
    fn replace(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
        if context.tag_id() == HtmlTagId::Image {
            writer.write_empty_element_tag(context.tag_name()).unwrap();
            context.delete_end_tag = true;
            for attribute in context.attributes() {
                if attribute.name().eq_ignore_ascii_case("src") {
                    let name = attribute.value().unwrap().rsplit('/').next().unwrap();
                    writer.write_attribute(attribute.name(), name).unwrap();
                } else {
                    writer.write_html_attribute(attribute).unwrap();
                }
            }
        } else {
            keep(context, writer);
        }
    }
    check(
        &HtmlToHtml::new(),
        include_str!("data/html_to_html/xamarin3.html"),
        include_str!("data/html_to_html/xamarin3.xhtml"),
        replace,
    );
}

#[test]
fn truncated_tags_and_bogus_doctype_keywords() {
    // EOF-truncated tags never reach callbacks. Writing them could allow
    // later concatenated markup to complete a tag the callback never saw.
    for &(input, expected) in cases::TRUNCATED_TAGS_ARE_DROPPED
        .iter()
        .chain(cases::SHORT_BOGUS_DOC_TYPE_KEYWORD)
    {
        assert_eq!(HtmlToHtml::new().convert(input).unwrap(), expected);
        check(&HtmlToHtml::new(), input, expected, keep);
    }
}

#[test]
fn fragments_and_implicit_head_closures() {
    let converter = HtmlToHtml {
        output_html_fragment: true,
        ..HtmlToHtml::new()
    };
    for &(input, expected) in cases::OUTPUT_HTML_FRAGMENT {
        let mut tags = Vec::new();
        let actual = converter
            .convert_with_tag_callback(input, |context, writer| {
                tags.push(context.tag_name().to_owned());
                keep(context, writer);
            })
            .unwrap();
        assert_eq!(actual, expected);
        // Document wrappers and removed head content bypass the callback.
        assert!(tags.iter().all(|tag| {
            !["html", "head", "body", "title", "meta", "style"].contains(&tag.as_str())
        }));
        check(&converter, input, expected, keep);
    }
    // Browsers close head at the first start tag or text that belongs in body.
    // Only the preceding head content is removed; rewriting is idempotent.
    for &(input, expected) in cases::OUTPUT_HTML_FRAGMENT_IMPLICITLY_CLOSES_HEAD {
        check(&converter, input, expected, keep);
        assert_eq!(converter.convert(expected).unwrap(), expected);
    }
    let html = "<!DOCTYPE html><html><head><title>t</title></head><body>x</body></html>";
    check(&HtmlToHtml::new(), html, html, keep);
}

#[test]
fn depth_overflow_encodes_remaining_input() {
    let converter = HtmlToHtml {
        max_element_depth: NonZeroUsize::new(3).unwrap(),
        ..HtmlToHtml::new()
    };
    // After depth overflow, encode the remainder so no later markup can hide
    // from callbacks. Identical runs do not consume additional depth entries.
    for &(input, expected) in cases::MAX_ELEMENT_DEPTH_EXCEEDED {
        check(&converter, input, expected, keep);
    }
    let input = format!(
        "{}<img src=x onerror=alert(1)>",
        "<table><tr><td>".repeat(100_000)
    );
    let output = HtmlToHtml::new().convert(&input).unwrap();
    assert!(output.ends_with("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(!output.contains("<img"));
}

#[test]
fn suppress_word_document_head_content() {
    let output = HtmlToHtml::new()
        .convert_with_tag_callback(cases::SUPPRESS_INNER_CONTENT_INPUT, |context, writer| {
            context.invoke_callback_for_end_tag = true;
            // Discard content from unnecessary head/script/style elements.
            if matches!(
                context.tag_id(),
                HtmlTagId::Head | HtmlTagId::Script | HtmlTagId::Style
            ) {
                context.suppress_inner_content = true;
            } else if context.tag_id() == HtmlTagId::Image && !context.is_end_tag() {
                for attr in context.attributes() {
                    if attr.name().eq_ignore_ascii_case("src") {
                        writer.write_text(&format!("{} ", attr.value().unwrap_or("")));
                    }
                }
            } else if context.tag_id() == HtmlTagId::A {
                for attr in context.attributes() {
                    if attr.name().eq_ignore_ascii_case("href") {
                        writer.write_text(&format!(" [ {} ] ", attr.value().unwrap_or("")));
                    }
                }
            } else if matches!(
                context.tag_id(),
                HtmlTagId::P | HtmlTagId::Div | HtmlTagId::Br
            ) {
                // Paragraphs, divisions and breaks introduce a newline.
                writer.write_text("\r\n");
            } else {
                for attr in context.attributes() {
                    if attr.name().eq_ignore_ascii_case("src") {
                        writer.write_text(attr.value().unwrap_or(""));
                    }
                }
            }
        })
        .unwrap();
    assert_eq!(output, "Here is the body content which seems fine so far");
}

#[test]
fn noscript_modes_and_callback_visibility() {
    let input = "<p>text</p><noscript><img src=\"http://example.com/tracker.png\"></noscript>";
    for (mode, expected) in [
        (HtmlNoScriptHandling::Unwrap, "<p>text</p>"),
        (HtmlNoScriptHandling::ScriptingEnabled, input),
        (
            HtmlNoScriptHandling::ScriptingDisabled,
            "<p>text</p><noscript></noscript>",
        ),
    ] {
        let converter = HtmlToHtml {
            no_script_handling: mode,
            ..HtmlToHtml::new()
        };
        // Enabled scripting leaves raw noscript content invisible to callbacks;
        // disabled scripting exposes markup, and unwrapping omits the wrappers.
        check(&converter, input, expected, remove_image);
    }
    check(
        &HtmlToHtml::new(),
        "<p>text</p><noscript><img src=\"http://example.com/tracker.png\"><p>fallback</p></noscript>",
        "<p>text</p><p>fallback</p>",
        remove_image,
    );
    check(
        &HtmlToHtml::new(),
        "<head><noscript><link rel=\"stylesheet\" href=\"x.css\"></noscript></head><body><noscript/>a<NOSCRIPT>b</NoScript>c</body>",
        "<head><link rel=\"stylesheet\" href=\"x.css\"/></head><body>abc</body>",
        keep,
    );
    let mut tags = Vec::new();
    let output = HtmlToHtml::new()
        .convert_with_tag_callback(
            "<div><noscript><b>x</b></noscript></div>",
            |context, writer| {
                context.invoke_callback_for_end_tag = true;
                tags.push(format!(
                    "{}{}",
                    if context.is_end_tag() { "/" } else { "" },
                    context.tag_name()
                ));
                keep(context, writer);
            },
        )
        .unwrap();
    assert_eq!(output, "<div><b>x</b></div>");
    assert_eq!(tags, ["div", "b", "/b", "/div"]);
}

#[test]
fn noscript_hidden_ends_and_ignored_ends() {
    // Without noscript wrappers, embedded </noscript> in raw elements or
    // comments cannot change interpretation when renderer scripting changes.
    for &(input, expected) in cases::NO_SCRIPT_UNWRAP_XSS {
        check(&HtmlToHtml::new(), input, expected, remove_image);
    }
    fn remove(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
        if matches!(context.tag_id(), HtmlTagId::Image | HtmlTagId::NoScript) {
            context.delete_tag = true;
            context.delete_end_tag = true;
        } else {
            keep(context, writer);
        }
    }
    let converter = HtmlToHtml {
        no_script_handling: HtmlNoScriptHandling::ScriptingDisabled,
        ..HtmlToHtml::new()
    };
    for &(input, expected) in cases::NO_SCRIPT_SCRIPTING_DISABLED_REMOVE_NO_SCRIPT_TAGS {
        check(&converter, input, expected, remove);
    }
    // A special noscript element can block an end tag. After unwrapping that
    // end would close foreign content, so it must also be omitted.
    for &(input, expected) in cases::NO_SCRIPT_UNWRAP_IGNORED_END_TAG {
        check(&HtmlToHtml::new(), input, expected, keep);
        assert_eq!(HtmlToHtml::new().convert(expected).unwrap(), expected);
    }
}

#[test]
fn frameset_ignored_tags_and_starts() {
    // Frameset-ignored tags did not switch scanner state. Removing the
    // frameset must not make those tags expose previously inert content.
    for &(input, remove_frameset, expected) in cases::FRAMESET_IGNORED_TAGS {
        let run = |input| {
            HtmlToHtml::new()
                .convert_with_tag_callback(input, |context, writer| {
                    if context.tag_id() == HtmlTagId::Image
                        || (remove_frameset && context.tag_id() == HtmlTagId::FrameSet)
                    {
                        context.delete_tag = true;
                        context.delete_end_tag = true;
                    } else {
                        keep(context, writer);
                    }
                })
                .unwrap()
        };
        assert_eq!(run(input), expected);
        assert_eq!(run(expected), expected);
    }
    // Ignored starts must not become effective after stripping the head/body
    // tokens that originally cleared the frameset-ok flag.
    for &(input, fragment, expected) in cases::IGNORED_FRAMESET_START_TAG {
        let converter = HtmlToHtml {
            output_html_fragment: fragment,
            ..HtmlToHtml::new()
        };
        check(&converter, input, expected, keep);
        assert_eq!(converter.convert(expected).unwrap(), expected);
    }
}

#[test]
fn comments_decorations_and_preserved_references() {
    let converter = HtmlToHtml {
        filter_comments: true,
        ..HtmlToHtml::new()
    };
    check(
        &converter,
        "<html><head><!-- this is a comment --></head><body>Here is the body content <!-- this is another comment -->which seems fine so far</body></html>",
        "<html><head></head><body>Here is the body content which seems fine so far</body></html>",
        keep,
    );
    for (format, expected) in [
        (
            HeaderFooterFormat::Html,
            "<html><head></head><body>Here is the body content which seems fine so far</body></html>",
        ),
        (
            HeaderFooterFormat::Text,
            "&lt;html&gt;&lt;head&gt;&lt;/head&gt;<br/><body>Here is the body content which seems fine so far</body>&lt;/html&gt;<br/>",
        ),
    ] {
        let converter = HtmlToHtml {
            header: Some("<html><head></head>".into()),
            footer: Some("</html>".into()),
            header_format: format,
            footer_format: format,
            ..HtmlToHtml::new()
        };
        check(
            &converter,
            "<body>Here is the body content which seems fine so far</body>",
            expected,
            keep,
        );
    }
    let input = "<html><body>I'm on holiday until&nbsp; June 17, 2022.&#13;</body></html>";
    check(&HtmlToHtml::new(), input, input, keep);
}

#[test]
fn out_of_order_end_tags() {
    for &(input, expected) in cases::OUT_OF_ORDER_END_TAGS {
        check(&HtmlToHtml::new(), input, expected, suppress_x);
    }
}

#[test]
fn doctype_identifier_quotes() {
    // Identifiers containing a quote must retain a usable delimiter on output.
    for &(input, public, system) in cases::DOC_TYPE_IDENTIFIER_QUOTES {
        check(&HtmlToHtml::new(), input, input, keep);
        let mut reader = HtmlTokenizerReader::from_text(input);
        let Some(HtmlToken::DocType(doctype)) = reader.read_next_token().unwrap() else {
            panic!()
        };
        assert_eq!(doctype.public_identifier(), public);
        assert_eq!(doctype.system_identifier(), Some(system));
        assert!(matches!(
            reader.read_next_token().unwrap(),
            Some(HtmlToken::Tag(_))
        ));
    }
    let mut reader = HtmlTokenizerReader::from_text("<!DOCTYPE x SYSTEM 'a'>");
    let Some(HtmlToken::DocType(mut doctype)) = reader.read_next_token().unwrap() else {
        panic!()
    };
    doctype.set_system_identifier(Some("it's".into())).unwrap();
    assert_eq!(
        HtmlToken::DocType(doctype).to_html(),
        "<!DOCTYPE x SYSTEM \"it's\">"
    );
}

#[test]
fn implicit_foreign_closures_and_adoption_agency() {
    fn remove(context: &mut HtmlTagContext, writer: &mut HtmlWriter) {
        if matches!(context.tag_id(), HtmlTagId::Image | HtmlTagId::Div) {
            context.delete_tag = true;
            context.delete_end_tag = true;
        } else {
            keep(context, writer);
        }
    }
    // If a breakout tag is removed, explicitly close foreign content so later
    // style/script content is not reinterpreted as foreign markup.
    for &(input, fragment, expected) in cases::IMPLICITLY_CLOSED_FOREIGN_ELEMENTS {
        let converter = HtmlToHtml {
            output_html_fragment: fragment,
            ..HtmlToHtml::new()
        };
        check(&converter, input, expected, remove);
        check(&converter, expected, expected, remove);
    }
    for delete_formatting in [false, true] {
        let expected = if delete_formatting {
            "<p><svg></svg>"
        } else {
            "<a><p><svg></svg></a>"
        };
        let mut scripts = 0;
        let mut callback = |context: &mut HtmlTagContext, writer: &mut HtmlWriter| {
            if context.tag_id() == HtmlTagId::Script {
                scripts += usize::from(!context.is_end_tag());
                context.delete_tag = true;
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            } else if delete_formatting && context.tag_id() == HtmlTagId::A {
                context.delete_tag = true;
                context.delete_end_tag = true;
            } else {
                keep(context, writer);
            }
        };
        let converter = HtmlToHtml::new();
        assert_eq!(
            converter
                .convert_with_tag_callback(
                    "<a><p><svg></a><script><!--</script><script></script>",
                    &mut callback
                )
                .unwrap(),
            expected
        );
        assert_eq!(
            converter
                .convert_with_tag_callback(expected, &mut callback)
                .unwrap(),
            expected
        );
        assert_eq!(scripts, 2);
    }
}

#[test]
fn implicit_closures_with_deleted_ends_and_suppression() {
    let converter = HtmlToHtml {
        output_html_fragment: true,
        ..HtmlToHtml::new()
    };
    check(
        &converter,
        "<svg><g><a><body><style><img src=x onerror=alert(1)></style>",
        "<svg><g><a></a></svg><style><img src=x onerror=alert(1)></style>",
        |context, writer| {
            if context.tag_name() == "g" {
                context.delete_end_tag = true;
                context.write_tag(writer, false).unwrap();
            } else {
                keep(context, writer);
            }
        },
    );
    // Content after the implicit close is no longer suppressed by svg.
    check(
        &converter,
        "<svg><circle><body><b>x</b></svg>y",
        "<b>x</b>y",
        |context, writer| {
            if context.tag_id() == HtmlTagId::Unknown && context.tag_name() == "svg" {
                context.delete_tag = true;
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            } else {
                keep(context, writer);
            }
        },
    );
}

#[test]
fn ignored_end_tags_do_not_pop_contexts() {
    // MathML scope ignores these ends; tr eventually closes the elements.
    // Popping earlier would leave math open and reinterpret script on reparse.
    let input = "<mi><table><math><mi><ne></mi></math><tr><script><!--x--></script>";
    let expected =
        "<mi><table><math><mi><ne></mi></math></ne></mi></math><tr><script><!--x--></script>";
    let converter = HtmlToHtml {
        filter_comments: true,
        ..HtmlToHtml::new()
    };
    check(&converter, input, expected, keep);
    assert_eq!(converter.convert(expected).unwrap(), expected);
    // The td scope boundary keeps div open and its suppression in effect.
    check(
        &HtmlToHtml {
            output_html_fragment: true,
            ..HtmlToHtml::new()
        },
        "<div><table><tr><td>x</div>y</td></tr></table></div>z",
        "z",
        |context, writer| {
            if context.tag_id() == HtmlTagId::Div {
                context.delete_tag = true;
                context.delete_end_tag = true;
                context.suppress_inner_content = true;
            } else {
                keep(context, writer);
            }
        },
    );
}

#[test]
fn large_open_context_chains_do_not_require_depth_scans() {
    // Before indexed context chains these unmatched ends/reopened outer tags
    // took quadratic time (100,000 unmatched ends took over two minutes).
    let converter = HtmlToHtml::new();
    for input in [
        format!("{}x{}", "<b>".repeat(100_000), "</i>".repeat(100_000)),
        format!("<a>{}{}", "<b>".repeat(100_000), "</a><a>".repeat(100_000)),
    ] {
        assert_eq!(converter.convert(&input).unwrap(), input);
    }
    let input = format!(
        "<x>{}{}</x>z",
        "<b>y".repeat(100_000),
        "</i>".repeat(100_000)
    );
    assert_eq!(
        converter
            .convert_with_tag_callback(&input, suppress_x)
            .unwrap(),
        "<x></x>z"
    );
}

#[test]
fn callback_sees_reconsumed_tags_and_self_closing_raw_elements() {
    // These scanner reconsume cases previously hid the image from callbacks.
    for &input in cases::TAG_CALLBACK_SEES_ALL_TAGS {
        let output = HtmlToHtml::new()
            .convert_with_tag_callback(input, remove_image)
            .unwrap();
        assert!(!output.contains("onerror"), "{input}");
    }
    // Browsers ignore self-closing syntax on script/style; their raw content
    // must remain suppressed until the actual end.
    for &(input, expected) in cases::SUPPRESS_SELF_CLOSING_RAW_TEXT_ELEMENT {
        check(&HtmlToHtml::new(), input, expected, suppress_raw);
    }
}

#[test]
fn dropped_markup_cannot_complete_a_new_tag() {
    // A literal '<' ending data cannot combine with later data after removal.
    for &(input, filter_comments, expected) in cases::DROPPED_MARKUP_DOES_NOT_FORM_NEW_TAG {
        let converter = HtmlToHtml {
            filter_comments,
            ..HtmlToHtml::new()
        };
        check(&converter, input, expected, |context, writer| {
            if context.tag_id() == HtmlTagId::B {
                context.delete_tag = true;
                context.delete_end_tag = true;
            } else {
                keep(context, writer);
            }
        });
    }
}

#[test]
fn malformed_attributes_and_bogus_comments_round_trip() {
    // A name beginning '=' must not become a preceding attribute's value.
    let input = "<a href/=javascript:x>y</a>";
    let expected = "<a href=\"\" =javascript:x>y</a>";
    assert_eq!(HtmlToHtml::new().convert(input).unwrap(), expected);
    check(&HtmlToHtml::new(), input, expected, keep);
    // Preserve bogus comment delimiters so they cannot become live tags.
    for &(input, expected) in cases::BOGUS_COMMENT_ROUND_TRIP {
        check(&HtmlToHtml::new(), input, expected, keep);
        let mut reader = HtmlTokenizerReader::from_text(expected);
        let Some(HtmlToken::Comment(comment)) = reader.read_next_token().unwrap() else {
            panic!()
        };
        assert!(comment.is_bogus_comment());
        assert!(reader.read_next_token().unwrap().is_none());
    }
}

#[test]
fn original_names_end_attributes_and_callback_flag_precedence() {
    let mut events = Vec::new();
    let html = HtmlToHtml::new()
        .convert_with_tag_callback(
            "<CuStOm start=x>text</cUsToM end=y></unmatched>",
            |context, writer| {
                events.push((
                    context.tag_name().to_owned(),
                    context.is_end_tag(),
                    context
                        .attributes()
                        .iter()
                        .map(|attribute| attribute.name().to_owned())
                        .collect::<Vec<_>>(),
                ));
                context.delete_tag = true;
                context.delete_end_tag = true;
                context.invoke_callback_for_end_tag = true;
                // Deletion flags do not undo callback writes; requested HTML end
                // callbacks override end deletion, unlike generated RTF end tags.
                keep(context, writer);
            },
        )
        .unwrap();
    assert_eq!(html, "<CuStOm start=\"x\">text</cUsToM></unmatched>");
    assert_eq!(
        events,
        vec![
            ("CuStOm".into(), false, vec!["start".into()]),
            ("cUsToM".into(), true, vec!["end".into()]),
            ("unmatched".into(), true, vec![]),
        ]
    );
}

#[test]
fn buffer_drains_preserve_pending_tag_and_attribute_state() {
    let value = "x".repeat(5000);
    let html = HtmlToHtml::new()
        .convert_with_tag_callback("<p><b>", |context, writer| {
            if context.tag_id() == HtmlTagId::P {
                writer.write_start_tag("p").unwrap();
                writer.write_attribute("data", &value).unwrap();
                writer.write_attribute_name("title").unwrap();
            } else {
                writer.write_attribute_value("continued").unwrap();
                keep(context, writer);
            }
        })
        .unwrap();
    assert_eq!(html, format!("<p data=\"{value}\" title=\"continued\"><b>"));
}

struct FallibleSource<'a> {
    bytes: &'a [u8],
    remaining: usize,
    interrupted: bool,
}

impl Read for FallibleSource<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupted) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining == 0 {
            return Err(io::Error::other("read failed"));
        }
        let count = output.len().min(self.bytes.len()).min(self.remaining);
        output[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        self.remaining -= count;
        Ok(count)
    }
}

struct FallibleOutput {
    bytes: Vec<u8>,
    remaining: usize,
    interrupted: bool,
}

impl Write for FallibleOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupted) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining == 0 {
            return Err(io::Error::other("write failed"));
        }
        let count = bytes.len().min(self.remaining).min(3);
        self.bytes.extend_from_slice(&bytes[..count]);
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("converter must not flush the destination")
    }
}

#[test]
fn utf8_errors_io_failures_and_reuse() {
    let converter = HtmlToHtml {
        footer: Some("footer".into()),
        ..HtmlToHtml::new()
    };
    let input = "<p>caf\u{e9} \u{1f408}</p>";
    let expected = converter.convert(input).unwrap();
    for remaining in 0..=input.len() {
        let reader = BufReader::with_capacity(
            1,
            FallibleSource {
                bytes: input.as_bytes(),
                remaining,
                interrupted: true,
            },
        );
        let error = converter.convert_reader(reader).unwrap_err();
        assert_eq!(error.to_string(), "read failed");
    }

    for remaining in 0..expected.len() {
        let mut output = FallibleOutput {
            bytes: Vec::new(),
            remaining,
            interrupted: true,
        };
        assert_eq!(
            converter
                .convert_to(input.as_bytes(), &mut output)
                .unwrap_err()
                .to_string(),
            "write failed"
        );
        assert_eq!(output.bytes, expected.as_bytes()[..remaining]);
    }
    let mut output = FallibleOutput {
        bytes: Vec::new(),
        remaining: usize::MAX,
        interrupted: true,
    };
    let reader = BufReader::with_capacity(
        1,
        FallibleSource {
            bytes: input.as_bytes(),
            remaining: usize::MAX,
            interrupted: true,
        },
    );
    converter.convert_to(reader, &mut output).unwrap();
    assert_eq!(output.bytes, expected.as_bytes());
    for bytes in [
        &b"<p>\xff</p>"[..],
        &b"<script>\xff</script>"[..],
        &b"<p>\xf0\x9f"[..],
    ] {
        let mut output = Vec::new();
        let error = converter
            .convert_to_with_tag_callback(bytes, &mut output, suppress_raw)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.get_ref().unwrap().is::<HtmlTokenizerError>());
        assert!(!String::from_utf8_lossy(&output).contains("footer"));
    }
    assert_eq!(converter.convert(input).unwrap(), expected);
    check(&HtmlToHtml::new(), input, input, keep);
}

#[test]
fn benchmark_document_fingerprints_match_mimekit() {
    for (bytes, expected, filtered) in [
        (
            &include_bytes!("../benches/data/html/test.html")[..],
            (133097, 0xcefddf277019a6eau64),
            (118699, 0xcfd9c19c66c0fcffu64),
        ),
        (
            &include_bytes!("../benches/data/html/Gimhae_Kim_clan.utf-8.html")[..],
            (59316, 0x8d9aedbe882a48f9),
            (47776, 0xf5006ea1e0857ebc),
        ),
    ] {
        // Match File.ReadAllText's BOM removal and UTF-8 replacement decoding.
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let html = String::from_utf8_lossy(bytes);
        for filter in [false, true] {
            let converter = HtmlToHtml {
                filter_comments: filter,
                ..HtmlToHtml::new()
            };
            let output = converter
                .convert_with_tag_callback(&html, if filter { suppress_raw } else { keep })
                .unwrap();
            let hash = output.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
            assert_eq!(
                (output.len(), hash),
                if filter { filtered } else { expected }
            );
        }
    }
}

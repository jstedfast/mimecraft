// Adapted from MimeKit's UnitTests/Text/FlowedToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    encodings::{EncoderOptionsError, NewLineFormat},
    text::{FlowedToHtml, HeaderFooterFormat},
};

fn fragment(text: &str, delete_space: bool) -> String {
    let mut converter = FlowedToHtml::new();
    converter.output_html_fragment = true;
    converter.delete_space = delete_space;
    let html = converter.convert(text);
    assert_eq!(
        converter.convert_with_tag_callback(text, |context, writer| {
            context.write_tag(writer, true).unwrap();
        }),
        html
    );
    html
}

#[test]
fn defaults_and_document_or_fragment_output() {
    let mut converter = FlowedToHtml::new();
    assert_eq!(converter, FlowedToHtml::default());
    assert!(!converter.delete_space);
    assert!(!converter.output_html_fragment);
    assert_eq!(converter.header, None);
    assert_eq!(converter.footer, None);
    assert_eq!(converter.header_format, HeaderFooterFormat::Text);
    assert_eq!(converter.footer_format, HeaderFooterFormat::Text);
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    assert_eq!(converter.convert(""), "<html><body></body></html>");
    assert_eq!(
        converter.convert("body"),
        "<html><body><p>body</p>\r\n</body></html>"
    );
    converter.output_html_fragment = true;
    assert_eq!(converter.convert(""), "");
    assert_eq!(converter.convert("body"), "<p>body</p>\r\n");
}

#[test]
fn source_paragraphs_and_blank_lines() {
    let text = concat!(
        "This is some sample text that has been formatted \n",
        "according to the format=flowed rules defined in rfc3676. \n",
        "This text, once converted, should all be on a single line.\n",
        "\n\n\n\n",
        "And this line of text should be separate by 4 blank lines.\n"
    );
    let expected = concat!(
        "<p>This is some sample text that has been formatted ",
        "according to the format=flowed rules defined in rfc3676. ",
        "This text, once converted, should all be on a single line.</p>\r\n",
        "<br/>\r\n<br/>\r\n<br/>\r\n<br/>\r\n",
        "<p>And this line of text should be separate by 4 blank lines.</p>\r\n"
    );
    assert_eq!(fragment(text, false), expected);
}

#[test]
fn input_line_endings_and_explicit_output_newlines() {
    for newline in ["\r\n", "\n", "\r"] {
        assert_eq!(
            fragment(&format!("one {newline}two{newline}{newline}three"), false),
            "<p>one two</p>\r\n<br/>\r\n<p>three</p>\r\n"
        );
        assert_eq!(fragment(newline, false), "<br/>\r\n");
    }
    for (format, newline) in [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")] {
        let mut converter = FlowedToHtml::with_new_line_format(format).unwrap();
        converter.output_html_fragment = true;
        assert_eq!(converter.new_line_format(), format);
        assert_eq!(
            converter.convert("> one\r\ntwo\n\nthree\rfour "),
            format!(
                "<blockquote><p>one</p>{newline}</blockquote><p>two</p>{newline}<br/>{newline}<p>three</p>{newline}<p>four </p>{newline}"
            )
        );
    }
    assert!(matches!(
        FlowedToHtml::with_new_line_format(NewLineFormat::Mixed),
        Err(EncoderOptionsError::InvalidNewLineFormat(
            NewLineFormat::Mixed
        ))
    ));
}

#[test]
fn increasing_and_decreasing_quote_depths() {
    assert_eq!(
        fragment("> one \n> more\n>> two \n>> more\n>>> three", false),
        concat!(
            "<blockquote><p>one more</p>\r\n",
            "<blockquote><p>two more</p>\r\n",
            "<blockquote><p>three</p>\r\n",
            "</blockquote></blockquote></blockquote>"
        )
    );
    assert_eq!(
        fragment(">>> three\n>> two\n> one\nzero", false),
        concat!(
            "<blockquote><blockquote><blockquote><p>three</p>\r\n",
            "</blockquote><p>two</p>\r\n",
            "</blockquote><p>one</p>\r\n",
            "</blockquote><p>zero</p>\r\n"
        )
    );
}

#[test]
fn quote_depth_wins_over_soft_breaks() {
    // As in the brokenly quoted RFC 3676 example, a soft break must not
    // merge text at different quotation levels.
    assert_eq!(
        fragment("> alpha \n> beta \n>> next \nplain", false),
        "<blockquote><p>alpha beta </p>\r\n<blockquote><p>next </p>\r\n</blockquote></blockquote><p>plain</p>\r\n"
    );
    assert_eq!(
        fragment("> alpha \n> beta \n>> next \nplain", true),
        "<blockquote><p>alphabeta</p>\r\n<blockquote><p>next</p>\r\n</blockquote></blockquote><p>plain</p>\r\n"
    );
}

#[test]
fn contiguous_quotes_and_exactly_one_stuffing_space() {
    assert_eq!(
        fragment("> > a\n>> b\n>  c\n > literal\n>\n", false),
        concat!(
            "<blockquote><p>&gt; a</p>\r\n",
            "<blockquote><p>b</p>\r\n",
            "</blockquote><p> c</p>\r\n",
            "</blockquote><p>&gt; literal</p>\r\n",
            "<blockquote><br/>\r\n</blockquote>"
        )
    );
    assert_eq!(
        fragment(" From sender\n  two\n \n\tline\n\u{a0}line", false),
        "<p>From sender</p>\r\n<p> two</p>\r\n<br/>\r\n<p>\tline</p>\r\n<p>&#160;line</p>\r\n"
    );
}

#[test]
fn delete_space_only_removes_the_soft_break_marker() {
    assert_eq!(fragment("one  \ntwo", false), "<p>one  two</p>\r\n");
    assert_eq!(fragment("one  \ntwo", true), "<p>one two</p>\r\n");
    for delete_space in [false, true] {
        assert_eq!(
            fragment("one\t\ntwo\u{a0}\nthree\u{2003}", delete_space),
            "<p>one\t</p>\r\n<p>two&#160;</p>\r\n<p>three&#8195;</p>\r\n"
        );
    }
    // Preserve the source's signature-separator behavior, not strict RFC 3676.
    assert_eq!(fragment("-- \nsigned", false), "<p>-- signed</p>\r\n");
    assert_eq!(fragment("-- \nsigned", true), "<p>--signed</p>\r\n");
}

#[test]
fn pending_paragraphs_at_eof_and_empty_continuations() {
    for ending in ["", "\n", "\r\n"] {
        assert_eq!(
            fragment(&format!("tail {ending}"), false),
            "<p>tail </p>\r\n"
        );
        assert_eq!(fragment(&format!("tail {ending}"), true), "<p>tail</p>\r\n");
        assert_eq!(fragment(&format!("  {ending}"), true), "");
        assert_eq!(fragment(&format!("  {ending}"), false), "<p> </p>\r\n");
        assert_eq!(fragment(&format!(">  {ending}"), true), "");
    }
    // FlowedToHtml ignores empty pending text on quote changes, whereas
    // FlowedToText retains the pending quote-depth boundary.
    assert_eq!(fragment(">  \nplain", true), "<p>plain</p>\r\n");
    assert_eq!(
        fragment(">  \nplain", false),
        "<blockquote><p> </p>\r\n</blockquote><p>plain</p>\r\n"
    );
    assert_eq!(
        fragment("> fixed\n>>  ", true),
        "<blockquote><p>fixed</p>\r\n</blockquote>"
    );
    assert_eq!(fragment("keep \n\n", false), "<p>keep </p>\r\n");
    assert_eq!(fragment("keep \n\n\n", true), "<p>keep</p>\r\n<br/>\r\n");
}

#[test]
fn source_issue_1130_flushes_final_flowed_url() {
    let text = concat!(
        "We should have access, and apparently did a few months ago, but now there is \n",
        "a \"You do not currently have access to this content.\" at the bottom of the \n",
        "record\n\nThe URL in question URL:\nhttps://example.com/ "
    );
    assert_eq!(
        fragment(text, true),
        concat!(
            "<p>We should have access, and apparently did a few months ago, but now there isa ",
            "&quot;You do not currently have access to this content.&quot; at the bottom of therecord</p>\r\n",
            "<br/>\r\n<p>The URL in question URL:</p>\r\n",
            "<p><a href=\"https://example.com/\">https://example.com/</a></p>\r\n"
        )
    );
}

#[test]
fn links_are_scanned_after_unwrapping() {
    assert_eq!(
        fragment("> Visit http://exam \n> ple.com/path?_=1&x=2#f", true),
        "<blockquote><p>Visit <a href=\"http://example.com/path?_=1&amp;x=2#f\">http://example.com/path?_=1&amp;x=2#f</a></p>\r\n</blockquote>"
    );
    assert_eq!(
        fragment("user@exam \nple.xn--p1ai", true),
        "<p><a href=\"mailto:user@example.xn--p1ai\">user@example.xn--p1ai</a></p>\r\n"
    );
    assert_eq!(
        fragment("http://example.com/ \npath", false),
        "<p><a href=\"http://example.com/\">http://example.com/</a> path</p>\r\n"
    );
}

#[test]
fn markup_controls_and_unicode_are_escaped() {
    assert_eq!(
        fragment(
            "<b>'\"&amp;</b> \n\u{1f631}\0\u{7f}\u{80}\u{9f}\t\u{c}",
            false
        ),
        "<p>&lt;b&gt;&#39;&quot;&amp;amp;&lt;/b&gt; &#128561;\t\u{c}</p>\r\n"
    );
    // An input paragraph remains a paragraph even if encoding drops its text.
    assert_eq!(fragment("\0", false), "<p></p>\r\n");
}

#[test]
fn text_decorations_are_plain_and_independent_of_flowed_body() {
    let mut converter = FlowedToHtml::new();
    converter.delete_space = true;
    converter.header = Some("> header \n> next".into());
    converter.footer = Some("> > footer".into());
    assert_eq!(
        converter.convert("> body \n> next"),
        concat!(
            "<html><body><blockquote>header <br/>next<br/></blockquote>",
            "<blockquote><p>bodynext</p>\r\n</blockquote>",
            "<blockquote><blockquote>footer<br/></blockquote></blockquote></body></html>"
        )
    );
    converter.header = Some("www.example.com".into());
    converter.footer = None;
    assert_eq!(
        converter.convert(""),
        "<html><body><a href=\"http://www.example.com\">www.example.com</a><br/></body></html>"
    );
}

#[test]
fn html_decorations_and_newlines_are_preserved_verbatim() {
    let mut converter = FlowedToHtml::with_new_line_format(NewLineFormat::Unix).unwrap();
    converter.header = Some("<section>\r\n".into());
    converter.header_format = HeaderFooterFormat::Html;
    converter.footer = Some("</section>\r".into());
    converter.footer_format = HeaderFooterFormat::Html;
    assert_eq!(
        converter.convert("body"),
        "<html><body><section>\r\n<p>body</p>\n</section>\r</body></html>"
    );
    converter.header_format = HeaderFooterFormat::Text;
    assert_eq!(
        converter.convert(""),
        "<html><body>&lt;section&gt;<br/></section>\r</body></html>"
    );
    converter.header_format = HeaderFooterFormat::Html;
    converter.footer_format = HeaderFooterFormat::Text;
    assert_eq!(
        converter.convert(""),
        "<html><body><section>\r\n&lt;/section&gt;<br/></body></html>"
    );
}

#[test]
fn absent_and_empty_decorations_add_nothing() {
    for format in [HeaderFooterFormat::Text, HeaderFooterFormat::Html] {
        for header in [None, Some(String::new())] {
            for footer in [None, Some(String::new())] {
                let mut converter = FlowedToHtml::new();
                converter.output_html_fragment = true;
                converter.header = header.clone();
                converter.footer = footer;
                converter.header_format = format;
                converter.footer_format = format;
                assert_eq!(converter.convert(""), "");
                assert_eq!(converter.convert("body"), "<p>body</p>\r\n");
            }
        }
    }
}

#[test]
fn long_unicode_paragraphs_and_deep_quotes() {
    let depth = 10_000;
    let input = format!("{} text ", ">".repeat(depth));
    assert_eq!(
        fragment(&input, true),
        format!(
            "{}<p>text</p>\r\n{}",
            "<blockquote>".repeat(depth),
            "</blockquote>".repeat(depth)
        )
    );
    let text = format!("{} \n\u{1f631} ", "\u{e9}".repeat(5_000));
    assert_eq!(
        fragment(&text, true),
        format!("<p>{}&#128561;</p>\r\n", "&#233;".repeat(5_000))
    );
}

#[test]
fn repeated_conversion_and_cloning_do_not_retain_state() {
    let mut converter = FlowedToHtml::new();
    converter.output_html_fragment = true;
    for text in ["> tail ", "", "plain", "http://example.com", ">>  \n> end"] {
        assert_eq!(converter.convert(text), fragment(text, false));
    }
    converter.header = Some("header".into());
    let mut clone = converter.clone();
    assert_eq!(clone, converter);
    clone.header.as_mut().unwrap().push_str(" changed");
    clone.delete_space = true;
    assert_eq!(converter.convert("tail "), "header<br/><p>tail </p>\r\n");
    assert_eq!(clone.convert("tail "), "header changed<br/><p>tail</p>\r\n");
}

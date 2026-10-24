// Adapted from MimeKit's UnitTests/Text/RtfToHtmlTests.cs and RtfInterpreterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};
use std::num::NonZeroUsize;

use mimecraft::encodings::NewLineFormat;
use mimecraft::text::{
    HeaderFooterFormat, HtmlAttribute, HtmlAttributeId, HtmlTagId, HtmlWriter, RtfIssue, RtfToHtml,
    RtfToText, RtfViolation as V,
};

fn fragment() -> RtfToHtml {
    let mut converter = RtfToHtml::new();
    converter.output_html_fragment = true;
    converter
}

fn check(input: &[u8], expected: &str) {
    for capacity in [1, 2, 7, 4096] {
        let reader = BufReader::with_capacity(capacity, input);
        assert_eq!(
            fragment().convert(reader).unwrap(),
            expected,
            "capacity={capacity}, input={input:?}"
        );
        let reader = BufReader::with_capacity(capacity, input);
        assert_eq!(
            fragment().convert_with_issues(reader, |_| {}).unwrap(),
            expected
        );
        let reader = BufReader::with_capacity(capacity, input);
        assert_eq!(
            fragment()
                .convert_with_tag_callback(reader, |context, writer| {
                    context.write_tag(writer, true).unwrap();
                })
                .unwrap(),
            expected,
        );
    }
}

fn diagnosed(converter: &RtfToHtml, input: &[u8]) -> (String, Vec<RtfIssue>) {
    let mut issues = Vec::new();
    let html = converter
        .convert_with_issues(input, |issue| issues.push(issue))
        .unwrap();
    (html, issues)
}

#[test]
fn simple_empty_and_malformed_documents() {
    check(br"{\rtf1 Hello}", "<div>Hello</div>\r\n");
    check(b"", "");
    check(br"{\rtf1}", "");
    check(br"}}}{\rtf1 Hello", "<div>Hello</div>\r\n");
    check(
        br"{\rtf1 a{\b b",
        "<div>a<span style=\"font-weight: bold;\">b</span></div>\r\n",
    );
    let converter = RtfToHtml::new();
    assert_eq!(
        converter.convert(b"".as_slice()).unwrap(),
        "<html><body></body></html>"
    );
    assert_eq!(
        converter.convert(br"{\rtf1 Hello}".as_slice()).unwrap(),
        "<html><body><div>Hello</div>\r\n</body></html>"
    );
    assert_eq!(converter.max_group_depth.get(), 4096);
    assert_eq!(converter.max_font_table_entries.get(), 4096);
    assert_eq!(converter.max_color_table_entries.get(), 4096);
    assert_eq!(converter.max_issues_per_violation, 0);
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    assert_eq!(converter.header_format, HeaderFooterFormat::Text);
}

#[test]
fn character_formatting_colors_and_half_point_sizes() {
    let input = br"{\rtf1\ansi{\colortbl;\red255\green0\blue0;\red0\green0\blue255;}\pard a {\b b}{\i i}{\ul u}{\strike s}{\super sup}{\sub sub}{\cf1\cb2 c}{\fs36 big}{\fs25 odd}\par}";
    let expected = concat!(
        "<div>a <span style=\"font-weight: bold;\">b</span><span style=\"font-style: italic;\">i</span>",
        "<span style=\"text-decoration: underline;\">u</span><span style=\"text-decoration: line-through;\">s</span>",
        "<span style=\"vertical-align: super;\">sup</span><span style=\"vertical-align: sub;\">sub</span>",
        "<span style=\"color: #FF0000; background-color: #0000FF;\">c</span><span style=\"font-size: 18pt;\">big</span>",
        "<span style=\"font-size: 12.5pt;\">odd</span></div>\r\n",
    );
    check(input, expected);
    check(
        br"{\rtf1{\b\i\ul\strike x}}",
        "<div><span style=\"font-weight: bold; font-style: italic; text-decoration: underline line-through;\">x</span></div>\r\n",
    );
    check(
        br"{\rtf1\b a{\i b}c\plain d}",
        "<div><span style=\"font-weight: bold;\">a</span><span style=\"font-weight: bold; font-style: italic;\">b</span><span style=\"font-weight: bold;\">c</span>d</div>\r\n",
    );
}

#[test]
fn font_size_clamping_and_reset_do_not_hide_text() {
    for input in [
        br"{\rtf1\fs0 a}".as_slice(),
        br"{\rtf1\fs-10 a}",
        br"{\rtf1\fs a}",
    ] {
        check(input, "<div>a</div>\r\n");
    }
    check(
        br"{\rtf1\fs2147483647 a}",
        "<div><span style=\"font-size: 1638pt;\">a</span></div>\r\n",
    );
    check(
        br"{\rtf1\fs1 a}",
        "<div><span style=\"font-size: 0.5pt;\">a</span></div>\r\n",
    );
    check(
        br"{\rtf1\ul a\ulnone b\super c\nosupersub d}",
        "<div><span style=\"text-decoration: underline;\">a</span>b<span style=\"vertical-align: super;\">c</span>d</div>\r\n",
    );
}

#[test]
fn color_zero_and_missing_colors_render_as_auto() {
    check(
        br"{\rtf1{\colortbl\red0\green0\blue0;\red255\green0\blue0;}\cf0\cb0 a\cf1 b\cf99 c}",
        "<div>a<span style=\"color: #FF0000;\">b</span>c</div>\r\n",
    );
    check(
        br"{\rtf1{\colortbl;\red300\green-5\blue16{\*\unknown ;;;};}\cf1 x}",
        "<div><span style=\"color: #FF0010;\">x</span></div>\r\n",
    );
    check(
        br"{\rtf1{\colortbl;\red1\green2\blue3\'3b}\cf1 x}",
        "<div><span style=\"color: #010203;\">x</span></div>\r\n",
    );
    check(br"{\rtf1{\colortbl;\red255}\cf1 x}", "<div>x</div>\r\n");
    check(
        br"{\rtf1{\colortbl;\red255;}\cf1 a{\cf0 b}c}",
        "<div><span style=\"color: #FF0000;\">a</span>b<span style=\"color: #FF0000;\">c</span></div>\r\n",
    );
}

#[test]
fn paragraph_alignment_and_line_breaks() {
    check(
        br"{\rtf1\pard left\par\pard\qc center\par\pard\qr right\par\qj just\par\pard\ql left\par}",
        concat!(
            "<div>left</div>\r\n<div style=\"text-align: center;\">center</div>\r\n",
            "<div style=\"text-align: right;\">right</div>\r\n",
            "<div style=\"text-align: justify;\">just</div>\r\n<div>left</div>\r\n"
        ),
    );
    // The first content fixes alignment until the paragraph ends.
    check(
        br"{\rtf1 a\qc b\par c}",
        "<div>ab</div>\r\n<div style=\"text-align: center;\">c</div>\r\n",
    );
    check(
        br"{\rtf1\par\line\par}",
        "<div><br/></div>\r\n<div><br/></div>\r\n",
    );
}

#[test]
fn whitespace_and_markup_escaping_are_independent_of_token_boundaries() {
    check(
        b"{\\rtf1 a  b   c\\tab d<&>\"'\\par\\par x\\line y}",
        "<div>a &#160;b &#160; c&#160;&#160;&#160;&#160;d&lt;&amp;&gt;&quot;&#39;</div>\r\n<div><br/></div>\r\n<div>x<br/>y</div>\r\n",
    );
    check(
        br"{\rtf1  x  {\b  y} z}",
        "<div>&#160;x &#160;<span style=\"font-weight: bold;\"> y</span> z</div>\r\n",
    );
    check(
        br"{\rtf1\~\_\tab}",
        "<div>&#160;&#8209;&#160;&#160;&#160;&#160;</div>\r\n",
    );
    check(br"{\rtf1\u-10179?\u-8704?}", "<div>&#128512;</div>\r\n");
}

#[test]
fn unicode_code_pages_and_nested_font_metadata() {
    check(br"{\rtf1\ansi\ansicpg1252{\fonttbl{\f0 Arial;}{\f1\fcharset204 Arial Cyr;}}caf\'e9 {\f1 \'cf\'f0\'e8} \u8364?}",
        "<div>caf&#233; &#1055;&#1088;&#1080; &#8364;</div>\r\n");
    check(br"{\rtf1{\fonttbl{\f0\fswiss{\*\panose 020b0604020202020204}\fcharset204 Arial;}}\f0 \'cf}",
        "<div>&#1055;</div>\r\n");
    check(br"{\rtf1\ansicpg932 \'95\\}", "<div>&#34920;</div>\r\n");
    check(
        br"{\rtf1\ansicpg50220 \'1b$B0!\'1b(B}",
        "<div>&#20124;</div>\r\n",
    );
    check(
        br"{\rtf1 a{\upr{ansi}{\*\ud{\u233?}}}b}",
        "<div>a&#233;b</div>\r\n",
    );
}

#[test]
fn links_have_encoded_attributes_and_formatted_results() {
    check(br#"{\rtf1{\field{\*\fldinst HYPERLINK "http://example.com/?a=1&b=2"}{\fldrslt link}} {\field{\*\fldinst HYPERLINK "mailto:user@example.com"}{\fldrslt {\b mail}}}}"#,
        "<div><a href=\"http://example.com/?a=1&amp;b=2\">link</a> <a href=\"mailto:user@example.com\"><span style=\"font-weight: bold;\">mail</span></a></div>\r\n");
    check(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK \n "https://example.com/"}{\fldrslt x}}}"#,
        "<div><a href=\"https://example.com/\">x</a></div>\r\n",
    );
    check(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://e/?a=<x>&b='"}{\fldrslt <&>}}}"#,
        "<div><a href=\"https://e/?a=&lt;x&gt;&amp;b='\">&lt;&amp;&gt;</a></div>\r\n",
    );
}

#[test]
fn unsupported_fields_and_disallowed_link_schemes_keep_their_text() {
    for url in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///etc/passwd",
        " javascript:alert(1)",
        r"java\tab script:alert(1)",
    ] {
        check(
            format!("{{\\rtf1{{\\field{{\\*\\fldinst HYPERLINK \"{url}\"}}{{\\fldrslt bad}}}}}}")
                .as_bytes(),
            "<div>bad</div>\r\n",
        );
    }
    for instruction in [r#"HYPERLINK \\l "local""#, "DATE", ""] {
        check(
            format!("{{\\rtf1{{\\field{{\\*\\fldinst {instruction}}}{{\\fldrslt result}}}}}}")
                .as_bytes(),
            "<div>result</div>\r\n",
        );
    }
    for scheme in ["https", "http", "mailto", "ftp", "tel"] {
        check(
            format!(
                "{{\\rtf1{{\\field{{\\*\\fldinst HYPERLINK \"{scheme}:x\"}}{{\\fldrslt x}}}}}}"
            )
            .as_bytes(),
            &format!("<div><a href=\"{scheme}:x\">x</a></div>\r\n"),
        );
    }
}

#[test]
fn nested_fields_restore_outer_links_without_nested_anchors() {
    check(br#"{\rtf1{\field{\*\fldinst HYPERLINK "http://a/"}{\fldrslt a{\field{\*\fldinst HYPERLINK "http://b/"}{\fldrslt b}}c}}}"#,
        "<div><a href=\"http://a/\">a</a><a href=\"http://b/\">b</a><a href=\"http://a/\">c</a></div>\r\n");
    check(br#"{\rtf1{\field{\*\fldinst HYPERLINK "http://a/"}{\fldrslt a{\field{\*\fldinst DATE}{\fldrslt b}}c}}}"#,
        "<div><a href=\"http://a/\">a</a>b<a href=\"http://a/\">c</a></div>\r\n");
    // Equal target values can share rendered markup even when the saved
    // group states hold different reference identities.
    check(br#"{\rtf1{\field{\*\fldinst HYPERLINK "http://a/"}{\fldrslt a{\field{\*\fldinst HYPERLINK "http://a/"}{\fldrslt b}}c}}}"#,
        "<div><a href=\"http://a/\">abc</a></div>\r\n");
}

#[test]
fn tables_and_malformed_table_boundaries() {
    check(
        br"{\rtf1\trowd\cellx1000\cellx2000\pard\intbl a\cell b\cell\row\pard after\par}",
        "<table><tr><td><div>a</div>\r\n</td><td><div>b</div>\r\n</td></tr>\r\n</table>\r\n<div>after</div>\r\n",
    );
    check(
        br"{\rtf1 a\row b}",
        "<div>a<table><tr></tr>\r\n</table>\r\nb</div>\r\n",
    );
    check(
        br"{\rtf1\row\cell\row}",
        "<table><tr></tr>\r\n<tr><td></td></tr>\r\n</table>\r\n",
    );
    check(
        br"{\rtf1\intbl a\nestcell b\nestcell{\*\nesttableprops\nestrow}}",
        "<table><tr><td><div>a</div>\r\n</td><td><div>b</div>\r\n</td></tr>\r\n</table>\r\n",
    );
}

#[test]
fn hidden_and_non_text_destinations_do_not_change_layout() {
    check(
        br"{\rtf1 a{\v b\par c\line d\cell e\row}f}",
        "<div>af</div>\r\n",
    );
    check(
        br"{\rtf1 a{\pict\bin3 {{{}{\info secret}{\*\unknown discarded}b}",
        "<div>ab</div>\r\n",
    );
    check(
        br"{\rtf1 a{\object{\*\objdata abc}{\result b}}c}",
        "<div>abc</div>\r\n",
    );
}

#[test]
fn encapsulated_html_uses_only_the_rtf_rendering() {
    check(br"{\rtf1\ansi\fromhtml1{\*\htmltag <script>alert(1)</script><p>}A{\*\htmltag <b>}\htmlrtf{\b\htmlrtf0 B\htmlrtf}\htmlrtf0{\*\htmltag </b></p>}}",
        "<div>A<span style=\"font-weight: bold;\">B</span></div>\r\n");
    check(br"{\rtf1\fromhtml1{\*\htmltag <p>only markup</p>}}", "");
}

#[test]
fn decorations_and_explicit_newlines() {
    let mut converter = fragment();
    converter.header = Some("<p>header</p>".into());
    converter.header_format = HeaderFooterFormat::Html;
    converter.footer = Some("<footer>".into());
    assert_eq!(
        converter.convert(br"{\rtf1 Hello}".as_slice()).unwrap(),
        "<p>header</p><div>Hello</div>\r\n&lt;footer&gt;<br/>"
    );
    converter.output_html_fragment = false;
    assert_eq!(
        converter.convert(b"".as_slice()).unwrap(),
        "<html><body><p>header</p>&lt;footer&gt;<br/></body></html>"
    );
    let mut converter = RtfToHtml::with_new_line_format(NewLineFormat::Unix).unwrap();
    converter.output_html_fragment = true;
    assert_eq!(converter.new_line_format(), NewLineFormat::Unix);
    assert_eq!(
        converter.convert(br"{\rtf1 a\par b}".as_slice()).unwrap(),
        "<div>a</div>\n<div>b</div>\n"
    );
    assert!(RtfToHtml::with_new_line_format(NewLineFormat::Mixed).is_err());
}

fn image(index: usize, writer: &mut HtmlWriter) {
    writer.write_empty_element_tag("img").unwrap();
    writer
        .write_attribute("src", &format!("cid:{index}"))
        .unwrap();
}

#[test]
fn attachment_placeholders_preserve_inline_context_and_optional_spacing() {
    let cases = [
        (br"{\rtf1 A\objattph\'20 B{\b\objattph  C}}".as_slice(),
            "<div>A<img src=\"cid:0\"/> B<span style=\"font-weight: bold;\"><img src=\"cid:1\"/>C</span></div>\r\n",
            "<div>A B<span style=\"font-weight: bold;\">C</span></div>\r\n"),
        (br"{\rtf1\objattph\'20}", "<div><img src=\"cid:0\"/></div>\r\n", ""),
        (br"{\rtf1 A{\*\unknown \objattph}{\v\objattph}{\pict\objattph}{\fonttbl\objattph}{\*\htmltag\objattph}B\objattph\'20 C}",
            "<div>AB<img src=\"cid:0\"/> C</div>\r\n", "<div>AB C</div>\r\n"),
        (br"{\rtf1\objattph\'41\objattph x\objattph\par z}",
            "<div><img src=\"cid:0\"/>A<img src=\"cid:1\"/>x<img src=\"cid:2\"/></div>\r\n<div>z</div>\r\n",
            "<div>Ax</div>\r\n<div>z</div>\r\n"),
    ];
    for (input, expected, plain) in cases {
        for capacity in [1, 7, 4096] {
            let reader = BufReader::with_capacity(capacity, input);
            assert_eq!(
                fragment()
                    .convert_with_callbacks(reader, |_| {}, image)
                    .unwrap(),
                expected
            );
        }
        check(input, plain);
    }
    let output = fragment()
        .convert_with_callbacks(
            br"{\rtf1 A\objattph\'20}".as_slice(),
            |_| {},
            |_, writer| writer.write_text("<&>"),
        )
        .unwrap();
    assert_eq!(output, "<div>A&lt;&amp;&gt;</div>\r\n");
}

#[test]
fn deep_identical_groups_fold_but_distinct_states_are_limited() {
    let input = format!(
        "{{\\rtf1 a{}{{\\b deep}}{}b}}",
        "{".repeat(1_000_000),
        "}".repeat(1_000_000)
    );
    let mut converter = fragment();
    converter.max_group_depth = NonZeroUsize::new(8).unwrap();
    assert_eq!(
        converter.convert(input.as_bytes()).unwrap(),
        "<div>a<span style=\"font-weight: bold;\">deep</span>b</div>\r\n"
    );
    let input = format!(
        "{{\\rtf1 a{}deep{}b}}",
        "{\\b {\\b0 ".repeat(50),
        "}".repeat(100)
    );
    let (html, issues) = diagnosed(&converter, input.as_bytes());
    assert_eq!(html, "<div>ab</div>\r\n");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::GroupDepthLimitExceeded)
            .count(),
        1
    );
}

#[test]
fn font_and_color_floods_keep_only_configured_entries() {
    let mut converter = fragment();
    converter.max_color_table_entries = NonZeroUsize::new(10).unwrap();
    converter.max_font_table_entries = NonZeroUsize::new(10).unwrap();
    let mut input = format!(
        "{{\\rtf1{{\\colortbl;{}}}\\cf5 a\\cf50000 b{{\\fonttbl",
        "\\red255\\green0\\blue0;".repeat(100_000)
    );
    for i in 0..100_000 {
        input.push_str(&format!("{{\\f{i}\\fcharset204 Font;}}"))
    }
    input.push_str("}\\f5\\'cf\\f50000\\'cf}");
    let (html, issues) = diagnosed(&converter, input.as_bytes());
    assert_eq!(
        html,
        "<div><span style=\"color: #FF0000;\">a</span>b&#1055;&#207;</div>\r\n"
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::ColorTableLimitExceeded)
            .count(),
        1
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::FontTableLimitExceeded)
            .count(),
        1
    );
    assert!(
        !issues
            .iter()
            .any(|i| matches!(i.violation, V::UndefinedColor | V::UndefinedFont))
    );
}

#[test]
fn many_distinct_styles_and_reused_evicted_styles_render_correctly() {
    let mut input = String::from("{\\rtf1 ");
    let mut expected = String::from("<div>");
    for _ in 0..2 {
        for size in (26..626).step_by(2) {
            input.push_str(&format!("{{\\fs{size} x}}"));
            expected.push_str(&format!(
                "<span style=\"font-size: {}pt;\">x</span>",
                size / 2
            ));
        }
    }
    input.push('}');
    expected.push_str("</div>\r\n");
    check(input.as_bytes(), &expected);
}

#[test]
fn field_instruction_limit_is_exact_and_resets_for_each_field() {
    // Unterminated quoted targets intentionally use the retained prefix,
    // matching MimeKit.
    let prefix = "HYPERLINK \"https://";
    for length in [2047, 2048, 2049, 10_000] {
        let instruction = format!("{prefix}{}", "a".repeat(length - prefix.len()));
        let input = format!(
            "{{\\rtf1{{\\field{{\\*\\fldinst {instruction}}}{{\\fldrslt x}}}}{{\\field{{\\*\\fldinst HYPERLINK \"https://ok/\"}}{{\\fldrslt y}}}}}}"
        );
        let retained = instruction.len().min(2048) - prefix.len();
        let expected = format!(
            "<div><a href=\"https://{}\">x</a><a href=\"https://ok/\">y</a></div>\r\n",
            "a".repeat(retained)
        );
        for capacity in [1, 7, 4096] {
            let mut issues = Vec::new();
            let html = fragment()
                .convert_with_issues(
                    BufReader::with_capacity(capacity, input.as_bytes()),
                    |issue| issues.push(issue),
                )
                .unwrap();
            assert_eq!(html, expected);
            assert_eq!(
                issues
                    .iter()
                    .filter(|i| i.violation == V::FieldInstructionTooLong)
                    .count(),
                usize::from(length > 2048)
            );
        }
    }
}

#[test]
fn html_only_diagnostics_do_not_change_text_conversion() {
    let input = br"{\rtf1\cf99\cb-1 a}";
    let (html, issues) = diagnosed(&fragment(), input);
    assert_eq!(html, "<div>a</div>\r\n");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::UndefinedColor)
            .count(),
        2
    );
    let mut text_issues = Vec::new();
    assert_eq!(
        RtfToText::new()
            .convert_with_issues(input.as_slice(), |issue| text_issues.push(issue))
            .unwrap(),
        "a"
    );
    assert!(text_issues.is_empty());
    let mut converter = fragment();
    converter.max_issues_per_violation = 1;
    let (capped, issues) = diagnosed(&converter, input);
    assert_eq!(capped, html);
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[1].violation, V::TooManyComplianceIssues);
    assert_eq!(diagnosed(&converter, input), (capped, issues));
}

#[test]
fn utf8_field_limits_preserve_scalars_and_never_append_after_truncation() {
    let prefix = "HYPERLINK \"https://";
    for available in [1, 3] {
        let padding = "a".repeat(2048 - prefix.len() - available);
        let tail = if available == 1 { "Z\"" } else { "\"" };
        let input = format!(
            "{{\\rtf1{{\\field{{\\*\\fldinst {prefix}{padding}\\u233?{tail}}}{{\\fldrslt x}}}}}}"
        );
        let expected = format!(
            "<div><a href=\"https://{padding}{}\">x</a></div>\r\n",
            if available == 3 { "&#233;" } else { "" }
        );
        for capacity in [1, 7, 4096] {
            let mut issues = Vec::new();
            let html = fragment()
                .convert_with_issues(
                    BufReader::with_capacity(capacity, input.as_bytes()),
                    |issue| issues.push(issue),
                )
                .unwrap();
            assert_eq!(html, expected);
            assert_eq!(
                issues
                    .iter()
                    .filter(|i| i.violation == V::FieldInstructionTooLong)
                    .count(),
                usize::from(available == 1)
            );
        }
    }
}

#[test]
fn color_limit_reports_the_first_discarded_terminator_and_counts_auto_entries() {
    let input = br"{\rtf1{\colortbl;\red1;}\cf1 x}";
    let semicolon = input
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b';')
        .nth(1)
        .unwrap()
        .0;
    let mut converter = fragment();
    converter.max_color_table_entries = NonZeroUsize::new(2).unwrap();
    let (html, issues) = diagnosed(&converter, input);
    assert_eq!(
        html,
        "<div><span style=\"color: #010000;\">x</span></div>\r\n"
    );
    assert!(issues.is_empty());
    converter.max_color_table_entries = NonZeroUsize::new(1).unwrap();
    for capacity in [1, 7, 4096] {
        let mut issues = Vec::new();
        let html = converter
            .convert_with_issues(
                BufReader::with_capacity(capacity, input.as_slice()),
                |issue| issues.push(issue),
            )
            .unwrap();
        assert_eq!(html, "<div>x</div>\r\n");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, V::ColorTableLimitExceeded);
        assert_eq!(issues[0].position.offset, semicolon as u64);
    }
}

#[test]
fn diagnostics_share_independent_quotas_and_reset_between_conversions() {
    let huge = "X".repeat(3000);
    let input = format!(
        "{{\\rtf1\\cf99\\cf99\\fs0\\fs0\\u99999999999?\\u70000?{{\\field{{\\*\\fldinst {huge}}}{{\\fldrslt a}}}}{{\\field{{\\*\\fldinst {huge}}}{{\\fldrslt b}}}}}}"
    );
    let mut converter = fragment();
    let (expected, unlimited) = diagnosed(&converter, input.as_bytes());
    for violation in [
        V::UndefinedColor,
        V::InvalidFontSize,
        V::InvalidParameter,
        V::FieldInstructionTooLong,
    ] {
        assert_eq!(
            unlimited
                .iter()
                .filter(|i| i.violation == violation)
                .count(),
            2
        );
    }
    converter.max_issues_per_violation = 1;
    let (html, limited) = diagnosed(&converter, input.as_bytes());
    assert_eq!(html, expected);
    assert_eq!(
        limited
            .iter()
            .filter(|i| i.violation == V::TooManyComplianceIssues)
            .count(),
        1
    );
    for violation in [
        V::UndefinedColor,
        V::InvalidFontSize,
        V::InvalidParameter,
        V::FieldInstructionTooLong,
    ] {
        assert_eq!(
            limited.iter().filter(|i| i.violation == violation).count(),
            1
        );
    }
    assert_eq!(diagnosed(&converter, input.as_bytes()), (html, limited));
}

#[test]
fn decoder_buffer_boundaries_do_not_split_formatting_spans() {
    let input = format!("{{\\rtf1\\ansicpg932\\b a{}}}", "\\'82\\'a0".repeat(2000));
    check(
        input.as_bytes(),
        &format!(
            "<div><span style=\"font-weight: bold;\">a{}</span></div>\r\n",
            "&#12354;".repeat(2000)
        ),
    );
    let (html, issues) = diagnosed(&fragment(), br"{\rtf1\ansicpg932\'82}");
    assert_eq!(html, "<div>&#65533;</div>\r\n");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, V::InvalidByteSequence);
}

#[test]
fn placeholder_numbering_is_fresh_even_after_large_documents() {
    let input = format!("{{\\rtf1 {}}}", "\\objattph\\'20".repeat(10_000));
    let converter = fragment();
    for _ in 0..2 {
        let mut count = 0;
        let output = converter
            .convert_with_callbacks(
                input.as_bytes(),
                |_| {},
                |index, _| {
                    assert_eq!(index, count);
                    count += 1;
                },
            )
            .unwrap();
        assert_eq!(count, 10_000);
        assert_eq!(output, "<div></div>\r\n");
    }
}

#[test]
fn tag_callback_can_unwrap_character_formatting() {
    let html = fragment()
        .convert_with_tag_callback(br"{\rtf1{\b bold} plain}".as_slice(), |context, writer| {
            if context.tag_id() == HtmlTagId::Span {
                context.delete_tag = true;
                context.delete_end_tag = true;
            } else {
                context.write_tag(writer, true).unwrap();
            }
        })
        .unwrap();
    assert_eq!(html, "<div>bold plain</div>\r\n");
}

#[test]
fn tag_callback_suppression_and_replacement_end_tags_match_mimekit() {
    let input = br"{\rtf1 a{\b secret}b\par{\i x}\par c}";
    for capacity in [1, 2, 7, 4096] {
        let html = fragment()
            .convert_with_tag_callback(
                BufReader::with_capacity(capacity, input.as_slice()),
                |context, writer| {
                    if context.tag_id() == HtmlTagId::Span
                        && context
                            .attributes()
                            .get_by_id(HtmlAttributeId::Style)
                            .and_then(HtmlAttribute::value)
                            .is_some_and(|css| css.contains("bold"))
                    {
                        context.delete_tag = true;
                        context.delete_end_tag = true;
                        context.suppress_inner_content = true;
                    } else if context.tag_id() == HtmlTagId::Div {
                        if context.is_end_tag() {
                            writer.write_end_tag("p").unwrap();
                        } else {
                            writer.write_start_tag("p").unwrap();
                            context.invoke_callback_for_end_tag = true;
                        }
                    } else {
                        context.write_tag(writer, true).unwrap();
                    }
                },
            )
            .unwrap();
        assert_eq!(
            html,
            concat!(
                "<p>ab</p>\r\n",
                "<p><span style=\"font-style: italic;\">x</span></p>\r\n",
                "<p>c</p>\r\n",
            )
        );
    }
}

#[test]
fn all_generated_tags_are_exposed_and_empty_elements_never_close() {
    let input = br#"{\rtf1\intbl{\field{\*\fldinst HYPERLINK "https://e/"}{\fldrslt{\b x}}}\line\cell y\cell\row\pard z}"#;
    let mut events = Vec::new();
    let html = fragment()
        .convert_with_tag_callback(input.as_slice(), |context, writer| {
            assert_eq!(context.tag_id(), HtmlTagId::from_name(context.tag_name()));
            assert_eq!(
                context.is_empty_element(),
                context.tag_id() == HtmlTagId::Br
            );
            if context.is_end_tag() {
                assert!(context.invoke_callback_for_end_tag);
                events.push(format!("/{}", context.tag_name()));
            } else {
                events.push(context.tag_name().to_owned());
            }
            context.invoke_callback_for_end_tag = true;
            context.write_tag(writer, true).unwrap();
        })
        .unwrap();
    assert_eq!(html, fragment().convert(input.as_slice()).unwrap());
    assert_eq!(
        events,
        [
            "table", "tr", "td", "div", "a", "span", "/span", "/a", "br", "/div", "/td", "td",
            "div", "/div", "/td", "/tr", "/table", "div", "/div",
        ]
    );
}

#[test]
fn callback_can_select_attributes_and_end_context_keeps_original_attributes() {
    let input =
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/?a=1&b=2"}{\fldrslt x}}}"#;
    let mut calls = 0;
    let html = fragment()
        .convert_with_tag_callback(input.as_slice(), |context, writer| {
            if context.tag_id() != HtmlTagId::A {
                context.write_tag(writer, true).unwrap();
                return;
            }
            calls += 1;
            let attribute = context.attributes().get("href").unwrap();
            assert_eq!(attribute.value(), Some("https://example.com/?a=1&b=2"));
            if context.is_end_tag() {
                // Original attributes remain available, but writing the end tag
                // must not serialize them or reproduce callback-added attributes.
                assert!(context.invoke_callback_for_end_tag);
                context.write_tag(writer, true).unwrap();
            } else {
                context.write_tag(writer, false).unwrap();
                writer.write_html_attribute(attribute).unwrap();
                writer.write_attribute("title", "<&\"\u{e9}").unwrap();
                writer
                    .write_html_attribute(&HtmlAttribute::new("data-none", None).unwrap())
                    .unwrap();
                writer
                    .write_html_attribute(
                        &HtmlAttribute::new("data-empty", Some(String::new())).unwrap(),
                    )
                    .unwrap();
                context.invoke_callback_for_end_tag = true;
            }
        })
        .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(
        html,
        concat!(
            "<div><a href=\"https://example.com/?a=1&amp;b=2\"",
            " title=\"&lt;&amp;&quot;&#233;\" data-none data-empty=\"\">x</a></div>\r\n",
        )
    );
}

#[test]
fn tag_omission_and_end_tag_deletion_are_independent() {
    for (write_start, delete_end, expected) in [
        (false, false, "x</div>\r\n"),
        (false, true, "x"),
        (true, false, "<div>x</div>\r\n"),
        (true, true, "<div>x"),
    ] {
        let mut end_calls = 0;
        let html = fragment()
            .convert_with_tag_callback(br"{\rtf1 x}".as_slice(), |context, writer| {
                if context.is_end_tag() {
                    end_calls += 1;
                    context.write_tag(writer, true).unwrap();
                    return;
                }
                context.delete_tag = true;
                context.delete_end_tag = delete_end;
                context.invoke_callback_for_end_tag = true;
                if write_start {
                    context.write_tag(writer, true).unwrap();
                }
            })
            .unwrap();
        assert_eq!(html, expected);
        assert_eq!(end_calls, usize::from(!delete_end));
    }
}

#[test]
fn omitted_end_callback_output_still_has_the_configured_block_separator() {
    let mut converter = RtfToHtml::with_new_line_format(NewLineFormat::Unix).unwrap();
    converter.output_html_fragment = true;
    let html = converter
        .convert_with_tag_callback(br"{\rtf1 x}".as_slice(), |context, writer| {
            if !context.is_end_tag() {
                context.write_tag(writer, true).unwrap();
                context.invoke_callback_for_end_tag = true;
            }
        })
        .unwrap();
    assert_eq!(html, "<div>x\n");
}

#[test]
fn removing_a_line_break_does_not_suppress_following_content() {
    let mut breaks = 0;
    let html = fragment()
        .convert_with_tag_callback(
            br"{\rtf1 a\line b\par\par c}".as_slice(),
            |context, writer| {
                if context.tag_id() == HtmlTagId::Br {
                    breaks += 1;
                    context.delete_tag = true;
                    context.delete_end_tag = true;
                    context.invoke_callback_for_end_tag = true;
                    context.suppress_inner_content = true;
                } else {
                    context.write_tag(writer, true).unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!(breaks, 2);
    assert_eq!(html, "<div>ab</div>\r\n<div></div>\r\n<div>c</div>\r\n");
}

#[test]
fn suppression_skips_descendant_callbacks_but_not_own_end_or_diagnostics() {
    let input = br"{\rtf1\f99{\b secret\line\objattph\'20}{\i more}}";
    let mut issues = Vec::new();
    let mut events = Vec::new();
    let html = fragment()
        .convert_with_all_callbacks(
            input.as_slice(),
            |issue| issues.push(issue),
            |context, writer| {
                assert_eq!(context.tag_id(), HtmlTagId::Div);
                events.push(context.is_end_tag());
                if context.is_end_tag() {
                    assert!(context.suppress_inner_content);
                } else {
                    context.suppress_inner_content = true;
                    context.invoke_callback_for_end_tag = true;
                }
                context.write_tag(writer, true).unwrap();
            },
            |_, _| panic!("suppressed placeholder"),
        )
        .unwrap();
    assert_eq!(html, "<div></div>\r\n");
    assert_eq!(events, [false, true]);
    assert_eq!(issues, diagnosed(&fragment(), input).1);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, V::UndefinedFont);
}

#[test]
fn placeholder_indices_include_suppressed_content_and_markup_bypasses_tags() {
    let input = br"{\rtf1 A\objattph\'20{\b\objattph\'20}\objattph\'20B}";
    let converter = fragment();
    for _ in 0..2 {
        let mut tags = Vec::new();
        let mut indices = Vec::new();
        let html = converter
            .convert_with_all_callbacks(
                input.as_slice(),
                |_| {},
                |context, writer| {
                    tags.push(context.tag_id());
                    if context.tag_id() == HtmlTagId::Span {
                        context.suppress_inner_content = true;
                    }
                    context.write_tag(writer, true).unwrap();
                },
                |index, writer| {
                    indices.push(index);
                    writer.write_empty_element_tag("img").unwrap();
                    writer
                        .write_attribute("src", &format!("cid:{index}"))
                        .unwrap();
                },
            )
            .unwrap();
        assert_eq!(indices, [0, 2]);
        assert_eq!(tags, [HtmlTagId::Div, HtmlTagId::Span]);
        assert_eq!(
            html,
            concat!(
                "<div>A<img src=\"cid:0\"/><span style=\"font-weight: bold;\"></span>",
                "<img src=\"cid:2\"/>B</div>\r\n",
            )
        );
    }
}

#[test]
fn document_wrappers_and_decorations_bypass_tag_callbacks() {
    let mut converter = RtfToHtml::new();
    converter.header = Some("> header".into());
    converter.footer = Some("<footer>raw</footer>".into());
    converter.footer_format = HeaderFooterFormat::Html;
    let mut calls = 0;
    let html = converter
        .convert_with_tag_callback(br"{\rtf1{\b secret}\line text}".as_slice(), |context, _| {
            calls += 1;
            assert_eq!(context.tag_id(), HtmlTagId::Div);
            context.delete_end_tag = true;
            context.suppress_inner_content = true;
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        html,
        "<html><body><blockquote>header<br/></blockquote><footer>raw</footer></body></html>"
    );
}

#[test]
fn table_suppression_ends_before_following_ordinary_paragraph() {
    let input = br"{\rtf1\intbl a\cell{\b b}\cell\row\pard visible}";
    let mut tags = Vec::new();
    let html = fragment()
        .convert_with_tag_callback(input.as_slice(), |context, writer| {
            tags.push(context.tag_id());
            if context.tag_id() == HtmlTagId::Table {
                context.suppress_inner_content = true;
                context.delete_end_tag = true;
            } else {
                context.write_tag(writer, true).unwrap();
            }
        })
        .unwrap();
    assert_eq!(tags, [HtmlTagId::Table, HtmlTagId::Div]);
    assert_eq!(html, "<div>visible</div>\r\n");
}

#[test]
fn no_tag_callbacks_for_empty_or_hidden_input() {
    for input in [b"".as_slice(), br"{\rtf1}", br"{\rtf1{\v hidden\line}}"] {
        assert_eq!(
            fragment()
                .convert_with_tag_callback(input, |_, _| {
                    panic!("no generated body tags");
                })
                .unwrap(),
            ""
        );
    }
}

struct FailingReader<'a> {
    input: &'a [u8],
    remaining: usize,
}

impl Read for FailingReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let buffer = self.fill_buf()?;
        let count = output.len().min(buffer.len());
        output[..count].copy_from_slice(&buffer[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for FailingReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        Ok(&self.input[..self.input.len().min(self.remaining)])
    }
    fn consume(&mut self, count: usize) {
        self.input = &self.input[count..];
        self.remaining -= count;
    }
}

#[test]
fn io_failures_never_return_partial_html_and_callback_effects_are_not_undone() {
    let input = br"{\rtf1\f99\objattph\'20 {\b x}}";
    for remaining in 0..=input.len() {
        let reader = FailingReader { input, remaining };
        assert_eq!(
            fragment().convert(reader).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    let mut issues = Vec::new();
    let mut placeholders = Vec::new();
    let reader = FailingReader {
        input,
        remaining: input.len(),
    };
    assert!(
        fragment()
            .convert_with_callbacks(
                reader,
                |issue| issues.push(issue),
                |index, _| placeholders.push(index)
            )
            .is_err()
    );
    assert_eq!(placeholders, [0]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, V::UndefinedFont);
}

#[test]
fn io_failure_does_not_finalize_open_callback_contexts() {
    let input = br"{\rtf1 x\par y}";
    let mut events = Vec::new();
    let error = fragment()
        .convert_with_tag_callback(
            FailingReader {
                input,
                remaining: input.len(),
            },
            |context, writer| {
                events.push((context.tag_id(), context.is_end_tag()));
                context.invoke_callback_for_end_tag = true;
                context.write_tag(writer, true).unwrap();
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(
        events,
        [
            (HtmlTagId::Div, false),
            (HtmlTagId::Div, true),
            (HtmlTagId::Div, false),
        ]
    );
}

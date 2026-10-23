// Adapted from MimeKit's UnitTests/Text/TextToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{FlowedToHtml, HeaderFooterFormat, TextToHtml, html_encode};

fn fragment(text: &str) -> String {
    TextToHtml {
        output_html_fragment: true,
        ..TextToHtml::new()
    }
    .convert(text)
}

#[test]
fn defaults_and_document_or_fragment_output() {
    let mut converter = TextToHtml::new();
    assert_eq!(converter, TextToHtml::default());
    assert_eq!(converter.header, None);
    assert_eq!(converter.footer, None);
    assert_eq!(converter.header_format, HeaderFooterFormat::Text);
    assert_eq!(converter.footer_format, HeaderFooterFormat::Text);
    assert_eq!(HeaderFooterFormat::default(), HeaderFooterFormat::Text);
    assert!(!converter.output_html_fragment);
    assert_eq!(converter.convert(""), "<html><body></body></html>");
    assert_eq!(
        converter.convert("This is the html body"),
        "<html><body>This is the html body<br/></body></html>",
    );
    converter.output_html_fragment = true;
    assert_eq!(converter.convert(""), "");
    assert_eq!(
        converter.convert("This is the html body"),
        "This is the html body<br/>"
    );
}

#[test]
fn every_logical_line_gets_one_break() {
    for newline in ["\r\n", "\n", "\r"] {
        let text = format!("one{newline}two{newline}three");
        assert_eq!(fragment(&text), "one<br/>two<br/>three<br/>");
        assert_eq!(
            fragment(&format!("{text}{newline}")),
            "one<br/>two<br/>three<br/>"
        );
        assert_eq!(fragment(newline), "<br/>");
    }
    assert_eq!(fragment("\r\n\n\r"), "<br/><br/><br/>");
    assert_eq!(
        fragment("one\r\ntwo\nthree\rfour"),
        "one<br/>two<br/>three<br/>four<br/>"
    );
}

#[test]
fn text_escaping_controls_whitespace_and_emoji() {
    assert_eq!(fragment("\u{1f631}"), "&#128561;<br/>");
    for text in [
        "<script>alert('\"&')</script>",
        "&amp; &#65;",
        "a  \t b\u{c}c",
        "\0\u{b}\u{7f}\u{80}\u{9f}",
        "caf\u{e9}\u{a0}\u{2028}\u{feff}",
    ] {
        assert_eq!(fragment(text), format!("{}<br/>", html_encode(text)));
    }
}

#[test]
fn increasing_decreasing_and_spaced_quote_depths() {
    assert_eq!(
        fragment("> one\n>> two\n>>> three"),
        "<blockquote>one<br/><blockquote>two<br/><blockquote>three<br/></blockquote></blockquote></blockquote>",
    );
    assert_eq!(
        fragment(">>> one\n>> two\n> three\nzero"),
        "<blockquote><blockquote><blockquote>one<br/></blockquote>two<br/></blockquote>three<br/></blockquote>zero<br/>",
    );
    assert_eq!(
        fragment("> > a\n>> b\n>  c\n > literal\n>\n"),
        "<blockquote><blockquote>a<br/>b<br/></blockquote> c<br/></blockquote> &gt; literal<br/><blockquote><br/></blockquote>",
    );
    // The source closes outstanding blockquotes even without a final newline.
    assert_eq!(fragment("> one"), fragment("> one\r\n"));
}

#[test]
fn deep_quotes_are_closed_iteratively() {
    let depth = 10_000;
    let input = format!("{} text", ">".repeat(depth));
    let expected = format!(
        "{}text<br/>{}",
        "<blockquote>".repeat(depth),
        "</blockquote>".repeat(depth)
    );
    assert_eq!(fragment(&input), expected);
}

#[test]
fn source_links_and_invalid_candidates() {
    assert_eq!(
        fragment("Check out http://www.xamarin.com - it's amazing!"),
        "Check out <a href=\"http://www.xamarin.com\">http://www.xamarin.com</a> - it&#39;s amazing!<br/>",
    );
    assert_eq!(
        fragment("Email me @ home or visit http://www.example.com or me@example.com"),
        "Email me @ home or visit <a href=\"http://www.example.com\">http://www.example.com</a> or <a href=\"mailto:me@example.com\">me@example.com</a><br/>",
    );
    assert_eq!(
        fragment("http://example.com/\u{202e}gpj.exe"),
        "<a href=\"http://example.com/\">http://example.com/</a>&#8238;gpj.exe<br/>",
    );
}

#[test]
fn implicit_schemes_and_attribute_escaping() {
    assert_eq!(
        fragment("www.example.com ftp.example.org"),
        "<a href=\"http://www.example.com\">www.example.com</a> <a href=\"ftp://ftp.example.org\">ftp.example.org</a><br/>",
    );
    assert_eq!(
        fragment("\"quoted local part\"@example.com"),
        "<a href=\"mailto:&quot;quoted local part&quot;@example.com\">&quot;quoted local part&quot;@example.com</a><br/>",
    );
    assert_eq!(
        fragment("http://example.com/?a=1&b=\"x'\"c"),
        "<a href=\"http://example.com/?a=1&amp;b=&quot;x'&quot;c\">http://example.com/?a=1&amp;b=&quot;x&#39;&quot;c</a><br/>",
    );
    assert_eq!(
        fragment("jos\u{e9}@b\u{fc}cher.example"),
        "<a href=\"mailto:jos&#233;@b&#252;cher.example\">jos&#233;@b&#252;cher.example</a><br/>",
    );
}

#[test]
fn unsupported_schemes_and_input_markup_are_not_passed_through() {
    assert_eq!(
        fragment("javascript:alert(1) data:text/html,<b>x</b>"),
        "javascript:alert(1) data:text/html,&lt;b&gt;x&lt;/b&gt;<br/>",
    );
    assert_eq!(
        fragment("<a href=\"http://example.com\">x</a>"),
        "&lt;a href=&quot;<a href=\"http://example.com\">http://example.com</a>&quot;&gt;x&lt;/a&gt;<br/>",
    );
}

#[test]
fn text_headers_and_footers_are_independent_fragments() {
    let converter = TextToHtml {
        header: Some("This is the header".into()),
        footer: Some("This is the footer".into()),
        ..TextToHtml::new()
    };
    assert_eq!(
        converter.convert("This is the html body"),
        "<html><body>This is the header<br/>This is the html body<br/>This is the footer<br/></body></html>",
    );
    let converter = TextToHtml {
        output_html_fragment: true,
        header: Some("> & www.example.com".into()),
        footer: Some("> footer".into()),
        ..TextToHtml::new()
    };
    assert_eq!(
        converter.convert("> body"),
        "<blockquote>&amp; <a href=\"http://www.example.com\">www.example.com</a><br/></blockquote><blockquote>body<br/></blockquote><blockquote>footer<br/></blockquote>",
    );
}

#[test]
fn html_decorations_are_verbatim_and_formats_are_independent() {
    let mut converter = TextToHtml {
        header: Some("<div>\n".into()),
        header_format: HeaderFooterFormat::Html,
        footer: Some("</div>\r".into()),
        footer_format: HeaderFooterFormat::Html,
        ..TextToHtml::new()
    };
    assert_eq!(
        converter.convert("body"),
        "<html><body><div>\nbody<br/></div>\r</body></html>",
    );
    converter.header_format = HeaderFooterFormat::Text;
    assert_eq!(
        converter.convert(""),
        "<html><body>&lt;div&gt;<br/></div>\r</body></html>",
    );
    converter.header_format = HeaderFooterFormat::Html;
    converter.footer_format = HeaderFooterFormat::Text;
    assert_eq!(
        converter.convert(""),
        "<html><body><div>\n&lt;/div&gt;<br/></body></html>",
    );
}

#[test]
fn absent_and_empty_decorations_add_nothing() {
    for format in [HeaderFooterFormat::Text, HeaderFooterFormat::Html] {
        for header in [None, Some(String::new())] {
            for footer in [None, Some(String::new())] {
                let converter = TextToHtml {
                    output_html_fragment: true,
                    header: header.clone(),
                    header_format: format,
                    footer,
                    footer_format: format,
                };
                assert_eq!(converter.convert(""), "");
                assert_eq!(converter.convert("body"), "body<br/>");
            }
        }
    }
}

#[test]
fn repeated_conversion_and_cloning_do_not_retain_quote_or_scan_state() {
    let converter = TextToHtml {
        output_html_fragment: true,
        ..TextToHtml::new()
    };
    let mut clone = converter.clone();
    for text in ["> quoted", "www.example.com", "", "unquoted", "> quoted"] {
        assert_eq!(converter.convert(text), fragment(text));
        assert_eq!(clone, converter);
    }
    clone.header = Some("header".into());
    assert_eq!(clone.convert("body"), "header<br/>body<br/>");
    assert_eq!(converter.convert("body"), "body<br/>");
}

#[test]
fn upstream_url_fixes_render_complete_escaped_links() {
    let mut flowed = FlowedToHtml::new();
    flowed.output_html_fragment = true;
    for (text, expected) in [
        (
            "See (http://example.com/Foo_(bar)), [file:///a[0]] and (mailto:a@example.com?subject=(hi)).",
            concat!(
                "See (<a href=\"http://example.com/Foo_(bar)\">http://example.com/Foo_(bar)</a>), ",
                "[<a href=\"file:///a[0]\">file:///a[0]</a>] and ",
                "(<a href=\"mailto:a@example.com?subject=(hi)\">mailto:a@example.com?subject=(hi)</a>)."
            ),
        ),
        (
            "See <http://example.com?_=1&%78=\"x\"#section> & done.",
            concat!(
                "See &lt;<a href=\"http://example.com?_=1&amp;%78=&quot;x&quot;#section\">",
                "http://example.com?_=1&amp;%78=&quot;x&quot;#section</a>&gt; &amp; done."
            ),
        ),
        (
            "http://demo:ex&'ample@[::1]:080/path?_=1&-=2#f",
            concat!(
                "<a href=\"http://demo:ex&amp;'ample@[::1]:080/path?_=1&amp;-=2#f\">",
                "http://demo:ex&amp;&#39;ample@[::1]:080/path?_=1&amp;-=2#f</a>"
            ),
        ),
        (
            "Bad http://[1:2:3:4:5:6:7:] then www.example.com#f and a@[1.2.3.4]",
            concat!(
                "Bad http://[1:2:3:4:5:6:7:] then <a href=\"http://www.example.com#f\">www.example.com#f</a> ",
                "and <a href=\"mailto:a@[1.2.3.4]\">a@[1.2.3.4]</a>"
            ),
        ),
        (
            "user@example.xn--p1ai and a@[IPv6:::] and mailto:a@[1.2.3.4]",
            concat!(
                "<a href=\"mailto:user@example.xn--p1ai\">user@example.xn--p1ai</a> and ",
                "<a href=\"mailto:a@[IPv6:::]\">a@[IPv6:::]</a> and ",
                "<a href=\"mailto:a@[1.2.3.4]\">mailto:a@[1.2.3.4]</a>"
            ),
        ),
        (
            "http://example.com?_=1\u{202e}hidden",
            "<a href=\"http://example.com?_=1\">http://example.com?_=1</a>&#8238;hidden",
        ),
    ] {
        assert_eq!(fragment(&format!("{text}\n")), format!("{expected}<br/>"));
        assert_eq!(
            fragment(&format!("> {text}\nnext\n")),
            format!("<blockquote>{expected}<br/></blockquote>next<br/>")
        );
        assert_eq!(
            flowed.convert(&format!("{text}\n")),
            format!("<p>{expected}</p>\r\n")
        );
        assert_eq!(
            flowed.convert(&format!("> {text}\nnext\n")),
            format!("<blockquote><p>{expected}</p>\r\n</blockquote><p>next</p>\r\n")
        );
    }
}

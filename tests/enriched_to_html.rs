// Adapted from MimeKit's UnitTests/Text/EnrichedToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::text::{EnrichedToHtml, EnrichedToHtmlError as Error, HeaderFooterFormat};
use std::num::NonZeroUsize;

const PREFIX: &str = "<div style=\"white-space: pre-wrap;\">";

fn fragment() -> EnrichedToHtml {
    EnrichedToHtml {
        output_html_fragment: true,
        ..Default::default()
    }
}

fn check(input: &str, expected: &str) {
    assert_eq!(
        fragment().convert(input).unwrap(),
        format!("{PREFIX}{expected}</div>"),
        "{input:?}"
    );
}

#[test]
fn upstream_formatting() {
    for (input, expected) in [
        ("", ""),
        ("plain", "<div>plain</div>"),
        ("a  b\tc", "<div>a  b\tc</div>"),
        ("a & b << c > d", "<div>a &amp; b &lt; c &gt; d</div>"),
        ("&lt;b&gt;", "<div>&amp;lt;b&amp;gt;</div>"),
        (
            "<<script>alert(1)<</script>",
            "<div>&lt;script&gt;alert(1)&lt;/script&gt;</div>",
        ),
        (
            "a<bold>b</bold>c",
            "<div>a<span style=\"font-weight: bold;\">b</span>c</div>",
        ),
        (
            "<BoLd><ITALIC>x</italic></bOlD>",
            "<div><span style=\"font-weight: bold; font-style: italic;\">x</span></div>",
        ),
        (
            "<underline>x</underline>",
            "<div><span style=\"text-decoration: underline;\">x</span></div>",
        ),
        (
            "<fixed>x</fixed>",
            "<div><span style=\"font-family: monospace;\">x</span></div>",
        ),
        (
            "<bold>a<bold>b</bold>c</bold>",
            "<div><span style=\"font-weight: bold;\">a</span><span style=\"font-weight: bold;\">b</span><span style=\"font-weight: bold;\">c</span></div>",
        ),
        (
            "<bigger>x</bigger>",
            "<div><span style=\"font-size: 1.167em;\">x</span></div>",
        ),
        (
            "<smaller>x</smaller>",
            "<div><span style=\"font-size: 0.833em;\">x</span></div>",
        ),
        ("<bigger><smaller>x</smaller></bigger>", "<div>x</div>"),
        (
            "<bigger><bigger>x</bigger></bigger>",
            "<div><span style=\"font-size: 1.333em;\">x</span></div>",
        ),
        ("a\nb", "<div>a b</div>"),
        ("a\r\nb", "<div>a b</div>"),
        ("a\rb", "<div>a b</div>"),
        ("a\n\nb", "<div>a</div><div>b</div>"),
        ("a\r\n\r\nb", "<div>a</div><div>b</div>"),
        ("a\n\n\nb", "<div>a</div><div><br/></div><div>b</div>"),
        ("a\r\r\nb", "<div>a</div><div>b</div>"),
        ("\n\nx", "<div><br/></div><div>x</div>"),
        ("x\n", "<div>x </div>"),
        ("x\n\n", "<div>x</div><div><br/></div>"),
        ("\n", "<div> </div>"),
        ("a\n \nb", "<div>a   b</div>"),
        (
            "<nofill>a\nb</nofill>",
            "<div style=\"white-space: pre;\">a</div><div style=\"white-space: pre;\">b</div>",
        ),
        (
            "<nofill>a\r\n\r\nb</nofill>",
            "<div style=\"white-space: pre;\">a</div><div style=\"white-space: pre;\"><br/></div><div style=\"white-space: pre;\">b</div>",
        ),
        (
            "a<nofill>b</nofill>c",
            "<div>a</div><div style=\"white-space: pre;\">b</div><div>c</div>",
        ),
        (
            "a<center>b</center>c",
            "<div>a</div><div style=\"text-align: center;\">b</div><div>c</div>",
        ),
        (
            "<center>a</center>\n\nb",
            "<div style=\"text-align: center;\">a</div><div>b</div>",
        ),
        (
            "a<center>\n\nb</center>c",
            "<div>a</div><div style=\"text-align: center;\">b</div><div>c</div>",
        ),
        (
            "<center>a</center>\n\n\nb",
            "<div style=\"text-align: center;\">a</div><div><br/></div><div>b</div>",
        ),
        (
            "<center>a</center><bold></bold>\n\nb",
            "<div style=\"text-align: center;\">a</div><div>b</div>",
        ),
        (
            "<center>\n\nx</center>",
            "<div style=\"text-align: center;\"><br/></div><div style=\"text-align: center;\">x</div>",
        ),
        (
            "<nofill>a\n</nofill>",
            "<div style=\"white-space: pre;\">a</div><div style=\"white-space: pre;\"><br/></div>",
        ),
        (
            "<nofill>a\n\n</nofill>",
            "<div style=\"white-space: pre;\">a</div><div style=\"white-space: pre;\"><br/></div><div style=\"white-space: pre;\"><br/></div>",
        ),
        (
            "<flushboth><nofill>x</nofill></flushboth>",
            "<div style=\"white-space: pre; text-align: left;\">x</div>",
        ),
        (
            "<flushleft>x</flushleft>",
            "<div style=\"text-align: left;\">x</div>",
        ),
        (
            "<flushright>x</flushright>",
            "<div style=\"text-align: right;\">x</div>",
        ),
        (
            "<flushboth>x</flushboth>",
            "<div style=\"text-align: justify;\">x</div>",
        ),
        (
            "<center>a<flushright>b</flushright>c</center>",
            "<div style=\"text-align: center;\">a</div><div style=\"text-align: right;\">b</div><div style=\"text-align: center;\">c</div>",
        ),
        (
            "<bold>a<center>b</center>c</bold>",
            "<div><span style=\"font-weight: bold;\">a</span></div><div style=\"text-align: center;\"><span style=\"font-weight: bold;\">b</span></div><div><span style=\"font-weight: bold;\">c</span></div>",
        ),
        (
            "<excerpt>a<excerpt>b</excerpt>c</excerpt>",
            "<div style=\"margin-left: 4ch;\">a</div><div style=\"margin-left: 8ch;\">b</div><div style=\"margin-left: 4ch;\">c</div>",
        ),
        (
            "<excerpt><param>source</param>x</excerpt>",
            "<div style=\"margin-left: 4ch;\">x</div>",
        ),
        (
            "<paraindent><param>Left, RIGHT, in</param>x\n\ny</paraindent>",
            "<div style=\"margin-left: 4ch; margin-right: 4ch; text-indent: 4ch;\">x</div><div style=\"margin-left: 4ch; margin-right: 4ch; text-indent: 4ch;\">y</div>",
        ),
        (
            "<paraindent><param>left,left,out</param>x</paraindent>",
            "<div style=\"margin-left: 12ch; text-indent: -4ch;\">x</div>",
        ),
        (
            "<paraindent><param>left</param><paraindent><param>right</param>x</paraindent></paraindent>",
            "<div style=\"margin-left: 4ch; margin-right: 4ch;\">x</div>",
        ),
        (
            "<paraindent><param>in,out</param>x</paraindent>",
            "<div>x</div>",
        ),
        (
            "<paraindent><param>left,evil</param>x</paraindent>",
            "<div>x</div>",
        ),
        (
            "<color><param> ReD </param>x</color>",
            "<div><span style=\"color: red;\">x</span></div>",
        ),
        (
            "<color><param>FFFF,8000,0000</param>x</color>",
            "<div><span style=\"color: #ff8000;\">x</span></div>",
        ),
        (
            "<color><param>blue</param>a<color><param>white</param>b</color>c</color>",
            "<div><span style=\"color: blue;\">a</span><span style=\"color: white;\">b</span><span style=\"color: blue;\">c</span></div>",
        ),
        ("<color>x</color>", "<div>x</div>"),
        (
            "<fontfamily><param>Times New Roman</param>x</fontfamily>",
            "<div><span style=\"font-family: 'Times New Roman';\">x</span></div>",
        ),
        (
            "<fixed>a<fontfamily><param>Arial</param>b</fontfamily>c</fixed>",
            "<div><span style=\"font-family: monospace;\">a</span><span style=\"font-family: 'Arial';\">b</span><span style=\"font-family: monospace;\">c</span></div>",
        ),
        (
            "<lang><param>en-US</param>x</lang>",
            "<div><span lang=\"en-US\">x</span></div>",
        ),
        (
            "<lang><param>zh-Hant-TW</param>x</lang>",
            "<div><span lang=\"zh-Hant-TW\">x</span></div>",
        ),
        (
            "<lang><param>es-419</param>x</lang>",
            "<div><span lang=\"es-419\">x</span></div>",
        ),
        (
            "<color><param>ffff,ffff,ffff</param>x</color>",
            "<div><span style=\"color: #ffffff;\">x</span></div>",
        ),
        (
            "<color><param>0000,0000,0000</param>x</color>",
            "<div><span style=\"color: #000000;\">x</span></div>",
        ),
        (
            "<x-new><param>hidden</param>visible</x-new>",
            "<div>visible</div>",
        ),
        (
            "<indent>a</indent><indentright>b</indentright>",
            "<div>ab</div>",
        ),
        (
            "<bold><italic>x</bold>y</italic>",
            "<div><span style=\"font-weight: bold; font-style: italic;\">x</span>y</div>",
        ),
        ("a</unknown>b", "<div>ab</div>"),
        (
            "<bold>x",
            "<div><span style=\"font-weight: bold;\">x</span></div>",
        ),
        ("<param>hidden", ""),
        ("a<param>hidden</param>b", "<div>ab</div>"),
        (
            "<color><param>red<param>hidden</param>more</param>x</color>",
            "<div>x</div>",
        ),
        ("<param>hidden</bold>still hidden</param>x", "<div>x</div>"),
        (
            "<color><param><bold>red</bold></param>x</color>",
            "<div>x</div>",
        ),
        (
            "<color><param>blue</param><color><param>invalid</param>x</color></color>",
            "<div><span style=\"color: blue;\">x</span></div>",
        ),
        ("<color> <param>red</param>x</color>", "<div> x</div>"),
        ("<bold", "<div>&lt;bold</div>"),
        ("<", "<div>&lt;</div>"),
        ("</", "<div>&lt;/</div>"),
        ("<>", "<div>&lt;&gt;</div>"),
        (
            "<bold attr=x>visible",
            "<div>&lt;bold attr=x&gt;visible</div>",
        ),
    ] {
        check(input, expected);
    }
}

#[test]
fn rfc1896_example() {
    let input = concat!(
        "<bold>Now</bold> is the time for <italic>all</italic>\n",
        "good men\n<smaller>(and <<women>)</smaller> to\n<ignoreme>come</ignoreme>\n\n",
        "to the aid of their\n\n\n<color><param>red</param>beloved</color>\ncountry.\n\n",
        "By the way,\nI think that <paraindent><param>left</param><<smaller>\n\n",
        "</paraindent>should REALLY be called\n\n",
        "<paraindent><param>left</param><<tinier></paraindent>\n\n",
        "and that I am always right.\n\n-- the end"
    );
    check(
        input,
        concat!(
            "<div><span style=\"font-weight: bold;\">Now</span> is the time for ",
            "<span style=\"font-style: italic;\">all</span> good men ",
            "<span style=\"font-size: 0.833em;\">(and &lt;women&gt;)</span> to come</div>",
            "<div>to the aid of their</div><div><br/></div>",
            "<div><span style=\"color: red;\">beloved</span> country.</div>",
            "<div>By the way, I think that </div>",
            "<div style=\"margin-left: 4ch;\">&lt;smaller&gt;</div>",
            "<div>should REALLY be called</div>",
            "<div style=\"margin-left: 4ch;\">&lt;tinier&gt;</div>",
            "<div>and that I am always right.</div><div>-- the end</div>"
        ),
    );
}

#[test]
fn invalid_parameters_are_not_markup() {
    for (command, value) in [
        ("color", "red; background: url(https://example.com)"),
        ("color", "ffff,0000,000g"),
        ("color", "#ff0000"),
        ("fontfamily", "Arial';color:red;/*"),
        ("fontfamily", "Arial&quot; onclick=&quot;evil"),
        ("fontfamily", "url(https://example.com)"),
        ("fontfamily", "Arial\\27"),
        ("lang", "en\" onclick=\"evil"),
        ("lang", "-en"),
        ("lang", "en-"),
        ("lang", "en--US"),
        ("lang", "123"),
        ("lang", "toolonglanguage"),
        ("paraindent", "left; color:red"),
        ("fontfamily", ""),
        ("fontfamily", "x\u{00b2}"),
        ("fontfamily", "x\u{0301}"),
        ("fontfamily", "x\u{200b}"),
        ("color", "0000,0000,\u{00e9}aa"),
    ] {
        check(
            &format!("<{command}><param>{value}</param>x</{command}>"),
            "<div>x</div>",
        );
    }
}

#[test]
fn defaults_and_decorations() {
    let mut converter = EnrichedToHtml::new();
    assert!(!converter.output_html_fragment);
    assert_eq!(converter.max_element_depth.get(), 4096);
    assert_eq!(converter.max_parameter_length.get(), 4096);
    assert_eq!(converter.max_input_length.get(), 16 * 1024 * 1024);
    assert_eq!(converter.max_output_length.get(), 64 * 1024 * 1024);
    assert_eq!(converter.header_format, HeaderFooterFormat::Text);
    assert_eq!(converter.footer_format, HeaderFooterFormat::Text);
    assert_eq!(
        converter.convert("x").unwrap(),
        format!("<html><body>{PREFIX}<div>x</div></div></body></html>")
    );
    converter.header = Some("<header>".into());
    converter.footer = Some("<footer>".into());
    converter.output_html_fragment = true;
    assert_eq!(
        converter.convert("x").unwrap(),
        format!("&lt;header&gt;<br/>{PREFIX}<div>x</div></div>&lt;footer&gt;<br/>")
    );
    converter.header_format = HeaderFooterFormat::Html;
    converter.footer_format = HeaderFooterFormat::Html;
    assert_eq!(
        converter.convert("x").unwrap(),
        format!("<header>{PREFIX}<div>x</div></div><footer>")
    );
    converter.header = Some(String::new());
    converter.footer = None;
    assert_eq!(converter.convert("").unwrap(), format!("{PREFIX}</div>"));
}

#[test]
fn nesting_limits_and_reuse() {
    for command in ["bold", "x-unknown", "param"] {
        let converter = EnrichedToHtml {
            max_element_depth: NonZeroUsize::new(8).unwrap(),
            ..fragment()
        };
        let (start, end) = (format!("<{command}>"), format!("</{command}>"));
        let valid = format!("{}x{}", start.repeat(8), end.repeat(8));
        assert!(converter.convert(&valid).is_ok());
        assert_eq!(
            converter.convert(&format!("{start}{valid}{end}")),
            Err(Error::ElementDepth)
        );
        assert!(converter.convert("ok").unwrap().contains("<div>ok</div>"));
    }
}

#[test]
fn input_byte_limits() {
    for input in [
        "x\r\ny",
        "x\r",
        "x\r\n",
        "x\r\ry",
        "<bold>text</bold>",
        "<color><param>red</param>x</color>",
        "<param>hidden",
        "<incomplete",
        "\u{00e9}",
        "\u{1f600}",
    ] {
        let mut converter = EnrichedToHtml {
            max_input_length: NonZeroUsize::new(input.len()).unwrap(),
            ..fragment()
        };
        assert!(converter.convert(input).is_ok(), "{input:?}");
        converter.max_input_length = NonZeroUsize::new(input.len() - 1).unwrap();
        assert_eq!(converter.convert(input), Err(Error::InputLength));
        assert!(converter.convert("x").is_ok());
    }
}

#[test]
fn parameter_byte_limits() {
    for (command, value) in [
        ("fontfamily", "Arial"),
        ("color", "red"),
        ("x-unknown", "ignored"),
        ("x-unknown", "<<"),
        ("x-unknown", "\u{1f600}"),
        ("fontfamily", "\u{00e9}"),
    ] {
        let input = format!("<{command}><param>{value}</param>x</{command}>");
        let mut converter = EnrichedToHtml {
            max_parameter_length: NonZeroUsize::new(value.len()).unwrap(),
            ..fragment()
        };
        assert!(converter.convert(&input).is_ok());
        converter.max_parameter_length = NonZeroUsize::new(value.len() - 1).unwrap();
        assert_eq!(converter.convert(&input), Err(Error::ParameterLength));
    }
    let mut converter = EnrichedToHtml {
        max_parameter_length: NonZeroUsize::new(3).unwrap(),
        max_element_depth: NonZeroUsize::new(2).unwrap(),
        ..fragment()
    };
    assert!(
        converter
            .convert("<color><param>r\r\nb</param>x</color>")
            .is_ok()
    );
    assert_eq!(
        converter.convert("<color><param>r\r\ned</param>x</color>"),
        Err(Error::ParameterLength)
    );
    let nested = "<color><param><param>x</param></param>x</color>";
    assert_eq!(converter.convert(nested), Err(Error::ElementDepth));
    converter.max_element_depth = NonZeroUsize::new(3).unwrap();
    converter.max_parameter_length = NonZeroUsize::new(16).unwrap();
    assert!(converter.convert(nested).is_ok());
    converter.max_parameter_length = NonZeroUsize::new(15).unwrap();
    assert_eq!(converter.convert(nested), Err(Error::ParameterLength));
}

#[test]
fn output_byte_limits_include_all_markup() {
    for input in [
        "plain text",
        "<bold>& <<</bold>",
        "<paraindent><param>left,left</param>text</paraindent>",
        "<color><param>FFFF,0000,FFFF</param>text</color>",
        "\u{1f600}",
    ] {
        for format in [HeaderFooterFormat::Text, HeaderFooterFormat::Html] {
            let mut converter = EnrichedToHtml {
                header: Some("head\u{00e9}r".into()),
                footer: Some("foot\u{00e9}r".into()),
                header_format: format,
                footer_format: format,
                ..Default::default()
            };
            let expected = converter.convert(input).unwrap();
            converter.max_output_length = NonZeroUsize::new(expected.len()).unwrap();
            assert_eq!(converter.convert(input).unwrap(), expected);
            converter.max_output_length = NonZeroUsize::new(expected.len() - 1).unwrap();
            assert_eq!(converter.convert(input), Err(Error::OutputLength));
            assert!(converter.convert("x").is_ok());
        }
    }
    let converter = EnrichedToHtml {
        max_output_length: NonZeroUsize::new(64).unwrap(),
        ..fragment()
    };
    assert_eq!(
        converter.convert(&"&".repeat(20000)),
        Err(Error::OutputLength)
    );
    for length in 1..PREFIX.len() + 6 {
        let converter = EnrichedToHtml {
            max_output_length: NonZeroUsize::new(length).unwrap(),
            ..fragment()
        };
        assert_eq!(converter.convert(""), Err(Error::OutputLength));
    }
}

#[test]
fn command_lengths_buffers_and_unicode() {
    let command = "x".repeat(60);
    check(&format!("<{command}>text</{command}>"), "<div>text</div>");
    check(
        &format!("<{command}x>"),
        &format!("<div>&lt;{command}x&gt;</div>"),
    );
    for length in [1022, 1023, 1024, 4095, 4096, 20000] {
        let text = "a".repeat(length);
        check(
            &format!("{text}\r\n<bold>x</bold>"),
            &format!("<div>{text} <span style=\"font-weight: bold;\">x</span></div>"),
        );
        check(
            &format!("{text}\u{1f600}z"),
            &format!("<div>{text}&#128512;z</div>"),
        );
    }
    check(
        "caf\u{00e9} \u{65e5}\u{672c}\u{8a9e}",
        "<div>caf&#233; &#26085;&#26412;&#35486;</div>",
    );
    check(
        "<fontfamily><param>\u{65e5}\u{672c} A-1_.</param>x</fontfamily>",
        "<div><span style=\"font-family: '&#26085;&#26412; A-1_.';\">x</span></div>",
    );
    check(
        "<fontfamily><param>\u{10400}</param>x</fontfamily>",
        "<div><span style=\"font-family: '&#66560;';\">x</span></div>",
    );
    check("\0x\u{7f}\u{9f}", "<div>x</div>");
}

#[test]
fn many_commands_and_size_bounds() {
    check(
        &"<x-unknown>x</x-unknown></unmatched>".repeat(10000),
        &format!("<div>{}</div>", "x".repeat(10000)),
    );
    check(
        &format!("{}{}x", "<bold>".repeat(1000), "</unknown>".repeat(10000)),
        "<div><span style=\"font-weight: bold;\">x</span></div>",
    );
    for (command, size) in [("bigger", "12"), ("smaller", "0.167")] {
        check(
            &format!("{}x", format!("<{command}>").repeat(1000)),
            &format!("<div><span style=\"font-size: {size}em;\">x</span></div>"),
        );
    }
}

#[test]
fn malformed_input_cannot_panic_or_leak_parameters() {
    let converter = fragment();
    let tokens = [
        "<bold>",
        "</bold>",
        "<center>",
        "</center>",
        "<param>",
        "</param>",
        "<<",
        "<",
        "</",
        "\r",
        "\n",
        "&",
        "\u{1f600}",
        "x",
        "<color>",
        "</unknown>",
    ];
    let mut seed = 123456789u32;
    for _ in 0..1000 {
        let mut input = String::new();
        for _ in 0..64 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            input.push_str(tokens[(seed >> 16) as usize % tokens.len()]);
        }
        assert!(converter.convert(&input).is_ok(), "{input:?}");
    }
    check("<param>secret<bold>secret</bold>", "");
    check(
        "<bold>a<italic>b<bold>c</italic>d</bold>",
        "<div><span style=\"font-weight: bold;\">a</span><span style=\"font-weight: bold; font-style: italic;\">b</span><span style=\"font-weight: bold; font-style: italic;\">c</span><span style=\"font-weight: bold;\">d</span></div>",
    );
}

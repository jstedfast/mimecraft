// Adapted from MimeKit's UnitTests/Text/HtmlToTextTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, BufReader, Cursor, Read, Write},
    num::NonZeroUsize,
};

use mimecraft::{
    encodings::NewLineFormat,
    text::{HtmlToText, HtmlToTextError},
};

fn unix() -> HtmlToText {
    HtmlToText::with_new_line_format(NewLineFormat::Unix).unwrap()
}

fn assert_limit(error: io::Error, expected: HtmlToTextError) {
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error
            .get_ref()
            .and_then(|error| error.downcast_ref::<HtmlToTextError>()),
        Some(&expected),
    );
}

#[test]
fn default_properties_and_line_endings() {
    let converter = HtmlToText::new();
    assert!(converter.output_link_urls);
    assert!(converter.output_image_alt_text);
    assert_eq!(converter.max_element_depth.get(), 4096);
    assert_eq!(converter.max_input_length.get(), 16 * 1024 * 1024);
    assert_eq!(converter.max_output_length.get(), 64 * 1024 * 1024);
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    assert_eq!(unix().new_line_format(), NewLineFormat::Unix);
    assert!(HtmlToText::with_new_line_format(NewLineFormat::Mixed).is_err());
}

const FORMATTING_CASES: &[(&str, &str)] = &[
    ("", ""),
    (" \t\r\n ", ""),
    ("one \t\r\n two", "one two"),
    (
        "<p>Hello <b>bold</b> and <em>italic</em>.</p><p>Next.</p>",
        "Hello bold and italic.\n\nNext.",
    ),
    ("<div>one</div><div>two<br>three</div>", "one\ntwo\nthree"),
    ("one<br><br>two<br>", "one\n\ntwo\n"),
    ("<br>one</br>two", "\none\ntwo"),
    ("<p></p><div></div><p>one</p><p></p>", "one"),
    (
        "<h1>Title</h1><h2>Subtitle</h2><p>Body.</p><hr><p>End.</p>",
        "# Title\n\n## Subtitle\n\nBody.\n\n---\n\nEnd.",
    ),
    (
        "<h3>Three</h3><h4>Four</h4><h5>Five</h5><h6>Six</h6>",
        "### Three\n\n#### Four\n\n##### Five\n\n###### Six",
    ),
    ("<h1>Title</h2><p>Text", "# Title\n\nText"),
    (
        "<p>one<p>two<div>three</div>four",
        "one\n\ntwo\n\nthree\nfour",
    ),
    (
        "one<!-- comment -->two &amp; &lt;three&gt; &#x1f600; &nbsp; end",
        "onetwo & <three> \u{1f600} \u{a0} end",
    ),
    (
        "<html><head><title>Hidden</title><style>p { color:red }</style><script>x()</script></head><body><p>Visible</p></body></html>",
        "Visible",
    ),
    ("<head><title>Hidden</title><p>Visible", "Visible"),
    ("<title>Hidden</title>Visible", "Visible"),
    (
        "a<script/>bad <b>stuff</b></script>b<style/>bad</style>c",
        "abc",
    ),
    ("<div hidden><p>Hidden</p></div><p>Visible</p>", "Visible"),
    ("<body hidden><p>Hidden</p></body>", ""),
    ("<html hidden><body>Hidden</body></html>", ""),
    ("<p hidden>Hidden<p>Visible", "Visible"),
    ("<ul><li hidden>Hidden<li>Visible</ul>", "- Visible"),
    ("<h1 hidden>Hidden<h2>Visible", "## Visible"),
    (
        "<ul hidden><li>Hidden<li>Still hidden</ul>Visible",
        "Visible",
    ),
    (
        "<template><p>Hidden</p><template>Also hidden</template></template>Visible",
        "Visible",
    ),
    ("<noscript><p>Fallback</p></noscript>", "Fallback"),
    ("<p>x<template><div>Hidden</div></template>y</p>", "xy"),
    (
        "<a href='url'>x<template><a href='hidden'>Hidden</a></template>y</a>",
        "xy (url)",
    ),
    ("<p>x<div hidden><p>Hidden</p></div>y", "x\n\ny"),
    (
        "<svg><text>SVG &amp; text</text><![CDATA[ <raw> ]]></svg>",
        "SVG & text <raw>",
    ),
    (
        "<ul><li>one</li><li>two</li></ul><p>End</p>",
        "- one\n- two\n\nEnd",
    ),
    (
        "<ol start='3'><li>three<li value='7'>seven<li>eight</ol>",
        "3. three\n7. seven\n8. eight",
    ),
    ("<ol start='-2'><li>a<li>b</ol>", "-2. a\n-1. b"),
    ("<ol start='bad'><li value='bad'>a<li>b</ol>", "1. a\n2. b"),
    (
        "<ul><li>outer<ol><li>inner<br>continued</li><li>second</li></ol>tail</li><li>last</li></ul>",
        "- outer\n  1. inner\n     continued\n  2. second\n  tail\n- last",
    ),
    (
        "<ul><li><p>First.</p><p>Second.</p></li><li>Next.</li></ul>",
        "- First.\n\n  Second.\n- Next.",
    ),
    ("<ul><li></li><li>x</li></ul>", "-\n- x"),
    ("<ul><li><br>next</li><li>end</li></ul>", "-\n  next\n- end"),
    (
        "<blockquote>one<br>two<blockquote>inner</blockquote>tail</blockquote>end",
        "> one\n> two\n\n>> inner\n\n> tail\n\nend",
    ),
    (
        "<blockquote><ul><li>one<li>two</ul></blockquote>",
        "> - one\n> - two",
    ),
    (
        "<dl><dt>Term</dt><dd>Meaning</dd><dt>Other</dt><dd>Definition</dd></dl>",
        "Term\n  Meaning\nOther\n  Definition",
    ),
    (
        "<dl><dt>Term<dd>Meaning<dt>Other<dd>Definition</dl>",
        "Term\n  Meaning\nOther\n  Definition",
    ),
    (
        "<pre>\n  one\t two\n\nthree  </pre><p>End</p>",
        "  one\t two\n\nthree  \n\nEnd",
    ),
    ("<pre>\r\none\r\ntwo\rthree\n</pre>", "one\ntwo\nthree\n"),
    (
        "<pre>  &lt;b&gt; <b>bold</b>\t end</pre>",
        "  <b> bold\t end",
    ),
    ("<pre><!-- comment -->\nkeep</pre>", "\nkeep"),
    ("<pre><b>\nkeep</b></pre>", "\nkeep"),
    ("<pre>\n\nkeep</pre>", "\nkeep"),
    ("<pre>one<br>two</pre>", "one\ntwo"),
    ("<plaintext>\n  <b>literal</b>", "\n  <b>literal</b>"),
    (
        "<blockquote><pre>\na\n b\n</pre></blockquote>",
        "> a\n>  b\n",
    ),
    ("<textarea>\n a &amp; b\n</textarea>", " a & b\n"),
    ("<xmp><b>literal</b></xmp>", "<b>literal</b>"),
    (
        "Visit <a href='https://example.com/?a=1&amp;b=2'>Example</a>.",
        "Visit Example (https://example.com/?a=1&b=2).",
    ),
    (
        "See <a href='https://example.com'>https://example.com</a>.",
        "See https://example.com.",
    ),
    ("<a href='/relative'><b>Label</b></a>", "Label (/relative)"),
    ("<a href='url'></a>", "url"),
    ("<a href='url'>Label", "Label (url)"),
    ("<a href='url'><p>Label</p></a>", "Label (url)"),
    (
        "<a href='outer'>one<a href='inner'>two</a>",
        "one (outer)two (inner)",
    ),
    ("<a href='  '>Label</a><a>No URL</a>", "LabelNo URL"),
    ("<a href='url'> url </a>", "url"),
    (
        "<a href='url'>Label <script>Hidden</script> text</a>",
        "Label text (url)",
    ),
    (
        "An <img alt='A &amp; B'> image<img src='x'>.",
        "An A & B image.",
    ),
    ("<a href='url'><img alt='Image'></a>", "Image (url)"),
    (
        "<table><tr><th>Name</th><th>Value</th></tr><tr><td>A</td><td>B</td></tr></table>",
        "Name Value\nA B",
    ),
    ("<table><tr><td>A<td>B<tr><td>C<td>D</table>", "A B\nC D"),
    ("<div><b>one</div>two</b>", "one\ntwo"),
    ("<p>one</unknown>two</p>", "onetwo"),
    ("one<img alt='truncated", "one"),
    ("<div/>one</div>two", "one\ntwo"),
];

#[test]
fn upstream_formatting() {
    for &(html, expected) in FORMATTING_CASES {
        assert_eq!(
            HtmlToText::new().convert(html).unwrap(),
            expected.replace('\n', "\r\n"),
            "{html:?}",
        );
        for capacity in [1, 2, 7, 4096] {
            let reader = BufReader::with_capacity(capacity, html.as_bytes());
            assert_eq!(
                unix().convert_reader(reader).unwrap(),
                expected,
                "capacity={capacity}, html={html:?}",
            );
        }
    }
}

#[test]
fn upstream_document_fingerprints() {
    // Same UTF-8 replacement decoding and CRLF output as the upstream benchmark
    // setup. Fingerprints were generated by MimeKit's HtmlToText, not this port.
    for (bytes, length, expected_hash) in [
        (
            &include_bytes!("../benches/data/html/test.html")[..],
            87455,
            0x480aab3530ca9933u64,
        ),
        (
            &include_bytes!("../benches/data/html/Gimhae_Kim_clan.utf-8.html")[..],
            10359,
            0x9d26b93fe3966209,
        ),
    ] {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let html = String::from_utf8_lossy(bytes);
        for capacity in [1, 4096] {
            let input = BufReader::with_capacity(capacity, html.as_bytes());
            let text = HtmlToText::new().convert_reader(input).unwrap();
            let hash = text.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
            assert_eq!((text.len(), hash), (length, expected_hash));
        }
    }
}

#[test]
fn options_and_reuse() {
    let mut converter = HtmlToText::new();
    converter.output_link_urls = false;
    converter.output_image_alt_text = false;
    converter.header = Some("[header]".into());
    converter.footer = Some("[footer]".into());
    let html = "<p><a href='url'>Label</a><img alt='Image'></p>";
    assert_eq!(converter.convert(html).unwrap(), "[header]Label[footer]");
    converter.output_link_urls = true;
    converter.output_image_alt_text = true;
    converter.header = None;
    converter.footer = None;
    assert_eq!(converter.convert(html).unwrap(), "Label (url)Image");
    assert_eq!(converter.convert("plain").unwrap(), "plain");
}

#[test]
fn reader_writer_overloads_retain_borrowed_sources() {
    let converter = HtmlToText::new();
    let mut input = Cursor::new("<p>caf\u{e9}</p>".as_bytes());
    let mut output = Vec::new();
    converter.convert_to(&mut input, &mut output).unwrap();
    assert_eq!(output, "caf\u{e9}".as_bytes());
    assert_eq!(input.position(), input.get_ref().len() as u64);
    input.set_position(0);
    output.clear();
    converter.convert_to(&mut input, &mut output).unwrap();
    assert_eq!(output, "caf\u{e9}".as_bytes());
    output.clear();
    converter
        .convert_to("<h1>Title</h1>".as_bytes(), &mut output)
        .unwrap();
    assert_eq!(output, b"# Title");
}

#[test]
fn depth_limits_count_repeated_and_suppressed_elements() {
    for (start, end) in [
        ("<b>", "</b>"),
        ("<blockquote>", "</blockquote>"),
        ("<template>", "</template>"),
    ] {
        let mut converter = HtmlToText::new();
        converter.max_element_depth = NonZeroUsize::new(8).unwrap();
        let valid = format!("{}x{}", start.repeat(8), end.repeat(8));
        converter.convert(&valid).unwrap();
        assert_limit(
            converter
                .convert(&format!("{start}{valid}{end}"))
                .unwrap_err(),
            HtmlToTextError::ElementDepth,
        );
        assert_eq!(converter.convert("ok").unwrap(), "ok");
    }
}

#[test]
fn input_length_limits() {
    for html in [
        "plain text",
        "<script>hidden</script>",
        "<img alt='attribute'>",
        "<!-- comment -->",
    ] {
        let mut converter = HtmlToText::new();
        converter.max_input_length = NonZeroUsize::new(html.len()).unwrap();
        converter.convert(html).unwrap();
        converter.max_input_length = NonZeroUsize::new(html.len() - 1).unwrap();
        assert_limit(
            converter.convert(html).unwrap_err(),
            HtmlToTextError::InputLength,
        );
        assert_eq!(converter.convert("ok").unwrap(), "ok");
    }
}

#[test]
fn many_sequential_elements() {
    let mut converter = unix();
    converter.max_element_depth = NonZeroUsize::new(16).unwrap();
    let html = format!("<ol>{}</ol>", "<li>item".repeat(10_000));
    let expected = (1..=10_000)
        .map(|n| format!("{n}. item"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(converter.convert(&html).unwrap(), expected);
    let html = format!("<table>{}</table>", "<tr><td>a<td>b".repeat(10_000));
    assert_eq!(
        converter.convert(&html).unwrap(),
        vec!["a b"; 10_000].join("\n")
    );
}

#[test]
fn token_boundaries() {
    let converter = unix();
    let word = "a".repeat(20_000);
    for (html, expected) in [
        (
            format!("<pre>\n{word}\r\n  end</pre>"),
            format!("{word}\n  end"),
        ),
        (format!("{word} \t\r\n <b>end</b>"), format!("{word} end")),
        (format!("<a href='{word}'>{word}</a>"), word),
    ] {
        for capacity in [1, 4096] {
            assert_eq!(
                converter
                    .convert_reader(BufReader::with_capacity(capacity, html.as_bytes()))
                    .unwrap(),
                expected,
            );
        }
    }
}

#[test]
fn output_length_limits() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for html in [
            "<blockquote><ol><li>one<li>two</ol></blockquote>",
            "<p>one</p><p>two</p>",
            "<a href='destination'>Label</a>",
            "<pre>\n a\tb\n</pre>",
        ] {
            let mut converter = HtmlToText::with_new_line_format(format).unwrap();
            let expected = converter.convert(html).unwrap();
            converter.max_output_length = NonZeroUsize::new(expected.len()).unwrap();
            assert_eq!(converter.convert(html).unwrap(), expected);
            converter.max_output_length = NonZeroUsize::new(expected.len() - 1).unwrap();
            let mut output = Vec::new();
            assert_limit(
                converter
                    .convert_to(html.as_bytes(), &mut output)
                    .unwrap_err(),
                HtmlToTextError::OutputLength,
            );
            assert!(output.len() <= converter.max_output_length.get());
            assert_eq!(converter.convert("ok").unwrap(), "ok");
        }
    }
}

#[test]
fn input_limit_includes_truncated_tokens_and_stops_reading() {
    let mut converter = HtmlToText::new();
    converter.max_input_length = NonZeroUsize::new(64).unwrap();
    for prefix in ["<img alt='", "<script>", "<!--", "&", "<"] {
        let html = format!("{prefix}{}", "a".repeat(10_000));
        let mut input = BufReader::with_capacity(1, Cursor::new(html.as_bytes()));
        assert_limit(
            converter
                .convert_to(&mut input, &mut Vec::new())
                .unwrap_err(),
            HtmlToTextError::InputLength,
        );
        assert!(input.get_ref().position() <= 65, "{prefix}");
    }
}

#[test]
fn depth_limit_fails_before_end_of_input() {
    let mut converter = HtmlToText::new();
    converter.max_element_depth = NonZeroUsize::new(4).unwrap();
    let html = format!("<b><i><u><s><span>{}", "x".repeat(10_000));
    let mut input = BufReader::with_capacity(1, Cursor::new(html.as_bytes()));
    assert_limit(
        converter
            .convert_to(&mut input, &mut Vec::new())
            .unwrap_err(),
        HtmlToTextError::ElementDepth,
    );
    assert!(input.get_ref().position() < html.len() as u64);
}

#[test]
fn unmatched_end_tags() {
    let mut converter = HtmlToText::new();
    converter.max_element_depth = NonZeroUsize::new(32).unwrap();
    let html = format!("{}{}text", "<b>".repeat(16), "</unknown>".repeat(10_000));
    assert_eq!(converter.convert(&html).unwrap(), "text");
}

#[test]
fn unicode_links_duplicate_attributes_and_integer_boundaries() {
    for (html, expected) in [
        (
            "<a href='caf\u{e9}/\u{1f600}'>caf\u{e9}/\u{1f600}</a>",
            "caf\u{e9}/\u{1f600}",
        ),
        (
            "<a href='\u{1f600}'>x\u{1f600}</a>",
            "x\u{1f600} (\u{1f600})",
        ),
        ("<a href='abc'>abcdef</a>", "abcdef (abc)"),
        ("<a href='abc'>ab</a>", "ab (abc)"),
        ("<a href='a b'> a \t b </a>", "a b"),
        (
            "<a HREF='first' href='second'>x</a><img ALT='one' alt='two'>",
            "x (first)one",
        ),
        ("<DIV HIDDEN='false'>hidden</DIV>visible", "visible"),
        (
            "<ol start='2147483647'><li>a<li>b</ol>",
            "2147483647. a\n2147483648. b",
        ),
        (
            "<ol start='2147483648'><li>a<li value='-2147483648'>b<li>c</ol>",
            "1. a\n-2147483648. b\n-2147483647. c",
        ),
        (
            "<ol start=' +3 '><li>a<li value='-2147483649'>b</ol>",
            "3. a\n4. b",
        ),
        ("<ol start='\u{a0}3'><li>a</ol>", "1. a"),
        ("<template/><p>hidden</p></template>visible", "visible"),
        ("<custom/>one<div>two</div>", "one\ntwo"),
        (
            "<frameset><frame src='x'><noframes>hidden</noframes></frameset>",
            "",
        ),
        ("<html><body>one<body hidden>two</body></html>", "onetwo"),
        ("<pre>a\r<!--comment-->\nb</pre>", "a\n\nb"),
    ] {
        for capacity in [1, 7, 4096] {
            let input = BufReader::with_capacity(capacity, html.as_bytes());
            assert_eq!(unix().convert_reader(input).unwrap(), expected, "{html:?}");
        }
    }
}

#[test]
fn byte_limits_decorations_and_unicode() {
    let mut converter = unix();
    converter.header = Some("[header]\r\n".into());
    converter.footer = Some("\r[footer]".into());
    converter.max_input_length = NonZeroUsize::new(4).unwrap();
    converter.max_output_length = NonZeroUsize::new(4).unwrap();
    assert_eq!(
        converter.convert("\u{1f600}").unwrap(),
        "[header]\r\n\u{1f600}\r[footer]"
    );
    assert_eq!(converter.convert("").unwrap(), "[header]\r\n\r[footer]");
    converter.max_output_length = NonZeroUsize::new(3).unwrap();
    let mut output = Vec::new();
    assert_limit(
        converter
            .convert_to("\u{1f600}".as_bytes(), &mut output)
            .unwrap_err(),
        HtmlToTextError::OutputLength,
    );
    assert_eq!(output, b"[header]\r\n");
    converter.max_input_length = NonZeroUsize::new(3).unwrap();
    assert_limit(
        converter.convert("\u{1f600}").unwrap_err(),
        HtmlToTextError::InputLength,
    );
    converter.max_input_length = NonZeroUsize::new(32).unwrap();
    converter.header = None;
    converter.footer = None;
    assert_limit(
        converter.convert("&#x1f600;").unwrap_err(),
        HtmlToTextError::OutputLength,
    );
    assert_eq!(converter.convert("\u{e9}").unwrap(), "\u{e9}");
}

#[test]
fn invalid_utf8_is_not_replaced_or_sniffed() {
    for html in [
        &b"text\xff"[..],
        &b"<script>\xff</script>"[..],
        &b"<!--\xff-->"[..],
        &b"<img alt='\xff'>"[..],
        &b"\xf0\x9f"[..],
    ] {
        for capacity in [1, 4096] {
            let error = HtmlToText::new()
                .convert_reader(BufReader::with_capacity(capacity, html))
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{html:?}");
            assert!(
                error
                    .get_ref()
                    .unwrap()
                    .is::<mimecraft::text::HtmlTokenizerError>()
            );
        }
    }
    assert_eq!(unix().convert("\u{feff}text").unwrap(), "\u{feff}text");
}

struct FallibleReader<'a> {
    input: &'a [u8],
    remaining: usize,
    interrupt: bool,
}

impl Read for FallibleReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining == 0 {
            return Err(io::Error::other("source failure"));
        }
        let length = output.len().min(self.input.len()).min(self.remaining);
        output[..length].copy_from_slice(&self.input[..length]);
        self.input = &self.input[length..];
        self.remaining -= length;
        Ok(length)
    }
}

struct FallibleWriter {
    output: Vec<u8>,
    remaining: usize,
    interrupt: bool,
}

impl Write for FallibleWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let length = input.len().min(1);
        self.output.extend_from_slice(&input[..length]);
        self.remaining -= length;
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!("converter must not flush the borrowed writer");
    }
}

#[test]
fn io_errors_and_interrupted_short_io() {
    let mut converter = unix();
    converter.footer = Some("[footer]".into());
    let html = "<h1>caf\u{e9}</h1><p>Next</p>";
    for fail_at in 0..=html.len() {
        let mut reader = BufReader::with_capacity(
            1,
            FallibleReader {
                input: html.as_bytes(),
                remaining: fail_at,
                interrupt: true,
            },
        );
        let mut output = Vec::new();
        let error = converter.convert_to(&mut reader, &mut output).unwrap_err();
        assert_eq!(error.to_string(), "source failure");
        assert!(!output.ends_with(b"[footer]"));
    }
    let expected = converter.convert(html).unwrap();
    for fail_at in 0..expected.len() {
        let mut writer = FallibleWriter {
            output: Vec::new(),
            remaining: fail_at,
            interrupt: true,
        };
        assert_eq!(
            converter
                .convert_to(html.as_bytes(), &mut writer)
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe,
        );
        assert_eq!(writer.output, expected.as_bytes()[..fail_at]);
    }
    let reader = BufReader::with_capacity(
        1,
        FallibleReader {
            input: html.as_bytes(),
            remaining: usize::MAX,
            interrupt: true,
        },
    );
    let mut writer = FallibleWriter {
        output: Vec::new(),
        remaining: usize::MAX,
        interrupt: true,
    };
    converter.convert_to(reader, &mut writer).unwrap();
    assert_eq!(writer.output, expected.as_bytes());
    assert_eq!(converter.convert("ok").unwrap(), "ok[footer]");
}

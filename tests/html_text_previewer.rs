// Adapted from MimeKit's UnitTests/Text/HtmlTextPreviewerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor, Read};

use mimecraft::text::{HtmlTextPreviewer, HtmlTokenizerError};

fn check(limit: usize, html: &str, expected: &str) {
    let previewer = HtmlTextPreviewer::with_maximum_preview_length(limit).unwrap();
    assert_eq!(previewer.preview(html), expected, "{limit}: {html:?}");
    for capacity in [1, 2, 7, 4096] {
        let mut reader = BufReader::with_capacity(capacity, html.as_bytes());
        assert_eq!(
            previewer.preview_reader(&mut reader).unwrap(),
            expected,
            "capacity={capacity}, limit={limit}, html={html:?}",
        );
    }
}

#[test]
fn defaults_and_length_bounds() {
    assert_eq!(HtmlTextPreviewer::new(), HtmlTextPreviewer::default());
    assert_eq!(HtmlTextPreviewer::new().maximum_preview_length(), 230);
    for limit in [1, 230, 1024] {
        assert_eq!(
            HtmlTextPreviewer::with_maximum_preview_length(limit)
                .unwrap()
                .maximum_preview_length(),
            limit,
        );
    }
    for limit in [0, 1025, usize::MAX] {
        let error = HtmlTextPreviewer::with_maximum_preview_length(limit).unwrap_err();
        assert_eq!(error.requested(), limit);
        assert_eq!(
            error.to_string(),
            format!("preview length must be between 1 and 1024 characters, got {limit}"),
        );
    }
}

#[test]
fn empty_and_whitespace_only_input() {
    for limit in [1, 2, 230, 1024] {
        for html in [
            "",
            " ",
            "<body>",
            "<body></body>",
            "<body> \t\r\n",
            "<body>\u{a0}\u{200b}\u{200c}\u{200d}</body>",
            "plain text",
            "<p>fragment</p>",
            "<body/>text",
        ] {
            check(limit, html, "");
        }
    }
}

#[test]
fn upstream_document_previews() {
    for (bytes, expected110, expected230) in [
        (
            &include_bytes!("data/text/homedepot-check-inside-now.html")[..],
            "FREE DELIVERY Appliance Purchases $396 or More",
            "FREE DELIVERY Appliance Purchases $396 or More",
        ),
        (
            &include_bytes!("data/text/mimekit.net.html")[..],
            "Toggle navigation MimeKit Home About Help Documentation Donate \u{d7} Install with NuGet (recommended) NuGet PM> I\u{2026}",
            "Toggle navigation MimeKit Home About Help Documentation Donate \u{d7} Install with NuGet (recommended) NuGet PM> Install-Package MimeKit PM> Install-Package MailKit or Install via VS Package Management window. Direct Download ZIP fil\u{2026}",
        ),
        (
            &include_bytes!("data/text/planet-fitness.html")[..],
            "Don\u{2019}t miss our celebrity guest Monday evening",
            "Don\u{2019}t miss our celebrity guest Monday evening",
        ),
    ] {
        // Upstream previews only the first 16 KiB, decoded with UTF-8 replacement.
        // A byte cutoff can split a scalar; decoding is outside the previewer.
        let html = String::from_utf8_lossy(&bytes[..bytes.len().min(16 * 1024)]);
        check(110, &html, expected110);
        check(230, &html, expected230);
    }
}

#[test]
fn upstream_nested_and_misnested_elements() {
    for (html, expected) in [
        ("<body><ol><li>a<ul><li>b</ul><li>c</ol>d", "1. a b 2. cd"),
        ("<body><ol><li>a<ul><li>b</ol><li>c</ul>d", "1. a b cd"),
        ("<body><ul><li>a<ol><li>b</ul><li>c</ol>d", "a 1. b 2. cd"),
        (
            "<body><ol><li>a<ol><li>b</ol><li>c</ol>d",
            "1. a 1. b 2. cd",
        ),
        ("<body><b>a<script>b<i>c</b>d</script>e</i>f", "aef"),
        ("<body><table><td>a</table>b<tr>c</td>d</tr>e", "abe"),
        ("<body>a</body>b<body>c", "ac"),
    ] {
        check(230, html, expected);
    }
}

// Before the O(1) open-element stack, each case took quadratic time in the
// number of unclosed tags. Keep the upstream 100,000-element workloads.
#[test]
fn many_unclosed_tags_followed_by_unmatched_end_tags() {
    let html = format!(
        "<body>{}{}text",
        "<b>".repeat(100_000),
        "</i>".repeat(100_000)
    );
    check(230, &html, "text");
}

#[test]
fn many_list_items_without_list() {
    let html = format!("<body>{}text", "<li>".repeat(100_000));
    check(230, &html, "text");
}

#[test]
fn many_unclosed_tags_followed_by_reopened_list() {
    let html = format!(
        "<body><ul>{}{}<li>text",
        "<b>".repeat(100_000),
        "</ul><ul>".repeat(100_000)
    );
    check(230, &html, "text");
}

#[test]
fn preview_rules_are_distinct_from_full_html_to_text() {
    for (html, expected) in [
        ("<body><p>one</p><p>two<br>three</body>", "one two three"),
        (
            "<body><h1>one</h1><div>two</div>three</body>",
            "onetwothree",
        ),
        ("<body><a href='url'>label</a><a href='empty'></a>", "label"),
        (
            "<body><ol start='9'><li value='20'>one<li>two</ol>",
            "1. one 2. two",
        ),
        ("<body><ul>hidden<li>one<li>two</ul>", "one two"),
        ("<body><table>hidden<td>one</td>hidden</table>two", "onetwo"),
        ("<body><div hidden>still visible</div>", "still visible"),
        ("<body><template>still visible</template>", "still visible"),
        (
            "<body><noscript>raw &amp; text</noscript>",
            "raw &amp; text",
        ),
        (
            "<body><svg><![CDATA[omitted]]><text>visible</text></svg>",
            "visible",
        ),
        (
            "<body>one<!--comment--><img alt=' A &amp; B ' alt='ignored'>two",
            "one A & B two",
        ),
        ("<body><p>one</p><img alt><p>two", "one two"),
        (
            "<body><script/>hidden</script>visible<style/>hidden</style>end",
            "visibleend",
        ),
        ("<body>one<img alt='unfinished", "one"),
        (
            "<head><title>hidden</title></head><body>visible</body>ignored",
            "visible",
        ),
    ] {
        check(230, html, expected);
    }
}

#[test]
fn truncation_is_driven_by_eligible_text_not_tags() {
    for (limit, html, expected) in [
        (1, "<body>a</body>", "a"),
        (1, "<body>ab</body>", "\u{2026}"),
        (1, "<body>a </body>", "\u{2026}"),
        (2, "<body>a b", "\u{2026}"),
        (3, "<body>abc</body>", "abc"),
        (3, "<body>abc</body>ignored", "abc"),
        (3, "<body>abc<script>ignored</script><!--comment-->", "abc"),
        (3, "<body>abc<br>", "abc"),
        (3, "<body>abc<img alt>", "abc"),
        (3, "<body>abc<br><img alt>", "ab\u{2026}"),
        (3, "<body>abc<b>d</b>", "ab\u{2026}"),
        (3, "<body>ab <b> </b>", "a\u{2026}"),
        (3, "<body>abc<b> </b>", "ab\u{2026}"),
        (3, "<body>ab </body>", "ab"),
        (3, "<body>ab  </body>", "a\u{2026}"),
        (3, "<body>a  b", "a b"),
        (3, "<body><img alt='abcd'>", "ab\u{2026}"),
        (3, "<body><ol><li>abc", "1\u{2026}"),
    ] {
        check(limit, html, expected);
    }
}

#[test]
fn unicode_scalars_whitespace_and_reuse() {
    check(
        3,
        "<body>\u{1f408}\u{65e5}\u{e9}",
        "\u{1f408}\u{65e5}\u{e9}",
    );
    check(2, "<body>\u{1f408}\u{65e5}\u{e9}", "\u{1f408}\u{2026}");
    check(1, "<body>\u{1f408}x", "\u{2026}");
    check(2, "<body>e\u{301}x", "e\u{2026}");
    check(
        230,
        "<body>\u{1f469}\u{200d}\u{1f4bb}",
        "\u{1f469} \u{1f4bb}",
    );
    for space in [
        '\t', '\n', '\x0b', '\x0c', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
        '\u{2008}', '\u{2009}', '\u{200a}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ] {
        check(230, &format!("<body>{space}a{space}{space}b{space}"), "a b");
    }
    let previewer = HtmlTextPreviewer::with_maximum_preview_length(4).unwrap();
    assert_eq!(previewer.preview("<body><ol><li>first"), "1. \u{2026}");
    assert_eq!(previewer.preview("<body>next"), "next");
    assert_eq!(previewer.preview(""), "");
    for limit in [230, 1024] {
        let text = "\u{1f408}".repeat(limit);
        check(limit, &format!("<body>{text}"), &text);
        check(
            limit,
            &format!("<body>{text}x"),
            &format!("{}\u{2026}", "\u{1f408}".repeat(limit - 1)),
        );
    }
}

#[test]
fn reader_stops_after_truncation_but_may_consume_ahead() {
    let previewer = HtmlTextPreviewer::with_maximum_preview_length(3).unwrap();
    let html = format!("ignored<body>abcdef</body>{}", "x".repeat(100_000));
    let mut input = Cursor::new(html.as_bytes());
    input.set_position(7);
    assert_eq!(previewer.preview_reader(&mut input).unwrap(), "ab\u{2026}");
    assert!(input.position() < html.len() as u64);
    assert!(input.position() <= 7 + 4096);
    let mut source = b"<body>again".as_slice();
    let reader: &mut dyn io::BufRead = &mut source;
    assert_eq!(previewer.preview_reader(reader).unwrap(), "ag\u{2026}");
}

struct FallibleReader<'a> {
    input: &'a [u8],
    remaining: usize,
    interrupted: bool,
}

impl Read for FallibleReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupted) {
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

#[test]
fn reader_errors_and_interrupted_reads() {
    let html = "<body>caf\u{e9}</body>";
    for fail_at in 0..=html.len() {
        let mut reader = BufReader::with_capacity(
            1,
            FallibleReader {
                input: html.as_bytes(),
                remaining: fail_at,
                interrupted: true,
            },
        );
        assert_eq!(
            HtmlTextPreviewer::new()
                .preview_reader(&mut reader)
                .unwrap_err()
                .to_string(),
            "source failure",
        );
    }
    let mut reader = BufReader::with_capacity(
        1,
        FallibleReader {
            input: html.as_bytes(),
            remaining: usize::MAX,
            interrupted: true,
        },
    );
    assert_eq!(
        HtmlTextPreviewer::new()
            .preview_reader(&mut reader)
            .unwrap(),
        "caf\u{e9}"
    );

    let mut reader = BufReader::with_capacity(
        1,
        FallibleReader {
            input: b"<body>abcdef</body>",
            remaining: 15,
            interrupted: false,
        },
    );
    assert_eq!(
        HtmlTextPreviewer::with_maximum_preview_length(3)
            .unwrap()
            .preview_reader(&mut reader)
            .unwrap(),
        "ab\u{2026}",
    );
}

#[test]
fn utf8_errors_are_token_scoped() {
    for html in [
        &b"<body>\xff"[..],
        &b"<body>\xf0\x9f"[..],
        &b"<body>abcdef\xff</body>"[..],
        &b"<script>\xff</script><body>text"[..],
    ] {
        for capacity in [1, 4096] {
            let mut reader = BufReader::with_capacity(capacity, html);
            let error = HtmlTextPreviewer::with_maximum_preview_length(3)
                .unwrap()
                .preview_reader(&mut reader)
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.get_ref().unwrap().is::<HtmlTokenizerError>());
        }
    }
    for capacity in [1, 4096] {
        let mut reader = BufReader::with_capacity(capacity, b"<body>abcdef</body>\xff".as_slice());
        assert_eq!(
            HtmlTextPreviewer::with_maximum_preview_length(3)
                .unwrap()
                .preview_reader(&mut reader)
                .unwrap(),
            "ab\u{2026}",
        );
    }
    check(230, "<body>\u{feff}text", "\u{feff}text");
}

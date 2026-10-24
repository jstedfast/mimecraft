// Adapted from MimeKit's UnitTests/Text/RtfInterpreterTests.cs and RtfToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};
use std::num::NonZeroUsize;

use mimecraft::text::{RtfHtmlExtractor, RtfIssue, RtfToHtml, RtfToText, RtfViolation as V};

fn check(input: &[u8], expected: Option<&str>) {
    for capacity in [1, 2, 7, 4096] {
        let extractor = RtfHtmlExtractor::new();
        let source = BufReader::with_capacity(capacity, input);
        assert_eq!(
            extractor.extract(source).unwrap().as_deref(),
            expected,
            "capacity={capacity}"
        );
        let source = BufReader::with_capacity(capacity, input);
        assert_eq!(
            extractor
                .extract_with_issues(source, |_| {})
                .unwrap()
                .as_deref(),
            expected
        );
    }
}

fn diagnosed(extractor: &RtfHtmlExtractor, input: &[u8]) -> (Option<String>, Vec<RtfIssue>) {
    let mut issues = Vec::new();
    let html = extractor
        .extract_with_issues(input, |issue| issues.push(issue))
        .unwrap();
    (html, issues)
}

const ENCAPSULATED_HTML: &str = concat!(
    "{\\rtf1\\ansi\\ansicpg1252\\fromhtml1 \\deff0{\\fonttbl{\\f0\\fswiss Arial;}{\\f1\\fcharset204 Arial Cyr;}}\r\n",
    "{\\*\\htmltag19 <html>}{\\*\\htmltag34 <head>}{\\*\\htmltag41 </head>}{\\*\\htmltag50 <body>}\\htmlrtf {\\htmlrtf0 \r\n",
    "{\\*\\htmltag64 <p class=\"x\\'e9\">}\\htmlrtf {\\htmlrtf0 caf\\'e9 {\\f1 \\'cf}\\htmlrtf\\par\\htmlrtf0 \r\n",
    "{\\*\\htmltag72 </p>}\\htmlrtf }\\htmlrtf0 {\\*\\htmltag a\\_b\\par\\tab c}\r\n",
    "\\htmlrtf }\\htmlrtf0 {\\*\\htmltag58 </body>}{\\*\\htmltag27 </html>}}",
);

#[test]
fn canonical_encapsulated_document() {
    // HTMLTAG uses the document code page, ordinary text uses the current font,
    // and HTMLRTF display-only content must not leak into the original HTML.
    check(
        ENCAPSULATED_HTML.as_bytes(),
        Some(concat!(
            "<html><head></head><body><p class=\"x\u{e9}\">caf\u{e9} \u{41f}</p>",
            "a\u{ad}b\r\n\tc</body></html>",
        )),
    );
    assert_eq!(
        RtfToText::new()
            .convert(ENCAPSULATED_HTML.as_bytes())
            .unwrap(),
        "caf\u{e9} \u{41f}\r\n"
    );
}

#[test]
fn extraction_does_not_change_existing_rendering_modes() {
    let input = br"{\rtf1\fromhtml1 {\*\htmltag <script>x</script>}a{\htmlrtf b}c}";
    check(input, Some("<script>x</script>ac"));
    assert_eq!(RtfToText::new().convert(input.as_slice()).unwrap(), "abc");
    let mut renderer = RtfToHtml::new();
    renderer.output_html_fragment = true;
    assert_eq!(
        renderer.convert(input.as_slice()).unwrap(),
        "<div>abc</div>\r\n"
    );
    assert_eq!(
        renderer
            .convert_with_tag_callback(input.as_slice(), |context, writer| {
                context.write_tag(writer, true).unwrap();
            })
            .unwrap(),
        "<div>abc</div>\r\n"
    );
}

#[test]
fn no_html_is_distinct_from_recognized_empty_html() {
    for input in [
        "",
        "{\\rtf1}",
        "{\\rtf1 plain}",
        "{\\rtf1{\\*\\htmltag <b>}}",
    ] {
        check(input.as_bytes(), None);
    }
    for input in [
        "{\\rtf1\\fromhtml1}",
        "{\\rtf1\\fromhtml1",
        "{\\rtf1\\fromhtml1\\htmlrtf hidden}",
    ] {
        check(input.as_bytes(), Some(""));
    }
}

#[test]
fn recognition_stops_after_exactly_ten_tokens() {
    for count in [0, 6, 7, 8, 10, 100] {
        let input = format!(
            "{{\\rtf1{}\\fromhtml1 {{\\*\\htmltag <b>}}x}}",
            "\\ansi".repeat(count)
        );
        // '{' and \rtf1 take two slots, leaving seven before \fromhtml.
        check(
            input.as_bytes(),
            if count <= 7 { Some("<b>x") } else { None },
        );
    }
    // Group starts consume slots too; they are not just ignored whitespace.
    for count in [7, 8] {
        let input = format!(
            "{{\\rtf1{}\\fromhtml1 x{}}}",
            "{".repeat(count),
            "}".repeat(count)
        );
        check(input.as_bytes(), if count == 7 { Some("x") } else { None });
    }
}

#[test]
fn recognition_ends_at_nonprefix_tokens_or_emitted_content() {
    for prefix in [
        " text",
        "}",
        "\\'20",
        "\\~",
        "\\*",
        "\\tab",
        "\\par",
        "\\line",
        "\\cell",
        "\\objattph",
    ] {
        let input = format!("{{\\rtf1{prefix}\\fromhtml1 {{\\*\\htmltag <b>}}x}}");
        check(input.as_bytes(), None);
    }
    // Physical RTF newlines are ignored by the tokenizer, not text tokens.
    check(b"{\\rtf1\r\n\\fromhtml1 x}", Some("x"));
    // Suppressed controls do not emit content and therefore do not end the
    // recognition window early merely because they could have produced text.
    check(br"{\rtf1\v\tab\fromhtml1\v0 x}", Some("x"));
}

#[test]
fn fromhtml_toggles_only_change_the_recognition_prefix() {
    for toggle in ["\\fromhtml", "\\fromhtml1", "\\fromhtml2", "\\fromhtml-1"] {
        check(format!("{{\\rtf1{toggle} x}}").as_bytes(), Some("x"));
    }
    check(br"{\rtf1\fromhtml0 x}", None);
    check(br"{\rtf1\fromhtml1\fromhtml0 x}", None);
    check(br"{\rtf1\fromhtml0\fromhtml1 x}", Some("x"));
    check(br"{\rtf1\FROMHTML1 x}", None);
    check(
        br"{\rtf1\fromhtml1 x\fromhtml0 {\*\htmltag <b>}y}",
        Some("x<b>y"),
    );
    check(br"{\rtf1 x\fromhtml1 {\*\htmltag <b>}y}", None);
}

#[test]
fn htmlrtf_state_is_group_scoped_and_still_tracks_font_changes() {
    check(br"{\rtf1\fromhtml1 a{\htmlrtf b}c}", Some("ac"));
    check(
        br"{\rtf1\fromhtml1 a\htmlrtf b{\htmlrtf0 c}d\htmlrtf0 e}",
        Some("ace"),
    );
    check(br"{\rtf1\fromhtml1{\fonttbl{\f0 A;}{\f1\fcharset204 B;}}\htmlrtf\f1 ignored\par\line\cell\row\htmlrtf0\'cf}",
        Some("\u{41f}"));
}

#[test]
fn htmltag_uses_document_encoding_and_restores_font_encoding() {
    check(br"{\rtf1\fromhtml1\ansicpg1252{\fonttbl{\f1\fcharset204 Cyrillic;}}\f1\'cf{\*\htmltag \'cf}\'cf}",
        Some("\u{41f}\u{cf}\u{41f}"));
    check(br"{\rtf1\fromhtml1\ansicpg932{\fonttbl{\f1\fcharset204 Cyrillic;}}\f1{\*\htmltag \'82\'a0}\'cf}",
        Some("\u{3042}\u{41f}"));
    check(
        br"{\rtf1\fromhtml1{\*\htmltag-999 a\_b}c\_d}",
        Some("a\u{ad}bc\u{2011}d"),
    );
}

#[test]
fn htmltag_newlines_and_tabs_are_copied_without_table_markup() {
    // Within HTMLTAG, \par and \line become CRLF; \cell and \row have no meaning.
    check(
        br"{\rtf1\fromhtml1{\*\htmltag <p>\par x\line y\cell\row\tab z}}",
        Some("<p>\r\nx\r\ny\tz"),
    );
    check(
        br"{\rtf1\fromhtml1 a\par b\line c\tab d\cell e\row f}",
        Some("a\r\nb\r\nc\tdef"),
    );
    check(
        b"{\\rtf1\\fromhtml1{\\*\\htmltag a\\\r\nb}}",
        Some("a\r\nb"),
    );
}

#[test]
fn ordinary_hidden_content_is_suppressed_but_htmltag_is_copied() {
    check(
        br"{\rtf1\fromhtml1 a{\v b\par c\line d\cell e\row}f}",
        Some("af"),
    );
    // Keep MimeKit's destination-specific behavior, rather than applying the
    // ordinary RTF visibility flags to original HTMLTAG markup.
    check(
        br"{\rtf1\fromhtml1{\v\htmlrtf hidden{\*\htmltag <p>\par x\line y}hidden}z}",
        Some("<p>\r\nx\r\nyz"),
    );
    check(
        br"{\rtf1\fromhtml1\htmlrtf\plain hidden\htmlrtf0 x}",
        Some("x"),
    );
}

#[test]
fn unicode_and_byte_decoding_are_shared_with_the_rtf_interpreter() {
    check(
        br"{\rtf1\fromhtml1{\*\htmltag \u233? \u-10179?\u-8704?}}",
        Some("\u{e9} \u{1f600}"),
    );
    check(
        br"{\rtf1\fromhtml1\uc2{\*\htmltag \u233\'3f?x}}",
        Some("\u{e9}x"),
    );
    check(
        br"{\rtf1\fromhtml1{\*\htmltag \u-10179?}}",
        Some("\u{fffd}"),
    );
    check(
        br"{\rtf1\fromhtml1\ansicpg932{\*\htmltag \'95\\}}",
        Some("\u{8868}"),
    );
    let input = format!(
        "{{\\rtf1\\fromhtml1\\ansicpg932{{\\*\\htmltag {}{}}}}}",
        "a".repeat(1023),
        "\\'82\\'a0".repeat(1500)
    );
    check(
        input.as_bytes(),
        Some(&format!("{}{}", "a".repeat(1023), "\u{3042}".repeat(1500))),
    );
}

#[test]
fn nontext_destinations_binary_payloads_and_attachments_do_not_leak() {
    check(br"{\rtf1\fromhtml1 a{\*\mhtmltag duplicate}{\*\unknown ignored}{\pict\bin4 {}\x}{\object{\*\objdata discarded}}b}",
        Some("ab"));
    check(
        br"{\rtf1\fromhtml1{\object{\*\objdata discarded}{\result fallback}}}",
        Some("fallback"),
    );
    check(
        br"{\rtf1\fromhtml1 a\objattph\'20b\objattph  c\objattph!d}",
        Some("abc!d"),
    );
    check(
        br"{\rtf1\fromhtml1{\*\htmltag a\objattph\'20b}}",
        Some("a b"),
    );
    check(
        br#"{\rtf1\fromhtml1{\field{\*\fldinst HYPERLINK "javascript:bad"}{\fldrslt label}}}"#,
        Some("label"),
    );
    check(
        br"{\rtf1\fromhtml1{\upr{ANSI}{\*\ud Unicode}}}",
        Some("Unicode"),
    );
}

#[test]
fn output_is_raw_unfiltered_html_not_token_serialization() {
    let html = "<!doctype html><SCRIPT src='remote'></SCRIPT><img onerror='run()' src='x'>&amp; &#x41; <!-- odd -->";
    let input = format!("{{\\rtf1\\fromhtml1 {{\\*\\htmltag {html}}}}}");
    check(input.as_bytes(), Some(html));
    check(
        br"{\rtf1\fromhtml1 <p>a &amp; b</p>",
        Some("<p>a &amp; b</p>"),
    );
    check(
        br"{\rtf1\fromhtml1{\*\htmltag <unclosed}",
        Some("<unclosed"),
    );
}

#[test]
fn limits_and_diagnostics_do_not_change_between_extractions() {
    let mut extractor = RtfHtmlExtractor::new();
    assert_eq!(extractor, extractor.clone());
    assert_eq!(extractor.max_group_depth.get(), 4096);
    assert_eq!(extractor.max_font_table_entries.get(), 4096);
    assert_eq!(extractor.max_issues_per_violation, 0);
    extractor.max_group_depth = NonZeroUsize::new(1).unwrap();
    let input = br"{\rtf1\fromhtml1 a{\htmlrtf suppressed{\htmlrtf0 lost}}b}";
    for _ in 0..2 {
        let (html, issues) = diagnosed(&extractor, input);
        assert_eq!(html.as_deref(), Some("ab"));
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == V::GroupDepthLimitExceeded)
                .count(),
            1
        );
        assert_eq!(
            extractor.extract(br"{\rtf1 plain}".as_slice()).unwrap(),
            None
        );
    }
}

#[test]
fn deep_htmltag_groups_fold_and_skipped_subtrees_remain_hidden() {
    let extractor = RtfHtmlExtractor {
        max_group_depth: NonZeroUsize::new(2).unwrap(),
        ..RtfHtmlExtractor::new()
    };
    let input = format!(
        "{{\\rtf1\\fromhtml1{{\\*\\htmltag <p>{}deep{}</p>}}{{\\*\\unknown {}hidden{}}}}}",
        "{".repeat(100_000),
        "}".repeat(100_000),
        "{".repeat(100_000),
        "}".repeat(100_000),
    );
    let (html, issues) = diagnosed(&extractor, input.as_bytes());
    assert_eq!(html.as_deref(), Some("<p>deep</p>"));
    assert!(issues.is_empty());
}

#[test]
fn font_limits_and_htmltag_decoding_have_independent_effects() {
    let extractor = RtfHtmlExtractor {
        max_font_table_entries: NonZeroUsize::new(1).unwrap(),
        ..RtfHtmlExtractor::new()
    };
    let (html, issues) = diagnosed(&extractor,
        br"{\rtf1\fromhtml1{\fonttbl{\f0 A;}{\f1 B;}{\f0\fcharset204 C;}}\f0\'cf{\*\htmltag \'cf}\f1\'cf}");
    assert_eq!(html.as_deref(), Some("\u{41f}\u{cf}\u{cf}"));
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::FontTableLimitExceeded)
            .count(),
        1
    );
}

#[test]
fn decoder_diagnostics_inside_htmltag_keep_source_positions() {
    let input = b"{\\rtf1\\fromhtml1\r\n{\\*\\htmltag \xe9\\u-10179?}}";
    let expected = diagnosed(&RtfHtmlExtractor::new(), input);
    assert_eq!(expected.0.as_deref(), Some("\u{e9}\u{fffd}"));
    assert_eq!(expected.1.len(), 2);
    assert_eq!(expected.1[0].violation, V::UnescapedEightBitCharacter);
    assert_eq!(
        expected.1[0].position.offset,
        input.iter().position(|&b| b == 0xe9).unwrap() as u64
    );
    assert_eq!(expected.1[0].position.line_number, 2);
    assert_eq!(expected.1[0].position.line_position, 13);
    assert_eq!(expected.1[1].violation, V::UnpairedSurrogate);
    for capacity in [1, 2, 7, 4096] {
        let mut issues = Vec::new();
        let html = RtfHtmlExtractor::new()
            .extract_with_issues(
                BufReader::with_capacity(capacity, input.as_slice()),
                |issue| issues.push(issue),
            )
            .unwrap();
        assert_eq!((html, issues), expected);
    }
}

#[test]
fn shared_diagnostic_quotas_do_not_affect_extracted_output() {
    let input = br"{\rtf1\fromhtml1{\*\htmltag \u99999999999?\u70000?\u-10179?\u-10179?}";
    let (expected, unlimited) = diagnosed(&RtfHtmlExtractor::new(), input);
    let extractor = RtfHtmlExtractor {
        max_issues_per_violation: 1,
        ..RtfHtmlExtractor::new()
    };
    let (actual, issues) = diagnosed(&extractor, input);
    assert_eq!(actual, expected);
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::InvalidParameter)
            .count(),
        1
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::UnpairedSurrogate)
            .count(),
        1
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::TooManyComplianceIssues)
            .count(),
        1
    );
    let suppressed = unlimited
        .iter()
        .filter(|i| i.violation == V::InvalidParameter)
        .nth(1)
        .unwrap();
    let marker = issues
        .iter()
        .find(|i| i.violation == V::TooManyComplianceIssues)
        .unwrap();
    assert_eq!(marker.position, suppressed.position);
}

#[test]
fn malformed_nonhtml_still_reports_issues_and_consumes_the_source() {
    let (html, issues) = diagnosed(&RtfHtmlExtractor::new(), br"{\rtf1\f99 text");
    assert_eq!(html, None);
    assert!(issues.iter().any(|i| i.violation == V::UndefinedFont));
    assert!(issues.iter().any(|i| i.violation == V::TruncatedStream));
    // Recognizing the encapsulation marker does not require a valid RTF header.
    let (html, issues) = diagnosed(&RtfHtmlExtractor::new(), br"\fromhtml1 <p>");
    assert_eq!(html.as_deref(), Some("<p>"));
    assert!(issues.iter().any(|i| i.violation == V::MissingRtfHeader));
}

struct FailingReader<'a> {
    input: &'a [u8],
    remaining: usize,
}

impl Read for FailingReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let buffer = self.fill_buf()?;
        let count = buffer.len().min(output.len());
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
fn io_errors_never_return_partial_html_or_nonhtml_success() {
    for input in [
        br"{\rtf1\fromhtml1{\*\htmltag <p>}\f99 x}".as_slice(),
        br"{\rtf1\f99 x}".as_slice(),
    ] {
        for remaining in 0..=input.len() {
            let source = FailingReader { input, remaining };
            assert_eq!(
                RtfHtmlExtractor::new().extract(source).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
        let source = FailingReader {
            input,
            remaining: input.len(),
        };
        let mut issues = Vec::new();
        let error = RtfHtmlExtractor::new()
            .extract_with_issues(source, |issue| issues.push(issue))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(issues.iter().any(|i| i.violation == V::UndefinedFont));
    }
}

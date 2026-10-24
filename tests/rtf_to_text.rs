// Adapted from MimeKit's UnitTests/Text/RtfToTextTests.cs and RtfInterpreterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};
use std::num::NonZeroUsize;

use mimecraft::encodings::NewLineFormat;
use mimecraft::text::{RtfIssue, RtfToText, RtfViolation as V};

fn check(input: &[u8], expected: &str) {
    for capacity in [1, 2, 7, 4096] {
        let source = BufReader::with_capacity(capacity, input);
        assert_eq!(
            RtfToText::new().convert(source).unwrap(),
            expected,
            "capacity={capacity}, input={input:?}"
        );
        let source = BufReader::with_capacity(capacity, input);
        assert_eq!(
            RtfToText::new()
                .convert_with_issues(source, |_| {})
                .unwrap(),
            expected
        );
    }
}

fn diagnosed(converter: &RtfToText, input: &[u8]) -> (String, Vec<RtfIssue>) {
    let mut issues = Vec::new();
    let text = converter
        .convert_with_issues(input, |issue| issues.push(issue))
        .unwrap();
    (text, issues)
}

#[test]
fn canonical_simple_document() {
    let input = concat!(
        "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0\\fswiss Arial;}}",
        "{\\colortbl;\\red255\\green0\\blue0;}{\\*\\generator Riched20 10.0;}",
        "\\viewkind4\\uc1\\pard\\f0\\fs20 Hello \\b World\\b0 !\\par\r\n",
        "Second\\line line\\tab x\\par\r\n}"
    );
    check(input.as_bytes(), "Hello World!\r\nSecond\r\nline\tx\r\n");
}

#[test]
fn decorations_defaults_and_line_endings() {
    let mut converter = RtfToText::new();
    assert_eq!(converter.max_group_depth.get(), 4096);
    assert_eq!(converter.max_font_table_entries.get(), 4096);
    assert_eq!(converter.max_issues_per_violation, 0);
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    converter.header = Some("header ".into());
    converter.footer = Some(" footer".into());
    assert_eq!(
        converter.convert(b"{\\rtf1 body}".as_slice()).unwrap(),
        "header body footer"
    );
    assert_eq!(converter.convert(b"".as_slice()).unwrap(), "header  footer");
    let converter = RtfToText::with_new_line_format(NewLineFormat::Unix).unwrap();
    assert_eq!(converter.new_line_format(), NewLineFormat::Unix);
    assert_eq!(
        converter
            .convert(b"{\\rtf1 a\\line b\\par\\row}".as_slice())
            .unwrap(),
        "a\nb\n\n"
    );
    assert!(RtfToText::with_new_line_format(NewLineFormat::Mixed).is_err());
}

#[test]
fn empty_raw_lines_and_malformed_input() {
    for (input, expected) in [
        ("", ""),
        ("{\\rtf1}", ""),
        ("{\\rtf1 one\r\ntwo\rthree\nfour}", "onetwothreefour"),
        ("{\\rtf1 one\\\r\ntwo}", "one\r\ntwo"),
        ("}}}{\\rtf1 a}}}}b", "ab"),
        ("{\\rtf1 abc{\\b def\\'", "abcdef"),
        ("{\\rtf1 abc{\\b def\\par", "abcdef\r\n"),
        ("{\\rtf1 abc\\'4", "abc\u{4}"),
    ] {
        check(input.as_bytes(), expected)
    }
}

#[test]
fn special_words_and_symbols() {
    check(
        br"{\rtf1 \{\}\\\~\_\-\emdash\endash\bullet\lquote\rquote\ldblquote\rdblquote\emspace\enspace\qmspace\zwj\zwnj\ltrmark\rtlmark}",
        "{}\\\u{a0}\u{2011}\u{2014}\u{2013}\u{2022}\u{2018}\u{2019}\u{201c}\u{201d}\u{2003}\u{2002}\u{2005}\u{200d}\u{200c}\u{200e}\u{200f}",
    );
    check(
        b"{\\rtf1 a\\~b\\_c\\-d\\\te\\:f\\|g\\\nh}",
        "a\u{a0}b\u{2011}cd\tefg\r\nh",
    );
}

#[test]
fn unicode_fallback_and_signed_units() {
    for (content, expected) in [
        (r"caf\u233?!", "caf\u{e9}!"),
        (r"\uc2\u8364 XX!", "\u{20ac}!"),
        (r"\uc0\u8364!", "\u{20ac}!"),
        (r"\u8364\'80!", "\u{20ac}!"),
        (r"{\uc2\u8364 XX}\u8364 X!", "\u{20ac}\u{20ac}!"),
        (r"\u-200?!", "\u{ff38}!"),
        (r"\uc3\u8364\'80\{\b x", "\u{20ac}x"),
        (r"\uc5\u8364{a}bc", "\u{20ac}abc"),
        (r"\uc2147483647\u65 abc{x}y", "Axy"),
        (r"\uc-5\u65 B", "AB"),
        (r"\uc0\u65 B", "AB"),
        (r"\u65\u66 xy", "Axy"),
        (r"\u-3913?", "\u{f0b7}"),
        (r"\u70000?", "\u{fffd}"),
        (r"{\uc2 \u65 xy}z", "Az"),
        (r"\u-65537?", "\u{fffd}"),
        (r"\u-65536?", "\0"),
        (r"\u65535?", "\u{ffff}"),
        (r"\uc0\u", "\0"),
    ] {
        check(format!("{{\\rtf1 {content}}}").as_bytes(), expected);
    }
}

#[test]
fn surrogate_pairs_and_unpaired_recovery() {
    for (content, expected) in [
        (r"\u-10179?\u-8704?", "\u{1f600}"),
        (r"\uc0\u-10179\u-8704", "\u{1f600}"),
        (r"a\u-10240?b", "a\u{fffd}b"),
        (r"a\u-8704?b", "a\u{fffd}b"),
        (r"a\u-10240?", "a\u{fffd}"),
        (r"a\u-10240?\par b", "a\u{fffd}\r\nb"),
        (r"{a\u-10240?}b", "a\u{fffd}b"),
        (r"\u-10240?\u-10179?\u-8704?", "\u{fffd}\u{1f600}"),
        (r"\u-10240?\u65?", "\u{fffd}A"),
        (r"\uc0\u55296\u56320", "\u{10000}"),
        (r"\uc0\u56319\u57343", "\u{10ffff}"),
    ] {
        check(format!("{{\\rtf1 {content}}}").as_bytes(), expected)
    }
    check(br"{\rtf1 \u-10179?", "\u{fffd}");
}

#[test]
fn unicode_alternative_destination() {
    check(
        br"{\rtf1 a{\upr{\'e9\~\{x\u66 y}{\*\ud{\'e9\u66 y}}}b}",
        "a\u{e9}Bb",
    );
}

#[test]
fn document_and_font_code_pages() {
    for (content, expected) in [
        (r"\ansi \'80", "\u{20ac}"),
        (r"\mac \'8e", "\u{e9}"),
        (
            r"\ansicpg1251 \'cf\'f0\'e8\'e2\'e5\'f2",
            "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
        ),
        (r"\ansicpg0 \'e9", "\u{e9}"),
        (r"\ansicpg-5 \'e9", "\u{e9}"),
        (r"\ansicpg99999 \'e9", "\u{e9}"),
        (r"\ansicpg50220 \'1b$B0!\'1b(B", "\u{4e9c}"),
        (r"\ansicpg932 \'82\'a0", "\u{3042}"),
        (r"\ansicpg932 \'95\\", "\u{8868}"),
        (r"\ansicpg65001 \'ef\'bb\'bf", "\u{feff}"),
    ] {
        check(format!("{{\\rtf1 {content}}}").as_bytes(), expected)
    }
    check(br"{\rtf1\deff0{\fonttbl{\f0\fcharset0 A;}{\f1\fcharset204 B;}{\f2\fcharset161 C;}}\'e9{\f1 \'cf\'f0\'e8}{\f2 \'e1}\'e9}", "\u{e9}\u{41f}\u{440}\u{438}\u{3b1}\u{e9}");
    check(
        br"{\rtf1{\fonttbl{\f0\fcharset0\cpg1251 A;}}\f0 \'cf}",
        "\u{41f}",
    );
    check(
        br"{\rtf1\deff1{\fonttbl{\f0\fcharset161 A;}{\f1\fcharset204 B;}}\f0 \'e1\plain \'cf}",
        "\u{3b1}\u{41f}",
    );
    check(b"{\\rtf1\\ansicpg1251 \xcf}", "\u{41f}");
}

#[test]
fn font_entries_end_at_their_own_group_or_next_font() {
    for input in [
        br"{\rtf1{\fonttbl\f0\fcharset0 A;\f1\fcharset204 B;}\f1 \'cf}".as_slice(),
        br"{\rtf1{\fonttbl{\f0 A;}{\f1\fcharset204 B}}\f1 \'cf}",
        br"{\rtf1{\fonttbl\f0\fcharset0 A\f1\fcharset204 B;}\f1 \'cf}",
        br"{\rtf1{\fonttbl{\f1{\*\panose x}\fcharset204{\*\falt Foo;}{\pict x;}{\*\unknown y;}Cyr;}}\f1\'cf}",
        br"{\rtf1{\fonttbl{\f1{\fcharset204}A;}}\f1\'cf}",
        br"{\rtf1{\fonttbl{\f1\fcharset204 A\'3b}}\f1\'cf}",
    ] { check(input, "\u{41f}") }
    check(
        br"{\rtf1{\fonttbl{\f0\fcharset204 A}{\f1\fcharset161 B}}\f0\'cf\f1\'e1}",
        "\u{41f}\u{3b1}",
    );
}

#[test]
fn hidden_text_and_plain_reset_are_group_scoped() {
    check(br"{\rtf1 a{\v hidden}b\v c\v0 d}", "abd");
    check(br"{\rtf1 a{\v hidden\plain shown}b}", "ashownb");
    check(br"{\rtf1 {\v\u-10179?}x}", "x");
    check(br"{\rtf1 a\unknownword123 b{\anotherunknown c}}", "abc");
}

#[test]
fn non_text_destinations_binary_payloads_and_object_results() {
    check(br"{\rtf1{\fonttbl{\f0 Arial;}}{\colortbl;\red0\green0\blue0;}{\stylesheet{\s0 Normal;}}{\info{\title Title}{\author Author}}{\*\unknowndest junk}{\pict\wmetafile8 0102030405}{\object{\*\objdata 0102}}{\header Header}{\footnote Footnote}a{\*\bkmkstart b1}{\*\bkmkend b1}b}", "ab");
    check(br"{\rtf1 a{\pict\bin3 {{{}b}", "ab");
    check(br"{\rtf1 a{\object\objemb{\*\objclass Package}{\*\objdata 0105}{\objdata 0105}{\result b}}c}", "abc");
    check(
        br"{\rtf1 a}}{\*\htmltag0 <x>}{\*\unknown y}{\info z}b}",
        "ab",
    );
    // A misplaced skipped destination in the initial root must not erase it.
    check(br"{\rtf1\pict visible}", "visible");
}

#[test]
fn fields_only_render_results_and_do_not_collect_large_instructions() {
    check(
        br#"{\rtf1 see {\field{\*\fldinst{HYPERLINK "http://example.com/"}}{\fldrslt{link}}} now}"#,
        "see link now",
    );
    check(br#"{\rtf1{\field{\*\fldinst HYPERLINK "a"}{\fldrslt A{\field{\*\fldinst HYPERLINK "b"}{\fldrslt B}}C}}}"#, "ABC");
    let input = format!(
        "{{\\rtf1{{\\field{{\\*\\fldinst {}}}{{\\fldrslt x}}}}}}",
        "a".repeat(1_000_000)
    );
    assert_eq!(RtfToText::new().convert(input.as_bytes()).unwrap(), "x");
}

#[test]
fn table_separators_are_delayed_until_content_or_another_cell() {
    check(br"{\rtf1\trowd\cellx1000\cellx2000\pard\intbl a\cell b\cell\row\trowd\cellx1000\cellx2000\pard\intbl c\cell d\cell\row\pard after\par}", "a\tb\r\nc\td\r\nafter\r\n");
    check(br"{\rtf1 a\cell}", "a");
    check(br"{\rtf1 a\cell\cell\row}", "a\t\r\n");
    check(br"{\rtf1 a\cell\par}", "a\t\r\n");
    check(br"{\rtf1 a\cell{\v hidden}b}", "a\tb");
    check(br"{\rtf1\intbl\itap2 a\nestcell b\nestcell{\*\nesttableprops\trowd\nestrow}{\nonesttables\par a b}\cell\row}", "a\tb\r\n\r\n");
    check(
        br"{\rtf1{\*\nesttableprops\trowd Hello\nestcell\nestrow}}",
        "Hello\r\n",
    );
}

#[test]
fn encapsulated_html_renders_rtf_instead_of_markup() {
    check(br"{\rtf1\fromhtml1{\*\htmltag1 <html>}{\*\htmltag <b>}\htmlrtf{\b\htmlrtf0 bold\htmlrtf}\htmlrtf0{\*\htmltag </b>}\htmlrtf\par\htmlrtf0}", "bold\r\n");
}

#[test]
fn placeholder_callbacks_only_replace_immediate_spaces() {
    let cases = [
        (br"{\rtf1 A\objattph\'20 B\objattph  C}".as_slice(), "A[0] B[1]C", "A BC"),
        (br"{\rtf1 A\objattph\'41B}", "A[0]AB", "AAB"),
        (br"{\rtf1 A\objattph xB}", "A[0]xB", "AxB"),
        (br"{\rtf1 A\objattph\par B}", "A[0]\r\nB", "A\r\nB"),
        (br"{\rtf1 A\objattph}", "A[0]", "A"),
        (br"{\rtf1 A{\*\unknown \objattph}{\v\objattph}{\pict\objattph}{\fonttbl\objattph}{\*\htmltag\objattph}B\objattph\'20 C}", "AB[0] C", "AB C"),
        (br"{\rtf1 a\cell\objattph\'20\cell\row}", "a\t[0]\r\n", "a\t\r\n"),
        (br"{\rtf1\fromhtml1{\*\htmltag <p>}A\objattph\'20 B{\*\htmltag </p>}}", "A[0] B", "A B"),
    ];
    for (input, expected, plain) in cases {
        for capacity in [1, 7, 4096] {
            let output = RtfToText::new()
                .convert_with_callbacks(
                    BufReader::with_capacity(capacity, input),
                    |_| {},
                    |index, output| output.push_str(&format!("[{index}]")),
                )
                .unwrap();
            assert_eq!(output, expected);
        }
        check(input, plain);
    }
}

#[test]
fn placeholders_are_numbered_afresh_each_conversion() {
    let input = format!("{{\\rtf1 {}}}", "\\objattph\\'20".repeat(10_000));
    for _ in 0..2 {
        let mut count = 0;
        let output = RtfToText::new()
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
        assert!(output.is_empty());
    }
}

#[test]
fn identical_groups_fold_and_skipped_nesting_uses_a_counter() {
    let mut converter = RtfToText::new();
    converter.max_group_depth = NonZeroUsize::new(1).unwrap();
    let input = format!(
        "{{\\rtf1 a{}deep{}b}}",
        "{".repeat(1_000_000),
        "}".repeat(1_000_000)
    );
    let (text, issues) = diagnosed(&converter, input.as_bytes());
    assert_eq!(text, "adeepb");
    assert!(issues.is_empty());
    let input = format!(
        "{{\\rtf1 a{{\\*\\unknown {}x{}}}b}}",
        "{".repeat(100_000),
        "}".repeat(100_000)
    );
    assert_eq!(converter.convert(input.as_bytes()).unwrap(), "ab");
}

#[test]
fn distinct_state_limits_discard_only_unpushed_groups() {
    let mut input = String::from("{\\rtf1 a");
    for i in 0..100 {
        input.push_str(if i % 2 == 0 { "{\\b " } else { "{\\b0 " })
    }
    input.push_str("deep");
    input.push_str(&"}".repeat(100));
    input.push_str("b}");
    assert_eq!(
        RtfToText::new().convert(input.as_bytes()).unwrap(),
        "adeepb"
    );
    let mut converter = RtfToText::new();
    converter.max_group_depth = NonZeroUsize::new(8).unwrap();
    let (text, issues) = diagnosed(&converter, input.as_bytes());
    assert_eq!(text, "ab");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::GroupDepthLimitExceeded)
            .count(),
        1
    );
    converter.max_group_depth = NonZeroUsize::new(1).unwrap();
    let (text, issues) = diagnosed(&converter, br"{\rtf1 a{b}\b c{\i d}{e}f}");
    assert_eq!(text, "abcf");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::GroupDepthLimitExceeded)
            .count(),
        1
    );
    // \pict skips an already-pushed group: its closing brace must restore state.
    check(br"{\rtf1 a{\v\pict discarded}b}", "ab");
}

#[test]
fn font_table_limits_drop_new_numbers_but_allow_updates() {
    let mut converter = RtfToText::new();
    converter.max_font_table_entries = NonZeroUsize::new(1).unwrap();
    let (text, issues) = diagnosed(
        &converter,
        br"{\rtf1{\fonttbl{\f0 A;}{\f1 B;}{\f0\fcharset204 C;}}\f0\'cf\f1\'cf}",
    );
    assert_eq!(text, "\u{41f}\u{cf}");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::FontTableLimitExceeded)
            .count(),
        1
    );
    assert!(!issues.iter().any(|i| i.violation == V::UndefinedFont));
    let mut input = String::from("{\\rtf1{\\fonttbl");
    for i in 0..10_000 {
        input.push_str(&format!("{{\\f{i}\\fcharset204 Font;}}"))
    }
    input.push_str("}\\f9999\\'cf\\f0\\'cf}");
    assert_eq!(
        converter.convert(input.as_bytes()).unwrap(),
        "\u{cf}\u{41f}"
    );
}

#[test]
fn decoder_boundaries_preserve_dbcs_and_flush_trailing_lead_bytes() {
    let input = format!("{{\\rtf1\\ansicpg932 a{}}}", "\\'82\\'a0".repeat(2000));
    check(input.as_bytes(), &format!("a{}", "\u{3042}".repeat(2000)));
    let input = format!("{{\\rtf1\\ansicpg932 {}\\'82}}", "a".repeat(1024));
    let (text, issues) = diagnosed(&RtfToText::new(), input.as_bytes());
    assert_eq!(text, format!("{}\u{fffd}", "a".repeat(1024)));
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::InvalidByteSequence)
            .count(),
        1
    );
    check(br"{\rtf1\ansicpg932 \'82\b\'a0}", "\u{fffd}\u{fffd}");
    // Decoder shift state is finalized before changing code pages.
    check(br"{\rtf1\ansicpg50220 \'1b$B0!\ansi\'e9}", "\u{4e9c}\u{e9}");
}

#[test]
fn unsupported_pages_and_cache_overflow_are_explicit_and_bounded() {
    let converter = RtfToText::new();
    let (text, issues) = diagnosed(&converter, br"{\rtf1\pc\'9b\pca\'9b\ansicpg1361 abc}");
    assert_eq!(text, "\u{203a}\u{203a}abc");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::UnsupportedCodePage)
            .count(),
        3
    );
    let mut input = String::from("{\\rtf1 ");
    for i in 0..100 {
        input.push_str(&format!("\\ansicpg{}\\'e9", 70000 + i))
    }
    input.push_str("\\ansicpg70000\\'e9}");
    let (text, issues) = diagnosed(&converter, input.as_bytes());
    assert_eq!(text, "\u{e9}".repeat(101));
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::UnsupportedCodePage)
            .count(),
        32
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::CodePageLimitExceeded)
            .count(),
        1
    );
}

#[test]
fn raw_controls_are_dropped_but_escaped_controls_are_preserved() {
    let mut input = b"{\\rtf1 ".to_vec();
    input.extend(0..32);
    input.extend_from_slice(br"\'00\u1?}");
    check(&input, "\t\0\u{1}");
    let (text, issues) = diagnosed(&RtfToText::new(), b"{\\rtf1 caf\xe9 \xe9\\par \xe9}");
    assert_eq!(text, "caf\u{e9} \u{e9}\r\n\u{e9}");
    let high: Vec<_> = issues
        .iter()
        .filter(|i| i.violation == V::UnescapedEightBitCharacter)
        .collect();
    assert_eq!(high.len(), 2);
    assert_eq!(high[0].position.offset, 10);
    assert_eq!(high[1].position.offset, 18);
    let (_, issues) = diagnosed(&RtfToText::new(), b"{\\rtf1{\\pict \xff}x}");
    assert!(
        !issues
            .iter()
            .any(|i| i.violation == V::UnescapedEightBitCharacter)
    );
}

#[test]
fn lexical_structural_and_interpreter_reports_share_one_budget() {
    let mut converter = RtfToText::new();
    let input = br"{\rtf1\fs0\fs0\f9\f9\u70000?\u70000?\u-8704?\u-8704?}}}tail";
    let (expected, unlimited) = diagnosed(&converter, input);
    converter.max_issues_per_violation = 1;
    let (actual, capped) = diagnosed(&converter, input);
    assert_eq!(actual, expected);
    assert_eq!(
        capped
            .iter()
            .filter(|i| i.violation == V::TooManyComplianceIssues)
            .count(),
        1
    );
    for violation in [
        V::InvalidFontSize,
        V::UndefinedFont,
        V::InvalidParameter,
        V::UnpairedSurrogate,
        V::UnbalancedGroupEnd,
    ] {
        assert_eq!(
            unlimited
                .iter()
                .filter(|i| i.violation == violation)
                .count(),
            2
        );
        assert_eq!(
            capped.iter().filter(|i| i.violation == violation).count(),
            1
        );
    }
    assert_eq!(diagnosed(&converter, input), (actual, capped));
    let (_, issues) = diagnosed(&converter, br"{\rtf1\u99999999999?}");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::InvalidParameter)
            .count(),
        1
    );
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
fn source_failures_propagate_instead_of_returning_partial_success() {
    let input = br"{\rtf1 a\u233?\par b}";
    for remaining in 0..=input.len() {
        let reader = FailingReader { input, remaining };
        assert_eq!(
            RtfToText::new().convert(reader).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn cached_supported_pages_remain_usable_after_the_cache_fills() {
    let mut input = String::from("{\\rtf1\\ansicpg1251\\'cf");
    for page in 70000..70031 {
        input.push_str(&format!("\\ansicpg{page}\\'cf"));
    }
    input.push_str("\\ansicpg1253\\'cf\\ansicpg1251\\'cf}");
    let (text, issues) = diagnosed(&RtfToText::new(), input.as_bytes());
    assert_eq!(text, format!("\u{41f}{}\u{41f}", "\u{cf}".repeat(32)));
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::UnsupportedCodePage)
            .count(),
        31
    );
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == V::CodePageLimitExceeded)
            .count(),
        1
    );
}

#[test]
fn raw_byte_diagnostics_keep_original_positions_across_input_chunks() {
    for capacity in [1, 2, 7, 4096] {
        let input = b"{\\rtf1 abc\r\n\xe9\xff\\par \xfe}".as_slice();
        let reader = BufReader::with_capacity(capacity, input);
        let mut issues = Vec::new();
        let output = RtfToText::new()
            .convert_with_issues(reader, |issue| issues.push(issue))
            .unwrap();
        assert_eq!(output, "abc\u{e9}\u{ff}\r\n\u{fe}");
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].violation, V::UnescapedEightBitCharacter);
        assert_eq!(issues[0].position.offset, 12);
        assert_eq!(issues[0].position.line_number, 2);
        assert_eq!(issues[0].position.line_position, 1);
        assert_eq!(issues[1].position.offset, 19);
    }
}

#[test]
fn lexical_and_unicode_parameter_errors_use_the_same_quota() {
    let input = br"{\rtf1\u99999999999?\u70000?}";
    let mut converter = RtfToText::new();
    let (expected, unlimited) = diagnosed(&converter, input);
    assert_eq!(expected, "\u{fffd}\u{fffd}");
    assert_eq!(
        unlimited
            .iter()
            .filter(|i| i.violation == V::InvalidParameter)
            .count(),
        2
    );
    converter.max_issues_per_violation = 1;
    let (actual, limited) = diagnosed(&converter, input);
    assert_eq!(actual, expected);
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[0].violation, V::InvalidParameter);
    assert_eq!(limited[1].violation, V::TooManyComplianceIssues);
    assert_eq!(limited[1].position, unlimited[1].position);
}

#[test]
fn callbacks_delivered_before_an_io_failure_are_not_rolled_back() {
    let input = br"{\rtf1\f99\objattph x}";
    let reader = FailingReader {
        input,
        remaining: input.len(),
    };
    let mut issues = Vec::new();
    let mut placeholders = Vec::new();
    let error = RtfToText::new()
        .convert_with_callbacks(
            reader,
            |issue| issues.push(issue),
            |index, _| placeholders.push(index),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(placeholders, [0]);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, V::UndefinedFont);
}

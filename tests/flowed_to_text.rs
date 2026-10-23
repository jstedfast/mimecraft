// Adapted from MimeKit's UnitTests/Text/FlowedToTextTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    encodings::{EncoderOptionsError, NewLineFormat},
    text::FlowedToText,
};

#[test]
fn defaults_and_explicit_newline_policy_are_platform_independent() {
    let converter = FlowedToText::new();
    assert_eq!(converter, FlowedToText::default());
    assert!(!converter.delete_space);
    assert_eq!(converter.new_line_format(), NewLineFormat::Dos);
    assert_eq!(converter.convert("one\ntwo\n"), "one\r\ntwo\r\n");
    let unix = FlowedToText::with_new_line_format(NewLineFormat::Unix).unwrap();
    assert_eq!(unix.new_line_format(), NewLineFormat::Unix);
    assert_eq!(unix.convert("one\r\ntwo\r"), "one\ntwo\n");
    assert_eq!(
        FlowedToText::with_new_line_format(NewLineFormat::Mixed),
        Err(EncoderOptionsError::InvalidNewLineFormat(
            NewLineFormat::Mixed
        ))
    );
}

#[test]
fn simple_flowed_paragraphs_join_without_losing_separating_spaces() {
    let converter = FlowedToText::new();
    let input = "This is some sample text that has been formatted \r\n\
according to the format=flowed rules defined in rfc3676. \r\n\
This text, once converted, should all be on a single line.\r\n";
    let expected = "This is some sample text that has been formatted \
according to the format=flowed rules defined in rfc3676. \
This text, once converted, should all be on a single line.\r\n";
    assert_eq!(converter.convert(input), expected);
    assert_eq!(converter.convert("new \ncall\n"), "new call\r\n");
}

#[test]
fn quote_prefixes_are_unwrapped_and_normalized_at_each_depth() {
    let converter = FlowedToText::new();
    for depth in 1..=6 {
        let quote = ">".repeat(depth);
        let input = format!("{quote} one \n{quote}two\n");
        assert_eq!(converter.convert(&input), format!("{quote} one two\r\n"));
    }
    assert_eq!(converter.convert(">\n>>\n"), "> \r\n>> \r\n");
}

#[test]
fn quote_depth_changes_win_over_soft_line_breaks() {
    let mut converter = FlowedToText::new();
    let input = "> one \n>> two \nplain\n";
    assert_eq!(converter.convert(input), "> one \r\n>> two \r\nplain\r\n");
    converter.delete_space = true;
    assert_eq!(converter.convert(input), "> one\r\n>> two\r\nplain\r\n");
    assert_eq!(converter.convert(">  \nplain"), "> \r\nplain\r\n");
}

#[test]
fn unstuffing_happens_after_quote_detection_and_removes_exactly_one_space() {
    let converter = FlowedToText::new();
    for (input, expected) in [
        (" From sender\n", "From sender\r\n"),
        (" >literal\n", ">literal\r\n"),
        ("  leading\n", " leading\r\n"),
        (">  leading\n", ">  leading\r\n"),
        ("> >literal\n", "> >literal\r\n"),
        ("\tleading\n", "\tleading\r\n"),
        (" \n", "\r\n"),
    ] {
        assert_eq!(converter.convert(input), expected, "{input:?}");
    }
}

#[test]
fn delete_space_removes_only_one_soft_break_space_including_at_eof() {
    let mut converter = FlowedToText::new();
    converter.delete_space = true;
    assert_eq!(converter.convert("first  \nsecond"), "first second\r\n");
    assert_eq!(converter.convert("soft \nbreak"), "softbreak\r\n");
    assert_eq!(
        converter.convert("URL:\nhttps://example.com/ "),
        "URL:\r\nhttps://example.com/"
    );
    assert_eq!(converter.convert("> end "), "> end");
    assert_eq!(converter.convert("  "), "");
}

#[test]
fn hard_and_soft_endings_empty_lines_and_bare_cr_follow_textreader_rules() {
    for (input, expected) in [
        ("", ""),
        ("\r\n", "\r\n"),
        ("\n\n", "\r\n\r\n"),
        ("a", "a\r\n"),
        ("a\r\n", "a\r\n"),
        ("a ", "a "),
        ("a \r\n", "a "),
        ("a \n\n", "a \r\n"),
        ("a \rb\rc\r\n", "a b\r\nc\r\n"),
        ("a\t\nb\n", "a\t\r\nb\r\n"),
        ("a\u{a0}\nb\n", "a\u{a0}\r\nb\r\n"),
        ("\u{feff}a \nb", "\u{feff}a b\r\n"),
        (
            "\u{1f408} \n\u{65e5}\u{672c}",
            "\u{1f408} \u{65e5}\u{672c}\r\n",
        ),
    ] {
        assert_eq!(FlowedToText::new().convert(input), expected, "{input:?}");
    }
}

#[test]
fn signature_separator_retains_mimekit_compatibility_behavior() {
    let mut converter = FlowedToText::new();
    assert_eq!(converter.convert("-- \nSignature\n"), "-- Signature\r\n");
    converter.delete_space = true;
    assert_eq!(converter.convert("-- \nSignature\n"), "--Signature\r\n");
}

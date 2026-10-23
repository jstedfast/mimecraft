// Adapted from MimeKit's UnitTests/Text/HtmlTokenizerComplianceTests.cs
// and HtmlCappedComplianceLoggerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use HtmlEntityViolation::*;
use mimecraft::text::{
    DecodedHtml, HtmlDecodeIssue, HtmlDecodeOptions, HtmlEntityDecoder, HtmlEntityViolation,
    html_attribute_decode, html_attribute_decode_with_options, html_decode,
    html_decode_with_options,
};

fn decode(text: &str) -> DecodedHtml {
    html_decode_with_options(text, &HtmlDecodeOptions::default())
}

fn decode_attribute(text: &str) -> DecodedHtml {
    html_attribute_decode_with_options(text, &HtmlDecodeOptions::default())
}

fn issue(violation: HtmlEntityViolation, offset: usize) -> HtmlDecodeIssue {
    HtmlDecodeIssue { violation, offset }
}

fn pushed(text: &str) -> HtmlEntityDecoder {
    let mut decoder = HtmlEntityDecoder::new();
    for character in text.chars() {
        assert!(decoder.push(character).unwrap(), "{text:?}, {character:?}");
    }
    decoder
}

#[test]
fn canonical_reference_diagnostics_and_recovered_output() {
    for (text, expected, violations) in [
        (
            "&#;",
            "&#;",
            vec![AbsenceOfDigitsInNumericCharacterReference],
        ),
        (
            "&#x;",
            "&#x;",
            vec![AbsenceOfDigitsInNumericCharacterReference],
        ),
        (
            "&#x110000;",
            "\u{fffd}",
            vec![CharacterReferenceOutsideUnicodeRange],
        ),
        ("&#x80;", "\u{20ac}", vec![ControlCharacterReference]),
        ("&#13;", "\r", vec![ControlCharacterReference]),
        ("&#1;", "\u{1}", vec![ControlCharacterReference]),
        ("&#9;", "\t", vec![]),
        (
            "&amp x",
            "& x",
            vec![MissingSemicolonAfterCharacterReference],
        ),
        (
            "&#65 x",
            "A x",
            vec![MissingSemicolonAfterCharacterReference],
        ),
        ("&#65", "A", vec![MissingSemicolonAfterCharacterReference]),
        ("&amp", "&", vec![MissingSemicolonAfterCharacterReference]),
        ("&#xFFFE;", "\u{fffe}", vec![NoncharacterCharacterReference]),
        ("&#xFDD0;", "\u{fdd0}", vec![NoncharacterCharacterReference]),
        ("&#0;", "\u{fffd}", vec![NullCharacterReference]),
        ("&#xD800;", "\u{fffd}", vec![SurrogateCharacterReference]),
        ("&foo;", "&foo;", vec![UnknownNamedCharacterReference]),
        ("&foo bar", "&foo bar", vec![]),
        ("&", "&", vec![]),
        ("& x", "& x", vec![]),
    ] {
        let result = decode(text);
        assert_eq!(result.text, expected, "{text}");
        assert_eq!(
            result.issues,
            violations
                .into_iter()
                .map(|kind| issue(kind, 0))
                .collect::<Vec<_>>(),
            "{text}"
        );
    }
}

#[test]
fn malformed_numeric_prefixes_report_only_absent_digits() {
    for text in [
        "&#",
        "&#x",
        "&#X",
        "&#;",
        "&#x;",
        "&#X;",
        "&#a",
        "&#/",
        "&#x@",
        "&#xG",
        "&#xg",
        "&#-1;",
        "&#+1;",
        "&# 65;",
        "&#\u{661};",
    ] {
        for result in [decode(text), decode_attribute(text)] {
            assert_eq!(result.text, text, "{text}");
            assert_eq!(
                result.issues,
                [issue(AbsenceOfDigitsInNumericCharacterReference, 0)],
                "{text}"
            );
        }
    }
}

#[test]
fn missing_semicolon_precedes_numeric_value_violations_at_the_same_offset() {
    for (text, violation) in [
        ("&#0", NullCharacterReference),
        ("&#x110000", CharacterReferenceOutsideUnicodeRange),
        ("&#xD800", SurrogateCharacterReference),
        ("&#xFFFE", NoncharacterCharacterReference),
        ("&#13", ControlCharacterReference),
    ] {
        for result in [
            decode(&format!("x {text}")),
            decode_attribute(&format!("x {text}")),
        ] {
            assert_eq!(
                result.issues,
                [
                    issue(MissingSemicolonAfterCharacterReference, 2),
                    issue(violation, 2),
                ]
            );
        }
        let decoder = pushed(text);
        assert_eq!(
            decoder.violations().collect::<Vec<_>>(),
            [MissingSemicolonAfterCharacterReference, violation,]
        );
        assert_eq!(
            decoder.attribute_violations(Some('=')).collect::<Vec<_>>(),
            [MissingSemicolonAfterCharacterReference, violation,]
        );
    }
}

#[test]
fn every_byte_sized_numeric_value_is_classified_before_recovery() {
    for value in 0u32..=255 {
        let violation = if value == 0 {
            Some(NullCharacterReference)
        } else if value == 13
            || ((value < 32 || (127..=159).contains(&value)) && ![9, 10, 12].contains(&value))
        {
            Some(ControlCharacterReference)
        } else {
            None
        };
        for text in [format!("&#{value};"), format!("&#x{value:X};")] {
            let expected = violation
                .into_iter()
                .map(|kind| issue(kind, 0))
                .collect::<Vec<_>>();
            for result in [decode(&text), decode_attribute(&text)] {
                assert_eq!(result.text, html_decode(&text));
                assert_eq!(result.issues, expected, "{text}");
            }
            assert_eq!(
                pushed(&text).violations().collect::<Vec<_>>(),
                violation.into_iter().collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn unicode_boundaries_and_every_planes_noncharacters() {
    for (value, violation) in [
        (0xd7ff, None),
        (0xd800, Some(SurrogateCharacterReference)),
        (0xdfff, Some(SurrogateCharacterReference)),
        (0xe000, None),
        (0xfdcf, None),
        (0xfdd0, Some(NoncharacterCharacterReference)),
        (0xfdef, Some(NoncharacterCharacterReference)),
        (0xfdf0, None),
        (0x10000, None),
        (0x10fffd, None),
        (0x110000, Some(CharacterReferenceOutsideUnicodeRange)),
        (u32::MAX, Some(CharacterReferenceOutsideUnicodeRange)),
    ] {
        let text = format!("&#x{value:X};");
        assert_eq!(
            decode(&text).issues,
            violation
                .into_iter()
                .map(|kind| issue(kind, 0))
                .collect::<Vec<_>>(),
            "{text}"
        );
    }
    for plane in 0..=16 {
        for low in [0xfffe, 0xffff] {
            let value = (plane << 16) | low;
            let text = format!("&#x{value:X};");
            let result = decode(&text);
            assert_eq!(result.text, char::from_u32(value).unwrap().to_string());
            assert_eq!(
                result.issues,
                [issue(NoncharacterCharacterReference, 0)],
                "{text}"
            );
        }
    }
}

#[test]
fn historical_attribute_exceptions_use_the_longest_match_suffix() {
    for text in ["&copy=1", "&copycat", "&noti;", "&notin", "&notA", "&not0"] {
        assert_eq!(
            decode(text).issues,
            [issue(MissingSemicolonAfterCharacterReference, 0)],
            "{text}"
        );
        let result = decode_attribute(text);
        assert_eq!(result.text, text);
        assert!(result.issues.is_empty(), "{text}");
    }
    for text in ["&copy", "&copy!", "&copy x", "&copy\u{e9}", "&copy\u{661}"] {
        assert_eq!(
            decode_attribute(text).issues,
            [issue(MissingSemicolonAfterCharacterReference, 0),]
        );
    }
    for text in ["&not;", "&notin;", "&not;=", "&copy;cat"] {
        assert!(decode(text).issues.is_empty());
        assert!(decode_attribute(text).issues.is_empty());
    }
    let decoder = pushed("&noti");
    for next in [None, Some(';'), Some('!')] {
        assert!(decoder.attribute_violations(next).next().is_none());
        assert_eq!(
            decoder.violations().collect::<Vec<_>>(),
            [MissingSemicolonAfterCharacterReference]
        );
    }
    let decoder = pushed("&copy");
    for next in [Some('='), Some('0'), Some('A'), Some('z')] {
        assert!(decoder.attribute_violations(next).next().is_none());
    }
    for next in [None, Some('!'), Some('\u{e9}'), Some('\0')] {
        assert_eq!(
            decoder.attribute_violations(next).collect::<Vec<_>>(),
            [MissingSemicolonAfterCharacterReference,]
        );
    }
}

#[test]
fn unknown_names_require_an_ascii_name_semicolon_and_no_legacy_match() {
    for text in [
        "&foo;",
        "&unknown;",
        "&notarealname;",
        "&0;",
        "&;",
        "&\u{e9};",
        "&;foo;",
    ] {
        let expected = match text {
            "&notarealname;" => vec![issue(MissingSemicolonAfterCharacterReference, 0)],
            "&foo;" | "&unknown;" | "&0;" => vec![issue(UnknownNamedCharacterReference, 0)],
            _ => vec![],
        };
        assert_eq!(decode(text).issues, expected, "{text}");
    }
    for text in [
        "&unknown",
        "&foo bar;",
        "&foo\u{e9};",
        "&\u{e9}foo;",
        "&foo=bar;",
        "&foo_bar;",
        "&foo-;",
        "&foo\0;",
        "&noti;",
        "&notin",
    ] {
        assert!(decode_attribute(text).issues.is_empty(), "{text}");
    }
    let result = decode("&foo;&amp;&unknown;");
    assert_eq!(result.text, "&foo;&&unknown;");
    assert_eq!(
        result.issues,
        [
            issue(UnknownNamedCharacterReference, 0),
            issue(UnknownNamedCharacterReference, 10),
        ]
    );
    // A character-fed decoder stops at an impossible name prefix. The
    // whole-string API scans the remaining alphanumerics to find the ';'.
    let mut decoder = HtmlEntityDecoder::new();
    for character in "&foo;".chars() {
        if !decoder.push(character).unwrap() {
            break;
        }
    }
    assert!(decoder.violations().next().is_none());
    assert!(decoder.attribute_violations(Some(';')).next().is_none());
}

#[test]
fn issue_offsets_use_original_utf8_bytes_without_normalization() {
    let text = "\u{feff}\u{1f408}\r\nx &#0;\n&#x80; &foo;";
    let result = decode(text);
    assert_eq!(
        result.text,
        "\u{feff}\u{1f408}\r\nx \u{fffd}\n\u{20ac} &foo;"
    );
    assert_eq!(
        result.issues,
        [
            issue(NullCharacterReference, 11),
            issue(ControlCharacterReference, 16),
            issue(UnknownNamedCharacterReference, 23),
        ]
    );
    for found in &result.issues {
        assert!(text[found.offset..].starts_with('&'));
    }
    assert_eq!(decode_attribute(text).issues, result.issues);
    assert_eq!(
        decode("line1\nline2 &#0;").issues,
        [issue(NullCharacterReference, 12)]
    );
    assert_eq!(
        decode_attribute("\"&#0;\"").issues,
        [issue(NullCharacterReference, 1)]
    );
}

#[test]
fn reporting_is_single_pass_and_does_not_validate_surrounding_html() {
    for text in [
        "",
        "\0\u{7f}\u{80}",
        "<",
        "<!bad>",
        "&amp;foo;",
        "&#38;foo;",
    ] {
        let result = decode(text);
        assert_eq!(result.text, html_decode(text));
        assert!(result.issues.is_empty(), "{text:?}");
    }
    // Unlike a tokenizer, this API treats even apparent script/comment content
    // as character data. It does not infer an HTML context from markup.
    for text in [
        "<!--&#0;-->",
        "<script>&#0;</script>",
        "<a title='&copy=1'>",
    ] {
        let result = decode(text);
        assert_eq!(result.text, html_decode(text));
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].offset, text.find('&').unwrap());
    }
}

#[test]
fn diagnostics_do_not_consume_or_change_incremental_state() {
    let mut decoder = HtmlEntityDecoder::new();
    assert!(decoder.violations().next().is_none());
    assert!(decoder.attribute_violations(None).next().is_none());
    for text in ["&", "&#", "&#x", "&#65", "&#0;", "&noti", "&amp;"] {
        for character in text.chars() {
            assert!(decoder.push(character).unwrap());
        }
        let original = decoder.clone();
        let first = decoder.violations().collect::<Vec<_>>();
        assert_eq!(decoder.violations().collect::<Vec<_>>(), first);
        let _ = decoder.attribute_violations(Some('=')).collect::<Vec<_>>();
        assert_eq!(decoder, original);
        decoder.reset();
        assert!(decoder.violations().next().is_none());
    }
    // Reading at a temporary chunk boundary cannot decide whether the
    // reference is complete; a subsequent chunk can remove the violation.
    let mut decoder = pushed("&notin");
    assert_eq!(
        decoder.violations().collect::<Vec<_>>(),
        [MissingSemicolonAfterCharacterReference]
    );
    assert!(decoder.push(';').unwrap());
    assert!(decoder.violations().next().is_none());
}

#[test]
fn unlimited_options_report_every_issue_and_are_independent_values() {
    let mut options = HtmlDecodeOptions::default();
    assert_eq!(options.max_issues_per_violation, 0);
    let unlimited = options;
    options.max_issues_per_violation = 2;
    let text = "&#0;".repeat(100);
    let result = html_decode_with_options(&text, &unlimited);
    assert_eq!(result.issues.len(), 100);
    for (index, found) in result.issues.iter().enumerate() {
        assert_eq!(*found, issue(NullCharacterReference, index * 4));
    }
    assert_eq!(html_decode_with_options(&text, &options).issues.len(), 3);
    let max = HtmlDecodeOptions {
        max_issues_per_violation: usize::MAX,
    };
    assert_eq!(html_decode_with_options(&text, &max), result);
}

#[test]
fn limit_marks_the_first_suppressed_issue_not_the_last_allowed_one() {
    let options = HtmlDecodeOptions {
        max_issues_per_violation: 2,
    };
    assert_eq!(
        html_decode_with_options("&#0;&#0;", &options).issues,
        [
            issue(NullCharacterReference, 0),
            issue(NullCharacterReference, 4),
        ]
    );
    let result = html_decode_with_options("&#0;&#0;&#0;&#0;&#1;", &options);
    assert_eq!(result.text, "\u{fffd}\u{fffd}\u{fffd}\u{fffd}\u{1}");
    assert_eq!(
        result.issues,
        [
            issue(NullCharacterReference, 0),
            issue(NullCharacterReference, 4),
            issue(TooManyComplianceIssues, 8),
            issue(ControlCharacterReference, 16),
        ]
    );
}

#[test]
fn every_violation_has_a_separate_budget_and_only_one_marker_is_emitted() {
    let samples = [
        ("&#;", AbsenceOfDigitsInNumericCharacterReference),
        ("&#x110000;", CharacterReferenceOutsideUnicodeRange),
        ("&#1;", ControlCharacterReference),
        ("&copy!", MissingSemicolonAfterCharacterReference),
        ("&#xFDD0;", NoncharacterCharacterReference),
        ("&#0;", NullCharacterReference),
        ("&#xD800;", SurrogateCharacterReference),
        ("&foo;", UnknownNamedCharacterReference),
    ];
    let options = HtmlDecodeOptions {
        max_issues_per_violation: 1,
    };
    let mut text = String::new();
    let mut expected = Vec::new();
    for (sample, violation) in samples {
        let offset = text.len();
        expected.push(issue(violation, offset));
        text.push_str(sample);
        if expected.len() == 1 {
            expected.push(issue(TooManyComplianceIssues, text.len()));
        }
        for _ in 0..20 {
            text.push_str(sample);
        }
    }
    for result in [
        html_decode_with_options(&text, &options),
        html_attribute_decode_with_options(&text, &options),
    ] {
        assert_eq!(result.issues, expected);
        assert_eq!(result.issues.len(), 9);
    }
}

#[test]
fn multiple_issues_at_one_offset_are_budgeted_independently() {
    let options = HtmlDecodeOptions {
        max_issues_per_violation: 1,
    };
    let result = html_decode_with_options("&#65 &#0 &#0 &#1", &options);
    assert_eq!(
        result.issues,
        [
            issue(MissingSemicolonAfterCharacterReference, 0),
            issue(TooManyComplianceIssues, 5),
            issue(NullCharacterReference, 5),
            issue(ControlCharacterReference, 13),
        ]
    );
    assert_eq!(result.text, "A \u{fffd} \u{fffd} \u{1}");
}

#[test]
fn reporting_budgets_reset_for_each_call_and_context() {
    let options = HtmlDecodeOptions {
        max_issues_per_violation: 1,
    };
    let text = "&#0;&#0;";
    let expected = [
        issue(NullCharacterReference, 0),
        issue(TooManyComplianceIssues, 4),
    ];
    for _ in 0..3 {
        assert_eq!(html_decode_with_options(text, &options).issues, expected);
        assert_eq!(
            html_attribute_decode_with_options(text, &options).issues,
            expected
        );
    }
    // A literal historical attribute exception must not consume the budget.
    assert_eq!(
        html_attribute_decode_with_options("&copy=1 &copy! &copy!", &options).issues,
        [
            issue(MissingSemicolonAfterCharacterReference, 8),
            issue(TooManyComplianceIssues, 15)
        ]
    );
}

#[test]
fn long_numeric_runs_and_unknown_names_are_fully_consumed() {
    let zeros = "0".repeat(100_000);
    let nines = "9".repeat(100_000);
    let unknown = "z".repeat(100_000);
    for (text, expected, violations) in [
        (format!("&#{zeros}65;tail"), "Atail".to_owned(), vec![]),
        (format!("&#x{zeros}41;tail"), "Atail".to_owned(), vec![]),
        (
            format!("&#{nines};tail"),
            "\u{fffd}tail".to_owned(),
            vec![CharacterReferenceOutsideUnicodeRange],
        ),
        (
            format!("&#x{nines}end"),
            "\u{fffd}nd".to_owned(),
            vec![
                MissingSemicolonAfterCharacterReference,
                CharacterReferenceOutsideUnicodeRange,
            ],
        ),
        (
            format!("&{unknown};&amp;"),
            format!("&{unknown};&"),
            vec![UnknownNamedCharacterReference],
        ),
    ] {
        for result in [decode(&text), decode_attribute(&text)] {
            assert_eq!(result.text, expected);
            assert_eq!(
                result.issues,
                violations
                    .iter()
                    .map(|&kind| issue(kind, 0))
                    .collect::<Vec<_>>()
            );
        }
    }
    let options = HtmlDecodeOptions {
        max_issues_per_violation: 2,
    };
    let text = format!("{}&foo;", "&#0;".repeat(100_000));
    let result = html_decode_with_options(&text, &options);
    assert_eq!(result.text, html_decode(&text));
    assert_eq!(
        result.issues,
        [
            issue(NullCharacterReference, 0),
            issue(NullCharacterReference, 4),
            issue(TooManyComplianceIssues, 8),
            issue(UnknownNamedCharacterReference, 400_000),
        ]
    );
}

#[test]
fn diagnostics_never_change_output_for_mixed_fragments() {
    let fragments = [
        "plain",
        "\u{1f408}",
        "\0\r\n",
        "&",
        "&;",
        "&#",
        "&#x",
        "&#0",
        "&#128;",
        "&#xD800;",
        "&#x110000",
        "&not",
        "&noti",
        "&notin;",
        "&apos",
        "&AMP",
        "&Amp;",
        "&foo",
        "&foo;",
        "&#xG",
        "=",
        ";",
        "A",
        "0",
        "&amp;lt;",
    ];
    for first in fragments {
        for second in fragments {
            let text = format!("{first}{second}");
            for limit in [0, 1, 2] {
                let options = HtmlDecodeOptions {
                    max_issues_per_violation: limit,
                };
                assert_eq!(
                    html_decode_with_options(&text, &options).text,
                    html_decode(&text),
                    "{text:?}"
                );
                assert_eq!(
                    html_attribute_decode_with_options(&text, &options).text,
                    html_attribute_decode(&text),
                    "{text:?}"
                );
            }
        }
    }
}

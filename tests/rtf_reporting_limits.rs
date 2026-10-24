// Adapted from MimeKit's UnitTests/Text/RtfCappedComplianceLoggerTests.cs
// and RtfConverterComplianceTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, io::BufReader};

use mimecraft::text::{
    RtfLexicalIssue, RtfLexicalViolation as V, RtfSourcePosition, RtfToken, RtfTokenizer,
    RtfTokenizerOptions,
};

fn tokenize(input: &[u8], capacity: usize, limit: usize) -> (Vec<RtfToken>, Vec<RtfLexicalIssue>) {
    let options = RtfTokenizerOptions {
        max_issues_per_violation: limit,
    };
    let mut tokenizer =
        RtfTokenizer::with_options(BufReader::with_capacity(capacity, input), &options);
    let mut tokens = Vec::new();
    let mut issues = Vec::new();
    while let Some(token) = tokenizer
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
    {
        tokens.push(token);
    }
    (tokens, issues)
}

fn violations(issues: &[RtfLexicalIssue]) -> Vec<V> {
    issues.iter().map(|issue| issue.violation).collect()
}

#[test]
fn default_and_explicit_zero_are_unlimited() {
    assert_eq!(RtfTokenizerOptions::default().max_issues_per_violation, 0);
    let input = "\\'z".repeat(100);
    let expected = tokenize(input.as_bytes(), 2, 0);
    assert_eq!(expected.1.len(), 100);
    assert!(
        expected
            .1
            .iter()
            .all(|issue| issue.violation == V::InvalidHexEscape)
    );
    let mut tokenizer = RtfTokenizer::new(BufReader::with_capacity(2, input.as_bytes()));
    let mut tokens = Vec::new();
    let mut issues = Vec::new();
    while let Some(token) = tokenizer
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
    {
        tokens.push(token);
    }
    assert_eq!((tokens, issues), expected);
}

#[test]
fn excess_issues_are_suppressed_at_the_exact_limit() {
    let input = "\\'z".repeat(10);
    for limit in 1..=9 {
        let (_, issues) = tokenize(input.as_bytes(), 1, limit);
        assert_eq!(issues.len(), limit + 1);
        assert!(
            issues[..limit]
                .iter()
                .all(|issue| issue.violation == V::InvalidHexEscape)
        );
        assert_eq!(issues[limit].violation, V::TooManyComplianceIssues);
        assert_eq!(issues[limit].position.offset, (limit * 3) as u64);
    }
}

#[test]
fn merely_reaching_a_limit_does_not_emit_a_marker() {
    for limit in [1, 2, 3, 100] {
        let input = "\\'z".repeat(limit);
        let (_, issues) = tokenize(input.as_bytes(), 7, limit);
        assert_eq!(issues.len(), limit);
        assert!(
            issues
                .iter()
                .all(|issue| issue.violation == V::InvalidHexEscape)
        );
    }
}

#[test]
fn marker_retains_offset_line_column_and_group_depth() {
    for capacity in [1, 2, 7, 4096] {
        let (_, issues) = tokenize(b"{\\'z\n{\\'z}}", capacity, 1);
        assert_eq!(
            violations(&issues),
            [V::InvalidHexEscape, V::TooManyComplianceIssues]
        );
        assert_eq!(
            issues[1].position,
            RtfSourcePosition {
                offset: 6,
                line_number: 2,
                line_position: 2,
                depth: 2,
            }
        );
    }
}

#[test]
fn closing_groups_does_not_reset_the_stream_budget() {
    let (_, issues) = tokenize(b"{\\'z}{\\'z}", 1, 1);
    assert_eq!(
        violations(&issues),
        [V::InvalidHexEscape, V::TooManyComplianceIssues]
    );
    assert_eq!(issues[0].position.offset, 1);
    assert_eq!(issues[1].position.offset, 6);
}

#[test]
fn flooding_one_kind_cannot_hide_other_kinds() {
    let word = format!("\\{}99999999999 ", "a".repeat(33));
    let input = format!("{{{}{}{}\\bin100 payload", "\\'z".repeat(1000), word, word);
    let (_, issues) = tokenize(input.as_bytes(), 3, 1);
    assert_eq!(
        violations(&issues),
        [
            V::InvalidHexEscape,
            V::TooManyComplianceIssues,
            V::KeywordTooLong,
            V::InvalidParameter,
            V::TruncatedStream,
        ]
    );
    assert_eq!(issues.last().unwrap().position.offset, input.len() as u64);
}

#[test]
fn marker_and_later_issue_of_the_same_word_keep_source_order() {
    let name = "a".repeat(33);
    let first = format!("\\{name} ");
    let input = format!("{first}\\{name}99999999999 ");
    let (_, issues) = tokenize(input.as_bytes(), 2, 1);
    assert_eq!(
        violations(&issues),
        [
            V::KeywordTooLong,
            V::TooManyComplianceIssues,
            V::InvalidParameter,
        ]
    );
    assert_eq!(issues[1].position.offset, first.len() as u64);
    assert_eq!(issues[1].position, issues[2].position);
}

#[test]
fn capped_reports_are_an_ordered_projection_of_unlimited_reports() {
    let input = format!(
        "{{{}{}\\bin7 a\n{{\\}}",
        "\\'z\\fs- x".repeat(5),
        format!("\\{}99999999999 ", "a".repeat(33)).repeat(4),
    );
    for capacity in [1, 2, 7, 4096] {
        let (expected_tokens, full) = tokenize(input.as_bytes(), capacity, 0);
        for limit in [1, 2, 3, 4, usize::MAX] {
            let mut occurrences = HashMap::new();
            let mut expected = Vec::new();
            let mut first_suppressed = None;
            for issue in &full {
                let count = occurrences.entry(issue.violation).or_insert(0);
                *count += 1;
                if *count <= limit {
                    expected.push(*issue);
                } else if first_suppressed.is_none() {
                    first_suppressed = Some(issue.position);
                    expected.push(RtfLexicalIssue {
                        violation: V::TooManyComplianceIssues,
                        position: issue.position,
                    });
                }
            }
            let (tokens, issues) = tokenize(input.as_bytes(), capacity, limit);
            assert_eq!(
                tokens, expected_tokens,
                "capacity={capacity}, limit={limit}"
            );
            assert_eq!(issues, expected, "capacity={capacity}, limit={limit}");
        }
    }
}

#[test]
fn changing_callbacks_does_not_reset_budgets() {
    let options = RtfTokenizerOptions {
        max_issues_per_violation: 1,
    };
    let mut tokenizer = RtfTokenizer::with_options(b"\\'z\\'z\\'z".as_slice(), &options);
    let mut first = Vec::new();
    let mut second = Vec::new();
    let mut third = Vec::new();
    assert!(
        tokenizer
            .read_token_with_issues(|issue| first.push(issue))
            .unwrap()
            .is_some()
    );
    assert!(
        tokenizer
            .read_token_with_issues(|issue| second.push(issue))
            .unwrap()
            .is_some()
    );
    assert!(
        tokenizer
            .read_token_with_issues(|issue| third.push(issue))
            .unwrap()
            .is_some()
    );
    assert_eq!(violations(&first), [V::InvalidHexEscape]);
    assert_eq!(violations(&second), [V::TooManyComplianceIssues]);
    assert!(third.is_empty());
}

#[test]
fn callback_free_reads_do_not_spend_budgets() {
    let options = RtfTokenizerOptions {
        max_issues_per_violation: 1,
    };
    let mut tokenizer = RtfTokenizer::with_options(b"\\'z\\'z\\'z".as_slice(), &options);
    assert!(tokenizer.read_token().unwrap().is_some());
    let mut issues = Vec::new();
    while tokenizer
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
        .is_some()
    {}
    assert_eq!(
        violations(&issues),
        [V::InvalidHexEscape, V::TooManyComplianceIssues]
    );
    assert_eq!(issues[0].position.offset, 3);
    assert_eq!(issues[1].position.offset, 6);
}

#[test]
fn callback_free_reads_do_not_discard_the_future_limit_marker() {
    let options = RtfTokenizerOptions {
        max_issues_per_violation: 1,
    };
    let mut tokenizer = RtfTokenizer::with_options(b"\\'z\\'z\\'z".as_slice(), &options);
    let mut issues = Vec::new();
    assert!(
        tokenizer
            .read_token_with_issues(|issue| issues.push(issue))
            .unwrap()
            .is_some()
    );
    assert!(tokenizer.read_token().unwrap().is_some());
    assert!(
        tokenizer
            .read_token_with_issues(|issue| issues.push(issue))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        violations(&issues),
        [V::InvalidHexEscape, V::TooManyComplianceIssues]
    );
    assert_eq!(issues[1].position.offset, 6);
}

#[test]
fn discarded_issues_are_not_deferred_until_reporting_starts() {
    let options = RtfTokenizerOptions {
        max_issues_per_violation: 1,
    };
    let mut tokenizer = RtfTokenizer::with_options(b"{\\'z\\'z".as_slice(), &options);
    while tokenizer.read_token().unwrap().is_some() {}
    for _ in 0..3 {
        assert!(
            tokenizer
                .read_token_with_issues(|_| panic!("discarded issue was replayed"))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn options_are_copied_and_each_tokenizer_gets_a_fresh_budget() {
    let input = b"\\'z\\'z\\'z".as_slice();
    let mut options = RtfTokenizerOptions {
        max_issues_per_violation: 1,
    };
    let mut first = RtfTokenizer::with_options(input, &options);
    options.max_issues_per_violation = 3;
    let mut second = RtfTokenizer::with_options(input, &options);
    let mut first_issues = Vec::new();
    let mut second_issues = Vec::new();
    while first
        .read_token_with_issues(|issue| first_issues.push(issue))
        .unwrap()
        .is_some()
    {}
    while second
        .read_token_with_issues(|issue| second_issues.push(issue))
        .unwrap()
        .is_some()
    {}
    assert_eq!(
        violations(&first_issues),
        [V::InvalidHexEscape, V::TooManyComplianceIssues]
    );
    assert_eq!(violations(&second_issues), [V::InvalidHexEscape; 3]);
    assert_eq!(tokenize(input, 4096, 1).1, first_issues);
}

#[test]
fn reporting_a_large_flood_keeps_only_the_configured_budget() {
    let word = format!("\\{}99999999999 ", "a".repeat(33));
    let input = format!("{{{}{}", "\\'z".repeat(10_000), word.repeat(10_000));
    let options = RtfTokenizerOptions {
        max_issues_per_violation: 3,
    };
    let mut tokenizer = RtfTokenizer::with_options(input.as_bytes(), &options);
    let mut issues = Vec::new();
    while tokenizer
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
        .is_some()
    {}
    assert_eq!(issues.len(), 11); // Three per repeatable defect, one marker, one EOF issue.
    for kind in [V::InvalidHexEscape, V::KeywordTooLong, V::InvalidParameter] {
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.violation == kind)
                .count(),
            3
        );
    }
    assert_eq!(
        issues
            .iter()
            .filter(|issue| issue.violation == V::TooManyComplianceIssues)
            .count(),
        1
    );
    assert_eq!(
        issues
            .iter()
            .filter(|issue| issue.violation == V::TruncatedStream)
            .count(),
        1
    );
}

#[test]
fn empty_and_valid_streams_do_not_report_limit_markers() {
    for input in [
        b"".as_slice(),
        b"{\\rtf1 hello}",
        b"\\'41",
        b"{\\bin3 {\\}}",
    ] {
        assert!(tokenize(input, 1, 1).1.is_empty());
    }
}

// Structural fixtures adapted from MimeKit's UnitTests/Text/RtfInterpreterComplianceTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};

use mimecraft::text::{
    RtfIssue, RtfLexicalIssue, RtfLexicalViolation, RtfSourcePosition, RtfToken, RtfTokenKind,
    RtfTokenizer, RtfValidationOptions, RtfValidator, RtfViolation as V,
};

fn issue(violation: V, offset: u64, line: u64, column: u64, depth: usize) -> RtfIssue {
    RtfIssue {
        violation,
        position: RtfSourcePosition {
            offset,
            line_number: line,
            line_position: column,
            depth,
        },
    }
}

fn validate(input: &[u8], capacity: usize, limit: usize) -> (Vec<RtfToken>, Vec<RtfIssue>) {
    let options = RtfValidationOptions {
        max_issues_per_violation: limit,
    };
    let mut validator =
        RtfValidator::with_options(BufReader::with_capacity(capacity, input), &options);
    let mut tokens = Vec::new();
    let mut issues = Vec::new();
    while let Some(token) = validator
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
    {
        tokens.push(token);
    }
    for _ in 0..3 {
        assert!(
            validator
                .read_token_with_issues(|_| panic!("repeated EOF issue"))
                .unwrap()
                .is_none()
        );
    }
    (tokens, issues)
}

fn kinds(issues: &[RtfIssue]) -> Vec<V> {
    issues.iter().map(|issue| issue.violation).collect()
}

#[test]
fn canonical_structure_and_lexical_cases_preserve_positions() {
    let cases: &[(&[u8], &[RtfIssue])] = &[
        (b"{\\rtf1 hello}", &[]),
        (b"{\\rtf1 x} \t\0\r\n", &[]),
        (b"", &[issue(V::MissingRtfHeader, 0, 1, 1, 0)]),
        (b"hello", &[issue(V::MissingRtfHeader, 0, 1, 1, 0)]),
        (b"{hello}", &[issue(V::MissingRtfHeader, 1, 1, 2, 1)]),
        (
            b"{",
            &[
                issue(V::TruncatedStream, 1, 1, 2, 1),
                issue(V::MissingRtfHeader, 1, 1, 2, 1),
            ],
        ),
        (b"{\\rtf1 x}}", &[issue(V::UnbalancedGroupEnd, 9, 1, 10, 0)]),
        (
            b"{\\rtf1 x}junk{more}",
            &[issue(V::ContentAfterDocument, 9, 1, 10, 0)],
        ),
        (
            b"{\\rtf1 x}\n\\par",
            &[issue(V::ContentAfterDocument, 10, 2, 1, 0)],
        ),
        (
            b"{\\rtf1{\\b x}",
            &[issue(V::TruncatedStream, 12, 1, 13, 1)],
        ),
        (
            b"{\\rtf1{\\pict \\bin100 abc}",
            &[issue(V::TruncatedStream, 25, 1, 26, 2)],
        ),
        (
            b"{\\rtf1 \\li99999999999 x}",
            &[issue(V::InvalidParameter, 7, 1, 8, 1)],
        ),
        (b"{\\rtf1 \\'zz}", &[issue(V::InvalidHexEscape, 7, 1, 8, 1)]),
        (
            b"{\\rtf1 \\'",
            &[
                issue(V::InvalidHexEscape, 7, 1, 8, 1),
                issue(V::TruncatedStream, 9, 1, 10, 1),
            ],
        ),
        (
            b"{\\rtf1\r\n\\par\n\\'zz}",
            &[issue(V::InvalidHexEscape, 13, 3, 1, 1)],
        ),
    ];
    for &(input, expected) in cases {
        for capacity in [1, 2, 3, 7, 64, 4096] {
            assert_eq!(
                validate(input, capacity, 0).1,
                expected,
                "input={input:?}, capacity={capacity}"
            );
        }
    }
}

#[test]
fn header_presence_does_not_validate_versions_or_interpret_formatting() {
    for input in [
        b"{\\rtf}".as_slice(),
        b"{\\rtf0}",
        b"{\\rtf-1}",
        b"{\\rtf123}",
        b"\r\n{\n\\rtf1 x}",
        b"{\\rtf1\\ansicpg99999\\f5\\fs0\\cf9 \\u70000?\\u-10179?}",
        b"{\\rtf1{\\fonttbl{\\f0 Arial;}}{\\colortbl;\\red255;}\\f0 x}",
    ] {
        assert!(validate(input, 1, 0).1.is_empty(), "{input:?}");
    }
    assert_eq!(
        kinds(&validate(b"{\\rtf99999999999}", 1, 0).1),
        [V::InvalidParameter]
    );
}

#[test]
fn header_is_case_sensitive_and_only_checked_at_the_start() {
    for input in [
        b"{\\RTF1}".as_slice(),
        b"{\\ansi\\rtf1}",
        b"{{\\rtf1}}",
        b" \\rtf1",
        b"\\rtf1",
        b"{}",
        b"{\\{\\rtf1}",
        b"{\\'72tf1}",
    ] {
        assert_eq!(
            kinds(&validate(input, 1, 0).1),
            [V::MissingRtfHeader],
            "{input:?}"
        );
    }
    assert_eq!(
        kinds(&validate(b"x{\\rtf1}y{\\rtf1}", 1, 0).1),
        [V::MissingRtfHeader, V::ContentAfterDocument]
    );
}

#[test]
fn missing_header_at_eof_uses_the_cursor_not_the_last_token() {
    for (input, expected) in [
        (b"\r\n".as_slice(), issue(V::MissingRtfHeader, 2, 2, 1, 0)),
        (b"\\", issue(V::MissingRtfHeader, 1, 1, 2, 0)),
        (b"{\r\n\\", issue(V::MissingRtfHeader, 4, 2, 2, 1)),
    ] {
        for capacity in [1, 16] {
            let (_, issues) = validate(input, capacity, 0);
            assert_eq!(issues.last(), Some(&expected));
        }
    }
}

#[test]
fn an_initial_closing_brace_reports_both_defects_in_order() {
    assert_eq!(
        validate(b"}", 1, 0).1,
        [
            issue(V::MissingRtfHeader, 0, 1, 1, 0),
            issue(V::UnbalancedGroupEnd, 0, 1, 1, 0),
        ]
    );
    assert_eq!(
        kinds(&validate(b"}}text", 1, 0).1),
        [
            V::MissingRtfHeader,
            V::UnbalancedGroupEnd,
            V::UnbalancedGroupEnd,
        ]
    );
}

#[test]
fn lexical_issues_precede_structural_issues_for_the_same_token() {
    let input = format!("\\{}99999999999 ", "a".repeat(33));
    let (_, issues) = validate(input.as_bytes(), 1, 0);
    assert_eq!(
        kinds(&issues),
        [V::KeywordTooLong, V::InvalidParameter, V::MissingRtfHeader]
    );
    assert!(issues.iter().all(|issue| issue.position.offset == 0));
    assert_eq!(
        kinds(&validate(b"\\'z", 1, 0).1),
        [V::InvalidHexEscape, V::MissingRtfHeader]
    );
}

#[test]
fn content_after_the_first_root_is_reported_once_and_braces_still_balance() {
    let input = b"{\\rtf1}{{x}}junk}{\\rtf1 more}}";
    assert_eq!(
        kinds(&validate(input, 1, 0).1),
        [
            V::ContentAfterDocument,
            V::UnbalancedGroupEnd,
            V::UnbalancedGroupEnd,
        ]
    );
    assert_eq!(
        kinds(&validate(b"{\\rtf1}} \t\0\r\n", 1, 0).1),
        [V::UnbalancedGroupEnd]
    );
}

#[test]
fn only_raw_space_tab_and_nul_are_harmless_trailing_text() {
    for byte in 0..=u8::MAX {
        if matches!(byte, b'{' | b'}' | b'\\') {
            continue;
        }
        let mut input = b"{\\rtf1}".to_vec();
        input.push(byte);
        let (_, issues) = validate(&input, 1, 0);
        if matches!(byte, b' ' | b'\t' | b'\0' | b'\r' | b'\n') {
            assert!(issues.is_empty(), "byte={byte}");
        } else {
            assert_eq!(kinds(&issues), [V::ContentAfterDocument], "byte={byte}");
        }
    }
    for suffix in [b"\\'20".as_slice(), b"\\tab", b"\\\n", b"\\~", b"\\bin1 x"] {
        let mut input = b"{\\rtf1}".to_vec();
        input.extend_from_slice(suffix);
        assert_eq!(kinds(&validate(&input, 1, 0).1), [V::ContentAfterDocument]);
    }
}

#[test]
fn trailing_text_issues_point_to_the_token_start_including_any_padding() {
    let input = b"{\\rtf1} \tX";
    // Like MimeKit, report the text token's start, not the first non-padding
    // byte inside it. Chunk boundaries can therefore change this location.
    assert_eq!(
        validate(input, 64, 0).1,
        [issue(V::ContentAfterDocument, 7, 1, 8, 0)]
    );
    assert_eq!(
        validate(input, 1, 0).1,
        [issue(V::ContentAfterDocument, 9, 1, 10, 0)]
    );
}

#[test]
fn binary_braces_and_escaped_braces_do_not_change_structure() {
    let input = b"{\\rtf1{\\pict\\bin5 }\n{\\x}\\{\\}\\'7b\\'7d}";
    for capacity in [1, 2, 7, 64] {
        assert!(validate(input, capacity, 0).1.is_empty());
    }
    let (_, issues) = validate(b"{\\rtf1\\bin4 a\nb\n}}\n\\par", 1, 0);
    assert_eq!(
        issues,
        [
            issue(V::UnbalancedGroupEnd, 17, 3, 2, 0),
            issue(V::ContentAfterDocument, 19, 4, 1, 0),
        ]
    );
}

#[test]
fn validation_never_discards_destination_contents_or_changes_tokens() {
    let input = b"{\\rtf1{\\*\\unknown hidden{nested}}{\\pict\\bin5 }\n{\\x}\\u65?\\'z}}tail";
    for capacity in [1, 2, 7, 4096] {
        let mut tokenizer = RtfTokenizer::new(BufReader::with_capacity(capacity, input.as_slice()));
        let mut expected = Vec::new();
        while let Some(token) = tokenizer.read_token().unwrap() {
            expected.push(token);
        }
        for limit in [0, 1, 2, usize::MAX] {
            assert_eq!(validate(input, capacity, limit).0, expected);
        }
        let mut validator = RtfValidator::new(BufReader::with_capacity(capacity, input.as_slice()));
        let mut actual = Vec::new();
        while let Some(token) = validator.read_token().unwrap() {
            actual.push(token);
        }
        assert_eq!(actual, expected);
    }
}

#[test]
fn deeply_nested_groups_need_no_stack_and_keep_outer_structure() {
    let mut input = b"{\\rtf1".to_vec();
    input.extend(std::iter::repeat_n(b'{', 100_000));
    input.push(b'x');
    input.extend(std::iter::repeat_n(b'}', 100_001));
    input.extend_from_slice(b"}tail");
    let mut validator = RtfValidator::new(input.as_slice());
    let mut issues = Vec::new();
    let mut count = 0;
    while validator
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
        .is_some()
    {
        count += 1;
    }
    assert_eq!(count, 200_006);
    assert_eq!(
        kinds(&issues),
        [V::UnbalancedGroupEnd, V::ContentAfterDocument]
    );
}

#[test]
fn exact_limits_preserve_independent_quotas_and_one_combined_marker() {
    let word = format!("\\{}99999999999 ", "a".repeat(33));
    let input = format!("{{\\rtf1 {}}}{}tail", word.repeat(8), "}".repeat(8));
    for limit in 1..=9 {
        let (tokens, issues) = validate(input.as_bytes(), 3, limit);
        assert_eq!(tokens, validate(input.as_bytes(), 3, 0).0);
        for violation in [
            V::KeywordTooLong,
            V::InvalidParameter,
            V::UnbalancedGroupEnd,
        ] {
            assert_eq!(
                issues
                    .iter()
                    .filter(|issue| issue.violation == violation)
                    .count(),
                limit.min(8),
                "limit={limit}, violation={violation:?}"
            );
        }
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.violation == V::TooManyComplianceIssues)
                .count(),
            usize::from(limit < 8)
        );
        assert_eq!(issues.last().unwrap().violation, V::ContentAfterDocument);
    }
}

#[test]
fn structural_suppression_cannot_hide_later_lexical_issues() {
    let (_, issues) = validate(b"{\\rtf1}}}\\'z\\'z", 1, 1);
    assert_eq!(
        kinds(&issues),
        [
            V::UnbalancedGroupEnd,
            V::TooManyComplianceIssues,
            V::InvalidHexEscape,
            V::ContentAfterDocument,
        ]
    );
    assert_eq!(
        issues[1].position,
        issue(V::TooManyComplianceIssues, 8, 1, 9, 0).position
    );
}

#[test]
fn capped_reports_are_an_ordered_projection_across_all_seven_defects() {
    let word = format!("\\{}99999999999 ", "a".repeat(33));
    let input = format!(
        "{word}{{\\rtf1 {}{}}}{}tail{{",
        word.repeat(5),
        "\\'z".repeat(7),
        "}".repeat(6)
    );
    for capacity in [1, 3, 4096] {
        let (tokens, unlimited) = validate(input.as_bytes(), capacity, 0);
        for violation in [
            V::KeywordTooLong,
            V::InvalidParameter,
            V::InvalidHexEscape,
            V::TruncatedStream,
            V::MissingRtfHeader,
            V::UnbalancedGroupEnd,
            V::ContentAfterDocument,
        ] {
            assert!(unlimited.iter().any(|issue| issue.violation == violation));
        }
        for limit in 1..=8 {
            let mut counts = [0; 7];
            let mut marked = false;
            let mut expected = Vec::new();
            for &issue in &unlimited {
                let count = &mut counts[issue.violation as usize];
                if *count < limit {
                    *count += 1;
                    expected.push(issue);
                } else if !marked {
                    marked = true;
                    expected.push(RtfIssue {
                        violation: V::TooManyComplianceIssues,
                        ..issue
                    });
                }
            }
            let (actual_tokens, actual) = validate(input.as_bytes(), capacity, limit);
            assert_eq!(actual_tokens, tokens);
            assert_eq!(actual, expected);
            assert!(actual.len() <= 7 * limit + 1);
        }
    }
}

#[test]
fn callback_free_reads_track_structure_without_deferred_reports() {
    let mut validator = RtfValidator::new(b"{\\rtf1}xy}".as_slice());
    // Consume the complete root without reporting. Later content must not be
    // mistaken for a missing header just because the callback changed.
    for _ in 0..3 {
        validator.read_token().unwrap().unwrap();
    }
    let mut issues = Vec::new();
    while validator
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
        .is_some()
    {}
    assert_eq!(
        kinds(&issues),
        [V::ContentAfterDocument, V::UnbalancedGroupEnd]
    );

    let mut validator = RtfValidator::new(b"x{\\rtf1}tail\\par".as_slice());
    while let Some(token) = validator.read_token().unwrap() {
        if matches!(token.kind, RtfTokenKind::Text(ref text) if text == b"tail") {
            break;
        }
    }
    while validator
        .read_token_with_issues(|issue| panic!("discarded issue replayed: {issue:?}"))
        .unwrap()
        .is_some()
    {}
}

#[test]
fn callback_free_reads_do_not_spend_quotas_or_the_suppression_marker() {
    let options = RtfValidationOptions {
        max_issues_per_violation: 1,
    };
    let mut validator = RtfValidator::with_options(b"{\\rtf1}}}}".as_slice(), &options);
    for _ in 0..3 {
        validator.read_token().unwrap().unwrap();
    }
    let mut first = Vec::new();
    validator
        .read_token_with_issues(|issue| first.push(issue))
        .unwrap();
    validator.read_token().unwrap().unwrap();
    let mut second = Vec::new();
    validator
        .read_token_with_issues(|issue| second.push(issue))
        .unwrap();
    assert_eq!(kinds(&first), [V::UnbalancedGroupEnd]);
    assert_eq!(second, [issue(V::TooManyComplianceIssues, 9, 1, 10, 0)]);

    let mut validator = RtfValidator::with_options(b"{\\rtf1}}}".as_slice(), &options);
    for _ in 0..4 {
        validator.read_token().unwrap().unwrap();
    }
    let mut issues = Vec::new();
    validator
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap();
    assert_eq!(kinds(&issues), [V::UnbalancedGroupEnd]);
}

#[test]
fn eof_issues_discarded_without_callbacks_are_not_replayed() {
    for input in [b"".as_slice(), b"{", b"{\\rtf1"] {
        let mut validator = RtfValidator::new(input);
        while validator.read_token().unwrap().is_some() {}
        assert!(
            validator
                .read_token_with_issues(|issue| panic!("replayed {issue:?}"))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn options_are_copied_and_new_validators_get_fresh_quotas() {
    let mut options = RtfValidationOptions {
        max_issues_per_violation: 1,
    };
    let input = b"{\\rtf1}}}";
    let mut first = RtfValidator::with_options(input.as_slice(), &options);
    options.max_issues_per_violation = 0;
    let mut second = RtfValidator::with_options(input.as_slice(), &options);
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
        kinds(&first_issues),
        [V::UnbalancedGroupEnd, V::TooManyComplianceIssues]
    );
    assert_eq!(
        kinds(&second_issues),
        [V::UnbalancedGroupEnd, V::UnbalancedGroupEnd]
    );
    assert_eq!(validate(input, 1, 1).1, first_issues);
    assert_eq!(RtfValidationOptions::default().max_issues_per_violation, 0);
}

#[test]
fn source_access_preserves_unconsumed_bytes_and_does_not_reset_eof() {
    let mut validator = RtfValidator::new(b"{\\rtf1 x}".as_slice());
    assert_eq!(*validator.get_ref(), b"{\\rtf1 x}");
    validator.read_token().unwrap().unwrap();
    assert_eq!(*validator.get_ref(), b"\\rtf1 x}");
    assert_eq!(validator.into_inner(), b"\\rtf1 x}");

    let mut validator = RtfValidator::new(b"".as_slice());
    assert!(validator.read_token().unwrap().is_none());
    *validator.get_mut() = b"{\\rtf1 new}";
    assert!(
        validator
            .read_token_with_issues(|_| panic!("EOF must remain sticky"))
            .unwrap()
            .is_none()
    );
    assert_eq!(validator.into_inner(), b"{\\rtf1 new}");
}

#[test]
fn every_lexical_issue_converts_without_losing_its_position() {
    for (lexical, violation) in [
        (RtfLexicalViolation::KeywordTooLong, V::KeywordTooLong),
        (RtfLexicalViolation::InvalidParameter, V::InvalidParameter),
        (RtfLexicalViolation::InvalidHexEscape, V::InvalidHexEscape),
        (RtfLexicalViolation::TruncatedStream, V::TruncatedStream),
        (
            RtfLexicalViolation::TooManyComplianceIssues,
            V::TooManyComplianceIssues,
        ),
    ] {
        let expected = issue(violation, 123, 4, 17, 9);
        assert_eq!(
            RtfIssue::from(RtfLexicalIssue {
                violation: lexical,
                position: expected.position,
            }),
            expected
        );
    }
}

struct FaultReader<'a> {
    input: &'a [u8],
    capacity: usize,
    calls: usize,
    fail_at: usize,
    kind: io::ErrorKind,
}

impl Read for FaultReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let buffer = self.fill_buf()?;
        let count = buffer.len().min(output.len());
        output[..count].copy_from_slice(&buffer[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for FaultReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let call = self.calls;
        self.calls += 1;
        if call == self.fail_at {
            return Err(io::Error::new(self.kind, "injected source error"));
        }
        Ok(&self.input[..self.input.len().min(self.capacity)])
    }

    fn consume(&mut self, count: usize) {
        self.input = &self.input[count..];
    }
}

fn run_faulting(
    input: &[u8],
    capacity: usize,
    limit: usize,
    fail_at: usize,
    kind: io::ErrorKind,
) -> (Vec<RtfToken>, Vec<RtfIssue>, usize, usize) {
    let source = FaultReader {
        input,
        capacity,
        calls: 0,
        fail_at,
        kind,
    };
    let options = RtfValidationOptions {
        max_issues_per_violation: limit,
    };
    let mut validator = RtfValidator::with_options(source, &options);
    assert_eq!(validator.get_ref().calls, 0, "construction must not read");
    let mut tokens = Vec::new();
    let mut issues = Vec::new();
    let mut errors = 0;
    loop {
        match validator.read_token_with_issues(|issue| issues.push(issue)) {
            Ok(Some(token)) => tokens.push(token),
            Ok(None) => break,
            Err(error) => {
                assert_eq!(error.kind(), kind);
                errors += 1;
                assert_eq!(errors, 1, "only one injected failure");
            }
        }
    }
    let calls = validator.get_ref().calls;
    for _ in 0..3 {
        assert!(
            validator
                .read_token_with_issues(|_| panic!("replayed at EOF"))
                .unwrap()
                .is_none()
        );
        assert!(validator.read_token().unwrap().is_none());
    }
    assert_eq!(validator.get_ref().calls, calls, "EOF must not read");
    (tokens, issues, errors, calls)
}

#[test]
fn errors_at_every_fill_preserve_tokens_diagnostics_and_budgets() {
    let word = format!("\\{}99999999999 ", "a".repeat(33));
    let input = format!("{{\\rtf1 {word}{word}\\'z\\'z\\bin5 }}\n{{\\x}}}}tail{{");
    for input in [input.as_bytes(), b"", b"{\r\n\\", b"}"] {
        for capacity in [1, 7, 4096] {
            for limit in [0, 1, 2] {
                let baseline =
                    run_faulting(input, capacity, limit, usize::MAX, io::ErrorKind::Other);
                assert_eq!(baseline.2, 0);
                for kind in [
                    io::ErrorKind::Interrupted,
                    io::ErrorKind::WouldBlock,
                    io::ErrorKind::Other,
                ] {
                    for fail_at in 0..baseline.3 {
                        let actual = run_faulting(input, capacity, limit, fail_at, kind);
                        assert_eq!(actual.0, baseline.0, "tokens: call={fail_at}");
                        assert_eq!(actual.1, baseline.1, "issues: call={fail_at}");
                        assert_eq!(actual.2, 1);
                    }
                }
            }
        }
    }
}

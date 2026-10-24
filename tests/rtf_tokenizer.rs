// Adapted from MimeKit's UnitTests/Text/RtfTokenizerTests.cs
// and RtfInterpreterComplianceTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Read};

use mimecraft::text::{
    RtfKeyword, RtfLexicalIssue, RtfLexicalViolation, RtfSourcePosition, RtfToken, RtfTokenKind,
    RtfTokenizer, RtfTokenizerOptions,
};

fn tokenize(input: &[u8], capacity: usize) -> (Vec<RtfToken>, Vec<RtfLexicalIssue>) {
    collect_tokens(RtfTokenizer::new(BufReader::with_capacity(capacity, input)))
}

fn collect_tokens<R: BufRead>(
    mut tokenizer: RtfTokenizer<R>,
) -> (Vec<RtfToken>, Vec<RtfLexicalIssue>) {
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

fn describe(tokens: &[RtfToken]) -> Vec<String> {
    let mut output = Vec::new();
    let mut text = Vec::new();
    for token in tokens {
        // Text boundaries depend on reader buffering; merge adjacent text
        // tokens before comparing, as in the upstream tests.
        if let RtfTokenKind::Text(bytes) = &token.kind {
            text.extend(bytes);
            continue;
        }
        if !text.is_empty() {
            output.push(format!(
                "text:{}",
                String::from_utf8(std::mem::take(&mut text)).unwrap()
            ));
        }
        output.push(match &token.kind {
            RtfTokenKind::GroupStart => "{".to_owned(),
            RtfTokenKind::GroupEnd => "}".to_owned(),
            RtfTokenKind::ControlSymbol(byte) => format!("symbol:{}", char::from(*byte)),
            RtfTokenKind::HexChar(byte) => format!("hex:{byte:02x}"),
            RtfTokenKind::ControlWord(word) => {
                let parameter = word
                    .parameter()
                    .map(|value| format!("={value}"))
                    .unwrap_or_default();
                let truncated = if word.is_name_truncated() {
                    " (truncated)"
                } else {
                    ""
                };
                format!(
                    "word:{}{parameter}{truncated} [{:?}]",
                    word.name(),
                    word.id()
                )
            }
            RtfTokenKind::Text(_) => unreachable!(),
        });
    }
    if !text.is_empty() {
        output.push(format!("text:{}", String::from_utf8(text).unwrap()));
    }
    output
}

#[test]
fn canonical_tokens_across_short_buffer_boundaries() {
    let input = b"{\\rtf1\\ansi Hello\r\n \\'E9\\\\\\{\\}\\fs-20\\foo x\\*\\\n\\par}";
    let expected = [
        "{",
        "word:rtf=1 [Rtf]",
        "word:ansi [Ansi]",
        "text:Hello ",
        "hex:e9",
        "symbol:\\",
        "symbol:{",
        "symbol:}",
        "word:fs=-20 [Fs]",
        "word:foo [Unknown]",
        "text:x",
        "symbol:*",
        "symbol:\n",
        "word:par [Par]",
        "}",
    ];
    for capacity in [1, 2, 3, 7, 4096] {
        let (tokens, issues) = tokenize(input, capacity);
        assert_eq!(describe(&tokens), expected, "capacity={capacity}");
        assert!(issues.is_empty());
    }
}

#[test]
fn malformed_escape_recovery_matches_upstream() {
    let cases: &[(&[u8], &[&str])] = &[
        (b"\\fs-x", &["word:fs [Fs]", "text:x"]),
        (b"\\fs- x", &["word:fs [Fs]", "text: x"]),
        (b"\\'zz", &["text:zz"]),
        (b"\\'a}", &["hex:0a", "}"]),
        (b"\\'A9\\'Fe", &["hex:a9", "hex:fe"]),
        (b"\\'\\b", &["word:b [B]"]),
        (b"\\'4", &["hex:04"]),
        (b"x\\'", &["text:x"]),
        (b"x\\", &["text:x"]),
        (b"\\fs", &["word:fs [Fs]"]),
        (b"\\fs-", &["word:fs [Fs]"]),
        (b"\\fs-12", &["word:fs=-12 [Fs]"]),
        (b"\\\r\\\t\\~", &["symbol:\n", "symbol:\t", "symbol:~"]),
        (b"\\bin-5 x", &["word:bin=-5 [Bin]", "text:x"]),
        (b"\\bin0 x", &["word:bin=0 [Bin]", "text:x"]),
        (b"a\r\n\r\nb", &["text:ab"]),
    ];
    for &(input, expected) in cases {
        for capacity in [1, 2, 3, 4096] {
            assert_eq!(
                describe(&tokenize(input, capacity).0),
                expected,
                "{input:?}, {capacity}"
            );
        }
    }
}

#[test]
fn a_single_space_is_consumed_only_after_a_word_or_digits() {
    for (input, expected) in [
        (r"\b  x", vec!["word:b [B]", "text: x"]),
        (r"\fs20  x", vec!["word:fs=20 [Fs]", "text: x"]),
        (r"\fs- x", vec!["word:fs [Fs]", "text: x"]),
        (r"\fs+20 x", vec!["word:fs [Fs]", "text:+20 x"]),
        ("\\b\tx", vec!["word:b [B]", "text:\tx"]),
        (r"\b{}x", vec!["word:b [B]", "{", "}", "text:x"]),
    ] {
        assert_eq!(describe(&tokenize(input.as_bytes(), 1).0), expected);
    }
}

#[test]
fn binary_data_is_skipped_before_interpreting_syntax() {
    for capacity in [1, 2, 7, 4096] {
        let (tokens, issues) = tokenize(b"{\\bin5 {}\\}{x}", capacity);
        assert_eq!(describe(&tokens), ["{", "word:bin=5 [Bin]", "text:x", "}"]);
        assert!(issues.is_empty());
        let (tokens, issues) = tokenize(b"{\\bin5{}\\}{x}", capacity);
        assert_eq!(describe(&tokens), ["{", "word:bin=5 [Bin]", "text:x", "}"]);
        assert!(issues.is_empty());
    }
    let (tokens, issues) = tokenize(b"\\bin4 \0\xff{\nZ", 1);
    assert_eq!(describe(&tokens), ["word:bin=4 [Bin]", "text:Z"]);
    assert!(issues.is_empty());
    assert_eq!(
        tokens.last().unwrap().position,
        RtfSourcePosition {
            offset: 10,
            line_number: 2,
            line_position: 1,
            depth: 0,
        }
    );
}

#[test]
fn huge_binary_lengths_do_not_allocate_or_consume_as_rtf() {
    for capacity in [1, 16, 4096] {
        let (tokens, issues) = tokenize(b"{\\bin2147483647 abc", capacity);
        assert_eq!(describe(&tokens), ["{", "word:bin=2147483647 [Bin]"]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, RtfLexicalViolation::TruncatedStream);
        assert_eq!(issues[0].position.offset, 19);
        assert_eq!(issues[0].position.depth, 1);
    }
    let (tokens, issues) = tokenize(b"\\bin3 abc", 1);
    assert_eq!(describe(&tokens), ["word:bin=3 [Bin]"]);
    assert!(issues.is_empty());
    let (tokens, issues) = tokenize(b"\\bin3", 1);
    assert_eq!(describe(&tokens), ["word:bin=3 [Bin]"]);
    assert_eq!(issues[0].violation, RtfLexicalViolation::TruncatedStream);
    assert_eq!(issues[0].position.offset, 5);
}

#[test]
fn binary_payloads_have_no_storage_proportional_to_their_length() {
    let size = 100_000;
    let mut input = format!("{{\\bin{size} ").into_bytes();
    input.extend(std::iter::repeat_n(b'{', size));
    input.extend_from_slice(b"x}");
    let (tokens, issues) = tokenize(&input, 7);
    assert_eq!(
        describe(&tokens),
        [
            "{".to_owned(),
            format!("word:bin={size} [Bin]"),
            "text:x".to_owned(),
            "}".to_owned(),
        ]
    );
    assert!(issues.is_empty());
    assert_eq!(tokens.last().unwrap().position.depth, 0);
}

#[test]
fn keyword_storage_is_bounded_and_overlong_words_stay_unknown() {
    for length in [31, 32, 33, 100_000] {
        let input = format!("\\{}123 x", "a".repeat(length));
        let (tokens, issues) = tokenize(input.as_bytes(), 7);
        let RtfTokenKind::ControlWord(word) = &tokens[0].kind else {
            panic!("expected word")
        };
        assert_eq!(word.name(), "a".repeat(length.min(32)));
        assert_eq!(word.id(), RtfKeyword::Unknown);
        assert_eq!(word.parameter(), Some(123));
        assert_eq!(word.is_name_truncated(), length > 32);
        assert!(!word.is_parameter_invalid());
        assert_eq!(issues.len(), usize::from(length > 32));
        if length > 32 {
            assert_eq!(issues[0].violation, RtfLexicalViolation::KeywordTooLong);
        }
    }
}

#[test]
fn parameters_keep_ten_digits_and_clamp_signed_boundaries() {
    for (digits, expected, invalid) in [
        ("0", 0, false),
        ("-0", 0, false),
        ("2147483647", i32::MAX, false),
        ("2147483648", i32::MAX, true),
        ("-2147483648", i32::MIN, false),
        ("-2147483649", i32::MIN, true),
        ("9999999999", i32::MAX, true),
        ("-9999999999", i32::MIN, true),
        ("0000000001", 1, false),
        ("00000000001", 0, true),
        ("00000000015", 1, true),
        ("-00000000015", -1, true),
    ] {
        for capacity in [1, 3, 4096] {
            let input = format!("\\fs{digits}");
            let (tokens, issues) = tokenize(input.as_bytes(), capacity);
            let RtfTokenKind::ControlWord(word) = &tokens[0].kind else {
                panic!("expected word")
            };
            assert_eq!(word.parameter(), Some(expected), "{digits}");
            assert_eq!(word.is_parameter_invalid(), invalid, "{digits}");
            assert_eq!(issues.len(), usize::from(invalid));
        }
    }
    for (digits, expected) in [
        ("9".repeat(100_000), i32::MAX),
        (format!("-{}", "9".repeat(100_000)), i32::MIN),
        ("0".repeat(100_000), 0),
    ] {
        let (tokens, issues) = tokenize(format!("\\fs{digits} x").as_bytes(), 5);
        let RtfTokenKind::ControlWord(word) = &tokens[0].kind else {
            panic!("expected word")
        };
        assert_eq!(word.parameter(), Some(expected));
        assert!(word.is_parameter_invalid());
        assert_eq!(issues.len(), 1);
        assert_eq!(describe(&tokens)[1], "text:x");
    }
}

#[test]
fn missing_parameters_and_eof_sign_quirk_are_preserved() {
    for (input, invalid) in [
        (r"\fs", false),
        (r"\fs-", false),
        (r"\fs- ", true),
        (r"\fs-x", true),
    ] {
        let (tokens, issues) = tokenize(input.as_bytes(), 1);
        let RtfTokenKind::ControlWord(word) = &tokens[0].kind else {
            panic!("expected word")
        };
        assert_eq!(word.parameter(), None);
        assert_eq!(word.is_parameter_invalid(), invalid);
        assert_eq!(issues.len(), usize::from(invalid));
    }
    let input = format!("\\{}{} ", "a".repeat(33), "9".repeat(11));
    let (_, issues) = tokenize(input.as_bytes(), 2);
    assert_eq!(
        issues
            .iter()
            .map(|issue| issue.violation)
            .collect::<Vec<_>>(),
        [
            RtfLexicalViolation::KeywordTooLong,
            RtfLexicalViolation::InvalidParameter,
        ]
    );
    assert_eq!(issues[0].position, issues[1].position);
}

#[test]
fn hexadecimal_bytes_and_malformed_diagnostics() {
    for value in 0u8..=255 {
        for input in [format!("\\'{value:02x}"), format!("\\'{value:02X}")] {
            let (tokens, issues) = tokenize(input.as_bytes(), 1);
            assert_eq!(tokens[0].kind, RtfTokenKind::HexChar(value));
            assert!(issues.is_empty());
        }
    }
    for input in [b"\\'".as_slice(), b"\\'z", b"\\'4", b"\\'4z", b"\\'\xff"] {
        let (_, issues) = tokenize(input, 1);
        assert_eq!(issues.len(), 1, "{input:?}");
        assert_eq!(issues[0].violation, RtfLexicalViolation::InvalidHexEscape);
        assert_eq!(issues[0].position.offset, 0);
    }
}

#[test]
fn byte_input_preserves_raw_values_and_non_ascii_symbols() {
    let input = b"\0\x80\xff\\\xfe\\abc\xc3\xa9";
    let (tokens, issues) = tokenize(input, 4096);
    assert!(issues.is_empty());
    assert_eq!(tokens[0].kind, RtfTokenKind::Text(vec![0, 0x80, 0xff]));
    assert_eq!(tokens[1].kind, RtfTokenKind::ControlSymbol(0xfe));
    let RtfTokenKind::ControlWord(word) = &tokens[2].kind else {
        panic!("expected word")
    };
    assert_eq!(word.name(), "abc");
    assert_eq!(tokens[3].kind, RtfTokenKind::Text(vec![0xc3, 0xa9]));
}

#[test]
fn large_source_buffers_still_produce_bounded_owned_text() {
    let input = vec![b'x'; 10_000];
    let mut tokenizer = RtfTokenizer::new(input.as_slice());
    let first = tokenizer.read_token().unwrap().unwrap();
    let second = tokenizer.read_token().unwrap().unwrap();
    let third = tokenizer.read_token().unwrap().unwrap();
    assert_eq!(first.kind, RtfTokenKind::Text(vec![b'x'; 4096]));
    assert_eq!(second.kind, first.kind);
    assert_eq!(third.kind, RtfTokenKind::Text(vec![b'x'; 1808]));
    assert_eq!(second.position.offset, 4096);
    assert_eq!(third.position.offset, 8192);
    assert!(tokenizer.read_token().unwrap().is_none());
    assert_eq!(first.kind, RtfTokenKind::Text(vec![b'x'; 4096]));
}

#[test]
fn positions_track_groups_line_feeds_and_skipped_binary_bytes() {
    let (tokens, issues) = tokenize(b"{{x}\n}\r\n}", 1);
    assert!(issues.is_empty()); // Unmatched ends are an interpreter concern.
    let positions: Vec<_> = tokens
        .iter()
        .map(|token| {
            (
                token.position.offset,
                token.position.line_number,
                token.position.line_position,
                token.position.depth,
            )
        })
        .collect();
    assert_eq!(
        positions,
        [
            (0, 1, 1, 0),
            (1, 1, 2, 1),
            (2, 1, 3, 2),
            (3, 1, 4, 1),
            (5, 2, 1, 0),
            (8, 3, 1, 0)
        ]
    );

    let (_, issues) = tokenize(b"{\\rtf1{\\pict\\bin5 a\n\nb\n}\\'zz}", 1);
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0],
        RtfLexicalIssue {
            violation: RtfLexicalViolation::InvalidHexEscape,
            position: RtfSourcePosition {
                offset: 24,
                line_number: 4,
                line_position: 2,
                depth: 1
            },
        }
    );
}

#[test]
fn truncation_is_reported_once_after_pending_tokens() {
    for (input, kinds) in [
        (
            "{\\'",
            vec![
                RtfLexicalViolation::InvalidHexEscape,
                RtfLexicalViolation::TruncatedStream,
            ],
        ),
        (
            "{\\'4",
            vec![
                RtfLexicalViolation::InvalidHexEscape,
                RtfLexicalViolation::TruncatedStream,
            ],
        ),
        ("{\\fs", vec![RtfLexicalViolation::TruncatedStream]),
        ("{\\bin100 x", vec![RtfLexicalViolation::TruncatedStream]),
    ] {
        let (_, issues) = tokenize(input.as_bytes(), 1);
        assert_eq!(
            issues
                .iter()
                .map(|issue| issue.violation)
                .collect::<Vec<_>>(),
            kinds
        );
        assert_eq!(issues.last().unwrap().position.offset, input.len() as u64);
    }
    let (_, issues) = tokenize(b"x\\", 1);
    assert!(issues.is_empty()); // Source intentionally drops a trailing backslash.
}

#[test]
fn keyword_aliases_are_case_sensitive_and_do_not_filter_destinations() {
    for (names, id) in [
        (
            &["ul", "uld", "ulthdashdd", "ululdbwave", "ulwave"][..],
            RtfKeyword::Ul,
        ),
        (&["cb", "chcbpat", "highlight"][..], RtfKeyword::Cb),
        (&["strike", "striked"][..], RtfKeyword::Strike),
        (&["par", "sect", "page"][..], RtfKeyword::Par),
        (&["cell", "nestcell"][..], RtfKeyword::Cell),
        (&["row", "nestrow"][..], RtfKeyword::Row),
        (
            &["pict", "objdata", "info", "mmathPr", "nonesttables"][..],
            RtfKeyword::SkipDestination,
        ),
        (
            &[
                "object", "result", "RTF", "mmathpr", "\\rtf", "rtf1", " rtf", "",
            ][..],
            RtfKeyword::Unknown,
        ),
    ] {
        for name in names {
            assert_eq!(RtfKeyword::from_name(name), id, "{name}");
        }
    }
    assert_eq!(
        RtfKeyword::from_name("nesttableprops"),
        RtfKeyword::Nesttableprops
    );
    assert_eq!(RtfKeyword::from_name("objattph"), RtfKeyword::Objattph);
    let (tokens, _) = tokenize(b"{\\pict visible}{\\mmathPr math}", 3);
    assert_eq!(
        describe(&tokens),
        [
            "{",
            "word:pict [SkipDestination]",
            "text:visible",
            "}",
            "{",
            "word:mmathPr [SkipDestination]",
            "text:math",
            "}",
        ]
    );
}

#[derive(Debug)]
struct FaultReader {
    bytes: Vec<u8>,
    offset: usize,
    fault_at: usize,
    fault_on_fill: usize,
    fired: bool,
    kind: io::ErrorKind,
    fills: usize,
}

impl BufRead for FaultReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.fills += 1;
        if !self.fired && (self.offset == self.fault_at || self.fills == self.fault_on_fill) {
            self.fired = true;
            return Err(io::Error::from(self.kind));
        }
        Ok(&self.bytes[self.offset..(self.offset + 1).min(self.bytes.len())])
    }

    fn consume(&mut self, count: usize) {
        self.offset += count;
    }
}

impl Read for FaultReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let buffer = self.fill_buf()?;
        let count = buffer.len().min(output.len());
        output[..count].copy_from_slice(&buffer[..count]);
        self.consume(count);
        Ok(count)
    }
}

#[test]
fn errors_at_every_byte_boundary_and_fill_preserve_tokens_and_issue_delivery() {
    let input = format!(
        "{{\\rtf1\\fs- x\\'zz\\'4z\\{}99999999999 \\bin5 a\n{{\\}}END",
        "a".repeat(33)
    )
    .into_bytes();
    let (expected_tokens, _) = tokenize(&input, 1);
    let mut baseline = RtfTokenizer::new(FaultReader {
        bytes: input.clone(),
        offset: 0,
        fault_at: usize::MAX,
        fault_on_fill: usize::MAX,
        fired: false,
        kind: io::ErrorKind::Other,
        fills: 0,
    });
    while baseline.read_token().unwrap().is_some() {}
    let fills = baseline.get_ref().fills;
    for limit in [0, 1, 2] {
        let options = RtfTokenizerOptions {
            max_issues_per_violation: limit,
        };
        let (_, expected_issues) = collect_tokens(RtfTokenizer::with_options(
            BufReader::with_capacity(1, input.as_slice()),
            &options,
        ));
        for kind in [
            io::ErrorKind::Interrupted,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Other,
        ] {
            let faults = (0..=input.len())
                .map(|offset| (offset, usize::MAX))
                .chain((1..=fills).map(|fill| (usize::MAX, fill)));
            for (fault_at, fault_on_fill) in faults {
                let source = FaultReader {
                    bytes: input.clone(),
                    offset: 0,
                    fault_at,
                    fault_on_fill,
                    fired: false,
                    kind,
                    fills: 0,
                };
                let mut tokenizer = RtfTokenizer::with_options(source, &options);
                let mut tokens = Vec::new();
                let mut issues = Vec::new();
                let mut errors = 0;
                loop {
                    match tokenizer.read_token_with_issues(|issue| issues.push(issue)) {
                        Ok(Some(token)) => tokens.push(token),
                        Ok(None) => break,
                        Err(error) => {
                            assert_eq!(error.kind(), kind);
                            errors += 1;
                            assert_eq!(errors, 1);
                        }
                    }
                }
                assert_eq!(errors, 1, "{fault_at}, {fault_on_fill}, {kind:?}");
                assert_eq!(
                    tokens, expected_tokens,
                    "{fault_at}, {fault_on_fill}, {kind:?}"
                );
                assert_eq!(
                    issues, expected_issues,
                    "{fault_at}, {fault_on_fill}, {kind:?}, limit={limit}"
                );
            }
        }
    }
}

#[test]
fn eof_is_sticky_and_source_recovery_retains_unconsumed_buffering() {
    let source = FaultReader {
        bytes: b"x".to_vec(),
        offset: 0,
        fault_at: usize::MAX,
        fault_on_fill: usize::MAX,
        fired: false,
        kind: io::ErrorKind::Other,
        fills: 0,
    };
    let mut tokenizer = RtfTokenizer::new(source);
    assert_eq!(tokenizer.get_ref().fills, 0);
    assert!(tokenizer.read_token().unwrap().is_some());
    assert!(tokenizer.read_token().unwrap().is_none());
    let fills = tokenizer.get_ref().fills;
    tokenizer.get_mut().bytes.extend_from_slice(b"more");
    for _ in 0..3 {
        assert!(tokenizer.read_token().unwrap().is_none());
    }
    assert_eq!(tokenizer.into_inner().fills, fills);

    let source = BufReader::with_capacity(100, &b"{rest}"[..]);
    let mut tokenizer = RtfTokenizer::new(source);
    assert_eq!(
        tokenizer.read_token().unwrap().unwrap().kind,
        RtfTokenKind::GroupStart
    );
    let mut source = tokenizer.into_inner();
    let mut remaining = Vec::new();
    source.read_to_end(&mut remaining).unwrap();
    assert_eq!(remaining, b"rest}");
}

#[test]
fn reporting_mode_does_not_change_tokens_or_defer_discarded_issues() {
    let input = b"{\\'zz\\fs9999999999999\\bin100 x";
    let expected = tokenize(input, 1).0;
    let mut tokenizer = RtfTokenizer::new(BufReader::with_capacity(1, input.as_slice()));
    let mut tokens = Vec::new();
    while let Some(token) = tokenizer.read_token().unwrap() {
        tokens.push(token);
    }
    assert_eq!(tokens, expected);
    let mut issues = Vec::new();
    assert!(
        tokenizer
            .read_token_with_issues(|issue| issues.push(issue))
            .unwrap()
            .is_none()
    );
    assert!(issues.is_empty());
}

#[test]
fn enabling_reporting_after_binary_data_retains_correct_line_positions() {
    let mut tokenizer = RtfTokenizer::new(b"\\bin3 \n\nxZ\\'z".as_slice());
    assert!(tokenizer.read_token().unwrap().is_some()); // bin
    assert!(tokenizer.read_token().unwrap().is_some()); // Z, after payload
    let mut issues = Vec::new();
    while tokenizer
        .read_token_with_issues(|issue| issues.push(issue))
        .unwrap()
        .is_some()
    {}
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].position,
        RtfSourcePosition {
            offset: 10,
            line_number: 3,
            line_position: 3,
            depth: 0,
        }
    );
}

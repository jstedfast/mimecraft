// Adapted from MimeKit's MimeComplianceEncodedWordTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeComplianceContext as Context, MimeComplianceIssue as Issue,
    MimeComplianceOptions as Options, MimeCompliancePositionKind as PositionKind,
    MimeComplianceValidators as Validators, MimeComplianceViolation as Violation,
    MimeEntityParseOptions, MimeEventFormat, MimeEventParser, MimeParser, MimeReader,
};
use std::io::{self, BufReader, Cursor};

#[path = "support/encoded_words.rs"]
mod encoded_words;

const INVALID: Violation = Violation::InvalidEncodedWord;
const PLACEMENT: Violation = Violation::InvalidEncodedWordPlacement;

fn assert_words(issues: &[Issue], invalid: usize, placement: usize) {
    assert_eq!(
        issues.iter().filter(|i| i.violation == INVALID).count(),
        invalid,
        "{issues:?}"
    );
    assert_eq!(
        issues.iter().filter(|i| i.violation == PLACEMENT).count(),
        placement,
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .filter(|i| matches!(i.violation, INVALID | PLACEMENT))
            .all(|i| i.position_kind == PositionKind::ElementStart)
    );
}

fn read(text: &str, capacity: usize, options: Options) -> Vec<Issue> {
    let mut reader = MimeReader::new(BufReader::with_capacity(
        capacity,
        Cursor::new(text.as_bytes()),
    ));
    reader.set_compliance_options(options);
    let mut issues = Vec::new();
    reader
        .read_message_with_logger(
            &mut |issue| {
                issues.push(issue);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
    issues
}

fn feed(text: &str, options: Options) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut parser = MimeEventParser::with_logger(
        MimeEventFormat::Message,
        Default::default(),
        options,
        &mut logger,
    )
    .unwrap();
    for byte in text.as_bytes() {
        assert_eq!(
            parser
                .parse(std::slice::from_ref(byte), |_| Ok(()))
                .unwrap(),
            1
        );
    }
    parser.finish(|_| Ok(())).unwrap();
    issues
}

#[test]
fn upstream_encoded_word_cases() {
    assert_eq!(encoded_words::SYNTAX.len(), 40);
    assert_eq!(encoded_words::ADDRESSES.len(), 37);
    for (header, invalid, placement) in encoded_words::headers() {
        let text = format!("{header}\r\n\r\nbody\r\n");
        let issues = read(&text, 4096, Options::default());
        assert_words(&issues, invalid, placement);
        assert_eq!(read(&text, 1, Options::default()), issues, "{header}");
        assert_eq!(feed(&text, Options::default()), issues, "{header}");
    }
}

#[test]
fn gating_preserves_decoded_values_body_and_serialization() {
    let mut expected = None;
    for capacity in [1, 4096] {
        for enabled_validators in [
            Validators::NONE,
            Validators::PARAMETERS,
            Validators::ENCODED_WORDS,
            Validators::ALL,
            Validators::BASE64
                | Validators::QUOTED_PRINTABLE
                | Validators::UU_ENCODE
                | Validators::PARAMETERS,
        ] {
            for logging in [false, true] {
                let mut parser = MimeParser::new(BufReader::with_capacity(
                    capacity,
                    Cursor::new(encoded_words::PRESERVED.as_bytes()),
                ));
                parser.set_compliance_options(Options {
                    enabled_validators,
                    ..Default::default()
                });
                let mut issues = Vec::new();
                let mut message = if logging {
                    parser
                        .parse_message_with_logger(&mut |issue| {
                            issues.push(issue);
                            Ok(())
                        })
                        .unwrap()
                } else {
                    parser.parse_message().unwrap()
                };
                let to = format!("{:?}", message.to());
                let subject = message.subject();
                let body = message.text_body(mimecraft::TextFormat::Plain).unwrap();
                assert_eq!(body.as_deref(), Some("body\r\n"));
                let mut serialized = Vec::new();
                message.write_to(&mut serialized).unwrap();
                let actual = (to, subject, body, serialized);
                assert_eq!(&actual, expected.get_or_insert_with(|| actual.clone()));
                let enabled = logging && enabled_validators.contains(Validators::ENCODED_WORDS);
                assert_words(&issues, if enabled { 2 } else { 0 }, usize::from(enabled));
            }
        }
    }
}

#[test]
fn positions_and_independent_budgets() {
    let text = encoded_words::POSITIONS;
    for max_issues_per_violation in [0, 1] {
        let options = Options {
            enabled_validators: Validators::ENCODED_WORDS,
            context: Context::Storage,
            max_issues_per_violation,
        };
        let issues = read(text, 1, options);
        assert_words(
            &issues,
            if max_issues_per_violation == 0 { 2 } else { 1 },
            1,
        );
        let words: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.violation, INVALID | PLACEMENT))
            .collect();
        for (issue, marker) in
            words
                .iter()
                .zip(["=?utf-8?Q?bad=GG", "=?utf-8?Q?other", "=?utf-8?Q?bad=G?"])
        {
            let offset = text.find(marker).unwrap();
            assert_eq!(issue.stream_offset, offset as u64);
            let prefix = &text.as_bytes()[..offset];
            assert_eq!(
                issue.line_number,
                1 + prefix.iter().filter(|&&b| b == b'\n').count() as u64
            );
            assert_eq!(
                issue.column_number,
                prefix
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(offset + 1, |lf| offset - lf) as u64
            );
            assert_eq!(issue.context, Context::Storage);
        }
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::TooManyComplianceIssues)
                .count(),
            max_issues_per_violation
        );
        assert_eq!(read(text, 4096, options), issues);
        assert_eq!(feed(text, options), issues);
    }
}

#[test]
fn nested_entities_and_case_insensitive_fields_keep_original_positions() {
    let text = "Content-Type: multipart/mixed; boundary=x\r\n\r\n\
--x\r\nContent-Type: message/rfc822\r\n\
Content-Description: =?utf-8?Q?description=GG?=\r\n\r\n\
sUbJeCt: =?utf-8?Q?subject=GG?=\r\n\
x-example: =?utf-8?Q?extension=GG?=\r\n\
To: \"=?utf-8?Q?mailbox?=\" <a@example.com>\r\n\r\nbody\r\n--x--\r\n";
    let options = Options {
        enabled_validators: Validators::ENCODED_WORDS,
        ..Default::default()
    };
    let issues = read(text, 1, options);
    assert_words(&issues, 3, 1);
    let words = issues
        .iter()
        .filter(|i| matches!(i.violation, INVALID | PLACEMENT));
    for (issue, marker) in words.zip(["description", "subject", "extension", "mailbox"]) {
        let offset = text.find(&format!("=?utf-8?Q?{marker}")).unwrap();
        let prefix = &text.as_bytes()[..offset];
        assert_eq!(issue.stream_offset, offset as u64);
        assert_eq!(
            issue.line_number,
            1 + prefix.iter().filter(|&&b| b == b'\n').count() as u64
        );
        assert_eq!(
            issue.column_number,
            (offset - prefix.iter().rposition(|&b| b == b'\n').unwrap()) as u64
        );
    }
    assert_eq!(read(text, 4096, options), issues);
    assert_eq!(feed(text, options), issues);
}

#[test]
fn parameter_and_encoded_word_validators_are_independent() {
    let text = "Subject: =?utf-8?Q?bad=GG?=\r\nTo: =?utf-8?Q?bad=GG?= <local>\r\n\
Content-Disposition: attachment; filename=\"=?utf-8?Q?bad=GG?=\"\r\n\r\nbody\r\n";
    for enabled_validators in [
        Validators::NONE,
        Validators::PARAMETERS,
        Validators::ENCODED_WORDS,
        Validators::ALL,
    ] {
        let options = Options {
            enabled_validators,
            ..Default::default()
        };
        let issues = read(text, 1, options);
        assert_words(
            &issues,
            if enabled_validators.contains(Validators::ENCODED_WORDS) {
                2
            } else {
                0
            },
            0,
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::EncodedWordInParameter)
                .count(),
            usize::from(enabled_validators.contains(Validators::PARAMETERS))
        );
        // Address syntax validation is not implemented; encoded-word checks must not enable it.
        assert!(
            !issues
                .iter()
                .any(|i| i.violation == Violation::AddressWithoutDomain)
        );
        assert_eq!(feed(text, options), issues);
    }
}

#[test]
fn header_length_limits_precede_encoded_word_scans() {
    for header in [
        format!("Subject: =?utf-8?Q?bad=GG?= {}", "a".repeat(256)),
        format!("To: =?utf-8?Q?bad=GG?= <a@{}>", "a".repeat(256)),
    ] {
        let text = format!("{header}\r\n\r\n");
        let options = MimeEntityParseOptions {
            max_header_length: 128,
            ..Default::default()
        };
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut reader = MimeReader::with_options(
            BufReader::with_capacity(1, Cursor::new(text.as_bytes())),
            options,
        )
        .unwrap();
        assert!(
            reader
                .read_message_with_logger(&mut logger, |_| Ok(()))
                .is_err()
        );
        assert_words(&issues, 0, 0);
        issues.clear();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut parser = MimeEventParser::with_logger(
            MimeEventFormat::Message,
            options,
            Default::default(),
            &mut logger,
        )
        .unwrap();
        assert!(parser.parse(text.as_bytes(), |_| Ok(())).is_err());
        assert_words(&issues, 0, 0);
    }
}

#[test]
fn logger_errors_stop_reporting_and_poison_sans_io() {
    let text = b"To: \"=?utf-8?Q?bad=GG?=\" <a@example.com>\r\n\r\nbody";
    for rejected in [INVALID, PLACEMENT] {
        let mut seen = Vec::new();
        let mut logger = |issue: Issue| {
            seen.push(issue.violation);
            if issue.violation == rejected {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "encoded-word logger",
                ))
            } else {
                Ok(())
            }
        };
        let mut reader = MimeReader::new(Cursor::new(text));
        let error = reader
            .read_message_with_logger(&mut logger, |event| {
                assert!(matches!(event, mimecraft::MimeEvent::MessageBegin { .. }));
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(seen.last(), Some(&rejected));
        seen.clear();
        let mut logger = |issue: Issue| {
            seen.push(issue.violation);
            if issue.violation == rejected {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "encoded-word logger",
                ))
            } else {
                Ok(())
            }
        };
        let mut parser = MimeEventParser::with_logger(
            MimeEventFormat::Message,
            Default::default(),
            Default::default(),
            &mut logger,
        )
        .unwrap();
        assert_eq!(
            parser
                .parse(text, |event| {
                    assert!(matches!(event, mimecraft::MimeEvent::MessageBegin { .. }));
                    Ok(())
                })
                .and_then(|_| parser.finish(|event| {
                    assert!(matches!(event, mimecraft::MimeEvent::MessageBegin { .. }));
                    Ok(())
                }))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(parser.finish(|_| panic!("poisoned parser")).is_err());
        assert_eq!(seen.last(), Some(&rejected));
    }
}

// Adapted from MimeKit's MimeComplianceParameterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor};

use mimecraft::{
    MimeComplianceContext as Context, MimeComplianceIssue as Issue,
    MimeComplianceOptions as Options, MimeCompliancePositionKind as PositionKind,
    MimeComplianceValidators as Validators, MimeComplianceViolation as Violation, MimeEntity,
    MimeEntityParseOptions, MimeEvent, MimeEventFormat, MimeEventParser, MimeParser, MimeReader,
};

const BOUNDARY: Violation = Violation::RepeatedBoundaryParameter;
const FILENAME: Violation = Violation::RepeatedFilenameParameter;
const PARAMETER: Violation = Violation::RepeatedParameter;
const DISPOSITION: Violation = Violation::RepeatedContentDisposition;

fn is_parameter(issue: &Issue) -> bool {
    matches!(issue.violation, BOUNDARY | FILENAME | PARAMETER)
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
        MimeEntityParseOptions::default(),
        options,
        &mut logger,
    )
    .unwrap();
    for byte in text.as_bytes().chunks(1) {
        assert_eq!(parser.parse(byte, |_| Ok(())).unwrap(), 1);
    }
    parser.finish(|_| Ok(())).unwrap();
    drop(parser);
    issues
}

fn assert_parameters(issues: &[Issue], expected: &[Violation]) {
    let parameters: Vec<_> = issues.iter().filter(|i| is_parameter(i)).collect();
    assert_eq!(
        parameters.iter().map(|i| i.violation).collect::<Vec<_>>(),
        expected
    );
    assert!(
        parameters
            .iter()
            .all(|i| i.position_kind == PositionKind::Exact)
    );
}

fn assert_position(text: &str, marker: &str, issue: &Issue, kind: PositionKind) {
    let index = text.find(marker).unwrap();
    let (mut line, mut column) = (1, 1);
    for &byte in &text.as_bytes()[..index] {
        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    assert_eq!(issue.stream_offset, index as u64);
    assert_eq!(issue.line_number, line);
    assert_eq!(issue.column_number, column);
    assert_eq!(issue.position_kind, kind);
}

#[test]
fn upstream_repeated_parameter_cases() {
    // Regression cases inspired by https://github.com/noxxi/mime-is-broken.
    let cases: &[(&str, &str, &[Violation])] = &[
        (
            "Content-Type",
            "multipart/mixed; boundary=a; BOUNDARY=b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a; boundary=a; boundary=a",
            &[BOUNDARY, BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a; boundary*=''b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*=''a; boundary=b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*=''a; boundary*=''b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*0=a; boundary*1=b",
            &[],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*1=b; boundary*0*=us-ascii''a",
            &[],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*0=a; boundary*00*=b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a; boundary*0=b; boundary*1=c",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*0=a; boundary*1=b; boundary=c",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*=''a; boundary*0=b; boundary*1=c",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary*0=a; boundary*1=b; boundary*=''c",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a(comment; boundary=x); boundary (x) = b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=\"a\" boundary=b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a(comment)boundary=b",
            &[BOUNDARY],
        ),
        (
            "Content-Type",
            "multipart/mixed; boundary=a boundary=b",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=file.png; NAME=file.zip",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename=file.png; filename=file.zip",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename*=''a; filename*=''b",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename=a; filename*=''b",
            &[],
        ),
        (
            "Content-Disposition",
            "attachment; filename*=''a; filename=b",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=fallback; name*0*=utf-8''file; name*1=.zip",
            &[],
        ),
        (
            "Content-Disposition",
            "attachment; filename*0=a; filename*1=b; filename=fallback",
            &[],
        ),
        (
            "Content-Disposition",
            "attachment; filename=a; filename*=''b; filename=c",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename*=''a; filename=b; filename*=''c",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename=a; filename*0=b; filename*0*=c",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename*=''a; filename*0=b; filename*1=c",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename*0=a; filename*1=b; filename*=''c",
            &[FILENAME],
        ),
        (
            "Content-Disposition",
            "attachment; filename * (x) 0 (x) * =''a; FILENAME*0=b",
            &[FILENAME],
        ),
        (
            "Content-Type",
            "text/plain; charset=utf-8; CHARSET=us-ascii",
            &[PARAMETER],
        ),
        (
            "Content-Type",
            "text/plain; charset=utf-8; charset*=us-ascii''utf-8",
            &[PARAMETER],
        ),
        (
            "Content-Type",
            "text/plain; filename=a; filename=b",
            &[PARAMETER],
        ),
        (
            "Content-Disposition",
            "attachment; name=a; name=b",
            &[PARAMETER],
        ),
        (
            "Content-Disposition",
            "attachment; boundary=a; boundary=b",
            &[PARAMETER],
        ),
        (
            "Content-Disposition",
            "attachment; filename=\"a; filename=b\"",
            &[],
        ),
        (
            "Content-Disposition",
            "attachment; filename=\"a\\\"; filename=b\"; size=1",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=a (nested (name=x); name=y)",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=a (escaped \\); name=x)",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=\"a\\\"; name=x\"; name=b",
            &[FILENAME],
        ),
        (
            "Content-Type",
            "text/plain;; broken; name=a; name=b",
            &[FILENAME],
        ),
        (
            "Content-Type",
            "text/plain; name*999999999999999999=x; name=a",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name*2147483647=a; name*2147483647=b",
            &[FILENAME],
        ),
        ("Content-Type", "text/plain; name*0x=a; name=a", &[]),
        ("Content-Type", "text/plain; name=a; name=", &[FILENAME]),
        (
            "Content-Type",
            "text/plain; name=\"unterminated; name=b",
            &[],
        ),
        (
            "Content-Type",
            "text/plain; name=a (unterminated; name=b",
            &[],
        ),
        ("Content-Type", "text/plain", &[]),
        ("X-Parameters", "text/plain; name=a; name=b", &[]),
    ];
    assert_eq!(cases.len(), 48);
    for &(field, value, expected) in cases {
        let text = format!("{field}: {value}\r\n\r\nbody\r\n");
        let issues = read(&text, 4096, Options::default());
        assert_parameters(&issues, expected);
        for capacity in [1, 7] {
            assert_eq!(read(&text, capacity, Options::default()), issues, "{text}");
        }
        assert_eq!(feed(&text, Options::default()), issues, "{text}");
    }
}

const FOLDED: &str = "X-Prefix: \u{e9}\r\nContent-Type \t: text/plain; charset=utf-8;\r\n\
\t(comment; charset=x)\r\n\t(\u{e9}) CHARSET=us-ascii; charset=iso-8859-1\r\n\r\nbody\r\n";
const REPEATED: &str = "Content-Type: text/plain\r\n\
Content-Type: text/plain; name=a; name=b\r\n\
Content-Disposition: attachment; filename=a\r\n\
Content-Disposition: inline; filename=b; filename=c\r\n\
Content-Disposition: attachment\r\n\r\nbody\r\n";
const NESTED: &str = "Content-Type: multipart/mixed; boundary=b\r\nContent-Disposition: inline\r\n\r\n\
--b\r\nContent-Disposition: attachment; filename=a\r\n\r\none\r\n\
--b\r\nContent-Type: message/rfc822\r\nContent-Disposition: inline\r\n\r\n\
Content-Disposition: inline\r\nContent-Disposition: attachment\r\n\r\ntwo\r\n--b--\r\n";
const LIMITED: &str = "Content-Type: text/plain; x=1; x=2; x=3; x=4; boundary=a; boundary=b\r\n\
Content-Disposition: attachment; filename=a; filename=b; filename=c\r\n\
Content-Disposition: inline\r\nContent-Disposition: attachment\r\n\r\nbody\r\n";

#[test]
fn folded_positions_are_byte_based_and_account_for_raw_field_whitespace() {
    for capacity in [1, 7, 4096] {
        let issues = read(FOLDED, capacity, Options::default());
        assert_parameters(&issues, &[PARAMETER, PARAMETER]);
        let parameters: Vec<_> = issues.iter().filter(|i| is_parameter(i)).collect();
        assert_position(FOLDED, "CHARSET=", parameters[0], PositionKind::Exact);
        assert_position(FOLDED, "charset=iso", parameters[1], PositionKind::Exact);
        assert_eq!(feed(FOLDED, Options::default()), issues);
    }
}

#[test]
fn every_header_is_checked_and_parameters_are_independently_selectable() {
    assert_eq!(Validators::PARAMETERS.bits(), 16);
    assert_eq!(Validators::ALL.bits(), 23);
    assert!(Validators::default().contains(Validators::PARAMETERS));
    assert_eq!(
        Validators::BASE64
            | Validators::QUOTED_PRINTABLE
            | Validators::UU_ENCODE
            | Validators::PARAMETERS,
        Validators::ALL
    );
    for enabled_validators in [
        Validators::ALL,
        Validators::PARAMETERS,
        Validators::BASE64,
        Validators::NONE,
    ] {
        let options = Options {
            enabled_validators,
            ..Default::default()
        };
        for capacity in [1, 4096] {
            let issues = read(REPEATED, capacity, options);
            let reported: Vec<_> = issues
                .iter()
                .filter(|i| {
                    is_parameter(i)
                        || matches!(i.violation, DISPOSITION | Violation::RepeatedContentType)
                })
                .map(|i| i.violation)
                .collect();
            assert_eq!(
                reported,
                if enabled_validators.contains(Validators::PARAMETERS) {
                    vec![
                        FILENAME,
                        Violation::RepeatedContentType,
                        FILENAME,
                        DISPOSITION,
                        DISPOSITION,
                    ]
                } else {
                    vec![Violation::RepeatedContentType, DISPOSITION, DISPOSITION]
                }
            );
            assert_parameters(
                &issues,
                if enabled_validators.contains(Validators::PARAMETERS) {
                    &[FILENAME, FILENAME]
                } else {
                    &[]
                },
            );
            let repeated: Vec<_> = issues
                .iter()
                .filter(|i| i.violation == DISPOSITION)
                .collect();
            assert_eq!(repeated.len(), 2);
            assert_position(
                REPEATED,
                "Content-Disposition: inline",
                repeated[0],
                PositionKind::ElementStart,
            );
            assert_position(
                REPEATED,
                "Content-Disposition: attachment\r\n",
                repeated[1],
                PositionKind::ElementStart,
            );
            assert_eq!(feed(REPEATED, options), issues);
        }
    }
}

#[test]
fn disposition_state_resets_for_each_nested_header_block() {
    for capacity in [1, 4096] {
        let issues = read(NESTED, capacity, Options::default());
        let repeated: Vec<_> = issues
            .iter()
            .filter(|i| i.violation == DISPOSITION)
            .collect();
        assert_eq!(repeated.len(), 1);
        assert_position(
            NESTED,
            "Content-Disposition: attachment\r\n",
            repeated[0],
            PositionKind::ElementStart,
        );
        assert_eq!(feed(NESTED, Options::default()), issues);
    }
}

#[test]
fn disposition_state_and_parameter_budgets_reset_between_mbox_messages() {
    for headers in [
        "Content-Disposition: inline",
        "Content-Disposition: inline; x=1; x=2; x=3",
    ] {
        let message =
            format!("From sender@example.com Mon Jan 1 00:00:00 2024\r\n{headers}\r\n\r\nbody\r\n");
        let text = message.repeat(2);
        let mut parser = MimeParser::new(BufReader::with_capacity(1, Cursor::new(text.as_bytes())));
        parser.set_compliance_options(Options {
            max_issues_per_violation: 1,
            ..Default::default()
        });
        for index in 0..2 {
            let mut issues = Vec::new();
            assert!(
                parser
                    .parse_mbox_message_with_logger(&mut |issue| {
                        issues.push(issue);
                        Ok(())
                    })
                    .unwrap()
                    .is_some()
            );
            assert!(!issues.iter().any(|i| i.violation == DISPOSITION));
            let parameters: Vec<_> = issues.iter().filter(|i| is_parameter(i)).collect();
            if headers.contains(';') {
                assert_eq!(parameters.len(), 1);
                assert_eq!(
                    parameters[0].stream_offset,
                    (index * message.len() + message.find("x=2").unwrap()) as u64
                );
                assert_eq!(
                    issues
                        .iter()
                        .filter(|i| i.violation == Violation::TooManyComplianceIssues)
                        .count(),
                    1
                );
            } else {
                assert!(parameters.is_empty());
            }
        }
        assert!(
            parser
                .parse_mbox_message_with_logger(&mut |_| Ok(()))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn header_length_limit_precedes_parameter_validation() {
    let text = format!(
        "Content-Type: text/plain; x=1; x=2; x={}\r\n\r\n",
        "a".repeat(256)
    );
    for capacity in [1, 4096] {
        let mut parser = MimeParser::with_options(
            BufReader::with_capacity(capacity, Cursor::new(text.as_bytes())),
            MimeEntityParseOptions {
                max_header_length: 128,
                ..Default::default()
            },
        )
        .unwrap();
        let mut issues = Vec::new();
        let error = parser
            .parse_message_with_logger(&mut |issue| {
                issues.push(issue);
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_parameters(&issues, &[]);
    }
}

#[test]
fn new_violations_have_separate_reporting_budgets() {
    let options = Options {
        max_issues_per_violation: 1,
        context: Context::Storage,
        ..Default::default()
    };
    let issues = read(LIMITED, 1, options);
    for violation in [
        BOUNDARY,
        FILENAME,
        PARAMETER,
        DISPOSITION,
        Violation::TooManyComplianceIssues,
    ] {
        assert_eq!(
            issues.iter().filter(|i| i.violation == violation).count(),
            1,
            "{violation:?}"
        );
    }
    assert!(issues.iter().all(|i| i.context == Context::Storage));
    assert_eq!(feed(LIMITED, options), issues);
}

#[test]
fn reporting_preserves_selected_headers_body_and_serialization() {
    let mut expected = None;
    for logging in [false, true] {
        for enabled_validators in [Validators::NONE, Validators::ALL] {
            let mut parser = MimeParser::new(BufReader::with_capacity(
                1,
                Cursor::new(REPEATED.as_bytes()),
            ));
            parser.set_compliance_options(Options {
                enabled_validators,
                ..Default::default()
            });
            let mut message = if logging {
                parser.parse_message_with_logger(&mut |_| Ok(())).unwrap()
            } else {
                parser.parse_message().unwrap()
            };
            let MimeEntity::Part(part) = message.body_mut().unwrap() else {
                panic!("expected text part")
            };
            assert_eq!(part.content_type().unwrap().unwrap().name(), None);
            assert_eq!(
                part.content_disposition().unwrap().unwrap().file_name(),
                Some("a")
            );
            let mut decoded = Vec::new();
            part.content_mut().decode_to(&mut decoded).unwrap();
            assert_eq!(decoded, b"body\r\n");
            let mut actual = Vec::new();
            message.write_to(&mut actual).unwrap();
            assert_eq!(&actual, expected.get_or_insert_with(|| actual.clone()));
        }
    }
}

#[test]
fn logger_errors_from_each_new_violation_and_the_budget_marker_propagate() {
    for rejected in [
        BOUNDARY,
        FILENAME,
        PARAMETER,
        DISPOSITION,
        Violation::TooManyComplianceIssues,
    ] {
        let mut reader = MimeReader::new(LIMITED.as_bytes());
        reader.set_compliance_options(Options {
            max_issues_per_violation: 1,
            ..Default::default()
        });
        let mut rejected_count = 0;
        let error = reader
            .read_message_with_logger(
                &mut |issue: Issue| {
                    assert_eq!(rejected_count, 0, "logging continued after rejection");
                    if issue.violation == rejected {
                        rejected_count += 1;
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "rejected parameter",
                        ));
                    }
                    Ok(())
                },
                |event| {
                    assert!(!matches!(event, MimeEvent::EntityBegin { .. }));
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "rejected parameter");
        assert_eq!(rejected_count, 1);
    }
}

#[test]
fn standalone_headers_report_parameters_and_repeated_disposition() {
    let mut parser = MimeParser::new(REPEATED.as_bytes());
    let mut issues = Vec::new();
    parser
        .parse_headers_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert_parameters(&issues, &[FILENAME, FILENAME]);
    assert_eq!(
        issues.iter().filter(|i| i.violation == DISPOSITION).count(),
        2
    );
    assert_eq!(parser.into_inner(), b"body\r\n");
}

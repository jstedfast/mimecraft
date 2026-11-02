// Adapted from MimeKit's MimeComplianceContentIdTests and repeated-header tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor};

use mimecraft::{
    HeaderId, MimeComplianceCategories as Categories, MimeComplianceContext as Context,
    MimeComplianceIssue as Issue, MimeComplianceOptions as Options,
    MimeCompliancePositionKind as PositionKind, MimeComplianceSeverity as Severity,
    MimeComplianceValidators as Validators, MimeComplianceViolation as V, MimeEntityParseOptions,
    MimeParser, MimeReader,
};

const NESTED: &str = concat!(
    "Content-Type: multipart/mixed; boundary=b\r\n",
    "Content-Id: <same@example.com>\r\nContent-Id: <outer@example.com>\r\n\r\n",
    "--b\r\nContent-Id: <same@example.com>\r\n\r\none\r\n",
    "--b\r\nContent-Type: multipart/alternative; boundary=c\r\nContent-Id: <same@example.com>\r\n\r\n",
    "--c\r\nContent-Id: <same@example.com>\r\nContent-Id: <child@example.com>\r\n\r\ntwo\r\n--c--\r\n",
    "--b\r\nContent-Type: message/rfc822\r\nContent-Id: <same@example.com>\r\n\r\n",
    "Content-Id: <same@example.com>\r\nContent-Id: <inner@example.com>\r\n\r\nthree\r\n",
    "--b\r\nContent-Id: <same@example.com>\r\n\r\nfour\r\n--b--\r\n",
);

const LIMITED: &str = concat!(
    "Content-Id: <first@example.com>\r\nContent-Id: <second@example.com>\r\n",
    "Content-Id: <third@example.com>\r\nContent-Id: <fourth@example.com>\r\n",
    "Content-Disposition: inline\r\nContent-Disposition: attachment\r\n\r\nbody\r\n",
);

fn read(text: &str, capacity: usize, message: bool, options: Options) -> Vec<Issue> {
    let mut reader = MimeReader::new(BufReader::with_capacity(
        capacity,
        Cursor::new(text.as_bytes()),
    ));
    reader.set_compliance_options(options);
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    if message {
        reader
            .read_message_with_logger(&mut logger, |_| Ok(()))
            .unwrap();
    } else {
        reader
            .read_entity_with_logger(&mut logger, |_| Ok(()))
            .unwrap();
    }
    issues
}

fn assert_content_ids(text: &str, issues: &[Issue], lines: &[u64]) {
    let repeated: Vec<_> = issues
        .iter()
        .filter(|i| i.violation == V::RepeatedContentId)
        .collect();
    assert_eq!(repeated.len(), lines.len(), "{text:?}");
    for (issue, &line) in repeated.iter().zip(lines) {
        let offset = text
            .as_bytes()
            .split_inclusive(|&b| b == b'\n')
            .take(line as usize - 1)
            .map(<[u8]>::len)
            .sum::<usize>();
        assert_eq!(issue.stream_offset, offset as u64, "{text:?}");
        assert_eq!(issue.line_number, line);
        assert_eq!(issue.column_number, 1);
        assert_eq!(issue.position_kind, PositionKind::ElementStart);
        assert_eq!(issue.severity(), Severity::Major);
        assert_eq!(
            issue.categories(),
            Categories::INTEROPERABILITY | Categories::SECURITY
        );
    }
}

#[test]
fn upstream_content_id_cases_report_each_additional_field_per_entity() {
    let cases: &[(&str, &[u64])] = &[
        (
            "Content-Id: <same@example.com>\r\nContent-Id: <same@example.com>\r\n\r\nbody\r\n",
            &[2],
        ),
        (
            "X-Prefix: \u{e9}\r\nContent-ID:\r\n <first@example.com>\r\ncOnTeNt-Id: <second@example.com>\r\nContent-Id: <third@example.com>\r\n\r\nbody\r\n",
            &[4, 5],
        ),
        (
            "Content-Id:\r\nContent-Id: <second@example.com>\r\n\r\nbody\r\n",
            &[2],
        ),
        (
            "Content-Id: invalid\r\nContent-Id: <second@example.com>\r\n\r\nbody\r\n",
            &[2],
        ),
        (
            "Content-Id: <first@example.com>\r\nContent-Id: invalid",
            &[2],
        ),
        ("Content-Id: <first@example.com>\r\nContent-Id:\r\n", &[2]),
        ("Content-Id: <first@example.com>\r\nContent-Id", &[]),
        (
            "Content-Id: <first@example.com>\r\n\tContent-Id: <second@example.com>\r\n\r\nbody\r\n",
            &[],
        ),
        ("Content-Id: <first@example.com>\r\n\r\nbody\r\n", &[]),
        (NESTED, &[3, 15, 24]),
    ];
    for &(text, lines) in cases {
        for capacity in [1, 2, 7, 4096] {
            for message in [false, true] {
                for enabled_validators in [Validators::NONE, Validators::ALL] {
                    let issues = read(
                        text,
                        capacity,
                        message,
                        Options {
                            enabled_validators,
                            ..Default::default()
                        },
                    );
                    assert_content_ids(text, &issues, lines);
                }
            }
        }
    }
}

#[test]
fn header_only_reporting_limits_and_validator_gating_are_independent() {
    for capacity in [1, 7, 4096] {
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, LIMITED.as_bytes()));
        let mut issues = Vec::new();
        reader
            .read_headers_with_logger(
                &mut |i| {
                    issues.push(i);
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        assert_content_ids(LIMITED, &issues, &[2, 3, 4]);
        assert_eq!(
            reader.position().offset,
            LIMITED.find("body").unwrap() as u64
        );
        for message in [false, true] {
            let issues = read(
                LIMITED,
                capacity,
                message,
                Options {
                    enabled_validators: Validators::NONE,
                    context: Context::Storage,
                    max_issues_per_violation: 1,
                },
            );
            assert_content_ids(LIMITED, &issues, &[2]);
            let capped: Vec<_> = issues
                .iter()
                .filter(|i| i.violation == V::TooManyComplianceIssues)
                .collect();
            assert_eq!(capped.len(), 1);
            assert_eq!(
                capped[0].stream_offset,
                LIMITED.find("Content-Id: <third").unwrap() as u64
            );
            assert_eq!(
                issues
                    .iter()
                    .filter(|i| i.violation == V::RepeatedContentDisposition)
                    .count(),
                1
            );
            assert!(issues.iter().all(|i| i.context == Context::Storage));
        }
    }
}

#[test]
fn content_id_state_and_report_budgets_reset_between_mbox_messages() {
    const MESSAGE: &str = "From sender@example.com Mon Jan 1 00:00:00 2024\r\nContent-Id: <same@example.com>\r\n\r\nbody\r\n";
    let duplicate = MESSAGE.replace("\r\n\r\n", "\r\nContent-Id: <other@example.com>\r\n\r\n");
    for capacity in [1, 7, 4096] {
        let text = format!("{MESSAGE}{MESSAGE}{duplicate}");
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let mut issues = Vec::new();
        for _ in 0..3 {
            assert!(
                reader
                    .read_mbox_message_with_logger(
                        &mut |i| {
                            issues.push(i);
                            Ok(())
                        },
                        |_| Ok(())
                    )
                    .unwrap()
            );
        }
        assert!(reader.is_end_of_stream());
        assert_content_ids(&text, &issues, &[11]);

        let capped_message =
            format!("From sender@example.com Mon Jan 1 00:00:00 2024\r\n{LIMITED}");
        let text = capped_message.repeat(2);
        let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
        reader.set_compliance_options(Options {
            max_issues_per_violation: 1,
            ..Default::default()
        });
        for _ in 0..2 {
            let mut issues = Vec::new();
            assert!(
                reader
                    .read_mbox_message_with_logger(
                        &mut |i| {
                            issues.push(i);
                            Ok(())
                        },
                        |_| Ok(())
                    )
                    .unwrap()
            );
            assert_eq!(
                issues
                    .iter()
                    .filter(|i| i.violation == V::RepeatedContentId)
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
        }
    }
}

#[test]
fn parsing_identifier_selection_and_serialization_do_not_depend_on_reporting() {
    for (first, expected) in [
        ("<first@example.com>", Some("first@example.com")),
        ("local", Some("local")),
        ("<missing", None),
        ("", None),
    ] {
        let text = format!(
            "Content-Type: text/plain\r\nContent-Id: {first}\r\nContent-Id: <second@example.com>\r\n\r\nbody\r\n"
        );
        for capacity in [1, 7, 4096] {
            for enabled_validators in [Validators::NONE, Validators::ALL] {
                for logging in [false, true] {
                    let mut parser =
                        MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
                    parser.set_compliance_options(Options {
                        enabled_validators,
                        ..Default::default()
                    });
                    let mut issues = Vec::new();
                    let mut message = if logging {
                        parser
                            .parse_message_with_logger(&mut |i| {
                                issues.push(i);
                                Ok(())
                            })
                            .unwrap()
                    } else {
                        parser.parse_message().unwrap()
                    };
                    let body = message.body().unwrap();
                    let id = body.content_id();
                    if let Some(expected) = expected {
                        assert_eq!(id.unwrap().unwrap().as_str(), expected);
                    } else {
                        assert!(
                            id.is_err(),
                            "invalid first identifier must not select a later duplicate"
                        );
                    }
                    assert_eq!(
                        body.headers()
                            .iter()
                            .filter(|h| h.id() == HeaderId::ContentId)
                            .count(),
                        2
                    );
                    let mut serialized = Vec::new();
                    message.write_to(&mut serialized).unwrap();
                    assert_eq!(serialized, text.as_bytes());
                    assert_content_ids(&text, &issues, if logging { &[3] } else { &[] });
                }
            }
        }
    }
}

#[test]
fn rejecting_repeated_ids_aborts_and_source_replacement_resets_state() {
    let mut reader = MimeReader::new(LIMITED.as_bytes());
    let error = reader
        .read_entity_with_logger(
            &mut |i: Issue| {
                assert_eq!(i.violation, V::RepeatedContentId);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "repeated identifier",
                ))
            },
            |_| panic!("rejected headers must not emit an entity"),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "repeated identifier");
    assert!(reader.read_entity(|_| Ok(())).is_err());
    reader.replace_source(b"Content-Id: <new@example.com>\r\n\r\nbody".as_slice());
    let mut issues = Vec::new();
    reader
        .read_entity_with_logger(
            &mut |i| {
                issues.push(i);
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
    assert!(issues.is_empty());

    let mut parser = MimeParser::new(LIMITED.as_bytes());
    let error = parser
        .parse_entity_with_logger(&mut |i: Issue| {
            assert_eq!(i.violation, V::RepeatedContentId);
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "repeated identifier",
            ))
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(parser.parse_entity().is_err());
}

#[test]
fn resent_block_resets_preserve_ordinary_message_header_bits() {
    for (field, repeated) in [
        ("Resent-Date", V::RepeatedResentDate),
        ("Resent-From", V::RepeatedResentFrom),
        ("Resent-Sender", V::RepeatedResentSender),
        ("Resent-To", V::RepeatedResentTo),
        ("Resent-Cc", V::RepeatedResentCc),
        ("Resent-Bcc", V::RepeatedResentBcc),
        ("Resent-Message-Id", V::RepeatedResentMessageId),
    ] {
        let text = format!(
            "From: one@example.com\r\nReturn-Path: <one@example.com>\r\n{field}: x\r\n{field}: y\r\nReceived: by relay\r\n{field}: z\r\nSubject: separator\r\n{field}: a\r\n{field}: b\r\nFrom: two@example.com\r\nReturn-Path: <two@example.com>\r\n\r\nbody\r\n"
        );
        for capacity in [1, 7, 4096] {
            let issues = read(&text, capacity, true, Options::default());
            let actual: Vec<_> = issues
                .iter()
                .filter(|i| {
                    matches!(i.violation, V::RepeatedFrom | V::RepeatedReturnPath)
                        || i.violation == repeated
                })
                .collect();
            assert_eq!(
                actual.iter().map(|i| i.violation).collect::<Vec<_>>(),
                [repeated, repeated, V::RepeatedFrom, V::RepeatedReturnPath]
            );
            assert_eq!(
                actual.iter().map(|i| i.line_number).collect::<Vec<_>>(),
                [4, 9, 10, 11]
            );
        }
    }
}

#[test]
fn upstream_content_length_range_and_first_valid_value_are_preserved() {
    const MARKER: &str = "From sender@example.com Mon Jan 1 00:00:00 2024\r\n";
    for (length, expected) in [
        ("0", 2),
        ("-1", 2),
        ("2147483647", 1),
        ("2147483648", 2),
        ("2147483648\r\nContent-Length: 2147483647", 1),
        ("0\r\nContent-Length: 2147483647", 2),
    ] {
        let text = format!(
            "{MARKER}Subject: first\r\nContent-Length: {length}\r\n\r\nbody\r\n{MARKER}Subject: second\r\n\r\nbody\r\n"
        );
        for capacity in [1, 7, 4096] {
            for logging in [false, true] {
                let mut reader = MimeReader::with_options(
                    BufReader::with_capacity(capacity, text.as_bytes()),
                    MimeEntityParseOptions {
                        respect_content_length: true,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut count = 0;
                loop {
                    let read = if logging {
                        reader
                            .read_mbox_message_with_logger(&mut |_| Ok(()), |_| Ok(()))
                            .unwrap()
                    } else {
                        reader.read_mbox_message(|_| Ok(())).unwrap()
                    };
                    if !read {
                        break;
                    }
                    count += 1;
                }
                assert_eq!(count, expected, "{length}, {capacity}, {logging}");
                assert!(reader.is_end_of_stream());
                assert_eq!(reader.position().offset, text.len() as u64);
            }
        }
    }
}

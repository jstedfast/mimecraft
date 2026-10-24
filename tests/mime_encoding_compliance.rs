// Adapted from MimeKit's streaming encoding validator and MimeReader tests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor};

use mimecraft::{
    MimeComplianceIssue as Issue, MimeComplianceOptions, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation, MimeContentKind, MimeEvent, MimeParser, MimeReader,
};

fn encoding_issue(issue: &Issue) -> bool {
    matches!(
        issue.violation,
        Violation::InvalidBase64Character
            | Violation::InvalidBase64Padding
            | Violation::Base64CharactersAfterPadding
            | Violation::IncompleteBase64Quantum
            | Violation::ObsoleteBase64Comment
            | Violation::InvalidQuotedPrintableEncoding
            | Violation::InvalidQuotedPrintableSoftBreak
            | Violation::InvalidUUEncodePretext
            | Violation::InvalidUUEncodeFileMode
            | Violation::InvalidUUEncodedLineLength
            | Violation::InvalidUUEncodedContent
            | Violation::InvalidUUEncodedLineExtraData
            | Violation::IncompleteUUEncodedLine
            | Violation::InvalidUUEncodeEndMarker
            | Violation::IncompleteUUEncodedContent
    )
}

fn scan(input: &[u8], capacity: usize, options: MimeComplianceOptions) -> (Vec<Issue>, Vec<u8>) {
    let mut reader = MimeReader::new(BufReader::with_capacity(capacity, Cursor::new(input)));
    reader.set_compliance_options(options);
    let mut issues = Vec::new();
    let mut body = Vec::new();
    reader
        .read_entity_with_logger(
            &mut |issue| {
                issues.push(issue);
                Ok(())
            },
            |event| {
                if let MimeEvent::Content {
                    bytes,
                    kind: MimeContentKind::Body,
                    ..
                } = event
                {
                    body.extend_from_slice(bytes);
                }
                Ok(())
            },
        )
        .unwrap();
    (issues, body)
}

#[test]
fn default_validators_report_without_changing_encoded_content() {
    for (encoding, content, expected) in [
        ("base64", "TQ", Violation::IncompleteBase64Quantum),
        (
            "quoted-printable",
            "text=",
            Violation::InvalidQuotedPrintableEncoding,
        ),
        (
            "x-uuencode",
            "begin 644 t\r\n#86)C\r\n",
            Violation::IncompleteUUEncodedContent,
        ),
    ] {
        let input = format!("Content-Transfer-Encoding: {encoding}\r\n\r\n{content}");
        let (reference, body) = scan(input.as_bytes(), input.len(), Default::default());
        assert_eq!(body, content.as_bytes());
        let defects: Vec<_> = reference.iter().filter(|i| encoding_issue(i)).collect();
        assert_eq!(defects.len(), 1, "{reference:?}");
        assert_eq!(defects[0].violation, expected);
        assert_eq!(defects[0].stream_offset, input.len() as u64);
        for capacity in [1, 2, 3, 17, 4096] {
            assert_eq!(
                scan(input.as_bytes(), capacity, Default::default()),
                (reference.clone(), body.clone())
            );
        }
        MimeParser::new(input.as_bytes()).parse_entity().unwrap();
        MimeParser::new(input.as_bytes())
            .parse_entity_with_logger(&mut |_| Ok(()))
            .unwrap();
    }
}

#[test]
fn large_encoded_bodies_preserve_events_and_diagnostics_across_refills() {
    for (encoding, body, expected) in [
        (
            "base64",
            format!("{}?TQ", "TWFu".repeat(32768)),
            vec![
                Violation::InvalidBase64Character,
                Violation::IncompleteBase64Quantum,
            ],
        ),
        (
            "quoted-printable",
            format!("{}=~\r\n{}=\r~", "x".repeat(131071), "y".repeat(65536)),
            vec![
                Violation::InvalidQuotedPrintableEncoding,
                Violation::InvalidQuotedPrintableSoftBreak,
            ],
        ),
        (
            "x-uuencode",
            format!("begin 644 t\r\n{}`\r\nend", "#86)C\r\n".repeat(18000)),
            vec![],
        ),
    ] {
        let input = format!("Content-Transfer-Encoding: {encoding}\r\n\r\n{body}");
        let (reference, content) = scan(input.as_bytes(), input.len(), Default::default());
        assert_eq!(content, body.as_bytes());
        let reference: Vec<_> = reference.into_iter().filter(encoding_issue).collect();
        assert_eq!(
            reference.iter().map(|i| i.violation).collect::<Vec<_>>(),
            expected
        );
        for capacity in [17, 4093, 8192, 65536] {
            let mut plain = MimeReader::new(BufReader::with_capacity(capacity, input.as_bytes()));
            let mut logged = MimeReader::new(BufReader::with_capacity(capacity, input.as_bytes()));
            let mut expected_events = Vec::new();
            let mut actual_events = Vec::new();
            plain
                .read_entity(|event| {
                    expected_events.push(format!("{event:?}"));
                    Ok(())
                })
                .unwrap();
            let mut issues = Vec::new();
            logged
                .read_entity_with_logger(
                    &mut |issue| {
                        if encoding_issue(&issue) {
                            issues.push(issue);
                        }
                        Ok(())
                    },
                    |event| {
                        actual_events.push(format!("{event:?}"));
                        Ok(())
                    },
                )
                .unwrap();
            assert_eq!(
                actual_events, expected_events,
                "{encoding}, capacity={capacity}"
            );
            assert_eq!(logged.position(), plain.position());
            assert_eq!(issues, reference, "{encoding}, capacity={capacity}");
        }
    }
}

#[test]
fn source_and_callback_errors_do_not_flush_partial_encoded_content() {
    struct Broken;
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "source stopped"))
        }
    }
    for (encoding, content) in [
        ("base64", "TQ"),
        ("quoted-printable", "="),
        ("x-uuencode", "begin 644 t\r\n#86)C\r\n"),
    ] {
        let input = format!("Content-Transfer-Encoding: {encoding}\r\n\r\n{content}");
        let source = io::Read::chain(input.as_bytes(), Broken);
        let mut parser = MimeParser::new(BufReader::with_capacity(7, source));
        let mut logger = |issue| {
            assert!(!encoding_issue(&issue));
            Ok(())
        };
        let error = parser.parse_entity_with_logger(&mut logger).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "source stopped");

        let mut reader = MimeReader::new(input.as_bytes());
        let error = reader
            .read_entity_with_logger(&mut logger, |event| {
                assert!(!matches!(event, MimeEvent::EntityEnd { .. }));
                if matches!(event, MimeEvent::Content { .. }) {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "callback stopped",
                    ))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "callback stopped");
    }
}

#[test]
fn validator_selection_does_not_disable_raw_byte_checks() {
    assert_eq!(Validators::default(), Validators::ALL);
    assert_eq!((Validators::BASE64 | Validators::UU_ENCODE).bits(), 5);
    assert_eq!(
        Validators::ALL & Validators::QUOTED_PRINTABLE,
        Validators::QUOTED_PRINTABLE
    );
    assert!(Validators::ALL.contains(Validators::BASE64 | Validators::QUOTED_PRINTABLE));
    assert!(!Validators::NONE.contains(Validators::BASE64));
    for (encoding, enabled, expected) in [
        (
            "base64",
            Validators::BASE64,
            Violation::InvalidBase64Character,
        ),
        (
            "quoted-printable",
            Validators::QUOTED_PRINTABLE,
            Violation::InvalidQuotedPrintableEncoding,
        ),
        (
            "x-uuencode",
            Validators::UU_ENCODE,
            Violation::InvalidUUEncodePretext,
        ),
    ] {
        let input = format!("Content-Transfer-Encoding: {encoding}\r\n\r\n?\0=~\n");
        let options = MimeComplianceOptions {
            enabled_validators: enabled,
            ..Default::default()
        };
        let (issues, body) = scan(input.as_bytes(), 1, options);
        assert!(issues.iter().any(|i| i.violation == expected));
        let options = MimeComplianceOptions {
            enabled_validators: Validators::NONE,
            ..Default::default()
        };
        let (issues, unvalidated) = scan(input.as_bytes(), 1, options);
        assert_eq!(unvalidated, body);
        assert!(!issues.iter().any(encoding_issue));
        assert!(
            issues
                .iter()
                .any(|i| i.violation == Violation::UnexpectedNullBytesInBody)
        );
        assert!(
            issues
                .iter()
                .any(|i| i.violation == Violation::BareLinefeedInBody)
        );
    }
}

#[test]
fn boundary_owned_newline_is_validated_but_delimiter_is_not() {
    for (encoding, content) in [
        ("base64", "TQ=="),
        ("quoted-printable", "="),
        ("x-uuencode", "begin 644 t\r\n`\r\nend"),
    ] {
        let input = format!(
            "Content-Type: multipart/mixed; boundary=b\r\n\r\npreamble\r\n\
            --b\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n{content}\r\n--b--\r\nepilogue"
        );
        for capacity in [1, 2, 17, 4096] {
            let (issues, body) = scan(input.as_bytes(), capacity, Default::default());
            assert_eq!(body, content.as_bytes());
            assert!(!issues.iter().any(encoding_issue), "{encoding}: {issues:?}");
        }
    }
    let input = "Content-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Transfer-Encoding: base64\r\n\r\nTQ\r\n--b--\r\n";
    let (issues, body) = scan(input.as_bytes(), 1, Default::default());
    assert_eq!(body, b"TQ");
    let issue = issues.iter().find(|i| encoding_issue(i)).unwrap();
    assert_eq!(issue.violation, Violation::IncompleteBase64Quantum);
    assert_eq!(issue.stream_offset, input.find("--b--").unwrap() as u64);
    assert_eq!(issue.column_number, 1);
}

#[test]
fn sibling_and_embedded_message_validators_are_independent() {
    let input = "Content-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Transfer-Encoding: base64\r\n\r\nTQ\r\n\
        --b\r\nContent-Type: message/rfc822\r\n\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n=~\r\n\
        --b\r\nContent-Transfer-Encoding: base64\r\n\r\nTQ\r\n--b--\r\n";
    for capacity in [1, 2, 17, 4096] {
        let (issues, body) = scan(input.as_bytes(), capacity, Default::default());
        assert_eq!(body, b"TQ=~TQ");
        let violations: Vec<_> = issues
            .iter()
            .filter(|i| encoding_issue(i))
            .map(|i| i.violation)
            .collect();
        assert_eq!(
            violations,
            [
                Violation::IncompleteBase64Quantum,
                Violation::InvalidQuotedPrintableEncoding,
                Violation::IncompleteBase64Quantum,
            ]
        );
    }
}

#[test]
fn finalization_logger_error_stops_end_events_and_discards_the_model() {
    #[derive(Debug)]
    struct Rejected;
    impl std::fmt::Display for Rejected {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("incomplete content")
        }
    }
    impl std::error::Error for Rejected {}
    let input = b"Content-Transfer-Encoding: base64\r\n\r\nTQ";
    let mut logger = |issue: Issue| {
        assert_eq!(issue.violation, Violation::IncompleteBase64Quantum);
        Err(io::Error::new(io::ErrorKind::PermissionDenied, Rejected))
    };
    let mut reader = MimeReader::new(input.as_slice());
    let mut content = Vec::new();
    let error = reader
        .read_entity_with_logger(&mut logger, |event| {
            assert!(!matches!(event, MimeEvent::EntityEnd { .. }));
            if let MimeEvent::Content { bytes, .. } = event {
                content.extend_from_slice(bytes);
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(content, b"TQ");
    assert!(error.get_ref().unwrap().is::<Rejected>());
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        reader.read_entity(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let mut parser = MimeParser::new(input.as_slice());
    assert!(
        parser
            .parse_entity_with_logger(&mut logger)
            .unwrap_err()
            .get_ref()
            .unwrap()
            .is::<Rejected>()
    );
    assert!(!parser.is_end_of_stream());
}

#[test]
fn encoding_diagnostics_use_shared_caps_and_propagate_write_errors() {
    let input = b"Content-Transfer-Encoding: quoted-printable\r\n\r\n=~=~\r\n=~=~\r\n=\r~";
    let options = MimeComplianceOptions {
        max_issues_per_violation: 1,
        ..Default::default()
    };
    let (issues, _) = scan(input, 1, options);
    assert_eq!(
        issues.iter().map(|i| i.violation).collect::<Vec<_>>(),
        [
            Violation::InvalidQuotedPrintableEncoding,
            Violation::TooManyComplianceIssues,
            Violation::InvalidQuotedPrintableSoftBreak,
        ]
    );
    let mut parser = MimeParser::new(input.as_slice());
    parser.set_compliance_options(options);
    let error = parser
        .parse_entity_with_logger(&mut |issue: Issue| {
            if issue.violation == Violation::TooManyComplianceIssues {
                Err(io::Error::other("capped"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "capped");
}

#[test]
fn mbox_validators_reset_and_report_archive_relative_positions() {
    let message = "From sender\r\nContent-Transfer-Encoding: base64\r\n\r\nTQ\r\n";
    let input = message.repeat(2);
    let mut parser = MimeParser::new(input.as_bytes());
    for n in 1..=2 {
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
        let issues: Vec<_> = issues.iter().filter(|i| encoding_issue(i)).collect();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::IncompleteBase64Quantum);
        assert_eq!(issues[0].stream_offset, (n * message.len()) as u64);
        assert_eq!(issues[0].column_number, 1);
    }
    assert!(
        parser
            .parse_mbox_message_with_logger(&mut |_| Ok(()))
            .unwrap()
            .is_none()
    );
}

#[test]
fn header_only_operations_do_not_construct_body_validators() {
    let input = b"Content-Transfer-Encoding: x-uuencode\r\n\r\nbody";
    let mut parser = MimeParser::new(input.as_slice());
    parser
        .parse_headers_with_logger(&mut |issue| {
            assert!(!encoding_issue(&issue));
            Ok(())
        })
        .unwrap();
    assert_eq!(parser.into_inner(), b"body");
}

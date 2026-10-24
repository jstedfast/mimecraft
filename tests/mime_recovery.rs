// Adapted from MimeKit's MimeReader and MimeParser recovery state machines.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeComplianceIssue as Issue, MimeCompliancePositionKind as PositionKind,
    MimeComplianceViolation as Violation, MimeEntity, MimeEntityParseOptions, MimeEvent,
    MimeParser, MimeReader,
};
use std::io::{self, BufReader};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn snapshot(entity: &MimeEntity) -> String {
    match entity {
        MimeEntity::Part(part) => format!("part({})", hex(part.content().get_ref().get_ref())),
        MimeEntity::Message(part) => format!(
            "message({})",
            part.message()
                .and_then(|message| message.body())
                .map(snapshot)
                .unwrap_or_default()
        ),
        MimeEntity::Multipart(part) => {
            let mut output = format!("multipart({};", hex(part.preamble()));
            for child in part.children() {
                output.push_str(&snapshot(child));
                output.push(';');
            }
            output.push_str(&hex(part.epilogue()));
            output.push(')');
            output
        }
    }
}

#[test]
fn canonical_recovery_topology_and_content_are_refill_independent() {
    let mut failures = Vec::new();
    for line in include_str!("mime_recovery_oracle.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('|').collect();
        let bytes: Vec<_> = (0..columns[1].len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&columns[1][index..index + 2], 16).unwrap())
            .collect();
        for capacity in [1, 2, 7, 4093, 8192] {
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, bytes.as_slice()));
            let entity = if columns[0].starts_with("message:") {
                parser
                    .parse_message()
                    .map(|mut message| message.set_body(None).unwrap())
            } else {
                parser.parse_entity()
            };
            let actual = match entity {
                Ok(entity) => snapshot(&entity),
                Err(_) => "error".into(),
            };
            if actual != columns[2] {
                failures.push(format!(
                    "{} / {capacity}: expected {:.200}, got {:.200}",
                    columns[0], columns[2], actual
                ));
                break;
            }
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, bytes.as_slice()));
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            let logged = if columns[0].starts_with("message:") {
                parser
                    .parse_message_with_logger(&mut logger)
                    .map(|mut message| message.set_body(None).unwrap())
            } else {
                parser.parse_entity_with_logger(&mut logger)
            };
            assert_eq!(
                logged
                    .map(|entity| snapshot(&entity))
                    .unwrap_or_else(|_| "error".into()),
                actual,
                "logged {}",
                columns[0]
            );
            assert!(
                issues
                    .iter()
                    .all(|issue| issue.stream_offset <= bytes.len() as u64),
                "{}",
                columns[0]
            );
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn upstream_reader_recovery_fixtures_preserve_issue_positions() {
    // MimeReaderTests.AssertMimeComplianceViolations checks both newline formats.
    // Async duplicates exercise the same recovery and are consolidated here.
    for (wire, violation, line, kind) in [
        (
            include_bytes!("data/compliance/invalid-content-type.eml").as_slice(),
            Violation::InvalidContentType,
            6,
            PositionKind::ElementStart,
        ),
        (
            include_bytes!("data/compliance/missing-body-separator.eml").as_slice(),
            Violation::MissingBodySeparator,
            8,
            PositionKind::Exact,
        ),
        (
            include_bytes!("data/compliance/missing-multipart-boundary-parameter.eml").as_slice(),
            Violation::MissingMultipartBoundaryParameter,
            6,
            PositionKind::ElementStart,
        ),
        (
            include_bytes!("data/compliance/missing-multipart-boundary.eml").as_slice(),
            Violation::MissingMultipartBoundary,
            19,
            PositionKind::Exact,
        ),
        (
            include_bytes!("data/compliance/missing-multipart-end-boundary.eml").as_slice(),
            Violation::MissingMultipartBoundary,
            19,
            PositionKind::Exact,
        ),
    ] {
        let unix = std::str::from_utf8(wire).unwrap().replace("\r\n", "\n");
        for wire in [unix.clone(), unix.replace('\n', "\r\n")] {
            let offset: usize = wire
                .as_bytes()
                .split_inclusive(|&byte| byte == b'\n')
                .take(line - 1)
                .map(<[u8]>::len)
                .sum();
            for capacity in [1, 7, 4096] {
                let mut reader =
                    MimeReader::new(BufReader::with_capacity(capacity, wire.as_bytes()));
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
                issues.retain(|issue| {
                    !matches!(
                        issue.violation,
                        Violation::BareLinefeedInHeader | Violation::BareLinefeedInBody
                    )
                });
                assert_eq!(issues.len(), 1, "{violation:?}: {issues:?}");
                let issue = issues[0];
                assert_eq!(issue.violation, violation);
                assert_eq!(issue.line_number, line as u64);
                assert_eq!(issue.column_number, 1);
                assert_eq!(issue.stream_offset, offset as u64);
                assert_eq!(issue.position_kind, kind);
            }
        }
    }
}

#[test]
fn upstream_invalid_known_header_at_eof_is_not_metadata() {
    // A header without ':' at EOF must not be interpreted as a known header:
    // its value would lie beyond the stream. A trailing newline is immaterial.
    for header in ["Content-Type", "Content-Transfer-Encoding"] {
        for ending in ["", "\r\n"] {
            let wire = format!("From: a@b.c\r\n{header}{ending}");
            let mut reader = MimeReader::new(wire.as_bytes());
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
            assert!(!issues.iter().any(|issue| matches!(
                issue.violation,
                Violation::InvalidContentType | Violation::InvalidContentTransferEncoding
            )));
            assert!(
                issues
                    .iter()
                    .all(|issue| issue.stream_offset <= wire.len() as u64)
            );
        }
    }
}

#[test]
fn malformed_parameters_keep_recognizable_types_and_report_missing_boundaries() {
    let wire = b"Content-Type: multipart/mixed; boundary=b; broken\r\n\r\n--b\r\n\r\nbody\r\n--b--";
    let mut parser = MimeParser::new(wire.as_slice());
    let mut issues = Vec::new();
    let MimeEntity::Multipart(part) = parser
        .parse_entity_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap()
    else {
        panic!("recognizable MIME type must survive broken parameters")
    };
    assert!(part.children().is_empty());
    assert_eq!(part.preamble(), b"--b\r\n\r\nbody\r\n--b--");
    assert!(
        issues
            .iter()
            .any(|issue| issue.violation == Violation::MissingMultipartBoundaryParameter)
    );
    assert!(
        !issues
            .iter()
            .any(|issue| issue.violation == Violation::InvalidContentType)
    );
}

#[test]
fn rejecting_recovery_stops_before_entity_completion() {
    let wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nSubject: truncated";
    let mut reader = MimeReader::new(wire.as_slice());
    let mut ends = 0;
    let error = reader
        .read_entity_with_logger(
            &mut |issue: Issue| {
                if issue.violation == Violation::IncompleteHeader {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "truncated header rejected",
                    ))
                } else {
                    Ok(())
                }
            },
            |event| {
                if matches!(event, MimeEvent::EntityEnd { .. }) {
                    ends += 1;
                }
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(ends, 0);
    assert_eq!(
        reader.read_entity(|_| Ok(())).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn ancestor_recovery_reports_the_marker_not_the_trimmed_content_end() {
    for tail in ["Subject: child\r\n", "\r\nbody\r\n"] {
        let wire = format!(
            "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=c\r\n\r\n--c\r\n{tail}--b--"
        );
        let offset = wire.rfind("--b--").unwrap();
        let line = wire[..offset].bytes().filter(|&byte| byte == b'\n').count() as u64 + 1;
        for capacity in [1, 7, 4096] {
            let mut reader = MimeReader::new(BufReader::with_capacity(capacity, wire.as_bytes()));
            let mut issues = Vec::new();
            reader
                .read_entity_with_logger(
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap();
            let expected_count = if tail.starts_with("Subject") { 2 } else { 1 };
            assert_eq!(issues.len(), expected_count, "{issues:?}");
            for issue in issues {
                assert_eq!(issue.stream_offset, offset as u64);
                assert_eq!(issue.line_number, line);
                assert_eq!(issue.column_number, 1);
                match issue.violation {
                    Violation::MissingBodySeparator => {
                        assert_eq!(issue.position_kind, PositionKind::ElementStart)
                    }

                    Violation::MissingMultipartBoundary => {
                        assert_eq!(issue.position_kind, PositionKind::Exact)
                    }
                    other => panic!("unexpected issue: {other:?}"),
                }
            }
        }
    }
}

#[test]
fn truncated_opening_marker_does_not_allocate_a_child_depth_level() {
    for marker in ["--b", "--b\r"] {
        let wire = format!("Content-Type: multipart/mixed; boundary=b\r\n\r\n{marker}");
        let options = MimeEntityParseOptions {
            max_depth: 1,
            ..Default::default()
        };
        let MimeEntity::Multipart(part) = MimeParser::with_options(wire.as_bytes(), options)
            .unwrap()
            .parse_entity()
            .unwrap()
        else {
            panic!("multipart expected")
        };
        assert!(part.children().is_empty());
        assert!(part.preamble().is_empty());
    }
    for marker in ["--b\n", "--b\r\n"] {
        let wire = format!("Content-Type: multipart/mixed; boundary=b\r\n\r\n{marker}");
        let options = MimeEntityParseOptions {
            max_depth: 1,
            ..Default::default()
        };
        assert_eq!(
            MimeParser::with_options(wire.as_bytes(), options)
                .unwrap()
                .parse_entity()
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn missing_multipart_markers_keep_the_physical_eof_column_when_unwinding() {
    for prefix in [
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=c\r\n\r\n--c\r\n\r\n",
    ] {
        for length in [1, 5, 4097] {
            let wire = format!("{prefix}{}", "x".repeat(length));
            for capacity in [1, 7, 4096] {
                let mut reader =
                    MimeReader::new(BufReader::with_capacity(capacity, wire.as_bytes()));
                let mut missing = Vec::new();
                reader
                    .read_entity_with_logger(
                        &mut |issue: Issue| {
                            if issue.violation == Violation::MissingMultipartBoundary {
                                missing.push(issue);
                            }
                            Ok(())
                        },
                        |_| Ok(()),
                    )
                    .unwrap();
                assert_eq!(
                    missing.len(),
                    if prefix.contains("boundary=c") { 2 } else { 1 }
                );
                for issue in missing {
                    assert_eq!(issue.stream_offset, wire.len() as u64);
                    assert_eq!(issue.column_number, length as u64 + 1);
                }
            }
        }
    }
}

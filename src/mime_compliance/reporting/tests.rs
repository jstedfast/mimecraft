// Adapted from MimeKit's MimeReaderScanContentTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;

fn scan_body(
    input: &[u8],
    encoding: ContentEncoding,
    chunk: usize,
    cap: usize,
    scanner: Scanner,
) -> Vec<MimeComplianceIssue> {
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut reporting = Reporting::new(
        Some(&mut logger),
        MimeComplianceOptions {
            enabled_validators: Validators::ALL,
            max_issues_per_violation: cap,
            ..Default::default()
        },
        false,
    );
    let mut position = MimePosition {
        offset: 73,
        line: 4,
    };
    let active = reporting.0.as_mut().unwrap();
    active.scanner = scanner;
    active.entity_begin(MimeEntityKind::Part, encoding, position);
    for bytes in input.chunks(chunk) {
        reporting.content(bytes, position).unwrap();
        position = position.advance(bytes);
    }
    reporting.0.as_mut().unwrap().finish_encoding().unwrap();
    issues
}

#[test]
fn fused_encoding_and_structural_scans_match_per_line_diagnostics() {
    let scanners = Scanner::available();
    let cases = [
        (ContentEncoding::Base64, "AAA\r\n".repeat(300).into_bytes()),
        (
            ContentEncoding::Base64,
            format!("{}Zh==\r\n", "AAAA\r\n".repeat(80)).into_bytes(),
        ),
        (
            ContentEncoding::Base64,
            format!("{}?*AAA?*\r\n{}=\r\n", "A".repeat(77), "A\r\n".repeat(60)).into_bytes(),
        ),
        (
            ContentEncoding::QuotedPrintable,
            "=41a=ff=\r\n".repeat(100).into_bytes(),
        ),
        (
            ContentEncoding::QuotedPrintable,
            format!(
                "{}--not-a-boundary\r\nok=20\r\n=GG\r\nbad=\rX=GG\r\nend=",
                "a=20\r\n".repeat(80)
            )
            .into_bytes(),
        ),
        (
            ContentEncoding::QuotedPrintable,
            "ok=20=\nplain\nbad=G\n".repeat(30).into_bytes(),
        ),
        (
            ContentEncoding::UuEncode,
            format!(
                "begin 644 file\r\n{}#36%N\r\n`\r\nend\r\n",
                format!("M{}\r\n", "A".repeat(60)).repeat(60)
            )
            .into_bytes(),
        ),
        (
            ContentEncoding::UuEncode,
            format!(
                "begin 644 file\r\n{}#A\r\n#AAAA!\r\n`\r\nbad",
                "#AAAA\r\n".repeat(40)
            )
            .into_bytes(),
        ),
        (
            ContentEncoding::SevenBit,
            [
                b"A\r\n".repeat(20),
                vec![b'a'; 1001],
                b"\0\xff\nmore\r\n".repeat(10),
            ]
            .concat(),
        ),
        (ContentEncoding::EightBit, b"\0\xff\r\nabc\n".repeat(80)),
        (ContentEncoding::Binary, b"\0\xff\r\nabc\n".repeat(80)),
    ];
    for (encoding, input) in cases {
        for chunk in [
            1,
            2,
            3,
            15,
            16,
            17,
            31,
            32,
            33,
            61,
            64,
            65,
            127,
            512,
            input.len(),
        ] {
            for cap in [0, 1, 3] {
                // Keep event segmentation identical: structural issues precede
                // encoding issues within each event, not across arbitrary refills.
                let expected = scan_body(&input, encoding, chunk, cap, Scanner::scalar());
                for &scanner in &scanners {
                    assert_eq!(
                        scan_body(&input, encoding, chunk, cap, scanner),
                        expected,
                        "encoding={encoding:?}, chunk={chunk}, cap={cap}, scanner={scanner:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn vector_edge_escapes_newlines_and_line_limits_match_per_line_scans() {
    let scanners = Scanner::available();
    for prefix in (0..=65).chain([997, 998, 999, 1000, 1001, 1032]) {
        for suffix in [
            b"=41\r\n".as_slice(),
            b"=\r\n",
            b"=\n",
            b"=G0\r\n",
            b"=0G\r\n",
            b"=0",
            b"=",
            b"\n",
            b"\r\n",
            b"\0\xff\r\n",
        ] {
            let input = [vec![b'A'; prefix], suffix.to_vec(), b"xyz\r\n".repeat(32)].concat();
            for chunk in [input.len(), 31, 64] {
                for encoding in [
                    ContentEncoding::QuotedPrintable,
                    ContentEncoding::Base64,
                    ContentEncoding::SevenBit,
                ] {
                    let expected = scan_body(&input, encoding, chunk, 0, Scanner::scalar());
                    for &scanner in &scanners {
                        assert_eq!(
                            scan_body(&input, encoding, chunk, 0, scanner),
                            expected,
                            "prefix={prefix}, suffix={suffix:?}, encoding={encoding:?}, chunk={chunk}, scanner={scanner:?}"
                        );
                    }
                }
            }
        }
    }
}

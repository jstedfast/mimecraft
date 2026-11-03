// Adapted from MimeKit's MimeReaderTests and MimeReaderScanContentTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::BufReader;

use mimecraft::{
    MimeComplianceIssue as Issue, MimeComplianceOptions,
    MimeCompliancePositionKind as PositionKind, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation, MimeEventFormat, MimeEventParser, MimeReader,
};

fn read(data: &[u8], chunk: usize, byte_fed: bool) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let options = MimeComplianceOptions {
        enabled_validators: Validators::NONE,
        max_issues_per_violation: 0,
        ..Default::default()
    };
    if byte_fed {
        let mut parser = MimeEventParser::with_logger(
            MimeEventFormat::Message,
            Default::default(),
            options,
            &mut logger,
        )
        .unwrap();
        for input in data.chunks(chunk) {
            assert_eq!(parser.parse(input, |_| Ok(())).unwrap(), input.len());
        }
        parser.finish(|_| Ok(())).unwrap();
    } else {
        let mut reader = MimeReader::new(BufReader::with_capacity(chunk, data));
        reader.set_compliance_options(options);
        reader
            .read_message_with_logger(&mut logger, |_| Ok(()))
            .unwrap();
    }
    issues
}

fn assert_exact(data: &[u8], issue: &Issue) {
    let offset = issue.stream_offset as usize;
    let prefix = &data[..offset];
    let line = prefix.iter().filter(|&&b| b == b'\n').count() as u64 + 1;
    let column = prefix
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(offset + 1, |lf| offset - lf) as u64;
    assert_eq!(issue.line_number, line, "{issue:?}");
    assert_eq!(issue.column_number, column, "{issue:?}");
    assert_eq!(issue.position_kind, PositionKind::Exact, "{issue:?}");
}

fn header_case(data: &[u8], nulls: &[usize], eight_bits: &[usize]) {
    let expected = read(data, data.len(), false);
    for (violation, offsets) in [
        (Violation::UnexpectedNullBytesInHeader, nulls),
        (Violation::Unexpected8BitBytesInHeader, eight_bits),
    ] {
        let actual: Vec<_> = expected
            .iter()
            .filter(|issue| issue.violation == violation)
            .collect();
        assert_eq!(
            actual
                .iter()
                .map(|i| i.stream_offset as usize)
                .collect::<Vec<_>>(),
            offsets
        );
        for issue in actual {
            assert_exact(data, issue);
            if violation == Violation::UnexpectedNullBytesInHeader {
                assert_eq!(data[issue.stream_offset as usize], 0);
            } else {
                assert!(data[issue.stream_offset as usize] >= 128);
            }
        }
    }
    for chunk in [
        1,
        2,
        3,
        7,
        15,
        16,
        17,
        31,
        32,
        33,
        63,
        64,
        65,
        127,
        128,
        129,
        255,
        256,
        257,
        1023,
        1024,
        4095,
        4096,
        data.len(),
    ] {
        for byte_fed in [false, true] {
            assert_eq!(
                read(data, chunk, byte_fed),
                expected,
                "chunk={chunk}, byte-fed={byte_fed}"
            );
        }
    }
}

#[test]
fn header_byte_positions_across_vector_and_refill_boundaries() {
    for newline in ["\n", "\r\n"] {
        for padding in [0, 15, 31, 63, 127, 4090] {
            for folded in [false, true] {
                let prefix = format!(
                    "X-First: ascii{newline}Subject: {}{}",
                    "a".repeat(padding),
                    if folded {
                        format!("{newline}\t")
                    } else {
                        String::new()
                    }
                );
                let mut data = prefix.as_bytes().to_vec();
                data.extend_from_slice(b"\0hello\xffworld\0\xfe");
                data.extend_from_slice(
                    format!("{newline}X-Last: ascii{newline}{newline}body").as_bytes(),
                );
                data.extend_from_slice(b"\0\xff");
                data.extend_from_slice(newline.as_bytes());
                header_case(&data, &[prefix.len()], &[prefix.len() + 6]);
            }
        }
    }
}

#[test]
fn header_bom_folds_invalid_fields_and_first_bytes() {
    for (data, nulls, eight_bits) in [
        (&b"Subject: \0\xff"[..], &[9][..], &[10][..]),
        (b"\xef\xbb\xbfSubject: \0\xff\r\n\r\n", &[12], &[13]),
        (
            b"\xef\xbb\xbfSubject: caf\xc3\xa9\r\n \xff\r\n\r\n",
            &[],
            &[20],
        ),
        (
            b"A: x\0\xff\0\xfe\r\nB: y\0\xfe\0\xff\r\n\r\n",
            &[4, 14],
            &[5, 15],
        ),
        (b"A: ok\r\nBad\0\xff: value\r\n\r\n", &[10], &[11]),
        (b"Subject: \xc3\xa9\0\xf0\x9f\x98\x80\r\n\r\n", &[11], &[]),
        (b"Subject: \xc3\xa9\r\n \xff\r\n\r\n", &[], &[14]),
        // Address-specific compliance is not ported yet. Retain the generic
        // issue rather than silently losing this defect when validators are off.
        (b"From: a@\xff.example\r\n\r\n", &[], &[8]),
        (b"Subject: ascii\r\n\r\nbody\0\xff\r\n", &[], &[]),
    ] {
        header_case(data, nulls, eight_bits);
    }
}

#[test]
fn header_malformed_utf8_points_to_leading_byte() {
    for newline in ["\n", "\r\n"] {
        for padding in [0, 31, 4090] {
            // Valid multibyte sequences precede the malformed sequence; report
            // neither the first non-ASCII byte nor a bad continuation byte.
            let prefix = format!(
                "X-First: ascii{newline}Subject: \u{e9} \u{65e5} \u{1f600}{newline}\t{}",
                "a".repeat(padding)
            );
            for malformed in [
                &b"\xff"[..],
                b"\x80",
                b"\xc0\xaf",
                b"\xe0\x80\x80",
                b"\xed\xa0\x80",
                b"\xf4\x90\x80\x80",
                b"\xc2!",
                b"\xe2\x82!",
                b"\xf0\x90\x80!",
                b"\xe2",
                b"\xf0\x90",
            ] {
                let mut data = prefix.as_bytes().to_vec();
                data.extend_from_slice(malformed);
                data.extend_from_slice(
                    format!("{newline}X-Last: ascii{newline}{newline}").as_bytes(),
                );
                header_case(&data, &[], &[prefix.len()]);
            }
        }
    }
    for suffix in [
        &b"\xe2\x82"[..],
        b"\xc3\r\n \xa9\r\n\r\n",
        b"\xff\xfe\r\n\r\n",
    ] {
        let prefix = "Subject: \u{e9} \u{1f600} ";
        let mut data = prefix.as_bytes().to_vec();
        data.extend_from_slice(suffix);
        header_case(&data, &[], &[prefix.len()]);
    }
    for (prefix, suffix) in [
        ("X-First: ascii\r\nBad-\u{e9}-", &b"\xff: value\r\n\r\n"[..]),
        (
            "Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nX-Name: \u{e9}\r\n \u{1f600} ",
            b"\xff\r\n\r\nbody\r\n--x--\r\n",
        ),
    ] {
        let mut data = prefix.as_bytes().to_vec();
        data.extend_from_slice(suffix);
        header_case(&data, &[], &[prefix.len()]);
    }
}

#[test]
fn content_byte_detection() {
    for encoding in ["7bit", "8bit", "binary"] {
        for padding in [0, 15, 16, 31, 32, 63, 64, 997, 4095] {
            for null_first in [false, true] {
                let (first, second) = if null_first { (0, 0xff) } else { (0xff, 0) };
                let mut data = format!(
                    "Content-Type: text/plain\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n"
                )
                .into_bytes();
                let prefix = vec![b'x'; padding];
                data.extend_from_slice(&prefix);
                data.extend_from_slice(b"\r\n");
                data.extend_from_slice(&prefix);
                data.extend_from_slice(&[first, b'\r', b'\n']);
                data.extend_from_slice(&prefix);
                data.extend_from_slice(&[second, b'\r', b'\n', first, second]);
                let expected = read(&data, data.len(), false);
                for (violation, count, line) in [
                    (
                        Violation::UnexpectedNullBytesInBody,
                        usize::from(encoding != "binary"),
                        if null_first { 5 } else { 6 },
                    ),
                    (
                        Violation::Unexpected8BitBytesInBody,
                        usize::from(encoding == "7bit"),
                        if null_first { 6 } else { 5 },
                    ),
                ] {
                    let issues: Vec<_> = expected
                        .iter()
                        .filter(|issue| issue.violation == violation)
                        .collect();
                    assert_eq!(issues.len(), count);
                    for issue in issues {
                        assert_eq!(issue.line_number, line);
                        assert_exact(&data, issue);
                    }
                }
                for chunk in [1, 2, 3, 7, 31, 33, 64, 4096, data.len()] {
                    for byte_fed in [false, true] {
                        assert_eq!(
                            read(&data, chunk, byte_fed),
                            expected,
                            "{encoding}, padding={padding}, null-first={null_first}, chunk={chunk}, byte-fed={byte_fed}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn body_issue_order_at_line_limit_and_unterminated_tail() {
    for padding in [998, 999, 1000, 1001, 4095] {
        for special in [0, 0xff] {
            for suffix in [&b""[..], b"\n", b"\r\n", b"\r"] {
                let mut data = b"Subject: body order\r\n\r\n".to_vec();
                let start = data.len();
                data.extend(std::iter::repeat_n(b'x', padding));
                data.push(special);
                data.extend_from_slice(suffix);
                let mut expected = Vec::new();
                let mut oversized = false;
                let mut cr = false;
                // Independent original bytewise ordering oracle.
                for (i, &byte) in data[start..].iter().enumerate() {
                    let column = i + 1;
                    if byte == special {
                        expected.push((
                            if special == 0 {
                                Violation::UnexpectedNullBytesInBody
                            } else {
                                Violation::Unexpected8BitBytesInBody
                            },
                            start + i,
                        ));
                    }
                    if !oversized && column > 1000 {
                        oversized = true;
                        expected.push((Violation::OversizedLine, start));
                    }
                    if byte == b'\n' && !cr {
                        expected.push((Violation::BareLinefeedInBody, start + i));
                    }
                    cr = byte == b'\r';
                }
                for chunk in [1, 31, 64, 997, 1000, 4096, data.len()] {
                    for byte_fed in [false, true] {
                        let issues = read(&data, chunk, byte_fed);
                        let actual: Vec<_> = issues
                            .iter()
                            .filter(|i| {
                                matches!(
                                    i.violation,
                                    Violation::UnexpectedNullBytesInBody
                                        | Violation::Unexpected8BitBytesInBody
                                        | Violation::OversizedLine
                                        | Violation::BareLinefeedInBody
                                )
                            })
                            .map(|i| (i.violation, i.stream_offset as usize))
                            .collect();
                        assert_eq!(
                            actual, expected,
                            "padding={padding}, special={special}, chunk={chunk}"
                        );
                    }
                }
            }
        }
    }
}

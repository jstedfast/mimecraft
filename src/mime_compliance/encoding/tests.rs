// Adapted from MimeKit's UnitTests/Encodings/*ValidatorTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;

fn validate(encoding: ContentEncoding, bytes: &[u8], chunk: usize) -> Vec<MimeComplianceIssue> {
    let mut issues = Vec::new();
    let mut validator = EncodingValidator::new(
        encoding,
        MimeComplianceOptions::default(),
        MimePosition::default(),
    )
    .unwrap();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    for bytes in bytes.chunks(chunk) {
        validator.write(bytes, &mut logger).unwrap();
    }
    validator.finish(&mut logger).unwrap();
    issues
}

fn check(encoding: ContentEncoding, input: &[u8], expected: &[(Violation, u64)]) {
    let reference = validate(encoding, input, 1);
    assert_eq!(
        reference
            .iter()
            .map(|i| (i.violation, i.stream_offset))
            .collect::<Vec<_>>(),
        expected,
        "{input:?}"
    );
    for chunk in [2, 3, 15, 16, 17, 31, 32, 33, 77, 4096] {
        assert_eq!(
            validate(encoding, input, chunk),
            reference,
            "chunk={chunk}, input={input:?}"
        );
    }
}

#[test]
fn base64_quantums_padding_and_per_line_latches() {
    use Violation::*;
    for input in [
        b"TWFu".as_slice(),
        b"TQ==",
        b"TWE=",
        b"T\r\nW F\tu",
        b"TQ=\r\n=",
    ] {
        // Base64 quantums may straddle line breaks: decoders ignore whitespace.
        check(ContentEncoding::Base64, input, &[]);
    }
    for (input, expected) in [
        ("=", vec![(InvalidBase64Padding, 0)]),
        ("T=", vec![(InvalidBase64Padding, 1)]),
        ("TQ===", vec![(InvalidBase64Padding, 4)]),
        ("TQ==\r\nT", vec![(Base64CharactersAfterPadding, 6)]),
        ("TQ=\r\n", vec![(IncompleteBase64Quantum, 5)]),
        (
            "????????\r\n????????\r\n",
            vec![(InvalidBase64Character, 0), (InvalidBase64Character, 10)],
        ),
        (
            "VGhp***********\r\ncyBp***********\r\n",
            vec![(ObsoleteBase64Comment, 4), (ObsoleteBase64Comment, 21)],
        ),
    ] {
        check(ContentEncoding::Base64, input.as_bytes(), &expected);
    }
}

#[test]
fn quoted_printable_states_and_independent_line_latches() {
    use Violation::*;
    for input in ["literal\r\ntext", "=C3=A9", "=\r\n", "=\n", "=ff"] {
        check(ContentEncoding::QuotedPrintable, input.as_bytes(), &[]);
    }
    for (input, expected) in [
        ("=XA", vec![(InvalidQuotedPrintableEncoding, 1)]),
        ("=AX", vec![(InvalidQuotedPrintableEncoding, 2)]),
        ("=A\n", vec![(InvalidQuotedPrintableEncoding, 2)]),
        ("=\rX", vec![(InvalidQuotedPrintableSoftBreak, 2)]),
        (
            "=~=~=~=~\r\n=~=~=~=~\r\n",
            vec![
                (InvalidQuotedPrintableEncoding, 1),
                (InvalidQuotedPrintableEncoding, 11),
            ],
        ),
        (
            "=~=\r~\r\n",
            vec![
                (InvalidQuotedPrintableEncoding, 1),
                (InvalidQuotedPrintableSoftBreak, 4),
            ],
        ),
        ("=", vec![(InvalidQuotedPrintableEncoding, 1)]),
        ("=A", vec![(InvalidQuotedPrintableEncoding, 2)]),
        ("=\r", vec![(InvalidQuotedPrintableSoftBreak, 2)]),
    ] {
        check(
            ContentEncoding::QuotedPrintable,
            input.as_bytes(),
            &expected,
        );
    }
    // Place escapes before, at and after the short scalar pass-through scan.
    for count in [0, 1, 15, 16, 17, 31, 32, 33, 100, 1000] {
        let input = format!(
            "{}=C3=A9\r\n{}=~=\r~\r\n",
            "x".repeat(count),
            "y".repeat(count)
        );
        check(
            ContentEncoding::QuotedPrintable,
            input.as_bytes(),
            &[
                (InvalidQuotedPrintableEncoding, (count * 2 + 9) as u64),
                (InvalidQuotedPrintableSoftBreak, (count * 2 + 12) as u64),
            ],
        );
    }
}

#[test]
fn uu_valid_markers_and_payload() {
    for input in [
        "begin 644 t.txt\r\n#86)C\r\n`\r\nend\r\n",
        " \t\r\nbegin 0644 t.txt\n`\nend",
    ] {
        check(ContentEncoding::UuEncode, input.as_bytes(), &[]);
    }
}

#[test]
fn all_byte_values_match_canonical_diagnostics_across_refills() {
    for line in include_str!("../../../tests/mime_encoding_oracle.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 3);
        let encoding = match fields[0] {
            "b64" => ContentEncoding::Base64,
            "qp" => ContentEncoding::QuotedPrintable,
            "uu" => ContentEncoding::UuEncode,
            _ => panic!("unknown fixture encoding"),
        };
        let bytes: Vec<_> = (0..fields[1].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&fields[1][i..i + 2], 16).unwrap())
            .collect();
        for chunk in [1, 2, 3, 17, 32, 4096] {
            let options = MimeComplianceOptions {
                context: MimeComplianceContext::Storage,
                ..Default::default()
            };
            let mut validator = EncodingValidator::new(
                encoding,
                options,
                MimePosition {
                    offset: 100,
                    line: 7,
                },
            )
            .unwrap();
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            for bytes in bytes.chunks(chunk) {
                validator.write(bytes, &mut logger).unwrap();
            }
            validator.finish(&mut logger).unwrap();
            let actual = issues
                .iter()
                .map(|i| {
                    assert_eq!(i.context, MimeComplianceContext::Storage);
                    assert_eq!(i.position_kind, MimeCompliancePositionKind::Exact);
                    format!(
                        "{},{},{},{}",
                        i.violation as u32, i.stream_offset, i.line_number, i.column_number
                    )
                })
                .collect::<Vec<_>>()
                .join(";");
            assert_eq!(actual, fields[2], "{line}, chunk={chunk}");
        }
    }
}
#[test]
fn uu_invalid_markers_line_lengths_and_extra_data() {
    use Violation::*;
    for (input, expected) in [
        ("", vec![(IncompleteUUEncodedContent, 0)]),
        (
            "b egin",
            vec![(InvalidUUEncodePretext, 1), (IncompleteUUEncodedContent, 6)],
        ),
        ("begin 12 t\n`\nend", vec![(InvalidUUEncodeFileMode, 8)]),
        ("begin 12345 t\n`\nend", vec![(InvalidUUEncodeFileMode, 10)]),
        (
            "begin 644 t\n#abc!\n`\nend",
            vec![(InvalidUUEncodedContent, 13)],
        ),
        (
            "begin 644 t\n#ABCD~~~~\n`\nend",
            vec![(InvalidUUEncodedLineExtraData, 17)],
        ),
        (
            "begin 644 t\n#ABC\n`\nend",
            vec![(IncompleteUUEncodedLine, 16)],
        ),
        ("begin 644 t\n`\nx", vec![(InvalidUUEncodeEndMarker, 14)]),
        (
            "begin 644 t\n`\nend\nx",
            vec![(InvalidUUEncodeEndMarker, 18)],
        ),
    ] {
        check(ContentEncoding::UuEncode, input.as_bytes(), &expected);
    }
    let input = format!("begin {} t\n`\nend", "6".repeat(1024));
    // A long mode must neither wrap its counter nor depend on refill boundaries.
    check(
        ContentEncoding::UuEncode,
        input.as_bytes(),
        &[(InvalidUUEncodeFileMode, 10)],
    );
}

// Adapted from MimeKit's Benchmarks/MimeComplianceValidatorBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fmt::Write;

use mimecraft::{
    MimeComplianceIssue, MimeComplianceOptions, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation, MimeReader,
};

pub const CASES: [&str; 22] = [
    "PlainSubject",
    "LongPlainSubject",
    "ShortQ",
    "ShortB",
    "FoldedQ",
    "FoldedB",
    "PlainDisplayName",
    "EncodedDisplayNames",
    "Comments",
    "QuotedWord",
    "MalformedQ",
    "MalformedB",
    "Unterminated",
    "SparseWords",
    "ParameterNone",
    "ParameterCharset",
    "ParameterFilename",
    "ParameterPair",
    "ParameterContinuation",
    "ParameterRepeated",
    "ParameterManyNames",
    "ParameterManySegments",
];

pub struct Workload {
    pub data: Vec<u8>,
    validators: Validators,
    expected: usize,
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(validate(self), self.expected);
    }
}

pub fn workload(name: &str) -> Workload {
    let mut field = if name.starts_with("Parameter") {
        "Content-Type"
    } else {
        "Subject"
    };
    let mut expected = 0;
    let text = match name {
        "PlainSubject" => " Monthly report\r\n".into(),
        "LongPlainSubject" => " Ordinary folded header text without encoded words.\r\n".repeat(80),
        "ShortQ" => " =?utf-8?Q?caf=C3=A9?=\r\n".into(),
        "ShortB" => " =?utf-8?B?Y2Fmw6k=?=\r\n".into(),
        "FoldedQ" => format!(" =?utf-8?Q?{}=20?=\r\n", "a".repeat(50)).repeat(64),
        "FoldedB" => format!(" =?utf-8?B?{}?=\r\n", "A".repeat(56)).repeat(64),
        "PlainDisplayName" => {
            field = "To";
            " Alice Example <alice@example.com>\r\n".into()
        }
        "EncodedDisplayNames" => {
            field = "To";
            let names = ["=?utf-8?Q?Alice_Example?= <alice@example.com>"; 20];
            format!(" {}\r\n", names.join(",\r\n "))
        }
        "Comments" => {
            field = "To";
            " (outer (=?utf-8?Q?caf=C3=A9?=)) Alice <alice@example.com>\r\n".into()
        }
        "QuotedWord" => {
            field = "To";
            expected = 1;
            " \"=?utf-8?Q?Alice_Example?=\" <alice@example.com>\r\n".into()
        }
        "MalformedQ" => {
            expected = 1;
            " =?utf-8?Q?bad=GG?=\r\n".into()
        }
        "MalformedB" => {
            expected = 1;
            " =?utf-8?B?YQ?=\r\n".into()
        }
        "Unterminated" => {
            expected = 128;
            format!(" {}\r\n", "=?utf-8?Q?unfinished".repeat(128))
        }
        "SparseWords" => {
            expected = 1;
            format!(
                " {}\r\n =?utf-8?Q?valid?=\r\n {}\r\n =?utf-8?Q?bad=GG?=\r\n",
                "a".repeat(2048),
                "a".repeat(2048)
            )
        }
        "ParameterNone" => " text/plain\r\n".into(),
        "ParameterCharset" => " text/plain; charset=utf-8\r\n".into(),
        "ParameterFilename" => {
            field = "Content-Disposition";
            " attachment; filename=\"report.pdf\"\r\n".into()
        }
        "ParameterPair" => " text/plain; charset=utf-8; format=flowed\r\n".into(),
        "ParameterContinuation" => {
            field = "Content-Disposition";
            " attachment; filename*0*=utf-8''monthly%20;\r\n filename*1*=report.pdf\r\n".into()
        }
        "ParameterRepeated" => {
            expected = 1;
            " multipart/mixed; boundary=one; boundary=two\r\n".into()
        }
        "ParameterManyNames" => {
            let mut text = String::from(" text/plain");
            for i in 0..100 {
                write!(text, "; x{i}=value").unwrap();
            }
            text.push_str("\r\n");
            text
        }
        "ParameterManySegments" => {
            field = "Content-Disposition";
            let mut text = String::from(" attachment; filename*0*=utf-8''a");
            for i in 1..100 {
                write!(text, "; filename*{i}*=a").unwrap();
            }
            text.push_str("\r\n");
            text
        }
        _ => panic!("unknown compliance workload: {name}"),
    };
    Workload {
        data: format!("{field}:{text}\r\n").into_bytes(),
        validators: if name.starts_with("Parameter") {
            Validators::PARAMETERS
        } else {
            Validators::ENCODED_WORDS
        },
        expected,
    }
}

pub fn validate(case: &Workload) -> usize {
    // The validators are private. Measure them through header reporting, including
    // raw header parsing/setup/drop, but without decoded address or message models.
    let mut reader = MimeReader::new(case.data.as_slice());
    reader.set_compliance_options(MimeComplianceOptions {
        enabled_validators: case.validators,
        max_issues_per_violation: 0,
        ..Default::default()
    });
    let mut count = 0;
    reader
        .read_headers_with_logger(
            &mut |issue: MimeComplianceIssue| {
                if matches!(
                    issue.violation,
                    Violation::InvalidEncodedWord
                        | Violation::InvalidEncodedWordPlacement
                        | Violation::InvalidParameter
                        | Violation::InvalidRfc2231Parameter
                        | Violation::EncodedWordInParameter
                        | Violation::RepeatedParameter
                        | Violation::RepeatedBoundaryParameter
                        | Violation::RepeatedFilenameParameter
                        | Violation::MissingParameterSegment
                ) {
                    count += 1;
                }
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
    count
}

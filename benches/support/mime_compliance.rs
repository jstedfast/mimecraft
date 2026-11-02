// Adapted from MimeKit's Benchmarks/MimeComplianceValidatorBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fmt::Write;

use mimecraft::{
    MimeComplianceIssue, MimeComplianceOptions, MimeComplianceValidators as Validators,
    MimeComplianceViolation as Violation, MimeReader,
};

pub const CASES: [&str; 34] = [
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
    "ContentIdSingle",
    "ContentIdRepeated",
    "ContentIdFolded",
    "ContentIdMany",
    "ContentIdNoLogger",
    "PadBitsBCanonical",
    "PadBitsBOne",
    "PadBitsBTwo",
    "Base64BodyCanonical",
    "Base64BodyPadBits",
    "Base64BodyWhitespace",
    "Base64BodyNoLogger",
];

pub struct Workload {
    pub data: Vec<u8>,
    validators: Validators,
    expected: usize,
    logging: bool,
    body: bool,
}

impl Workload {
    pub fn benchmark_name(&self) -> &'static str {
        if self.body {
            "MimeCompliance_ValidateBody"
        } else {
            "MimeCompliance_ValidateHeaders"
        }
    }

    pub fn check(&self) {
        assert_eq!(validate(self), self.expected);
    }
}

pub fn workload(name: &str) -> Workload {
    if name.starts_with("Base64Body") {
        let tail = match name {
            "Base64BodyCanonical" => "Zg==".into(),
            "Base64BodyPadBits" | "Base64BodyNoLogger" => "Zh==".into(),
            "Base64BodyWhitespace" => {
                format!("Zm9{}=", format!("\r\n{}", " ".repeat(64)).repeat(128))
            }
            _ => panic!("unknown compliance workload: {name}"),
        };
        return Workload {
            data: format!(
                "Content-Transfer-Encoding: base64\r\n\r\n{}{tail}\r\n",
                format!("{}\r\n", "A".repeat(76)).repeat(1024)
            )
            .into_bytes(),
            validators: Validators::BASE64,
            expected: usize::from(matches!(name, "Base64BodyPadBits" | "Base64BodyWhitespace")),
            logging: name != "Base64BodyNoLogger",
            body: true,
        };
    }
    let mut field = if name.starts_with("Parameter") {
        "Content-Type"
    } else if name.starts_with("ContentId") {
        "Content-Id"
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
        "PadBitsBCanonical" => " =?utf-8?B?Zg==?=\r\n".into(),
        "PadBitsBOne" => {
            expected = 1;
            " =?utf-8?B?Zm9=?=\r\n".into()
        }
        "PadBitsBTwo" => {
            expected = 1;
            " =?utf-8?B?Zh==?=\r\n".into()
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
        "ContentIdSingle" => " <first@example.com>\r\n".into(),
        "ContentIdRepeated" => {
            expected = 1;
            " <same@example.com>\r\nContent-Id: <same@example.com>\r\n".into()
        }
        "ContentIdFolded" => {
            field = "X-Prefix";
            expected = 2;
            " \u{e9}\r\nContent-ID:\r\n <first@example.com>\r\ncOnTeNt-Id: <second@example.com>\r\nContent-Id: <third@example.com>\r\n".into()
        }
        "ContentIdMany" | "ContentIdNoLogger" => {
            expected = if name == "ContentIdMany" { 255 } else { 0 };
            format!(
                " <first@example.com>\r\n{}",
                "Content-Id: <same@example.com>\r\n".repeat(255)
            )
        }
        _ => panic!("unknown compliance workload: {name}"),
    };
    Workload {
        data: format!("{field}:{text}\r\n").into_bytes(),
        validators: if name.starts_with("Parameter") {
            Validators::PARAMETERS
        } else if name.starts_with("ContentId") {
            Validators::NONE
        } else {
            Validators::ENCODED_WORDS
        },
        expected,
        logging: name != "ContentIdNoLogger",
        body: false,
    }
}

pub fn validate(case: &Workload) -> usize {
    // The validators are private. Measure them through reader reporting, including
    // raw parsing/setup/drop, but without decoded address or message models.
    let mut reader = MimeReader::new(case.data.as_slice());
    reader.set_compliance_options(MimeComplianceOptions {
        enabled_validators: case.validators,
        max_issues_per_violation: 0,
        ..Default::default()
    });
    if !case.logging {
        if case.body {
            reader.read_entity(|_| Ok(())).unwrap();
        } else {
            reader.read_headers(|_| Ok(())).unwrap();
        }
        return 0;
    }
    let mut count = 0;
    let mut logger = |issue: MimeComplianceIssue| {
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
                | Violation::RepeatedContentId
                | Violation::InvalidBase64Padding
        ) {
            count += 1;
        }
        Ok(())
    };
    if case.body {
        reader
            .read_entity_with_logger(&mut logger, |_| Ok(()))
            .unwrap();
    } else {
        reader
            .read_headers_with_logger(&mut logger, |_| Ok(()))
            .unwrap();
    }
    count
}

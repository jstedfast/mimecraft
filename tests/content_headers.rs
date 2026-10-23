// Compatibility vectors adapted from MimeKit's ContentTypeTests.cs and
// ContentDispositionTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentDispositionSizeError, ContentHeaderParseError,
    ContentHeaderParseErrorKind as ErrorKind, ContentType, DateTimeOffset, DateTimeOffsetError,
    HeaderParser, ParameterComplianceMode, ParameterIssueKind, ParameterList,
    ParameterParseErrorKind, ParameterParseOptions,
    charsets::{Charset, CharsetError},
    rfc2047::ComplianceMode,
};

#[test]
fn content_type_tokens_comments_and_matching() {
    let value = ContentType::parse(b" (a(b\\)c)) Text (x) / (y) Plain (z) ").unwrap();
    assert_eq!(value.media_type(), "Text");
    assert_eq!(value.media_subtype(), "Plain");
    assert_eq!(value.mime_type(), "Text/Plain");
    assert!(value.parameters().is_empty());
    for (media_type, subtype, expected) in [
        ("text", "plain", true),
        ("TEXT", "PLAIN", true),
        ("*", "plain", true),
        ("text", "*", true),
        ("*", "*", true),
        ("application", "*", false),
        ("*", "html", false),
        ("te*", "plain", false),
        ("text", "*+xml", false),
        ("", "", false),
    ] {
        assert_eq!(value.is_mime_type(media_type, subtype), expected);
    }
    for input in ["application/x-vnd.msdoc", "text/plain;", "text/plain; ; ;"] {
        assert!(
            input
                .parse::<ContentType>()
                .unwrap()
                .parameters()
                .is_empty()
        );
    }
    assert_eq!(value.clone(), value);
    // Preserve the source's distinct type-atom and subtype-token alphabets.
    assert_eq!(
        ContentType::parse(b"x=y?/a.b").unwrap().mime_type(),
        "x=y?/a.b"
    );
    assert_eq!(
        ContentType::parse(b"multipart/multipart/mixed; boundary=\"marker\"")
            .unwrap()
            .media_subtype(),
        "multipart/mixed"
    );
}

#[test]
fn common_content_type_parameters_and_charset_lookup() {
    let value = ContentType::parse(
        b"Text/Plain; CHARSET=utf-8; format=flowed; name*=utf-8''caf%C3%A9.txt; boundary=\"marker\"",
    ).unwrap();
    assert_eq!(value.charset(), Some("utf-8"));
    assert_eq!(value.charset_encoding(), Ok(Some(Charset::UTF8)));
    assert_eq!(value.format(), Some("flowed"));
    assert_eq!(value.name(), Some("caf\u{e9}.txt"));
    assert_eq!(value.boundary(), Some("marker"));
    assert_eq!(value.parameters().as_slice()[0].name(), "CHARSET");
    assert_eq!(value.mime_type(), "Text/Plain");
    let empty: ContentType = "text/plain".parse().unwrap();
    assert_eq!(empty.charset_encoding(), Ok(None));
    assert_eq!(empty.boundary(), None);
    assert_eq!(empty.format(), None);
    assert_eq!(empty.name(), None);
    for label in ["", "not-a-charset"] {
        let value: ContentType = format!("text/plain; charset=\"{label}\"").parse().unwrap();
        assert!(matches!(
            value.charset_encoding(),
            Err(CharsetError::UnsupportedLabel(_))
        ));
        assert_eq!(value.charset(), Some(label));
    }
}

#[test]
fn canonical_parameter_vectors() {
    for (input, name, expected) in [
        (
            "message/external-body; access-type=URL;\n URL*0=\"ftp://\";\n URL*1=\"cs.utk.edu/pub/moore/bulk-mailer/bulk-mailer.tar\"",
            "url",
            "ftp://cs.utk.edu/pub/moore/bulk-mailer/bulk-mailer.tar",
        ),
        (
            "application/x-stuff; title*=us-ascii'en-us'This%20is%20%2A%2A%2Afun%2A%2A%2A",
            "title",
            "This is ***fun***",
        ),
        (
            "application/x-stuff; title*1*=us-ascii'en'This%20is%20even%20more%20; title*2*=%2A%2A%2Afun%2A%2A%2A%20; title*3=\"isn't it!\"",
            "title",
            "This is even more ***fun*** isn't it!",
        ),
        (
            "application/x-stuff; title=\"some chinese characters =?big5?b?pKSk5Q==?= and stuff\"",
            "title",
            "some chinese characters \u{4e2d}\u{6587} and stuff",
        ),
        (
            "multipart/mixed;;\n Boundary=\"===========================_ _= 1212158(26598)\"",
            "boundary",
            "===========================_ _= 1212158(26598)",
        ),
    ] {
        let value: ContentType = input.parse().unwrap();
        assert_eq!(value.parameters().get(name), Some(expected));
    }
}

#[test]
fn disposition_tokens_and_filenames() {
    for (input, disposition, attached) in [
        (" (comment) ATTACHMENT (comment); ", "ATTACHMENT", true),
        ("inline", "inline", false),
        ("form-data; filename=\"form.txt\"", "form-data", false),
        ("x-custom/y=z", "x-custom/y=z", false),
    ] {
        let value: ContentDisposition = input.parse().unwrap();
        assert_eq!(value.disposition(), disposition);
        assert_eq!(value.is_attachment(), attached);
        assert_eq!(value.clone(), value);
    }
    assert_eq!(ContentDisposition::ATTACHMENT, "attachment");
    assert_eq!(ContentDisposition::INLINE, "inline");
    let value: ContentDisposition = "form-data; filename=\"form.txt\"; name=upload"
        .parse()
        .unwrap();
    assert_eq!(value.file_name(), Some("form.txt"));
    assert_eq!(value.parameters().get("name"), Some("upload"));
    for parameters in [
        "filename=\"Filename.doc\"; filename*0*=UTF-8''UnicodeFile; filename*1*=name.doc",
        "filename*0*=UTF-8''UnicodeFile; filename*1*=name.doc; filename=\"Filename.doc\"",
        "filename*0*=UTF-8''UnicodeFile; filename=\"Filename.doc\"; filename*1*=name.doc",
    ] {
        let value: ContentDisposition = format!("inline; {parameters}").parse().unwrap();
        assert_eq!(value.file_name(), Some("UnicodeFilename.doc"));
        assert!(
            value
                .parameters()
                .issues()
                .iter()
                .any(|issue| issue.kind == ParameterIssueKind::DuplicateParameter)
        );
    }
    let value =
        ContentDisposition::parse(b"attachment; filename=\"line\r\n\tbreak.doc\"; ignored=ok")
            .unwrap();
    assert_eq!(value.file_name(), Some("line\tbreak.doc"));
    let value = ContentDisposition::parse(b"attachment; filename*=utf-8''..%2Ffile%00%0A").unwrap();
    assert_eq!(value.file_name(), Some("../file\0\n"));
}

#[test]
fn typed_dates_size_and_absence() {
    let value = ContentDisposition::parse(
        b"attachment; filename=document.doc;\n\
          creation-date=\"Sat, 04 Jan 1997 15:22:17 -0400\";\n\
          modification-date=\"Thu, 04 Jan 2007 15:22:17 -0400\";\n\
          read-date=\"Wed, 04 Jan 2012 15:22:17 -0400\"; size=37001",
    )
    .unwrap();
    for (actual, text) in [
        (value.creation_date(), "Sat, 04 Jan 1997 15:22:17 -0400"),
        (value.modification_date(), "Thu, 04 Jan 2007 15:22:17 -0400"),
        (value.read_date(), "Wed, 04 Jan 2012 15:22:17 -0400"),
    ] {
        assert_eq!(
            actual,
            Ok(Some(DateTimeOffset::parse(text.as_bytes()).unwrap()))
        );
    }
    assert_eq!(value.size(), Ok(Some(37001)));
    let value: ContentDisposition = "inline".parse().unwrap();
    assert_eq!(value.creation_date(), Ok(None));
    assert_eq!(value.modification_date(), Ok(None));
    assert_eq!(value.read_date(), Ok(None));
    assert_eq!(value.size(), Ok(None));
    assert_eq!(value.file_name(), None);

    for text in ["", "   ", "not a date"] {
        let value: ContentDisposition = format!(
            "inline; creation-date=\"{text}\"; modification-date=\"{text}\"; read-date=\"{text}\""
        )
        .parse()
        .unwrap();
        assert_eq!(
            value.creation_date(),
            Err(DateTimeOffsetError::InvalidSyntax)
        );
        assert_eq!(
            value.modification_date(),
            Err(DateTimeOffsetError::InvalidSyntax)
        );
        assert_eq!(value.read_date(), Err(DateTimeOffsetError::InvalidSyntax));
        assert_eq!(value.parameters().get("creation-date"), Some(text));
    }
}

#[test]
fn size_digit_rules_and_exact_limits() {
    for (text, expected) in [
        ("0", 0),
        ("000123", 123),
        ("9223372036854775807", i64::MAX as u64),
    ] {
        let value: ContentDisposition = format!("attachment; size={text}").parse().unwrap();
        assert_eq!(value.size(), Ok(Some(expected)));
    }
    for text in ["", "-1", "+1", "1.0", "1e3", " 1", "1 ", "\u{0661}", "1\0"] {
        let value: ContentDisposition = format!("attachment; size=\"{text}\"").parse().unwrap();
        assert_eq!(
            value.size(),
            Err(ContentDispositionSizeError::InvalidDigits),
            "{text:?}"
        );
    }
    for text in [
        "9223372036854775808",
        "18446744073709551615",
        &"9".repeat(1000),
    ] {
        let value: ContentDisposition = format!("attachment; size={text}").parse().unwrap();
        assert_eq!(value.size(), Err(ContentDispositionSizeError::Overflow));
    }
    let value: ContentDisposition = format!("attachment; size={}1", "0".repeat(1000))
        .parse()
        .unwrap();
    assert_eq!(value.size(), Ok(Some(1)));
    let value = ContentDisposition::parse(b"inline; size*=us-ascii''%31%32").unwrap();
    assert_eq!(value.size(), Ok(Some(12)));
}

fn check_error(error: ContentHeaderParseError, kind: ErrorKind, offset: usize) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.offset(), offset);
    assert!(error.to_string().contains(&format!("byte {offset}")));
}

#[test]
fn content_type_syntax_errors_have_original_offsets() {
    for (input, kind, offset) in [
        ("", ErrorKind::InvalidMediaType, 0),
        (" \t", ErrorKind::InvalidMediaType, 2),
        (
            "application-x-gzip; name=document.xml.gz",
            ErrorKind::ExpectedSlash,
            18,
        ),
        (
            "\u{e5}pplication/octet-stream",
            ErrorKind::InvalidMediaType,
            0,
        ),
        ("application/\u{e5}tom", ErrorKind::InvalidMediaSubtype, 12),
        ("text/", ErrorKind::InvalidMediaSubtype, 5),
        ("text (x)", ErrorKind::ExpectedSlash, 8),
        (
            "application/octet-stream x",
            ErrorKind::ExpectedSeparator,
            25,
        ),
        ("(comment", ErrorKind::IncompleteComment, 0),
        ("text(comment", ErrorKind::IncompleteComment, 4),
        ("text/(comment", ErrorKind::IncompleteComment, 5),
        ("text/plain(comment", ErrorKind::IncompleteComment, 10),
        (
            "text/plain;(comment",
            ErrorKind::Parameter(ParameterParseErrorKind::IncompleteComment),
            11,
        ),
        (
            "text/plain; =",
            ErrorKind::Parameter(ParameterParseErrorKind::InvalidName),
            12,
        ),
        (
            "text/plain; name",
            ErrorKind::Parameter(ParameterParseErrorKind::IncompleteParameter),
            16,
        ),
        (
            "text/plain; name*0*=",
            ErrorKind::Parameter(ParameterParseErrorKind::IncompleteParameter),
            20,
        ),
        (
            "text/plain; name=\"unterminated",
            ErrorKind::Parameter(ParameterParseErrorKind::IncompleteQuotedValue),
            17,
        ),
    ] {
        check_error(
            ContentType::parse(input.as_bytes()).unwrap_err(),
            kind,
            offset,
        );
    }
}

#[test]
fn disposition_uses_throwing_parse_rules_and_explicit_ascii_errors() {
    for (input, kind, offset) in [
        (b" ".as_slice(), ErrorKind::InvalidDisposition, 1),
        (
            b" ; filename=\"test.txt\"",
            ErrorKind::InvalidDisposition,
            1,
        ),
        (
            b"\"inline\"; filename=\"test.txt\"",
            ErrorKind::InvalidDisposition,
            0,
        ),
        (b"\"\"", ErrorKind::InvalidDisposition, 0),
        (b"\"attachment", ErrorKind::InvalidDisposition, 0),
        (b"\x07ttachment", ErrorKind::InvalidDisposition, 0),
        (b"inline\xff", ErrorKind::InvalidDisposition, 6),
        (b"\xc3\xa9", ErrorKind::InvalidDisposition, 0),
        (b"attachment x", ErrorKind::ExpectedSeparator, 11),
        (b"attachment(comment", ErrorKind::IncompleteComment, 10),
        (
            b"attachment; name*",
            ErrorKind::Parameter(ParameterParseErrorKind::IncompleteParameter),
            17,
        ),
    ] {
        check_error(ContentDisposition::parse(input).unwrap_err(), kind, offset);
    }
}

#[test]
fn parameter_options_propagate_to_both_parsers() {
    for prefix in [b"text/plain; ".as_slice(), b"attachment; "] {
        let parse = |tail: &[u8], options: &ParameterParseOptions| {
            let input = [prefix, tail].concat();
            if prefix.starts_with(b"text") {
                ContentType::parse_with_options(&input, options).map(|v| v.parameters().clone())
            } else {
                ContentDisposition::parse_with_options(&input, options)
                    .map(|v| v.parameters().clone())
            }
        };
        let default = ParameterParseOptions::default();
        let strict = ParameterParseOptions {
            compliance_mode: ParameterComplianceMode::Strict,
            ..default
        };
        let tail = b"name=\"foo\" other=bar";
        assert_eq!(parse(tail, &default).unwrap().get("other"), Some("bar"));
        check_error(
            parse(tail, &strict).unwrap_err(),
            ErrorKind::Parameter(ParameterParseErrorKind::ExpectedSeparator),
            prefix.len() + 11,
        );
        let preferred = ParameterParseOptions {
            fallback_charset: Some(Charset::from_label("windows-1251").unwrap()),
            ..default
        };
        assert_eq!(
            parse(b"name=\"\xe0\"", &preferred).unwrap().get("name"),
            Some("\u{430}")
        );
        assert_eq!(
            parse(b"name=\"\xe0\"", &default).unwrap().get("name"),
            Some("\u{e0}")
        );
        assert_eq!(
            parse(b"name*=unknown''%E0", &preferred)
                .unwrap()
                .get("name"),
            Some("\u{e0}")
        );
        let strict_words = ParameterParseOptions {
            rfc2047_compliance_mode: ComplianceMode::Strict,
            ..default
        };
        let tail = b"name=\"pre=?utf-8?q?caf=C3=A9?=post\"";
        assert_eq!(
            parse(tail, &default).unwrap().get("name"),
            Some("precaf\u{e9}post")
        );
        assert_eq!(
            parse(tail, &strict_words).unwrap().get("name"),
            Some("pre=?utf-8?q?caf=C3=A9?=post")
        );
    }
}

#[test]
fn parameter_diagnostics_are_relative_to_whole_header() {
    let suffix = b" n=\"x\" n*=utf-8''%FF; x*1*=%A9; x*0*=utf-8''%C3";
    let standalone = ParameterList::parse(suffix).unwrap();
    for prefix in [
        b" (comment) text/plain;".as_slice(),
        b" (comment) attachment;",
    ] {
        let input = [prefix, suffix].concat();
        let list = if prefix.ends_with(b"plain;") {
            ContentType::parse(&input).unwrap().parameters().clone()
        } else {
            ContentDisposition::parse(&input)
                .unwrap()
                .parameters()
                .clone()
        };
        assert_eq!(list.as_slice(), standalone.as_slice());
        assert!(!list.issues().is_empty());
        assert_eq!(list.issues().len(), standalone.issues().len());
        for (actual, original) in list.issues().iter().zip(standalone.issues()) {
            assert_eq!(actual.kind, original.kind);
            assert_eq!(
                actual.range,
                original.range.start + prefix.len()..original.range.end + prefix.len()
            );
        }
    }
}

#[test]
fn raw_header_integration_is_lossless() {
    let input = b"Content-Type: text/plain; charset=UTF-8\r\n\
Content-Disposition: attachment;\r\n filename*0*=utf-8''caf%C3;\r\n filename*1*=%A9.txt\r\n\r\nbody";
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    let consumed = parser.parse(input, &mut headers).unwrap();
    assert_eq!(&input[consumed..], b"body");
    let originals: Vec<_> = headers.iter().map(|header| header.raw().to_vec()).collect();
    let content_type = ContentType::parse(headers[0].raw_value()).unwrap();
    let disposition = ContentDisposition::parse(headers[1].raw_value()).unwrap();
    assert_eq!(content_type.charset_encoding(), Ok(Some(Charset::UTF8)));
    assert_eq!(disposition.file_name(), Some("caf\u{e9}.txt"));
    for (header, original) in headers.iter().zip(originals) {
        assert_eq!(header.raw(), original);
    }
}

fn assert_bounded(input: &[u8], mode: ParameterComplianceMode) {
    let options = ParameterParseOptions {
        compliance_mode: mode,
        ..Default::default()
    };
    let results = [
        ContentType::parse_with_options(input, &options).map(|v| v.parameters().clone()),
        ContentDisposition::parse_with_options(input, &options).map(|v| v.parameters().clone()),
    ];
    for result in results {
        match result {
            Ok(parameters) => {
                let mut previous = 0;
                for issue in parameters.issues() {
                    assert!(previous <= issue.range.start);
                    assert!(issue.range.start <= issue.range.end && issue.range.end <= input.len());
                    previous = issue.range.start;
                }
            }
            Err(error) => assert!(error.offset() <= input.len()),
        }
    }
}

#[test]
fn truncated_arbitrary_and_large_headers_terminate() {
    for input in [
        b" (a(b\\)c)) text/plain; name*0*=utf-8''%C3; name*1*=%A9".as_slice(),
        b"attachment; filename=\"=?utf-8?q?caf=C3=A9?=\" size=1",
        b"text/plain; name*2147483648=bad; x=\"unterminated",
    ] {
        for start in 0..=input.len() {
            for end in start..=input.len() {
                for mode in [
                    ParameterComplianceMode::Loose,
                    ParameterComplianceMode::Strict,
                ] {
                    assert_bounded(&input[start..end], mode);
                }
            }
        }
    }
    let mut seed = 0x12345678u32;
    for length in 0..256 {
        let bytes: Vec<u8> = (0..length)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            assert_bounded(&bytes, mode);
            for prefix in [b"text/plain; name=".as_slice(), b"attachment; filename="] {
                assert_bounded(&[prefix, &bytes, b";"].concat(), mode);
            }
        }
    }
    let mut text = String::from("text/plain;");
    for index in 0..5000 {
        text.push_str(&format!(" x{index}=value;"));
    }
    assert_eq!(
        ContentType::parse(text.as_bytes())
            .unwrap()
            .parameters()
            .len(),
        5000
    );
}

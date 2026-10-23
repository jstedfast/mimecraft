// Includes cases adapted from MimeKit's ParameterListTests.cs,
// ContentTypeTests.cs, and ContentDispositionTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    HeaderParser, ParameterComplianceMode, ParameterIssueKind, ParameterList,
    ParameterParseErrorKind, ParameterParseOptions, charsets::Charset, encodings::Rfc2231Encoder,
    rfc2047::ComplianceMode,
};

fn parse(input: &[u8], mode: ParameterComplianceMode) -> ParameterList {
    ParameterList::parse_with_options(
        input,
        &ParameterParseOptions {
            compliance_mode: mode,
            ..Default::default()
        },
    )
    .unwrap()
}

fn assert_value(input: &str, name: &str, expected: &str) {
    let list = ParameterList::parse(input.as_bytes()).unwrap();
    assert_eq!(list.get(name), Some(expected), "{input}");
}

#[test]
fn empty_lists_order_case_and_borrowed_lookup() {
    for text in [
        "",
        " \r\n\t ",
        ";; ; (comment)",
        "(nested (and \\) escaped))",
    ] {
        let list: ParameterList = text.parse().unwrap();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert_eq!(list.get("missing"), None);
        assert!(list.issues().is_empty());
    }
    let list = ParameterList::parse(b"; Charset=utf-8; BOUNDARY=\"test\"; format=flowed").unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list.get("CHARSET"), Some("utf-8"));
    assert_eq!(list.get("boundary"), Some("test"));
    assert_eq!(list.get(" charset"), None);
    assert_eq!(
        list.as_slice()
            .iter()
            .map(|p| (p.name(), p.value()))
            .collect::<Vec<_>>(),
        [
            ("Charset", "utf-8"),
            ("BOUNDARY", "test"),
            ("format", "flowed")
        ]
    );
    assert_eq!(list.clone(), list);
    assert!(list.issues().is_empty());
    assert!(ParameterList::default().is_empty());
}

#[test]
fn canonical_plain_and_quoted_values() {
    for (text, name, value) in [
        ("boundary=\"boundary-text\"", "boundary", "boundary-text"),
        (
            ";;\n Boundary=\"===========================_ _= 1212158(26598)\"",
            "boundary",
            "===========================_ _= 1212158(26598)",
        ),
        ("name=Test;", "name", "Test"),
        ("name=;", "name", ""),
        ("name=\"\"", "name", ""),
        (
            "filename=\"a\\\"b\\\\c\\d.txt\"",
            "filename",
            "a\"b\\cd.txt",
        ),
        (
            "name=\"semicolons; equals= signs\"",
            "name",
            "semicolons; equals= signs",
        ),
        (
            "x-eai-please-do-not=\"abst\u{fc}rzen\"",
            "X-EAI-PLEASE-DO-NOT",
            "abst\u{fc}rzen",
        ),
        (
            "filename=\"report\r\n\t.docx\"\r\n",
            "filename",
            "report\t.docx",
        ),
        ("(before) name (name) = (value) \"x\" (after);", "name", "x"),
    ] {
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            let list = parse(text.as_bytes(), mode);
            assert_eq!(list.get(name), Some(value));
            assert!(list.issues().is_empty(), "{text}");
        }
    }
}

#[test]
fn canonical_unquoted_and_missing_separator_recovery() {
    for (text, expected) in [
        ("name=ab cd", "ab cd"),
        ("name=ab cd;", "ab cd"),
        ("name=ab cd ;charset=utf-8", "ab cd"),
        ("name=ab cd \t;charset=utf-8", "ab cd"),
        ("name=x ", "x"),
        ("name=x \t", "x"),
        ("name=x ;charset=utf-8", "x"),
        ("charset=utf-8; name=Test Name.pdf ", "Test Name.pdf"),
        (
            "name=Partnership Marketing Agreement\n\tForm - Mega Brands.pdf",
            "Partnership Marketing Agreement\tForm - Mega Brands.pdf",
        ),
        ("name=--boundary_0_8ab0e518\n ", "--boundary_0_8ab0e518"),
        ("name=a\r\nb\r\n\tc", "ab\tc"),
        ("name=a(comment)", "a(comment)"),
    ] {
        assert_value(text, "name", expected);
    }
    let input = b"name=\"smime.p7m\"\n smime-type=enveloped-data";
    let list = ParameterList::parse(input).unwrap();
    assert_eq!(list.get("name"), Some("smime.p7m"));
    assert_eq!(list.get("smime-type"), Some("enveloped-data"));
    assert_eq!(list.issues().len(), 1);
    assert_eq!(list.issues()[0].kind, ParameterIssueKind::MissingSeparator);
    assert_eq!(list.issues()[0].range, 18..18);
    let strict = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    assert_eq!(
        ParameterList::parse_with_options(input, &strict)
            .unwrap_err()
            .kind(),
        ParameterParseErrorKind::ExpectedSeparator
    );
    assert_eq!(
        ParameterList::parse_with_options(b"name=ab cd", &strict)
            .unwrap_err()
            .offset(),
        8
    );
    assert_eq!(
        parse(b"name=a(comment)", ParameterComplianceMode::Strict).get("name"),
        Some("a")
    );
}

#[test]
fn canonical_rfc2231_examples() {
    for mode in [
        ParameterComplianceMode::Loose,
        ParameterComplianceMode::Strict,
    ] {
        let list = parse(b"access-type=URL;\n URL*0=\"ftp://\";\n URL*1=\"cs.utk.edu/pub/moore/bulk-mailer/bulk-mailer.tar\"", mode);
        assert_eq!(list.get("access-type"), Some("URL"));
        assert_eq!(
            list.get("url"),
            Some("ftp://cs.utk.edu/pub/moore/bulk-mailer/bulk-mailer.tar")
        );
        assert!(list.issues().is_empty());

        let list = parse(
            b"title*=us-ascii'en-us'This%20is%20%2A%2A%2Afun%2A%2A%2A",
            mode,
        );
        assert_eq!(list.get("title"), Some("This is ***fun***"));
        assert!(list.issues().is_empty());

        let list = parse(b"title*1*=us-ascii'en'This%20is%20even%20more%20;\n title*2*=%2A%2A%2Afun%2A%2A%2A%20;\n title*3=\"isn't it!\"", mode);
        assert_eq!(
            list.get("title"),
            Some("This is even more ***fun*** isn't it!")
        );
        assert_eq!(list.issues().len(), 1);
        assert_eq!(
            list.issues()[0].kind,
            ParameterIssueKind::NonContiguousContinuation
        );

        let list = parse(b"name*=iso-8859-1''Kristoffer%20Br%E5nemyr", mode);
        assert_eq!(list.get("name"), Some("Kristoffer Br\u{e5}nemyr"));
        assert!(list.issues().is_empty());
    }
}

#[test]
fn quoted_extended_values_and_legacy_charsets() {
    for (input, expected) in [
        (
            "filename*=\"utf-8''SBD%20%C5%A0kodov%C3%A1k%2Ejpg\"",
            "SBD \u{160}kodov\u{e1}k.jpg",
        ),
        (
            "filename*0*=\"ISO-8859-2''%C8%50%50%20%2D%20%BE%E1%64%6F%73%74%20%6F%20%61%6B%63%65\";\n filename*1*=\"%70%74%61%63%69%20%73%6D%6C%6F%75%76%79%20%31%32%2E%31%32%2E\";\n filename*2*=\"%64%6F%63\"",
            "\u{10c}PP - \u{17e}\u{e1}dost o akceptaci smlouvy 12.12.doc",
        ),
        (
            "filename*=gb18030''%B2%E2%CA%D4%CE%C4%B1%BE.txt",
            "\u{6d4b}\u{8bd5}\u{6587}\u{672c}.txt",
        ),
        ("filename*=utf-16''%41%00%42%00", "AB"),
        ("filename*=\"utf-8''a\\b\"", "a\\b"),
    ] {
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            let list = parse(input.as_bytes(), mode);
            assert_eq!(list.get("filename"), Some(expected));
            assert!(list.issues().is_empty());
        }
    }
}

#[test]
fn continuation_sorting_gaps_duplicates_and_large_identifiers() {
    let list = ParameterList::parse(b"NaMe*2=c; x=one; NAME*0=a; name*1=b; y=two").unwrap();
    assert_eq!(list.get("name"), Some("abc"));
    assert_eq!(
        list.as_slice().iter().map(|p| p.name()).collect::<Vec<_>>(),
        ["NaMe", "x", "y"]
    );
    assert!(list.issues().is_empty());
    let list = ParameterList::parse(b"n*2=C; n*00=A; n*0=B; n*2147483647=D").unwrap();
    assert_eq!(list.get("n"), Some("ABCD"));
    assert_eq!(
        list.issues()
            .iter()
            .filter(|i| i.kind == ParameterIssueKind::NonContiguousContinuation)
            .count(),
        2
    );
    assert_eq!(
        list.issues()
            .iter()
            .filter(|i| i.kind == ParameterIssueKind::DuplicateContinuation)
            .count(),
        1
    );
    assert_value("n*0000000000000000000000000=a; n*1=b", "n", "ab");
    assert_value(
        "n (comment) * (comment) 0 (comment) * (comment) = utf-8''a; n*1*=b",
        "n",
        "ab",
    );
}

#[test]
fn extended_definitions_override_ordinary_duplicates() {
    for input in [
        "filename=\"Filename.doc\"; filename*0*=UTF-8''UnicodeFile; filename*1*=name.doc",
        "filename*0*=UTF-8''UnicodeFile; filename*1*=name.doc; filename=\"Filename.doc\"",
        "filename*0*=UTF-8''UnicodeFile; filename=\"Filename.doc\"; filename*1*=name.doc",
        "filename=\"Filename.doc\"; filename*=UTF-8''UnicodeFilename.doc",
        "filename*=UTF-8''UnicodeFilename.doc; filename=\"Filename.doc\"",
    ] {
        let list = ParameterList::parse(input.as_bytes()).unwrap();
        assert_eq!(list.get("filename"), Some("UnicodeFilename.doc"));
        assert_eq!(list.len(), 1);
        assert!(
            list.issues()
                .iter()
                .any(|i| i.kind == ParameterIssueKind::DuplicateParameter)
        );
    }
    let list = ParameterList::parse(
        b"NAME=first; name=second; x=ok; nAmE*=utf-8''third; NAME*=utf-8''last",
    )
    .unwrap();
    assert_eq!(list.get("name"), Some("last"));
    assert_eq!(list.as_slice()[0].name(), "NAME");
    assert_eq!(list.get("x"), Some("ok"));
    assert_eq!(list.issues().len(), 3);
    assert_value("n=first; N=second", "n", "first");
    assert_value("n*0=one; n*=utf-8''two; n*1=three", "n", "two");
    assert_value("n*=utf-8''one; n*0=two; n*1=three", "n", "twothree");
}

#[test]
fn missing_and_unsupported_rfc2231_charsets_use_latin1() {
    for (input, expected) in [
        (
            "name*0*=This%20is%20some%20encoded%20ascii%20text",
            "This is some encoded ascii text",
        ),
        (
            "name*0*=us-ascii'This%20is%20some%20encoded%20ascii%20text",
            "us-ascii'This is some encoded ascii text",
        ),
        ("name*=%C3%A9", "\u{c3}\u{a9}"),
        ("name*=''caf%C3%A9", "''caf\u{c3}\u{a9}"),
        ("name*='en'caf%C3%A9", "'en'caf\u{c3}\u{a9}"),
        ("name*=\"\"", ""),
    ] {
        let list = ParameterList::parse(input.as_bytes()).unwrap();
        assert_eq!(list.get("name"), Some(expected));
        assert_eq!(list.issues().len(), 1);
        assert_eq!(list.issues()[0].kind, ParameterIssueKind::MissingCharset);
    }
    let windows = Charset::from_label("windows-1252").unwrap();
    let options = ParameterParseOptions {
        fallback_charset: Some(windows),
        ..Default::default()
    };
    for input in [
        b"name*=x-unsupported''%C3%A9%80".as_slice(),
        b"name*0*=x-unsupported''%C3; name*1*=%A9%80",
        b"name*=\"x-\xff''%C3%A9%80\"",
    ] {
        let list = ParameterList::parse_with_options(input, &options).unwrap();
        assert_eq!(list.get("name"), Some("\u{c3}\u{a9}\u{80}"));
        assert_eq!(list.issues().len(), 1);
        assert_eq!(
            list.issues()[0].kind,
            ParameterIssueKind::UnsupportedCharset {
                selected: Charset::LATIN1
            }
        );
    }
}

#[test]
fn encoded_words_in_ordinary_values_only() {
    for (input, expected) in [
        (
            "title=\"some chinese characters =?utf-8?q?=E4=B8=AD=E6=96=87?= and stuff\"",
            "some chinese characters \u{4e2d}\u{6587} and stuff",
        ),
        (
            "title=\"some chinese characters =?big5?b?pKSk5Q==?= and stuff\"",
            "some chinese characters \u{4e2d}\u{6587} and stuff",
        ),
        ("title=\"=?utf-8?q?caf=C3=A9?=\"", "caf\u{e9}"),
        ("title=\"=?iso-8859-1?q?caf=E9?=\"", "caf\u{e9}"),
        ("title=\"=?unknown?q?=C3=A9?=\"", "\u{e9}"),
        ("title==?utf-8?q?caf=C3=A9?=", "caf\u{e9}"),
        (
            "title*0=\"=?utf-8?q?caf=C3=A9?=\"; title*1=\".txt\"",
            "=?utf-8?q?caf=C3=A9?=.txt",
        ),
        (
            "title*=utf-8''%3D%3Futf-8%3Fq%3Fcaf%3DC3%3DA9%3F%3D",
            "=?utf-8?q?caf=C3=A9?=",
        ),
    ] {
        assert_value(input, "title", expected);
    }
    let options = ParameterParseOptions {
        rfc2047_compliance_mode: ComplianceMode::Strict,
        ..Default::default()
    };
    let input = b"title=\"x=?utf-8?q?word?=y\"";
    assert_eq!(
        ParameterList::parse(input).unwrap().get("title"),
        Some("xwordy")
    );
    assert_eq!(
        ParameterList::parse_with_options(input, &options)
            .unwrap()
            .get("title"),
        Some("x=?utf-8?q?word?=y")
    );
}

#[test]
fn percent_and_character_sequences_cross_encoded_sections() {
    for (input, expected) in [
        ("name*0*=utf-8''%C; name*1*=3%A; name*2*=9", "\u{e9}"),
        ("name*0*=utf-8''%C3; name*1*=%A9", "\u{e9}"),
        (
            "name*0*=iso-2022-jp''%1B%24; name*1*=%42%24%22%1B%28%42",
            "\u{3042}",
        ),
        ("name*0*=utf-16''%3D%D8; name*1*=%00%DE", "\u{1f600}"),
        (
            "name*0=prefix; name*1*=utf-8''%C3%A9; name*2=suffix; name*3*=%C3%A9",
            "prefix\u{e9}suffix\u{e9}",
        ),
        ("name*0*=utf-8''%C; name*1=; name*2*=3%A9", "\u{e9}"),
        (
            "name*0*=utf-8''%C3; name*1=; name*2*=%A9",
            "\u{fffd}\u{fffd}",
        ),
        ("name*0*=utf-8''%C; name*1=\"\"; name*2*=3%A9", "3\u{fffd}"),
        ("name*0*=utf-8''%C; name*1=X; name*2*=3%A9", "X3\u{fffd}"),
    ] {
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            assert_eq!(
                parse(input.as_bytes(), mode).get("name"),
                Some(expected),
                "{input}"
            );
        }
    }
}

#[test]
fn all_percent_payload_splits_and_out_of_order_sections() {
    let text = "Ren\u{e9}e +_?% \u{674e}\u{1f600}\0\r\n";
    let mut payload = Vec::new();
    Rfc2231Encoder::new().encode(text.as_bytes(), &mut payload);
    let payload = std::str::from_utf8(&payload).unwrap();
    for split in 0..=payload.len() {
        let input = format!(
            "name*1*={}; other=ok; name*0*=utf-8''{};",
            &payload[split..],
            &payload[..split]
        );
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            let list = parse(input.as_bytes(), mode);
            assert_eq!(list.get("name"), Some(text), "{input}");
            assert_eq!(list.get("other"), Some("ok"));
            assert!(list.issues().is_empty(), "{input}: {:?}", list.issues());
        }
    }
}

#[test]
fn percent_tolerance_and_pending_escape_diagnostics() {
    for (input, expected, incomplete) in [
        ("name*=utf-8''a+b%20c", "a+b c", false),
        ("name*=utf-8''%XY%20bad", "%XY bad", false),
        ("name*=utf-8''abc%", "abc", true),
        ("name*=utf-8''abc%X", "abc", true),
        ("name*0*=utf-8''%C; name*1=x", "x", true),
        ("name*0*=utf-8''%C; name*1=; name*2*=3%A9", "\u{e9}", false),
    ] {
        let list = ParameterList::parse(input.as_bytes()).unwrap();
        assert_eq!(list.get("name"), Some(expected));
        assert_eq!(
            list.issues()
                .iter()
                .any(|i| i.kind == ParameterIssueKind::IncompletePercentEscape),
            incomplete
        );
    }
    assert_value("name*=utf-8''%EF%BB%BF%00%0D%0A", "name", "\u{feff}\0\r\n");
    assert_value(
        "filename*=utf-8''..%2F..%2Freport.txt",
        "filename",
        "../../report.txt",
    );
}

#[test]
fn replacement_and_raw_fallback_have_source_spans() {
    let input = b" name*=utf-8''%FF; other=x";
    let list = ParameterList::parse(input).unwrap();
    assert_eq!(list.get("name"), Some("\u{fffd}"));
    assert_eq!(list.issues().len(), 1);
    assert_eq!(list.issues()[0].range, 7..17);
    assert_eq!(
        list.issues()[0].kind,
        ParameterIssueKind::InvalidBytes {
            charset: Charset::UTF8
        }
    );
    let input = b"n=\"=?utf-8?q?=FF?=\"";
    let list = ParameterList::parse(input).unwrap();
    assert_eq!(list.issues()[0].range, 2..input.len());
    assert_eq!(
        list.issues()[0].kind,
        ParameterIssueKind::InvalidBytes {
            charset: Charset::UTF8
        }
    );
    let input = b"n*1*=%FF; other=x; n*0*=utf-8''abc";
    let list = ParameterList::parse(input).unwrap();
    assert_eq!(list.get("n"), Some("abc\u{fffd}"));
    assert_eq!(list.issues()[0].range, 5..input.len());

    let options = ParameterParseOptions {
        fallback_charset: Some(Charset::from_label("windows-1252").unwrap()),
        ..Default::default()
    };
    for input in [b"n=\"\x80\"".as_slice(), b"n*0=\"\x80\""] {
        let list = ParameterList::parse_with_options(input, &options).unwrap();
        assert_eq!(list.get("n"), Some("\u{20ac}"));
        assert_eq!(list.issues().len(), 1);
        assert_eq!(
            list.issues()[0].kind,
            ParameterIssueKind::UnlabelledText {
                selected: options.fallback_charset.unwrap()
            }
        );
    }
    let list = ParameterList::parse_with_options(b"n=\"\xc3\xa9\"", &options).unwrap();
    assert_eq!(list.get("n"), Some("\u{e9}"));
    assert!(list.issues().is_empty());
    assert_value("n*=utf-8''%EF%BF%BD", "n", "\u{fffd}");
    assert!(
        ParameterList::parse(b"n*=utf-8''%EF%BF%BD")
            .unwrap()
            .issues()
            .is_empty()
    );
}

#[test]
fn syntax_failures_are_explicit_and_offsets_are_relative() {
    for (input, kind, offset) in [
        ("=", ParameterParseErrorKind::InvalidName, 0),
        ("name", ParameterParseErrorKind::IncompleteParameter, 4),
        ("name*", ParameterParseErrorKind::IncompleteParameter, 5),
        ("name*0", ParameterParseErrorKind::IncompleteParameter, 6),
        ("name*0*", ParameterParseErrorKind::IncompleteParameter, 7),
        ("name*0*x", ParameterParseErrorKind::IncompleteParameter, 7),
        ("name*0*=", ParameterParseErrorKind::IncompleteParameter, 8),
        ("name=", ParameterParseErrorKind::IncompleteParameter, 5),
        (
            "name=\"unfinished",
            ParameterParseErrorKind::IncompleteQuotedValue,
            5,
        ),
        (
            "name=\"escaped\\\"",
            ParameterParseErrorKind::IncompleteQuotedValue,
            5,
        ),
        (
            "name*2147483648=x",
            ParameterParseErrorKind::ContinuationOverflow,
            14,
        ),
        (
            "name*999999999999999999999999999999=x",
            ParameterParseErrorKind::ContinuationOverflow,
            14,
        ),
    ] {
        for mode in [
            ParameterComplianceMode::Loose,
            ParameterComplianceMode::Strict,
        ] {
            let wrapped = format!("prefix {input}suffix");
            let slice = &wrapped.as_bytes()[7..7 + input.len()];
            let error = ParameterList::parse_with_options(
                slice,
                &ParameterParseOptions {
                    compliance_mode: mode,
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind, "{input}");
            assert_eq!(error.offset(), offset, "{input}");
            assert!(error.to_string().contains(&format!("byte {offset}")));
        }
    }
    for input in [
        "(comment",
        "name(comment",
        "name*(comment",
        "name*0(comment",
        "name*0*(comment",
        "name=(comment",
    ] {
        let error = ParameterList::parse(input.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), ParameterParseErrorKind::IncompleteComment);
        assert_eq!(error.offset(), input.find('(').unwrap());
    }
    let strict = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    let error = ParameterList::parse_with_options(b"name=value(comment", &strict).unwrap_err();
    assert_eq!(error.kind(), ParameterParseErrorKind::IncompleteComment);
    assert_eq!(error.offset(), 10);
    for input in [b"\xff=x".as_slice(), b"bad/name=x", b"name%=x", b"name'=x"] {
        assert!(ParameterList::parse(input).is_err());
    }
}

#[test]
fn attribute_names_match_percent_encoder_classification() {
    for byte in 0u8..=255 {
        let allowed = byte.is_ascii_alphanumeric() || b"!#$&+-.^_`{|}~".contains(&byte);
        let mut encoded = Vec::new();
        Rfc2231Encoder::new().encode(&[byte], &mut encoded);
        assert_eq!(encoded == [byte], allowed, "{byte}");
        if allowed {
            let input = [b'x', byte, b'y', b'=', b'z'];
            let list = ParameterList::parse(&input).unwrap();
            assert_eq!(list.as_slice()[0].name().as_bytes(), &input[..3]);
            assert_eq!(list.as_slice()[0].value(), "z");
        }
    }
}

#[test]
fn raw_header_integration_keeps_original_bytes() {
    let input = b"Content-Disposition: attachment;\r\n filename*0*=utf-8''caf%C3;\r\n filename*1*=%A9.txt\r\n\r\nbody";
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    let consumed = parser.parse(input, &mut headers).unwrap();
    assert_eq!(&input[consumed..], b"body");
    let raw = headers[0].raw().to_vec();
    let value = headers[0].raw_value();
    let parameter_start = value.iter().position(|&byte| byte == b';').unwrap();
    let list = ParameterList::parse(&value[parameter_start..]).unwrap();
    assert_eq!(list.get("filename"), Some("caf\u{e9}.txt"));
    assert!(list.issues().is_empty());
    assert_eq!(headers[0].raw(), raw);
}

fn assert_bounded(input: &[u8], mode: ParameterComplianceMode) {
    match ParameterList::parse_with_options(
        input,
        &ParameterParseOptions {
            compliance_mode: mode,
            ..Default::default()
        },
    ) {
        Ok(list) => {
            let mut previous_start = 0;
            for issue in list.issues() {
                assert!(previous_start <= issue.range.start);
                assert!(issue.range.start <= issue.range.end && issue.range.end <= input.len());
                previous_start = issue.range.start;
            }
            for parameter in list.as_slice() {
                assert!(!parameter.name().is_empty());
                assert!(parameter.name().is_ascii());
            }
        }
        Err(error) => assert!(error.offset() <= input.len()),
    }
}

#[test]
fn arbitrary_truncated_and_large_inputs_terminate() {
    for input in [
        b"; x (comment) * 0 * = utf-8''%C3; x*1*=%A9;".as_slice(),
        b"n=\"=?utf-8?q?=C3=A9?=\"; n*=unknown''%FF",
        b"(a(b\\)c)) x=; x*2147483647=\"z\"",
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
    for length in 0..512 {
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
            assert_bounded(&[b"x=".as_slice(), &bytes, b";"].concat(), mode);
        }
    }
    let mut input = String::new();
    for id in (0..5000).rev() {
        input.push_str(&format!("n*{id}=x;"));
    }
    let list = ParameterList::parse(input.as_bytes()).unwrap();
    assert_eq!(list.get("n"), Some("x".repeat(5000).as_str()));
    assert!(list.issues().is_empty());
    let mut input = String::new();
    for id in 0..5000 {
        input.push_str(&format!("n{id}=value;"));
    }
    let list = ParameterList::parse(input.as_bytes()).unwrap();
    assert_eq!(list.len(), 5000);
    assert!(list.issues().is_empty());
}

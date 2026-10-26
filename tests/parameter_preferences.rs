// Adapted from MimeKit's ParameterTests.cs and ParameterList.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentDisposition, ContentType, HeaderFormatOptions, Parameter, ParameterEncodingMethod,
    ParameterIssueKind, ParameterList, ParameterParseOptions, charsets::Charset, rfc2047,
};

fn check(parameter: &Parameter, method: Option<ParameterEncodingMethod>, charset: Option<Charset>) {
    assert_eq!(parameter.encoding_method(), method, "{parameter:?}");
    assert_eq!(parameter.charset(), charset, "{parameter:?}");
    assert!(!parameter.always_quote());
}

#[test]
fn extended_and_continued_preferences_follow_the_first_encoded_section() {
    for (text, value, charset) in [
        ("name*=UTF-8'en'caf%C3%A9", "caf\u{e9}", Some(Charset::UTF8)),
        (
            "name*1*=%A9; name*0*=utf-8''caf%C3",
            "caf\u{e9}",
            Some(Charset::UTF8),
        ),
        (
            "name*0=pre; name*1*=ISO_8859_1'fr'caf%E9; name*2=post",
            "precaf\u{e9}post",
            Some(Charset::LATIN1),
        ),
        (
            "name*0*=utf-8''; name*1=pre; name*2*=caf%C3%A9",
            "precaf\u{e9}",
            Some(Charset::UTF8),
        ),
        (
            "name*=\"iso-8859-1''caf%E9\"",
            "caf\u{e9}",
            Some(Charset::LATIN1),
        ),
        ("name*=utf-8''", "", Some(Charset::UTF8)),
        ("name*0=one; name*1=\"two\"", "onetwo", None),
        (
            "name*0=\"=?utf-8?q?caf=C3=A9?=\"",
            "=?utf-8?q?caf=C3=A9?=",
            None,
        ),
    ] {
        let parsed = ParameterList::parse(text.as_bytes()).unwrap();
        assert!(parsed.issues().is_empty(), "{text}");
        assert_eq!(parsed.get("name"), Some(value), "{text}");
        check(
            &parsed.as_slice()[0],
            Some(ParameterEncodingMethod::Rfc2231),
            charset,
        );
    }
}

#[test]
fn rfc2047_method_survives_repeated_parse_and_format() {
    for (word, charset) in [
        ("=?utf-8?q?caf=C3=A9?=", None),
        ("=?iso-8859-1?q?caf=E9?=", Some(Charset::LATIN1)),
    ] {
        let text = format!(" text/plain; name=\"{word}\"");
        let value = ContentType::parse(text.as_bytes()).unwrap();
        assert_eq!(value.name(), Some("caf\u{e9}"));
        check(
            &value.parameters().as_slice()[0],
            Some(ParameterEncodingMethod::Rfc2047),
            charset,
        );
        // Re-encoding should produce the same output each time rather than
        // alternating between RFC 2047 and RFC 2231.
        let encoded = value.to_header_string().unwrap();
        assert!(encoded.contains("=?"));
        for _ in 0..3 {
            let reparsed =
                ContentType::parse(encoded.split_once(':').unwrap().1.as_bytes()).unwrap();
            assert_eq!(reparsed.name(), Some("caf\u{e9}"));
            assert_eq!(reparsed.to_header_string().unwrap(), encoded);
        }
    }
}

#[test]
fn encoded_word_charsets_are_counted_by_word_with_first_encounter_ties() {
    for (words, charset, encoded) in [
        (
            "=?utf-8?q?a_longer_payload?= =?iso-8859-1?q?a?= =?ISO_8859_1?b?Yg==?=",
            Some(Charset::LATIN1),
            true,
        ),
        ("=?utf-8?q?a?= =?iso-8859-1?q?b?=", None, true),
        (
            "=?iso-8859-1?q?a?= =?utf-8?q?b?=",
            Some(Charset::LATIN1),
            true,
        ),
        ("=?us-ascii?q?a?= =?utf-8?q?b?=", Some(Charset::ASCII), true),
        ("=?x-unknown-a?q?a?= =?utf-8?q?b?=", None, false),
        (
            "=?iso-8859-1?q?a?= =?x-unknown-a?q?b?=",
            Some(Charset::LATIN1),
            true,
        ),
        (
            "=?iso-8859-1?q?a?= =?x-unknown-a?q?b?= =?x-unknown-b?q?c?=",
            None,
            false,
        ),
        (
            "=?utf-8*en?q?a?= =?ISO_8859_1*fr?q?b?= =?iso-8859-1?q?c?=",
            Some(Charset::LATIN1),
            true,
        ),
        (
            "=?iso-8859-1?q??= =?utf-8?q?b?=",
            Some(Charset::LATIN1),
            true,
        ),
    ] {
        let text = format!("name=\"{words}\"");
        let list = ParameterList::parse(text.as_bytes()).unwrap();
        let decoded = rfc2047::decode_text(words.as_bytes());
        assert_eq!(list.get("name"), Some(decoded.text.as_str()));
        assert_eq!(list.issues().len(), decoded.issues.len());
        check(
            &list.as_slice()[0],
            encoded.then_some(ParameterEncodingMethod::Rfc2047),
            charset,
        );
    }
}

#[test]
fn ordinary_values_and_unrecognized_wrappers_keep_inheriting() {
    for text in [
        "name=plain",
        "name=\"plain\"",
        "name=\"\"",
        "name=\"=?utf-8?x?not-a-word?=\"",
        "name=\"=?utf-8?q?unterminated\"",
        "name=\"caf\u{e9}\"",
    ] {
        let parsed = ParameterList::parse(text.as_bytes()).unwrap();
        check(&parsed.as_slice()[0], None, None);
    }
    let raw = ParameterList::parse(b"name=\"caf\xe9\"").unwrap();
    check(&raw.as_slice()[0], None, None);
    assert!(matches!(
        raw.issues()[0].kind,
        ParameterIssueKind::UnlabelledText { .. }
    ));

    for mode in [
        rfc2047::ComplianceMode::Loose,
        rfc2047::ComplianceMode::Strict,
    ] {
        let parsed = ParameterList::parse_with_options(
            b"name=\"prefix=?iso-8859-1?q?caf=E9?=\"",
            &ParameterParseOptions {
                rfc2047_compliance_mode: mode,
                ..Default::default()
            },
        )
        .unwrap();
        if mode == rfc2047::ComplianceMode::Loose {
            assert_eq!(parsed.get("name"), Some("prefixcaf\u{e9}"));
            check(
                &parsed.as_slice()[0],
                Some(ParameterEncodingMethod::Rfc2047),
                Some(Charset::LATIN1),
            );
        } else {
            assert_eq!(parsed.get("name"), Some("prefix=?iso-8859-1?q?caf=E9?="));
            check(&parsed.as_slice()[0], None, None);
        }
    }
}

#[test]
fn duplicate_winners_replace_preferences_without_changing_name_spelling() {
    for (text, value, method, charset) in [
        (
            "FileName=\"=?iso-8859-1?q?caf=E9?=\"; filename*=utf-8''new",
            "new",
            ParameterEncodingMethod::Rfc2231,
            Some(Charset::UTF8),
        ),
        (
            "FileName*=iso-8859-1''caf%E9; filename=\"=?utf-8?q?new?=\"",
            "caf\u{e9}",
            ParameterEncodingMethod::Rfc2231,
            Some(Charset::LATIN1),
        ),
        (
            "FileName*=utf-8''old; filename*0=new; filename*1=value",
            "newvalue",
            ParameterEncodingMethod::Rfc2231,
            None,
        ),
        (
            "FileName*0*=iso-8859-1''old; filename*=utf-8''new",
            "new",
            ParameterEncodingMethod::Rfc2231,
            Some(Charset::UTF8),
        ),
        (
            "FileName=\"=?utf-8?q?old?=\"; filename=\"=?iso-8859-1?q?new?=\"",
            "old",
            ParameterEncodingMethod::Rfc2047,
            None,
        ),
    ] {
        let parsed = ParameterList::parse(text.as_bytes()).unwrap();
        assert_eq!(parsed.len(), 1);
        let parameter = &parsed.as_slice()[0];
        assert_eq!(parameter.name(), "FileName");
        assert_eq!(parameter.value(), value);
        check(parameter, Some(method), charset);
        assert!(
            parsed
                .issues()
                .iter()
                .any(|issue| issue.kind == ParameterIssueKind::DuplicateParameter)
        );
    }
}

#[test]
fn recovery_preferences_do_not_hide_or_rebase_diagnostics() {
    for (text, expected_issue, charset, method) in [
        (
            "name*=x-unknown''caf%E9",
            ParameterIssueKind::UnsupportedCharset {
                selected: Charset::LATIN1,
            },
            Some(Charset::LATIN1),
            Some(ParameterEncodingMethod::Rfc2231),
        ),
        (
            "name*=caf%E9",
            ParameterIssueKind::MissingCharset,
            Some(Charset::LATIN1),
            Some(ParameterEncodingMethod::Rfc2231),
        ),
        (
            "name*=utf-8''%FF",
            ParameterIssueKind::InvalidBytes {
                charset: Charset::UTF8,
            },
            Some(Charset::UTF8),
            Some(ParameterEncodingMethod::Rfc2231),
        ),
        (
            "name=\"=?utf-8?q?=FF?=\"",
            ParameterIssueKind::InvalidBytes {
                charset: Charset::UTF8,
            },
            None,
            Some(ParameterEncodingMethod::Rfc2047),
        ),
        (
            "name=\"=?x-unknown?q?caf=E9?=\"",
            ParameterIssueKind::UnsupportedCharset {
                selected: Charset::LATIN1,
            },
            None,
            None,
        ),
    ] {
        let prefixed = format!("attachment; {text}");
        let parsed = ContentDisposition::parse(prefixed.as_bytes()).unwrap();
        let parameters = parsed.parameters();
        check(&parameters.as_slice()[0], method, charset);
        assert_eq!(parameters.issues().len(), 1, "{text}");
        assert_eq!(parameters.issues()[0].kind, expected_issue);
        assert_eq!(
            parameters.issues()[0].range,
            prefixed.find('=').unwrap() + 1..prefixed.len()
        );
    }
}

#[test]
fn recovered_preferences_override_options_and_can_be_cleared_after_edits() {
    let mut value =
        ContentDisposition::parse(b"attachment; filename=\"=?gb18030?b?suLK1M7Esb4uZG9j?=\"")
            .unwrap();
    let gb = Charset::from_label("gb18030").unwrap();
    check(
        &value.parameters().as_slice()[0],
        Some(ParameterEncodingMethod::Rfc2047),
        Some(gb),
    );
    value.set_file_name(Some("\u{6d4b}\u{8bd5}.txt"));
    let output = value.to_header_string().unwrap();
    assert!(output.contains("=?gb18030?"));

    let mut parameter = value.parameters().as_slice()[0].clone();
    parameter.set_encoding_method(None);
    parameter.set_charset(None);
    value.parameters_mut().replace(0, parameter).unwrap();
    let output = value.to_header_string().unwrap();
    assert!(output.contains("filename*=utf-8''"));
    assert!(!output.contains("gb18030"));

    let value =
        ContentDisposition::parse(b"attachment; filename*=utf-8''%E6%B5%8B%E8%AF%95").unwrap();
    let options = HeaderFormatOptions {
        parameter_charset: gb,
        parameter_encoding_method: ParameterEncodingMethod::Rfc2047,
        ..Default::default()
    };
    assert!(
        value
            .to_header_string_with_options(&options)
            .unwrap()
            .contains("filename*=utf-8''")
    );
    let value =
        ContentDisposition::parse(b"attachment; filename=\"=?utf-8?b?5rWL6K+V?=\"").unwrap();
    assert!(
        value
            .to_header_string_with_options(&options)
            .unwrap()
            .contains("=?gb18030?")
    );
}

#[test]
fn named_charsets_and_whole_value_fallback_normalize_stably() {
    for (label, text) in [
        ("gb18030", "\u{6d4b}\u{8bd5}"),
        ("iso-2022-jp", "\u{65e5}\u{672c}"),
        ("utf-16be", "\u{1f600}"),
        ("iso-8859-1", "caf\u{e9} \u{2603}"),
    ] {
        for method in [
            ParameterEncodingMethod::Rfc2231,
            ParameterEncodingMethod::Rfc2047,
        ] {
            let mut parameter = Parameter::new("filename", text.repeat(30)).unwrap();
            parameter.set_charset(Some(Charset::from_label(label).unwrap()));
            parameter.set_encoding_method(Some(method));
            let mut value = ContentDisposition::default();
            value.parameters_mut().add(parameter).unwrap();
            let mut output = value.to_header_string().unwrap();
            for _ in 0..3 {
                let parsed =
                    ContentDisposition::parse(output.split_once(':').unwrap().1.as_bytes())
                        .unwrap();
                assert!(parsed.parameters().issues().is_empty());
                assert_eq!(parsed.file_name(), value.file_name());
                let normalized = parsed.to_header_string().unwrap();
                assert_eq!(normalized, output);
                output = normalized;
            }
        }
    }
}

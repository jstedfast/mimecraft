// Adapted from MimeKit's UnitTests/ReceivedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    DateTimeOffset, Header, Received, ReceivedClause, ReceivedFormatError as Error,
    ReceivedFormatOptions as Options, ReceivedFormatPart as Part, encodings::NewLineFormat,
};

fn date() -> DateTimeOffset {
    DateTimeOffset::new(2026, 4, 18, 20, 5, 38, -240).unwrap()
}

fn authored() -> Received {
    let mut value = Received::with_hosts(
        "smtp.source.com",
        "192.168.1.1".parse().unwrap(),
        "smtp.target.com",
        "127.0.0.1".parse().unwrap(),
        date(),
    )
    .unwrap();
    value.set_via(Some("TCP")).unwrap();
    value.set_with(Some("ESMTPS")).unwrap();
    value
        .set_id(Some("<VAD7UBNO1TU4.JCMO6CD121AX1@office365.com>"))
        .unwrap();
    value
        .set_for_recipient(Some("unit-tests@mimekit.net"))
        .unwrap();
    value
}

fn parse_header(header: &str) -> Received {
    Received::parse(Header::parse(header.as_bytes()).unwrap().raw_value()).unwrap()
}

fn assert_lines(output: &str, limit: usize, newline: &str) {
    assert!(output.starts_with("Received:"));
    assert!(output.ends_with(newline));
    let lines: Vec<_> = output
        .strip_suffix(newline)
        .unwrap()
        .split(newline)
        .collect();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.len() <= limit, "{line:?}");
        assert!(!line.contains(['\r', '\n']));
        if index > 0 {
            assert!(line.starts_with('\t'), "{line:?}");
        }
    }
}

#[test]
fn upstream_authored_and_removed_fields() {
    let value = authored();
    assert_eq!(
        value.to_header_string().unwrap(),
        "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n"
    );
    let mut constructor = Received::with_hosts(
        "smtp.source.com",
        "192.168.1.1".parse().unwrap(),
        "smtp.target.com",
        "127.0.0.1".parse().unwrap(),
        date(),
    )
    .unwrap();
    assert_eq!(
        constructor.to_header_string().unwrap(),
        "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]); Sat, 18 Apr 2026 20:05:38 -0400\r\n"
    );
    constructor.set_from_tcp_info(None);
    constructor.set_by_tcp_info(None);
    assert_eq!(
        constructor.to_header_string().unwrap(),
        "Received: from smtp.source.com by smtp.target.com;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n"
    );

    for (index, expected) in [
        (
            0,
            "Received: by smtp.target.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
        (
            1,
            "Received: from smtp.source.com ([192.168.1.1]) via TCP with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
        (
            2,
            "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
        (
            3,
            "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) via TCP\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
        (
            4,
            "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tfor unit-tests@mimekit.net; Sat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
        (
            5,
            "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com>;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n",
        ),
    ] {
        let mut copy = value.clone();
        copy.clauses_mut().remove(index);
        assert_eq!(copy.to_header_string().unwrap(), expected);
    }
    let mut copy = value.clone();
    copy.set_date_time(None);
    assert_eq!(
        copy.to_header_string().unwrap(),
        "Received: from smtp.source.com ([192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tid <VAD7UBNO1TU4.JCMO6CD121AX1@office365.com> for unit-tests@mimekit.net\r\n"
    );
}

#[test]
fn upstream_comment_unfolding_and_backslash_escaping() {
    let input = " from smtp.source.com (\\\\escaped\\\\) by smtp.target.com;\r\n\tSat, 18 Apr 2026 20:05:38 -0400\r\n";
    let received = Received::parse(input.as_bytes()).unwrap();
    assert_eq!(
        received.to_header_string().unwrap(),
        format!("Received:{input}")
    );
    let mut received = Received::new();
    received.set_from(Some("smtp.source.com")).unwrap();
    received.clauses_mut()[0]
        .comments_mut()
        .push("comment\r\nBcc: evil@example.com".into());
    assert_eq!(
        received.to_header_string().unwrap(),
        "Received: from smtp.source.com (commentBcc: evil@example.com)\r\n"
    );
    received.set_from_tcp_info(Some("HELO\t[192.\r\n168.1.1]"));
    received.set_by(Some("smtp.target.com")).unwrap();
    received.set_by_tcp_info(Some("[127.0.\r\n0.1]"));
    received.set_date_time(Some(date()));
    assert_eq!(
        received.to_header_string().unwrap(),
        "Received: from smtp.source.com (HELO [192.168.1.1])\r\n\tby smtp.target.com ([127.0.0.1]); Sat, 18 Apr 2026 20:05:38 -0400\r\n"
    );
}

#[test]
fn empty_unknown_duplicate_and_comment_only_values() {
    assert_eq!(Received::new().to_header_string().unwrap(), "Received:\r\n");
    assert_eq!(
        Received::parse(b";").unwrap().to_header_string().unwrap(),
        "Received:\r\n"
    );
    let mut value = Received::new();
    value.set_from_tcp_info(Some("[192.168.1.1]"));
    value.set_by(Some("smtp.target.com")).unwrap();
    assert_eq!(
        value.to_header_string().unwrap(),
        "Received: from  ([192.168.1.1]) by smtp.target.com\r\n"
    );
    let mut leading = Received::parse(b"(first)(second) x-Custom value by host").unwrap();
    // As in MimeKit, leading-comment clauses emit no keyword/value.
    leading.clauses_mut()[0].set_value("not emitted").unwrap();
    assert_eq!(
        leading.to_header_string().unwrap(),
        "Received: (first) (second) x-Custom value by host\r\n"
    );
    leading
        .clauses_mut()
        .push(ReceivedClause::new("BY", "duplicate").unwrap());
    assert!(
        leading
            .to_header_string()
            .unwrap()
            .ends_with("by host BY duplicate\r\n")
    );
}

#[test]
fn malformed_parentheses_stay_inside_their_comment() {
    for comment in [
        "",
        "balanced (nested (deep))",
        "(unclosed",
        "unexpected)",
        ") by injected (",
        ")(balanced counts but not nesting",
        "\\(literal\\)",
        "slash\\",
        "before\r\n\t) ; injected-date (",
    ] {
        let mut value = Received::new();
        value.set_from(Some("host")).unwrap();
        value.clauses_mut()[0]
            .comments_mut()
            .push(comment.to_owned());
        let before = value.clone();
        let output = value.to_header_string().unwrap();
        let parsed = parse_header(&output);
        assert_eq!(parsed.clauses().len(), 1, "{output:?}");
        assert_eq!(parsed.from(), Some("host"));
        assert_eq!(parsed.date_time(), None);
        assert_eq!(
            parsed.from_tcp_info().unwrap(),
            comment.replace(['\r', '\n'], "").replace('\t', " ")
        );
        assert_eq!(value, before);
    }
    let mut value = Received::new();
    value.set_from_tcp_info(Some(") by injected ("));
    assert_eq!(
        value.to_header_string().unwrap(),
        "Received: from  (\\) by injected \\()\r\n"
    );
}

#[test]
fn cached_dates_are_preserved_until_assigned() {
    let mut value = Received::parse(b"from host; Fri, 29 Nov 91 07:13:33 EST (original)").unwrap();
    let output = value.to_header_string().unwrap();
    assert!(output.ends_with("; Fri, 29 Nov 91 07:13:33 EST (original)\r\n"));
    value.set_by(Some("target")).unwrap();
    assert!(
        value
            .to_header_string()
            .unwrap()
            .contains("29 Nov 91 07:13:33 EST (original)")
    );
    value.set_date_time(value.date_time());
    assert!(
        value
            .to_header_string()
            .unwrap()
            .contains("29 Nov 1991 07:13:33 -0500")
    );
    assert!(!value.to_header_string().unwrap().contains("(original)"));
    value.set_date_time(None);
    assert!(!value.to_header_string().unwrap().contains(';'));
    let mut only_date = Received::new();
    only_date.set_date_time(Some(date()));
    assert_eq!(
        only_date.to_header_string().unwrap(),
        "Received:; Sat, 18 Apr 2026 20:05:38 -0400\r\n"
    );
}

#[test]
fn exact_keyword_value_comment_and_date_line_limits() {
    let options = Options {
        max_line_length: 60,
        ..Default::default()
    };
    for length in [59, 60] {
        let mut keyword = Received::new();
        keyword
            .clauses_mut()
            .push(ReceivedClause::new(&"x".repeat(length), "v").unwrap());
        let mut value = Received::new();
        value.set_with(Some(&"v".repeat(length))).unwrap();
        let mut comment = Received::new();
        comment.set_from_tcp_info(Some(&"c".repeat(length - 2)));
        let base = date().to_string();
        let stamp = format!("{base} ({})", "d".repeat(length - base.len() - 3));
        let date = Received::parse(format!("; {stamp}").as_bytes()).unwrap();
        for received in [keyword, value, comment, date] {
            let before = received.clone();
            let formatted = received.to_header_string_with_options(&options);
            if length == 59 {
                let output = formatted.unwrap();
                assert_lines(&output, 60, "\r\n");
                assert!(output.split("\r\n").any(|line| line.len() == 60));
            } else {
                assert_eq!(
                    formatted,
                    Err(Error::LineTooLong {
                        required: 61,
                        limit: 60
                    })
                );
            }
            assert_eq!(received, before);
        }
    }
    let mut escaped = Received::new();
    escaped.set_from_tcp_info(Some(&format!("{}a", "\\".repeat(28))));
    let output = escaped.to_header_string_with_options(&options).unwrap();
    assert_lines(&output, 60, "\r\n");
    assert!(output.split("\r\n").any(|line| line.len() == 60));
    escaped.set_from_tcp_info(Some(&"\\".repeat(29)));
    assert_eq!(
        escaped.to_header_string_with_options(&options),
        Err(Error::LineTooLong {
            required: 61,
            limit: 60
        })
    );

    let mut semicolon = Received::new();
    semicolon.set_with(Some(&"x".repeat(45))).unwrap();
    semicolon.set_date_time(Some(date()));
    assert_eq!(
        semicolon.to_header_string_with_options(&options).unwrap(),
        format!("Received: with {}\r\n\t; {}\r\n", "x".repeat(45), date())
    );
}

#[test]
fn first_comment_lookahead_respects_exact_group_boundaries() {
    let options = Options {
        max_line_length: 60,
        ..Default::default()
    };
    let host = "h".repeat(10);
    for (length, prefix) in [
        (32, "Received: "),
        (33, "Received:\r\n\t"),
        (41, "Received:\r\n\t"),
        (42, "Received: "),
    ] {
        let comment = "c".repeat(length);
        let mut value = Received::new();
        value.set_from(Some(&host)).unwrap();
        value.set_from_tcp_info(Some(&comment));
        let output = value.to_header_string_with_options(&options).unwrap();
        let expected = if length == 42 {
            format!("{prefix}from {host}\r\n\t({comment})\r\n")
        } else {
            format!("{prefix}from {host} ({comment})\r\n")
        };
        assert_eq!(output, expected);
        assert_lines(&output, 60, "\r\n");
    }
    let mut value = Received::new();
    value.set_from(Some("host")).unwrap();
    value.clauses_mut()[0]
        .comments_mut()
        .push(format!("{}x", "\r\n".repeat(500)));
    assert_eq!(
        value.to_header_string_with_options(&options).unwrap(),
        "Received: from host (x)\r\n"
    );
}

#[test]
fn all_supported_limits_and_line_endings_are_enforced() {
    let value = authored();
    for limit in 60..=998 {
        for (new_line_format, newline) in
            [(NewLineFormat::Dos, "\r\n"), (NewLineFormat::Unix, "\n")]
        {
            let options = Options {
                max_line_length: limit,
                new_line_format,
                ..Default::default()
            };
            let output = value.to_header_string_with_options(&options).unwrap();
            assert_lines(&output, limit, newline);
            let parsed = parse_header(&output);
            assert_eq!(parsed.clauses(), value.clauses());
            assert_eq!(parsed.date_time(), value.date_time());
        }
    }
    for limit in [0, 1, 59, 999, usize::MAX] {
        let options = Options {
            max_line_length: limit,
            ..Default::default()
        };
        assert!(matches!(
            value.to_header_string_with_options(&options),
            Err(Error::InvalidLineLength(_))
        ));
    }
    let options = Options {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert_eq!(
        value.to_header_string_with_options(&options),
        Err(Error::InvalidNewLineFormat(NewLineFormat::Mixed))
    );
}

#[test]
fn utf8_requires_opt_in_and_limits_count_bytes() {
    let options = Options {
        international: true,
        max_line_length: 60,
        ..Default::default()
    };
    for (input, part, offset) in [
        ("\u{e9} value", Part::Keyword(0), 0),
        ("from caf\u{e9}.example", Part::Value(0), 3),
        (
            "from host (caf\u{e9})",
            Part::Comment {
                clause: 0,
                comment: 0,
            },
            3,
        ),
        (
            "; Sat, 18 Apr 2026 20:05:38 -0400 (\u{e9})",
            Part::DateTime,
            33,
        ),
    ] {
        let value = Received::parse(input.as_bytes()).unwrap();
        let result = value.to_header_string();
        assert_eq!(result, Err(Error::NonAscii { part, offset }));
        let output = value.to_header_string_with_options(&options).unwrap();
        assert_lines(&output, 60, "\r\n");
        assert_eq!(parse_header(&output), value);
    }
    let mut value = Received::new();
    value
        .set_with(Some(&format!("{}a", "\u{e9}".repeat(29))))
        .unwrap();
    let output = value.to_header_string_with_options(&options).unwrap();
    assert!(output.split("\r\n").any(|line| line.len() == 60));
    value.set_with(Some(&"\u{e9}".repeat(30))).unwrap();
    assert_eq!(
        value.to_header_string_with_options(&options),
        Err(Error::LineTooLong {
            required: 61,
            limit: 60
        })
    );
}

#[test]
fn controls_have_component_and_original_byte_offsets() {
    for (input, part, offset) in [
        ("x\0 value", Part::Keyword(0), 1),
        ("from host\u{7f}", Part::Value(0), 4),
        (
            "; Sat, 18 Apr 2026 20:05:38 -0400 (bad\0comment)",
            Part::DateTime,
            36,
        ),
    ] {
        let value = Received::parse(input.as_bytes()).unwrap();
        let before = value.clone();
        assert_eq!(
            value.to_header_string(),
            Err(Error::InvalidControl { part, offset })
        );
        assert_eq!(value, before);
    }
    for control in (0u8..=31)
        .chain([127])
        .filter(|byte| !b"\r\n\t".contains(byte))
    {
        let mut value = Received::new();
        value.set_from(Some("host")).unwrap();
        value.clauses_mut()[0]
            .comments_mut()
            .push(format!("a\r\n\tb{}", char::from(control)));
        assert_eq!(
            value.to_header_string(),
            Err(Error::InvalidControl {
                part: Part::Comment {
                    clause: 0,
                    comment: 0
                },
                offset: 5,
            })
        );
    }
    let mut value = Received::new();
    value.set_from(Some("host")).unwrap();
    value.clauses_mut()[0]
        .comments_mut()
        .extend(["first".into(), "x\r\n\u{e9}".into()]);
    assert_eq!(
        value.to_header_string(),
        Err(Error::NonAscii {
            part: Part::Comment {
                clause: 0,
                comment: 1
            },
            offset: 3,
        })
    );
}

#[test]
fn deeply_nested_or_large_comments_fail_without_recursion_or_mutation() {
    for comment in [
        format!("{}text{}", "(".repeat(50_000), ")".repeat(50_000)),
        "(".repeat(100_000),
        "x".repeat(100_000),
    ] {
        let mut value = Received::new();
        value.set_from_tcp_info(Some(&comment));
        let before = value.clone();
        assert!(matches!(
            value.to_header_string(),
            Err(Error::LineTooLong { .. })
        ));
        assert_eq!(value, before);
    }
}

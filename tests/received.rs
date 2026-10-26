// Adapted from MimeKit's UnitTests/ReceivedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    DateTimeOffset, Header, Received, ReceivedClauseId as Clause,
    ReceivedParseErrorKind as ErrorKind, ReceivedParseOptions, charsets::Charset,
};

type Case = (&'static str, &'static [(&'static str, &'static str)]);

const VALID: &[Case] = &[
    // Syntactically normal Received headers.
    // Example from RFC 5321.
    (
        " from bar.com by foo.com ; Thu, 21 May 1998\r\n\t05:33:29 -0700\r\n",
        &[
            ("From", "bar.com"),
            ("By", "foo.com"),
            ("DateTime", "Thu, 21 May 1998 05:33:29 -0700"),
        ],
    ),
    (
        " from thumper.bellcore.com by greenbush.bellcore.com (4.1/4.7)\r\n\tid <AA01648> for nsb; Fri, 29 Nov 91 07:13:33 EST\r\n",
        &[
            ("From", "thumper.bellcore.com"),
            ("By", "greenbush.bellcore.com"),
            ("ByTcpInfo", "4.1/4.7"),
            ("Id", "<AA01648>"),
            ("For", "nsb"),
            ("DateTime", "Fri, 29 Nov 1991 07:13:33 -0500"),
        ],
    ),
    (
        " from joyce.cs.su.oz.au by thumper.bellcore.com (4.1/4.7)\r\n\tid <AA11898> for nsb@greenbush; Fri, 29 Nov 91 07:11:57 EST\r\n",
        &[
            ("From", "joyce.cs.su.oz.au"),
            ("By", "thumper.bellcore.com"),
            ("ByTcpInfo", "4.1/4.7"),
            ("Id", "<AA11898>"),
            ("For", "nsb@greenbush"),
            ("DateTime", "Fri, 29 Nov 1991 07:11:57 -0500"),
        ],
    ),
    (
        " from Messages.8.5.N.CUILIB.3.45.SNAP.NOT.LINKED.greenbush.galaxy.sun4.41\r\n\tvia MS.5.6.greenbush.galaxy.sun4_41; Fri, 12 Jun 1992 13:29:05 -0400 (EDT)\r\n",
        &[
            (
                "From",
                "Messages.8.5.N.CUILIB.3.45.SNAP.NOT.LINKED.greenbush.galaxy.sun4.41",
            ),
            ("Via", "MS.5.6.greenbush.galaxy.sun4_41"),
            ("DateTime", "Fri, 12 Jun 1992 13:29:05 -0400"),
        ],
    ),
    (
        " from sqhilton.pc.cs.cmu.edu by po3.andrew.cmu.edu (5.54/3.15)\r\n\tid <AA21478> for beatty@cosmos.vlsi.cs.cmu.edu; Wed, 26 Aug 92 22:14:07 EDT\r\n",
        &[
            ("From", "sqhilton.pc.cs.cmu.edu"),
            ("By", "po3.andrew.cmu.edu"),
            ("ByTcpInfo", "5.54/3.15"),
            ("Id", "<AA21478>"),
            ("For", "beatty@cosmos.vlsi.cs.cmu.edu"),
            ("DateTime", "Wed, 26 Aug 1992 22:14:07 -0400"),
        ],
    ),
    (
        " from [127.0.0.1] by [127.0.0.1] id <AA21478> with sendmail (v1.8)\r\n\tfor <beatty@cosmos.vlsi.cs.cmu.edu>; Wed, 26 Aug 92 22:14:07 EDT\r\n",
        &[
            ("From", "[127.0.0.1]"),
            ("By", "[127.0.0.1]"),
            ("Id", "<AA21478>"),
            ("With", "sendmail"),
            ("For", "<beatty@cosmos.vlsi.cs.cmu.edu>"),
            ("DateTime", "Wed, 26 Aug 1992 22:14:07 -0400"),
        ],
    ),
    (
        " from smtp.domain.com (smtp.domain.com. [207.54.68.120])\r\n        by mx.google.com with ESMTPS id 4fb4d7f45d1cf-659329877a1si67605a12.45.2026.02.02.09.25.54\r\n        for <user@gmail.com>\r\n        (version=TLS1_2 cipher=ECDHE-ECDSA-CHACHA20-POLY1305 bits=256/256);\r\n        Mon, 02 Feb 2026 09:25:55 -0800 (PST)\r\n",
        &[
            ("From", "smtp.domain.com"),
            ("FromTcpInfo", "smtp.domain.com. [207.54.68.120]"),
            ("By", "mx.google.com"),
            ("With", "ESMTPS"),
            (
                "Id",
                "4fb4d7f45d1cf-659329877a1si67605a12.45.2026.02.02.09.25.54",
            ),
            ("For", "<user@gmail.com>"),
            ("DateTime", "Mon, 02 Feb 2026 09:25:55 -0800"),
        ],
    ),
    // No date-time token provided.
    (
        " from smtp.domain.com by mx.google.com with ESMTPS\r\n\tid 4fb4d7f45d1cf-659329877a1si67605a12.45.2026.02.02.09.25.54\r\n\tfor <user@gmail.com>\r\n\t(version=TLS1_2 cipher=ECDHE-ECDSA-CHACHA20-POLY1305 bits=256/256)\r\n",
        &[
            ("From", "smtp.domain.com"),
            ("By", "mx.google.com"),
            ("With", "ESMTPS"),
            (
                "Id",
                "4fb4d7f45d1cf-659329877a1si67605a12.45.2026.02.02.09.25.54",
            ),
            ("For", "<user@gmail.com>"),
        ],
    ),
    (
        " from relay301.mycloudmailbox.com ([207.126.101.249])\r\n\tby SJ0PR20MB4463.namprd00.prod.outlook.com with ESMTPS id\r\n\t<SJ0PR20MB446393F2410E314E4704AE53C7202@SJ0PR20MB4463.namprd00.prod.outlook.com>\r\n\tfor <unit-tests@mimekit.net>; Sun, 19 Apr 2026 15:47:52 -0400 (EDT)\r\n",
        &[
            ("From", "relay301.mycloudmailbox.com"),
            ("FromTcpInfo", "[207.126.101.249]"),
            ("By", "SJ0PR20MB4463.namprd00.prod.outlook.com"),
            ("With", "ESMTPS"),
            (
                "Id",
                "<SJ0PR20MB446393F2410E314E4704AE53C7202@SJ0PR20MB4463.namprd00.prod.outlook.com>",
            ),
            ("For", "<unit-tests@mimekit.net>"),
            ("DateTime", "Sun, 19 Apr 2026 15:47:52 -0400"),
        ],
    ),
    (
        " from relay301.mycloudmailbox.com ([207.126.101.249])\r\n\tby SJ0PR20MB4463.namprd00.prod.outlook.com ([127.0.0.1]) via TCP with ESMTPS\r\n\tid\r\n\t<SJ0PR20MB446393F2410E314E4704AE53C7202@SJ0PR20MB4463.namprd00.prod.outlook.com>\r\n\tfor <unit-tests@mimekit.net>; Sun, 19 Apr 2026 15:47:52 -0400 (EDT)\r\n",
        &[
            ("From", "relay301.mycloudmailbox.com"),
            ("FromTcpInfo", "[207.126.101.249]"),
            ("By", "SJ0PR20MB4463.namprd00.prod.outlook.com"),
            ("ByTcpInfo", "[127.0.0.1]"),
            ("Via", "TCP"),
            ("With", "ESMTPS"),
            (
                "Id",
                "<SJ0PR20MB446393F2410E314E4704AE53C7202@SJ0PR20MB4463.namprd00.prod.outlook.com>",
            ),
            ("For", "<unit-tests@mimekit.net>"),
            ("DateTime", "Sun, 19 Apr 2026 15:47:52 -0400"),
        ],
    ),
    // Non-compliant Received headers that actually exist in the wild.
    // Starts with a 'by' clause.
    (
        " by 2002:aa6:d946:0:b0:32a:1941:595b with SMTP id w6csp1900485lkc;\r\n        Mon, 2 Feb 2026 09:25:55 -0800 (PST)\r\n",
        &[
            ("By", "2002:aa6:d946:0:b0:32a:1941:595b"),
            ("With", "SMTP"),
            ("Id", "w6csp1900485lkc"),
            ("DateTime", "Mon, 02 Feb 2026 09:25:55 -0800"),
        ],
    ),
    // "Microsoft SMTP Server" and "Frontend Transport" are not single-word
    // values, but according to RFC 5321 they should be.
    (
        " from us-smtp-delivery-105.mimecast.com (216.205.24.105)\r\n\tby BN3NAM04FT018.mail.protection.outlook.com (10.152.92.162) with Microsoft\r\n\tSMTP Server (version=TLS1_2, cipher=TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384)\r\n\tid 15.20.1835.13 via Frontend Transport; Tue, 30 Apr 2019 19:10:19 +0000\r\n",
        &[
            ("From", "us-smtp-delivery-105.mimecast.com"),
            ("FromTcpInfo", "216.205.24.105"),
            ("By", "BN3NAM04FT018.mail.protection.outlook.com"),
            ("ByTcpInfo", "10.152.92.162"),
            ("With", "Microsoft SMTP Server"),
            ("Id", "15.20.1835.13"),
            ("Via", "Frontend Transport"),
            ("DateTime", "Tue, 30 Apr 2019 19:10:19 +0000"),
        ],
    ),
    // Nested comments.
    (
        " from [67.219.246.196] (using TLSv1.2 with cipher DHE-RSA-AES256-GCM-SHA384 (256 bits))\r\n\tby server-2.bemta.az-c.us-east-1.aws.symcld.net id 11/DD-19573-41C55BC5;\r\n\tTue, 16 Apr 2019 04:37:40 +0000\r\n",
        &[
            ("From", "[67.219.246.196]"),
            (
                "FromTcpInfo",
                "using TLSv1.2 with cipher DHE-RSA-AES256-GCM-SHA384 (256 bits)",
            ),
            ("By", "server-2.bemta.az-c.us-east-1.aws.symcld.net"),
            ("Id", "11/DD-19573-41C55BC5"),
            ("DateTime", "Tue, 16 Apr 2019 04:37:40 +0000"),
        ],
    ),
    // No clauses.
    (
        " (qmail 16244 invoked from network); 16 Apr 2019 04:37:38 -0000\r\n",
        &[("DateTime", "Tue, 16 Apr 2019 04:37:38 +0000")],
    ),
    // Multiple comments in the 'from' clause.
    (
        " from smtp.source.com (HELO localhost) ([160.46.252.39])\r\n\tby smtp.destination.com with ESMTP/TLS; 02 Feb 2026 18:25:54 +0100\r\n",
        &[
            ("From", "smtp.source.com"),
            ("FromTcpInfo", "HELO localhost"),
            ("By", "smtp.destination.com"),
            ("With", "ESMTP/TLS"),
            ("DateTime", "Mon, 02 Feb 2026 18:25:54 +0100"),
        ],
    ),
    (
        " from relay301.mycloudmailbox.com (unknown [207.126.101.249])\r\n\t(using TLSv1.2 with cipher ECDHE-RSA-AES128-SHA256 (128/128 bits))\r\n\t(No client certificate requested)\r\n\tby S15-GW103.mycloudmailbox.com (Postfix) with ESMTPS id 44th580QHjz2SnDr\r\n\tfor <unit-tests@mimekit.net>; Tue, 30 Apr 2019 08:42:52 -0400 (EDT)\r\n",
        &[
            ("From", "relay301.mycloudmailbox.com"),
            ("FromTcpInfo", "unknown [207.126.101.249]"),
            ("By", "S15-GW103.mycloudmailbox.com"),
            ("ByTcpInfo", "Postfix"),
            ("With", "ESMTPS"),
            ("Id", "44th580QHjz2SnDr"),
            ("For", "<unit-tests@mimekit.net>"),
            ("DateTime", "Tue, 30 Apr 2019 08:42:52 -0400"),
        ],
    ),
    // No value after 'from' keyword.
    (
        " from  (this 'from' clause has no value) by smtp.destination.com\r\n\twith ESMTP/TLS; 02 Feb 2026 18:25:54 +0100\r\n",
        &[
            ("From", ""),
            ("FromTcpInfo", "this 'from' clause has no value"),
            ("By", "smtp.destination.com"),
            ("With", "ESMTP/TLS"),
            ("DateTime", "Mon, 02 Feb 2026 18:25:54 +0100"),
        ],
    ),
    // Semicolon and no value.
    (
        " from ; 02 Feb 2026 18:25:54 +0100\r\n",
        &[
            ("From", ""),
            ("DateTime", "Mon, 02 Feb 2026 18:25:54 +0100"),
        ],
    ),
    // No date-time token provided after ';'.
    (
        " from smtp.domain.com by mx.google.com with ESMTPS;\r\n",
        &[
            ("From", "smtp.domain.com"),
            ("By", "mx.google.com"),
            ("With", "ESMTPS"),
        ],
    ),
];

fn check_fields(received: &Received, expected: &[(&str, &str)]) {
    let date = received.date_time().map(|date| date.to_string());
    for (name, actual) in [
        ("From", received.from()),
        ("FromTcpInfo", received.from_tcp_info()),
        ("By", received.by()),
        ("ByTcpInfo", received.by_tcp_info()),
        ("Via", received.via()),
        ("With", received.with()),
        ("Id", received.id()),
        ("For", received.for_recipient()),
        ("DateTime", date.as_deref()),
    ] {
        let expected = expected
            .iter()
            .find(|&&(key, _)| key == name)
            .map(|&(_, value)| value);
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn upstream_valid_header_values() {
    assert_eq!(VALID.len(), 19);
    for &(input, expected) in VALID {
        let received = Received::parse(input.as_bytes()).unwrap();
        check_fields(&received, expected);
        assert!(!received.had_charset_fallback());
        assert_eq!(input.parse::<Received>().unwrap(), received);
        assert_eq!(
            Received::parse_with_options(input.as_bytes(), &ReceivedParseOptions::default())
                .unwrap(),
            received
        );
        let framed = format!("ignored{input}ignored");
        assert_eq!(
            Received::parse(&framed.as_bytes()[7..framed.len() - 7]).unwrap(),
            received
        );
    }
}

#[test]
fn upstream_invalid_header_values_and_offsets() {
    use ErrorKind::{DuplicateClause, IncompleteComment, InvalidDateTime};
    for (input, kind, start, end) in [
        // Unterminated comments.
        (
            " (this is an unterminated comment...",
            IncompleteComment,
            1,
            None,
        ),
        (
            " from (this is an unterminated comment...",
            IncompleteComment,
            6,
            None,
        ),
        (
            " from remote-host.com (this is an unterminated comment...",
            IncompleteComment,
            22,
            None,
        ),
        (
            " from remote-host.com by smtp.local-host.com (this is an unterminated comment...",
            IncompleteComment,
            45,
            None,
        ),
        // Duplicate clauses.
        (
            " from remote-host.com from remote-host.com",
            DuplicateClause(Clause::From),
            22,
            Some(26),
        ),
        (
            " from remote-host.com by smtp.local-host.com by duplicate-host.com",
            DuplicateClause(Clause::By),
            45,
            Some(47),
        ),
        (
            " from remote-host.com by smtp.local-host.com via Frontend Transport via Backend Transport",
            DuplicateClause(Clause::Via),
            68,
            Some(71),
        ),
        (
            " from remote-host.com by smtp.local-host.com with SMTP with ESMTP",
            DuplicateClause(Clause::With),
            55,
            Some(59),
        ),
        (
            " from remote-host.com by smtp.local-host.com id <123@localhost> id <456@localhost>",
            DuplicateClause(Clause::Id),
            64,
            Some(66),
        ),
        (
            " from remote-host.com by smtp.local-host.com for user@domain.com for user@domain.com",
            DuplicateClause(Clause::For),
            65,
            Some(68),
        ),
        (
            " from remote-host.com by smtp.local-host.com for user@domain.com; !^#&*^*@%@^*&#*&!*",
            InvalidDateTime,
            66,
            Some(84),
        ),
    ] {
        let error = Received::parse(input.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.token_offset(), start);
        assert_eq!(error.offset(), end.unwrap_or(input.len()));
        assert!(error.to_string().contains(&format!("byte {start}")));
        let framed = format!("prefix{input}suffix");
        assert_eq!(
            Received::parse(&framed.as_bytes()[6..framed.len() - 6]).unwrap_err(),
            error
        );
    }
}

#[test]
fn parsed_comments_and_escapes() {
    let input = b" from smtp.source.com (HELO\r\n\t[192.168.1.1])\r\n\tby smtp.target.com (\r\n\t[127.0.0.1]\r\n\t); Sat, 18 Apr 2026 20:05:38 -0400\r\n";
    let received = Received::parse(input).unwrap();
    assert_eq!(received.clauses().len(), 2);
    assert_eq!(received.from_tcp_info(), Some("HELO [192.168.1.1]"));
    assert_eq!(received.by_tcp_info(), Some(" [127.0.0.1] "));
    assert_eq!(
        received.date_time(),
        Some(DateTimeOffset::new(2026, 4, 18, 20, 5, 38, -240).unwrap())
    );

    let received = Received::parse(
        b" from smtp.source.com (escaped\\\tspace\\\r\nignored) by smtp.target.com\r\n",
    )
    .unwrap();
    assert_eq!(received.from_tcp_info(), Some("escaped spaceignored"));
    assert_eq!(received.by(), Some("smtp.target.com"));
    let received =
        Received::parse(b" from smtp.source.com (\\\\escaped\\\\) by smtp.target.com\r\n").unwrap();
    assert_eq!(received.from_tcp_info(), Some("\\escaped\\"));

    let received = Received::parse(
        b"(before)(second) from (pre) host (one (nested)) (two\\(escaped\\)) by (empty)\r\n",
    )
    .unwrap();
    assert_eq!(received.clauses().len(), 3);
    let leading = &received.clauses()[0];
    assert_eq!(leading.id(), Clause::None);
    assert_eq!(leading.keyword(), "");
    assert_eq!(leading.value(), "");
    assert_eq!(leading.comments(), ["before", "second"]);
    assert_eq!(
        received.clauses()[1].comments(),
        ["pre", "one (nested)", "two(escaped)"]
    );
    assert_eq!(received.from_tcp_info(), Some("pre"));
    assert_eq!(received.by(), Some(""));
    assert_eq!(received.by_tcp_info(), Some("empty"));
}

#[test]
fn ordered_unknown_and_empty_clauses() {
    let received = Received::parse(b" x-custom value by smtp.target.com\r\n").unwrap();
    assert_eq!(received.clauses()[0].id(), Clause::Unknown);
    assert_eq!(received.clauses()[0].keyword(), "x-custom");
    assert_eq!(received.clauses()[0].value(), "value");
    assert_eq!(received.by(), Some("smtp.target.com"));
    let received = Received::parse(
        b"WITH SMTP From by target VIA ID For (note) x-custom one(comment)x-custom two",
    )
    .unwrap();
    let actual: Vec<_> = received
        .clauses()
        .iter()
        .map(|clause| (clause.id(), clause.keyword(), clause.value()))
        .collect();
    assert_eq!(
        actual,
        [
            (Clause::With, "WITH", "SMTP"),
            (Clause::From, "From", ""),
            (Clause::By, "by", "target"),
            (Clause::Via, "VIA", ""),
            (Clause::Id, "ID", ""),
            (Clause::For, "For", "x-custom one"),
            (Clause::Unknown, "x-custom", "two"),
        ]
    );
    let received = Received::parse(b"x-custom one(comment)x-custom two").unwrap();
    assert_eq!(received.clauses().len(), 2);
    assert!(
        received
            .clauses()
            .iter()
            .all(|clause| clause.id() == Clause::Unknown)
    );
    assert_eq!(received.clause(Clause::Unknown).unwrap().value(), "one");
    assert!(received.clause(Clause::From).is_none());
    for input in ["from from", "From FROM", "BY bY"] {
        assert!(matches!(
            Received::parse(input.as_bytes()).unwrap_err().kind(),
            ErrorKind::DuplicateClause(_)
        ));
    }
}

#[test]
fn date_spelling_is_preserved_separately() {
    let received =
        Received::parse(b"; \r\n Fri, 29 Nov 91\t07:13:33 EST\r\n \t(note)\t \r\n").unwrap();
    assert!(received.clauses().is_empty());
    assert_eq!(
        received.date_time_text(),
        Some("Fri, 29 Nov 91 07:13:33 EST  (note)")
    );
    assert_eq!(
        received.date_time().unwrap().to_string(),
        "Fri, 29 Nov 1991 07:13:33 -0500"
    );
    for input in ["", " \t\r\n", ";", " ; \t\r\n"] {
        let received = Received::parse(input.as_bytes()).unwrap();
        assert!(received.clauses().is_empty());
        assert!(received.date_time().is_none());
        assert!(received.date_time_text().is_none());
    }
    let received = Received::parse(b"from").unwrap();
    assert_eq!(received.from(), Some(""));
    let error = Received::parse(b"; \t not-a-date \t\r\n").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidDateTime);
    assert_eq!(error.token_offset(), 4);
    assert_eq!(error.offset(), 14);
    let received = Received::parse(b"; 16 Apr 2019 04:37:38 -0000 (unterminated").unwrap();
    assert!(received.date_time().is_some());
    assert!(
        received
            .date_time_text()
            .unwrap()
            .ends_with("(unterminated")
    );
}

#[test]
fn charset_fallback_is_explicit_and_per_token() {
    let mut options = ReceivedParseOptions {
        fallback_charset: Some(Charset::from_label("windows-1252").unwrap()),
    };
    let received = Received::parse_with_options(
        b"from caf\xc3\xa9 (\x80) by caf\xe9 x\x80; 16 Apr 2019 04:37:38 -0000 (\x80)",
        &options,
    )
    .unwrap();
    assert_eq!(received.from(), Some("caf\u{e9}"));
    assert_eq!(received.from_tcp_info(), Some("\u{20ac}"));
    assert_eq!(received.by(), Some("caf\u{e9} x\u{20ac}"));
    assert!(received.had_charset_fallback());
    assert!(received.date_time_text().unwrap().ends_with("(\u{20ac})"));
    let received = Received::parse(b"x\x80 v\x80 (c\x80)").unwrap();
    assert!(received.had_charset_fallback());
    assert_eq!(received.clauses()[0].keyword(), "x\u{80}");
    assert_eq!(received.clauses()[0].value(), "v\u{80}");
    assert_eq!(received.clauses()[0].comments(), ["c\u{80}"]);
    options.fallback_charset = Some(Charset::UTF8);
    assert_eq!(
        Received::parse_with_options(b"from \xff", &options)
            .unwrap()
            .from(),
        Some("\u{ff}")
    );
    options.fallback_charset = Some(Charset::from_label("shift_jis").unwrap());
    assert_eq!(
        Received::parse_with_options(b"from \x82", &options)
            .unwrap()
            .from(),
        Some("\u{82}")
    );
    assert_eq!(
        Received::parse_with_options(b"from \x82\xa0", &options)
            .unwrap()
            .from(),
        Some("\u{3042}")
    );
    let received: Received = "from caf\u{e9} (caf\u{e9})".parse().unwrap();
    assert!(!received.had_charset_fallback());
    let error = Received::parse("from \u{e9} from second".as_bytes()).unwrap_err();
    assert_eq!(error.token_offset(), 8);
    assert_eq!(error.offset(), 12);
}

#[test]
fn parser_does_not_validate_or_decode_values() {
    let received = Received::parse(b"from smtp..invalid by [not-an-ip] with Microsoft\r\n\tSMTP  Server id <> for =?utf-8?q?name?=").unwrap();
    assert_eq!(received.from(), Some("smtp..invalid"));
    assert_eq!(received.by(), Some("[not-an-ip]"));
    assert_eq!(received.with(), Some("Microsoft SMTP Server"));
    assert_eq!(received.id(), Some("<>"));
    assert_eq!(received.for_recipient(), Some("=?utf-8?q?name?="));
    let header = Header::parse(b"Received: from source by target").unwrap();
    let before = header.raw_value().to_vec();
    assert_eq!(
        Received::parse(header.raw_value()).unwrap().by(),
        Some("target")
    );
    assert_eq!(header.raw_value(), before);
    assert_eq!(
        Received::parse(b"from a\x0bb\0c").unwrap().from(),
        Some("a\u{b}b\0c")
    );
    assert_eq!(
        Received::parse(b"from \"unterminated").unwrap().from(),
        Some("\"unterminated")
    );
}

#[test]
fn deeply_nested_and_large_comments_are_iterative() {
    let depth = 50000;
    let input = format!("{}text{} from host", "(".repeat(depth), ")".repeat(depth));
    let received: Received = input.parse().unwrap();
    assert_eq!(received.clauses()[0].comments()[0].len(), depth * 2 + 2);
    assert_eq!(received.from(), Some("host"));
    assert_eq!(
        Received::parse("(".repeat(depth).as_bytes())
            .unwrap_err()
            .offset(),
        depth
    );
    let input = format!("from host ({})", "x".repeat(100000));
    assert_eq!(
        Received::parse(input.as_bytes())
            .unwrap()
            .from_tcp_info()
            .unwrap()
            .len(),
        100000
    );
}

#[test]
fn arbitrary_bytes_and_truncated_prefixes_make_progress() {
    for &(input, _) in VALID {
        for end in 0..=input.len() {
            if let Err(error) = Received::parse(&input.as_bytes()[..end]) {
                assert!(error.token_offset() <= error.offset());
                assert!(error.offset() <= end);
            }
        }
    }
    let mut seed = 123456789u32;
    for _ in 0..1000 {
        let input: Vec<_> = (0..128)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 16) as u8
            })
            .collect();
        if let Err(error) = Received::parse(&input) {
            assert!(error.token_offset() <= error.offset());
            assert!(error.offset() <= input.len());
        }
    }
}

// Adapted from MimeKit's UnitTests/ReceivedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, net::IpAddr};

use mimecraft::{
    DateTimeOffset, Received, ReceivedClause, ReceivedClauseId as Id, ReceivedEditError,
};

type Setter = fn(&mut Received, Option<&str>) -> Result<(), ReceivedEditError>;

const SETTERS: [(Id, Setter, &str); 6] = [
    (Id::From, Received::set_from, "smtp.source.com"),
    (Id::By, Received::set_by, "smtp.target.com"),
    (Id::Via, Received::set_via, "TCP"),
    (Id::With, Received::set_with, "ESMTPS"),
    (
        Id::Id,
        Received::set_id,
        "<VAD7UBNO1TU4.JCMO6CD121AX1@office365.com>",
    ),
    (
        Id::For,
        Received::set_for_recipient,
        "unit-tests@mimekit.net",
    ),
];

fn date() -> DateTimeOffset {
    DateTimeOffset::new(2026, 4, 18, 20, 5, 38, -240).unwrap()
}

#[test]
fn upstream_constructors_and_classification() {
    let empty = Received::new();
    assert_eq!(empty, Received::default());
    assert_eq!(empty, Received::parse(b"").unwrap());
    assert!(empty.clauses().is_empty());
    assert_eq!(empty.date_time(), None);
    assert_eq!(empty.date_time_text(), None);
    assert!(!empty.had_charset_fallback());

    let received = Received::with_hosts(
        "smtp.source.com",
        "192.168.1.1".parse().unwrap(),
        "smtp.target.com",
        "127.0.0.1".parse().unwrap(),
        date(),
    )
    .unwrap();
    assert_eq!(received.clauses().len(), 2);
    assert_eq!(received.from(), Some("smtp.source.com"));
    assert_eq!(received.from_tcp_info(), Some("[192.168.1.1]"));
    assert_eq!(received.by(), Some("smtp.target.com"));
    assert_eq!(received.by_tcp_info(), Some("[127.0.0.1]"));
    assert_eq!(received.date_time(), Some(date()));
    assert_eq!(received.date_time_text(), None);

    for (keyword, value, id) in [
        ("from", "value", Id::From),
        ("by", "value", Id::By),
        ("via", "value", Id::Via),
        ("with", "value", Id::With),
        ("id", "value", Id::Id),
        ("for", "value", Id::For),
        ("FrOm", "source.example", Id::From),
        ("x-custom", "multi word value", Id::Unknown),
        ("X-", " ", Id::Unknown),
    ] {
        let clause = ReceivedClause::with_comment(keyword, value, "comment").unwrap();
        assert_eq!(clause.id(), id);
        assert_eq!(clause.keyword(), keyword);
        assert_eq!(clause.value(), value);
        assert_eq!(clause.comments(), ["comment"]);
    }

    let ip: IpAddr = "::1".parse().unwrap();
    let received = Received::with_hosts("[::1]", ip, "[127.0.0.1]", ip, date()).unwrap();
    assert_eq!(received.from_tcp_info(), Some("[::1]"));
    assert_eq!(received.by_tcp_info(), Some("[::1]"));
    for (from, by) in [("", "target"), ("source", ""), ("bad host", "target")] {
        assert!(Received::with_hosts(from, ip, by, ip, date()).is_err());
    }
}

#[test]
fn upstream_invalid_assignments_are_atomic() {
    let hosts = [
        "",
        "[127.",
        "[xyz.!@#.$%^.&*()]",
        "smtp .gmail.com",
        "smtp\t.gmail.com",
        "smtp\u{7}.gmail.com",
        "smtp\u{7f}.gmail.com",
        "abc[def",
        "abc\\def",
        "smtp..server.com",
        "smtp.-server.com",
    ];
    let ids = [
        "",
        "this is invalid...",
        "<>",
        "<id@example.com",
        "<id@example.com> garbage",
        "id@example.com\r\nBcc: evil@example.com",
        "id@example.com\r\n (comment)",
        "id@example.com\0",
    ];
    for (id, setter, valid) in SETTERS {
        let invalid: &[&str] = match id {
            Id::From | Id::By => &hosts,
            Id::Id => &ids,
            Id::For => &["", "this is invalid..."],
            _ => &[
                "",
                "illegal\u{7}char",
                "semi;colon",
                "(comment)",
                "closing)",
            ],
        };
        let mut received = Received::new();
        for value in invalid {
            let before = received.clone();
            assert!(
                setter(&mut received, Some(value)).is_err(),
                "{id:?}: {value:?}"
            );
            assert_eq!(received, before);
        }
        setter(&mut received, Some(valid)).unwrap();
        received.clauses_mut()[0].comments_mut().push("kept".into());
        for value in invalid {
            let before = received.clone();
            assert!(
                setter(&mut received, Some(value)).is_err(),
                "{id:?}: {value:?}"
            );
            assert_eq!(received, before);
            assert!(received.clauses_mut()[0].set_value(value).is_err());
            assert_eq!(received, before);
        }
    }

    for (keyword, offset) in [
        ("", 0),
        ("keyword with spaces", 7),
        ("-keyword-starting-with-dash", 0),
        ("key!word", 3),
        ("x-1", 2),
        ("x-\u{e9}", 2),
        ("from\r\n", 4),
    ] {
        assert_eq!(
            ReceivedClause::new(keyword, "value"),
            Err(ReceivedEditError::InvalidKeyword { offset })
        );
    }
    assert_eq!(
        ReceivedClause::new("keyword", ""),
        Err(ReceivedEditError::EmptyValue {
            clause: Id::Unknown
        })
    );
}

#[test]
fn upstream_setting_updating_and_removing_properties() {
    let mut received = Received::new();
    for (_, setter, value) in SETTERS.into_iter().rev() {
        setter(&mut received, Some(value)).unwrap();
    }
    received.set_from_tcp_info(Some("[192.168.1.1]"));
    received.set_by_tcp_info(Some("[127.0.0.1]"));
    received.set_date_time(Some(date()));
    assert_eq!(received.clauses().len(), 6);
    assert_eq!(received.from(), Some("smtp.source.com"));
    assert_eq!(received.by(), Some("smtp.target.com"));
    assert_eq!(received.via(), Some("TCP"));
    assert_eq!(received.with(), Some("ESMTPS"));
    assert_eq!(received.id(), Some(SETTERS[4].2));
    assert_eq!(received.for_recipient(), Some("unit-tests@mimekit.net"));
    assert_eq!(received.date_time(), Some(date()));
    assert_eq!(
        received
            .clauses()
            .iter()
            .map(ReceivedClause::id)
            .collect::<Vec<_>>(),
        [Id::From, Id::By, Id::Via, Id::With, Id::Id, Id::For]
    );

    received.set_from(Some("smtp.gmail.com")).unwrap();
    assert_eq!(received.from(), Some("smtp.gmail.com"));
    assert_eq!(received.from_tcp_info(), Some("[192.168.1.1]"));
    for (id, setter, _) in SETTERS {
        let mut copy = received.clone();
        setter(&mut copy, None).unwrap();
        assert_eq!(copy.clause(id), None);
        assert_eq!(copy.clauses().len(), 5);
        // Removing an absent clause is a no-op.
        let before = copy.clone();
        setter(&mut copy, None).unwrap();
        assert_eq!(copy, before);
    }
    received.set_from(None).unwrap();
    received.set_by(None).unwrap();
    assert_eq!(received.from_tcp_info(), None);
    assert_eq!(received.by_tcp_info(), None);
    received.set_date_time(None);
    assert_eq!(received.date_time(), None);
}

#[test]
fn upstream_tcp_info_creation_unfolding_and_removal() {
    let mut received = Received::new();
    received.set_from_tcp_info(None);
    received.set_by_tcp_info(None);
    assert!(received.clauses().is_empty());
    received.set_by(Some("smtp.target.com")).unwrap();
    received.set_from_tcp_info(Some("[192.168.1.1]"));
    assert_eq!(received.from(), Some(""));
    assert_eq!(received.from_tcp_info(), Some("[192.168.1.1]"));
    assert_eq!(received.clauses()[0].id(), Id::From);
    let before = received.clone();
    assert!(received.set_from(Some("")).is_err());
    assert_eq!(received, before);
    received.set_from(Some("smtp.source.com")).unwrap();
    received.set_from_tcp_info(Some("HELO\t[192.\r\n168.1.1]"));
    received.set_by_tcp_info(Some("[127.0.\r\n0.1]"));
    assert_eq!(received.from_tcp_info(), Some("HELO [192.168.1.1]"));
    assert_eq!(received.by_tcp_info(), Some("[127.0.0.1]"));

    received.clauses_mut()[0]
        .comments_mut()
        .push("second".into());
    received.set_from_tcp_info(Some(" \u{e9}\r\n\t(comment)\\\0 "));
    assert_eq!(received.clauses()[0].comments(), [" \u{e9} (comment)\\\0 "]);
    // This should remove the FromTcpInfo, including all additional comments.
    received.set_from_tcp_info(None);
    // This should remove the ByTcpInfo.
    received.set_by_tcp_info(None);
    assert_eq!(received.from_tcp_info(), None);
    assert_eq!(received.by_tcp_info(), None);
    assert_eq!(received.clauses().len(), 2);
    received.set_from_tcp_info(Some(""));
    assert_eq!(received.from_tcp_info(), Some(""));

    let mut by_only = Received::new();
    by_only.set_by_tcp_info(Some("by comment"));
    assert_eq!(by_only.by(), Some(""));
    assert_eq!(by_only.by_tcp_info(), Some("by comment"));
    by_only.set_by_tcp_info(None);
    assert_eq!(by_only.by(), Some(""));
    assert_eq!(by_only.by_tcp_info(), None);

    let mut clause = ReceivedClause::with_comment("from", "host", "raw\r\n\tcomment").unwrap();
    assert_eq!(clause.comments(), ["raw\r\n\tcomment"]);
    clause
        .comments_mut()
        .push("comment\r\nBcc: evil@example.com".into());
    assert_eq!(clause.comments()[1], "comment\r\nBcc: evil@example.com");
}

#[test]
fn original_spelling_and_compatibility_checks() {
    let mut received = Received::new();
    for id in [
        "<id@example.com>",
        "id@example.com",
        "ABC123",
        "<id@example.com> (comment)",
    ] {
        received.set_id(Some(id)).unwrap();
        assert_eq!(received.id(), Some(id));
    }
    for recipient in [
        "User <user@example.com>",
        "<user@example.com>",
        "user@example.com (comment)",
        "local-only",
        "\"quoted local\"@example.com",
    ] {
        received.set_for_recipient(Some(recipient)).unwrap();
        assert_eq!(received.for_recipient(), Some(recipient));
    }
    for host in [
        "SMTP.Example.COM.",
        "local-host",
        "under_score.example",
        "b\u{fc}cher.example",
        "[192.0.2.1]",
        "[2001:db8::1]",
    ] {
        received.set_from(Some(host)).unwrap();
        assert_eq!(received.from(), Some(host));
    }
    for host in [
        ".host",
        "-host",
        "a..",
        "a.-b",
        "abc]def",
        "[127.1]",
        "[0177.0.0.1]",
        "[0x7f000001]",
        "[fe80::1%3]",
        "[IPv6:::1]",
    ] {
        assert!(received.set_from(Some(host)).is_err(), "{host}");
    }
    received.set_via(Some("Frontend Transport")).unwrap();
    received.set_with(Some("Microsoft SMTP Server")).unwrap();
    assert_eq!(received.via(), Some("Frontend Transport"));
    assert_eq!(received.with(), Some("Microsoft SMTP Server"));
    let clause = ReceivedClause::new("x-custom", "  caf\u{e9}  ").unwrap();
    assert_eq!(clause.value(), "  caf\u{e9}  ");
}

#[test]
fn control_rejection_and_error_sources() {
    for (id, setter, valid) in SETTERS {
        let mut received = Received::new();
        setter(&mut received, Some(valid)).unwrap();
        for control in (0u8..=31).chain([127]) {
            let value = format!("{valid}{}", char::from(control));
            let before = received.clone();
            let error = setter(&mut received, Some(&value)).unwrap_err();
            assert_eq!(
                error,
                ReceivedEditError::InvalidValue {
                    clause: id,
                    offset: valid.len()
                }
            );
            assert_eq!(received, before);
            assert!(!error.to_string().is_empty());
        }
    }
    let error = ReceivedClause::new("with", "caf\u{e9};bad").unwrap_err();
    assert_eq!(
        error,
        ReceivedEditError::InvalidValue {
            clause: Id::With,
            offset: 5
        }
    );
    assert!(error.source().is_none());
    let error = ReceivedClause::new("id", "<>").unwrap_err();
    assert!(matches!(error, ReceivedEditError::MessageId(_)));
    assert!(error.source().is_some());
    let error = ReceivedClause::new("for", "this is invalid...").unwrap_err();
    assert!(matches!(error, ReceivedEditError::Mailbox(_)));
    assert!(error.source().is_some());
    for value in [
        "user@example.com\r\n (comment)",
        "user@example.com (bad\0comment)",
        "user@example.com\r\nBcc: evil@example.com",
    ] {
        assert!(ReceivedClause::new("for", value).is_err());
    }
}

#[test]
fn insertion_sorts_but_updates_and_removals_do_not() {
    let mut received = Received::parse(b"(lead) BY target from source").unwrap();
    received
        .clauses_mut()
        .push(ReceivedClause::new("x-last", "one").unwrap());
    received
        .clauses_mut()
        .push(ReceivedClause::new("x-first", "two").unwrap());
    received.set_by(Some("new-target")).unwrap();
    assert_eq!(received.clauses()[1].keyword(), "BY");
    received.set_by_tcp_info(Some("new comment"));
    assert_eq!(received.clauses()[1].id(), Id::By);
    received.set_via(Some("TCP")).unwrap();
    assert_eq!(
        received
            .clauses()
            .iter()
            .map(ReceivedClause::keyword)
            .collect::<Vec<_>>(),
        ["", "from", "BY", "via", "x-last", "x-first"]
    );

    received.clauses_mut().reverse();
    received.set_via(None).unwrap();
    assert_eq!(
        received
            .clauses()
            .iter()
            .map(ReceivedClause::keyword)
            .collect::<Vec<_>>(),
        ["x-first", "x-last", "BY", "from", ""]
    );
    received.set_via(Some("TCP")).unwrap();
    assert_eq!(received.clauses()[0].id(), Id::None);
    received
        .clauses_mut()
        .insert(0, ReceivedClause::new("FROM", "first").unwrap());
    received.set_from(Some("updated")).unwrap();
    received.set_from_tcp_info(Some("first comment"));
    assert_eq!(received.clauses()[0].keyword(), "FROM");
    assert_eq!(received.clauses()[0].value(), "updated");
    assert_eq!(received.clauses()[2].value(), "source");
    assert!(received.clauses()[2].comments().is_empty());
    received.set_from(None).unwrap();
    assert_eq!(received.from(), Some("source"));
}

#[test]
fn date_cache_and_parse_diagnostics_survive_unrelated_edits() {
    let mut received =
        Received::parse(b"from source (caf\xe9) by target; Tue, 9 Jun 92 03:45:24 JST (original)")
            .unwrap();
    let text = received.date_time_text().unwrap().to_owned();
    let date = received.date_time().unwrap();
    assert!(received.had_charset_fallback());
    received.set_from(Some("changed")).unwrap();
    received.set_from_tcp_info(None);
    received.set_with(Some("ESMTPS")).unwrap();
    assert_eq!(received.date_time_text(), Some(text.as_str()));
    assert!(received.had_charset_fallback());
    received.set_date_time(Some(date));
    assert_eq!(received.date_time(), Some(date));
    assert_eq!(received.date_time_text(), None);
    received.set_date_time(None);
    assert_eq!(received.date_time(), None);
    assert_eq!(received.date_time_text(), None);
    received.clauses_mut().clear();
    assert!(received.had_charset_fallback());
}

#[test]
fn parsed_values_still_bypass_assignment_checks() {
    let mut received = Received::parse(b"from [not-an-ip] id <> with by host").unwrap();
    assert_eq!(received.from(), Some("[not-an-ip]"));
    assert_eq!(received.id(), Some("<>"));
    assert_eq!(received.with(), Some(""));
    let before = received.clone();
    assert!(received.set_from(Some("[not-an-ip]")).is_err());
    assert!(received.set_id(Some("<>")).is_err());
    assert!(received.set_with(Some("")).is_err());
    assert_eq!(received, before);
    received.set_from(Some("valid.example")).unwrap();
    assert_eq!(received.id(), Some("<>"));
    assert_eq!(received.with(), Some(""));
}

// Adapted from MimeKit's UnitTests/DomainListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{DomainList, DomainListParseErrorKind as Kind, MessageId};

#[test]
fn every_single_byte_atom_and_literal_classification() {
    for byte in 0..=u8::MAX {
        let atom = DomainList::parse(&[b'@', byte]);
        let is_atom = byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&byte);
        assert_eq!(atom.is_ok(), is_atom, "atom byte {byte}");
        if let Ok(route) = atom {
            assert_eq!(route.as_slice(), &[char::from(byte).to_string()]);
        }

        let literal = DomainList::parse(&[b'@', b'[', byte, b']']);
        let is_space = matches!(byte, b' ' | b'\r' | b'\n' | b'\t');
        let is_literal = (33..=126).contains(&byte) && !b"[\\]".contains(&byte);
        assert_eq!(
            literal.is_ok(),
            is_space || is_literal,
            "literal byte {byte}"
        );
        if let Ok(route) = literal {
            let expected = if is_space {
                "[]".into()
            } else {
                format!("[{}]", char::from(byte))
            };
            assert_eq!(route.as_slice(), &[expected]);
        }
    }
}

#[test]
fn large_routes_and_deep_comments_use_iterative_parsing() {
    let nesting = 4096;
    let input = format!(
        "{}note{}@a{}",
        "(".repeat(nesting),
        ")".repeat(nesting),
        ",,@a".repeat(8191)
    );
    let route: DomainList = input.parse().unwrap();
    assert_eq!(route.len(), 8192);
    assert!(route.as_slice().iter().all(|domain| domain == "a"));
    assert_eq!(route.to_string().parse::<DomainList>().unwrap(), route);
}

#[test]
fn canonical_empty_and_malformed_routes_fail() {
    for input in [
        "",
        " \t\r\n",
        " (this is an unterminated comment...",
        "@",
        "@ (this is an unterminated comment...",
        "@[invalid.domain",
        "@example.com (unterminated",
    ] {
        assert!(input.parse::<DomainList>().is_err(), "{input:?}");
    }
}

#[test]
fn canonical_null_and_unicode_whitespace_domains() {
    for input in [
        "@domain1,,@domain2",
        // Unicode whitespace is accepted as atom characters but is not retained.
        "@domain1,@\u{2008},@\u{3000}\u{2008}@domain2",
    ] {
        let route = input.parse::<DomainList>().unwrap();
        assert_eq!(route.as_slice(), &["domain1", "domain2"]);
        assert_eq!(route.to_string(), "@domain1,@domain2");
    }
    let route: DomainList = "@\u{2008},@\u{3000}".parse().unwrap();
    assert!(route.is_empty());
    assert!(route.to_string().is_empty());
}

#[test]
fn vector_edits_preserve_order_and_exact_strings() {
    let mut route = DomainList::new();
    assert!(route.is_empty());
    route.push("domain2").unwrap();
    route.insert(0, "domain0").unwrap();
    route.insert(1, "domain1").unwrap();
    assert_eq!(route.len(), 3);
    assert_eq!(route.as_slice(), &["domain0", "domain1", "domain2"]);
    assert!(route.as_slice().contains(&"domain1".into()));
    assert!(!route.as_slice().contains(&"DOMAIN1".into()));
    assert_eq!(route.remove(0), "domain0");
    route.set(0, "DOMAIN1").unwrap();
    route.push("DOMAIN1").unwrap();
    assert_eq!(route.to_string(), "@DOMAIN1,@domain2,@DOMAIN1");
    let copy = route.clone();
    route.clear();
    assert!(route.is_empty());
    assert_eq!(copy.into_vec(), ["DOMAIN1", "domain2", "DOMAIN1"]);
}

#[test]
fn construction_iteration_and_display_preserve_order_and_spelling() {
    let route = DomainList::try_from_iter(["route1", "route2"]).unwrap();
    assert_eq!(route.to_string(), "@route1,@route2");
    assert_eq!(route.len(), 2);
    assert_eq!(
        (&route).into_iter().map(String::as_str).collect::<Vec<_>>(),
        ["route1", "route2"]
    );
    assert_eq!(
        DomainList::try_from(route.clone().into_vec()).unwrap(),
        route
    );
    assert_eq!(DomainList::default(), DomainList::new());
}

#[test]
fn authored_domains_are_validated_without_dns_checks_or_normalization() {
    for domain in [
        "localhost",
        "Example.com",
        "b\u{fc}cher.example",
        "mail_host.example",
        "[127.0.0.1]",
        "[IPv6:2001:db8::1]",
        "[opaque-literal]",
        "[]",
        "\u{1f600}.example",
        "[\u{e9}]",
    ] {
        let mut list = DomainList::try_from_iter([domain]).unwrap();
        list.push(domain).unwrap();
        list.insert(0, domain).unwrap();
        list.set(1, "other.example").unwrap();
        list.set(1, domain).unwrap();
        assert_eq!(list.as_slice(), [domain, domain, domain]);
    }
    for domain in [
        "",
        " ",
        " a",
        "a ",
        "a..b",
        ".a",
        "a.",
        "a\r\nb",
        "a\0b",
        "bad domain",
        "a@b",
        "a,b",
        "a:b",
        "[unclosed",
        "[a]junk",
        "[a\\b]",
        "[a[b]",
        "[a b]",
        "exam\u{2008}ple.com",
        "a\u{85}b",
        "[\u{a0}]",
    ] {
        let mut list = DomainList::try_from_iter(["good.example"]).unwrap();
        let original = list.clone();
        let error = list.push(domain).unwrap_err();
        assert_eq!(error.domain_index(), 1, "{domain:?}");
        assert!(error.to_string().contains("index 1"));
        assert!(list.insert(0, domain).is_err());
        assert!(list.set(0, domain).is_err());
        assert_eq!(list, original);
        assert_eq!(
            DomainList::try_from_iter(["good", domain])
                .unwrap_err()
                .domain_index(),
            1
        );
        assert!(DomainList::try_from(vec![domain.to_owned()]).is_err());
    }
}

#[test]
fn recovered_domains_can_be_parsed_and_cloned_but_not_authored() {
    let route = DomainList::parse("@exam\u{2008}ple.com".as_bytes()).unwrap();
    assert_eq!(route.as_slice(), ["exam\u{2008}ple.com"]);
    assert!(DomainList::try_from_iter(&route).is_err());
    let mailbox =
        mimecraft::MailboxAddress::parse("<@exam\u{2008}ple.com:user@example.com>".as_bytes())
            .unwrap();
    let mut clone = mailbox.clone();
    assert_eq!(clone.route(), &route);
    clone.route_mut().set(0, "other.example").unwrap();
    assert_eq!(mailbox.route(), &route);
}

#[test]
fn comments_folding_and_domain_whitespace_are_normalized() {
    let route = DomainList::parse(
        b" (leading (nested)) @Relay(comment) \r\n . (label) Example \
          (escaped \\) comment), (between) ,\t@Gateway.Example (end)\r\n",
    )
    .unwrap();
    assert_eq!(route.as_slice(), &["Relay.Example", "Gateway.Example"]);
    assert_eq!(route.to_string(), "@Relay.Example,@Gateway.Example");
}

#[test]
fn domain_must_immediately_follow_at() {
    for input in [
        "@ example",
        "@(comment)example",
        "@\r\n\texample",
        "@@example",
    ] {
        let error = input.parse::<DomainList>().unwrap_err();
        assert_eq!(error.kind(), Kind::InvalidDomain, "{input:?}");
        assert_eq!(error.offset(), 1);
    }
}

#[test]
fn obsolete_missing_separators_and_trailing_commas_are_tolerated() {
    for input in [
        "@a@b",
        "@a (comment) @b",
        "@a,,(comment),@b,,",
        "@a,@b, (trailing)",
    ] {
        let route = input.parse::<DomainList>().unwrap();
        assert_eq!(route.as_slice(), &["a", "b"], "{input:?}");
    }
}

#[test]
fn utf8_atoms_preserve_case_duplicates_and_non_dns_characters() {
    let route: DomainList =
        "@b\u{fc}cher.example,@B\u{dc}CHER.example,@b\u{fc}cher.example,@a!b_/?=c"
            .parse()
            .unwrap();
    assert_eq!(
        route.as_slice(),
        &[
            "b\u{fc}cher.example",
            "B\u{dc}CHER.example",
            "b\u{fc}cher.example",
            "a!b_/?=c"
        ]
    );
    assert_eq!(route.to_string().parse::<DomainList>().unwrap(), route);
}

#[test]
fn literals_are_tokens_not_ip_address_validation() {
    let route: DomainList = "@[ 127.0.0.1 ],@[IPv6:\r\n ::1],@[not (an ip) ],@[]"
        .parse()
        .unwrap();
    assert_eq!(
        route.as_slice(),
        &["[127.0.0.1]", "[IPv6:::1]", "[not(anip)]", "[]"]
    );
    for input in [
        b"@[a\\b]".as_slice(),
        b"@[a[b]",
        b"@[\x00]",
        b"@[\x7f]",
        b"@[\xff]",
    ] {
        assert_eq!(
            DomainList::parse(input).unwrap_err().kind(),
            Kind::InvalidDomain
        );
    }
}

#[test]
fn trailing_dots_use_the_routes_own_sentinels() {
    assert_eq!(
        "@a. ,@b.(end)".parse::<DomainList>().unwrap().as_slice(),
        &["a", "b"]
    );
    let (route, consumed) = DomainList::parse_prefix(b"@a. (comment):local@domain").unwrap();
    assert_eq!(route.as_slice(), &["a"]);
    assert_eq!(&b"@a. (comment):local@domain"[consumed..], b":local@domain");
    for input in ["@a..b", "@a.@b", "@a.>"] {
        assert_eq!(
            input.parse::<DomainList>().unwrap_err().kind(),
            Kind::InvalidDomain
        );
    }
    // Message IDs retain their own '>' and '@' trailing-dot sentinels.
    assert_eq!(MessageId::parse(b"<local@a.>").unwrap().as_str(), "local@a");
    assert_eq!(
        MessageId::parse(b"<local@a.@b>").unwrap().as_str(),
        "local@a@b"
    );
}

#[test]
fn prefix_consumption_and_complete_parse_are_distinct() {
    for suffix in [":local@example.com", ">tail", "unparsed", ";"] {
        let prefix = " (lead) @a,@[::1], (tail) ";
        let input = format!("{prefix}{suffix}");
        let (route, consumed) = DomainList::parse_prefix(input.as_bytes()).unwrap();
        assert_eq!(route.as_slice(), &["a", "[::1]"]);
        assert_eq!(consumed, prefix.len());
        assert_eq!(&input[consumed..], suffix);
        let error = input.parse::<DomainList>().unwrap_err();
        assert_eq!(error.kind(), Kind::TrailingInput);
        assert_eq!(error.offset(), consumed);
    }
}

#[test]
fn explicit_leading_at_is_required() {
    for input in [
        "example.com",
        "xexample.com",
        ",@example.com",
        "(comment) example",
    ] {
        assert_eq!(
            input.parse::<DomainList>().unwrap_err().kind(),
            Kind::MissingAt
        );
    }
    assert_eq!(
        " (comment) @example.com"
            .parse::<DomainList>()
            .unwrap()
            .as_slice(),
        &["example.com"]
    );
}

#[test]
fn failures_report_original_byte_offsets() {
    let cases: &[(&[u8], Kind, usize)] = &[
        (b"", Kind::MissingAt, 0),
        (b" \t", Kind::MissingAt, 2),
        (b"@", Kind::InvalidDomain, 1),
        (b"@a,@", Kind::InvalidDomain, 4),
        (b"@[unfinished", Kind::IncompleteDomainLiteral, 1),
        (b"@a,@[unfinished", Kind::IncompleteDomainLiteral, 4),
        (b"(unfinished", Kind::IncompleteComment, 0),
        (b"@a (unfinished", Kind::IncompleteComment, 3),
        (b"@a. (unfinished", Kind::IncompleteComment, 4),
        (b"@a,,(unfinished", Kind::IncompleteComment, 4),
        (b"@a,\xff", Kind::TrailingInput, 3),
        (b"@a\xff", Kind::InvalidUtf8, 2),
        (b"@a.@\xff", Kind::InvalidDomain, 3),
        (b"@a.\xff", Kind::InvalidUtf8, 3),
        (b"@a\xc3", Kind::InvalidUtf8, 2),
        (b"@a\xc0\x80", Kind::InvalidUtf8, 2),
        (b"@[a\\]", Kind::InvalidDomain, 3),
    ];
    for &(input, kind, offset) in cases {
        let error = DomainList::parse(input).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        assert!(error.to_string().ends_with(&format!("at byte {offset}")));
        let source: &dyn std::error::Error = &error;
        assert!(source.source().is_none());
    }
    let error = "@\u{fc}.x\x01".parse::<DomainList>().unwrap_err();
    assert_eq!(error.kind(), Kind::TrailingInput);
    assert_eq!(error.offset(), 5);
}

#[test]
fn shared_domain_normalization_matches_message_ids() {
    for domain in [
        "Example.COM",
        "a(comment).b",
        "[ 127.0.0.1 ]",
        "[IPv6: ::1]",
        "a!b_/?=c",
        "b\u{fc}cher.example",
        "trailing.",
    ] {
        let route: DomainList = format!("@{domain}").parse().unwrap();
        let id = MessageId::parse(format!("<local@{domain}>").as_bytes()).unwrap();
        assert_eq!(id.as_str(), format!("local@{}", route.as_slice()[0]));
    }
}

#[test]
fn arbitrary_bytes_and_prefix_lengths_remain_bounded() {
    let mut state = 0x19ab_705cu32;
    for length in 0..96 {
        for _ in 0..32 {
            let mut input = Vec::with_capacity(length + 1);
            input.push(b'@');
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                input.push(state as u8);
            }

            match DomainList::parse_prefix(&input) {
                Ok((route, consumed)) => {
                    assert!((2..=input.len()).contains(&consumed));
                    let complete = DomainList::parse(&input);
                    if consumed == input.len() {
                        assert_eq!(complete.unwrap(), route);
                    } else {
                        let error = complete.unwrap_err();
                        assert_eq!(error.kind(), Kind::TrailingInput);
                        assert_eq!(error.offset(), consumed);
                    }
                }
                Err(error) => assert!(error.offset() <= input.len()),
            }
        }
    }
}

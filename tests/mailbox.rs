// Adapted from MimeKit's MailboxAddressTests.cs and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    AddrSpecParseErrorKind as AddrKind, AddressComplianceMode as Mode, MailboxAddress,
    MailboxParseErrorKind as Kind, MailboxParseOptions,
    charsets::Charset,
    rfc2047::{ComplianceMode, DecodeIssue},
};

fn options(mode: Mode) -> MailboxParseOptions {
    let mut options = MailboxParseOptions::default();
    options.address.compliance_mode = mode;
    options
}

#[test]
fn defaults_match_the_source_parser() {
    let options = MailboxParseOptions::default();
    assert_eq!(options.address.compliance_mode, Mode::Loose);
    assert!(options.address.allow_local_only);
    assert!(options.allow_unquoted_commas);
    assert_eq!(options.name_decoding.compliance_mode, ComplianceMode::Loose);
}

#[test]
fn canonical_bare_and_angle_mailboxes() {
    for (input, name, address) in [
        ("jeff@xamarin.com", "", "jeff@xamarin.com"),
        (
            "Jeffrey Stedfast <jestedfa@microsoft.com>",
            "Jeffrey Stedfast",
            "jestedfa@microsoft.com",
        ),
        ("< jeff@xamarin.com >", "", "jeff@xamarin.com"),
        ("jeff", "", "jeff"),
        ("jeff (Jeffrey Stedfast)", "Jeffrey Stedfast", "jeff"),
        (
            "jeff@xamarin.com (Jeffrey\tStedfast)",
            "Jeffrey Stedfast",
            "jeff@xamarin.com",
        ),
        (
            "jeff@[127.0.0.1] (Jeffrey Stedfast)",
            "Jeffrey Stedfast",
            "jeff@[127.0.0.1]",
        ),
        (
            "Warren Worthington, Jr. <warren@worthington.com>",
            "Warren Worthington, Jr.",
            "warren@worthington.com",
        ),
        (
            "Worthington, Warren <warren@worthington.com>",
            "Worthington, Warren",
            "warren@worthington.com",
        ),
        (
            " (leading) John (inside) Doe (outside) <john@EXAMPLE.com> (trailing) ",
            "John (inside) Doe",
            "john@EXAMPLE.com",
        ),
    ] {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            let mailbox =
                MailboxAddress::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
            assert_eq!(mailbox.display_name(), name, "{input:?}");
            assert_eq!(mailbox.address().as_str(), address, "{input:?}");
            assert!(mailbox.route().is_empty());
            assert!(mailbox.display_name_issues().is_empty());
        }
    }
}

#[test]
fn trim_and_unquote_names_before_decoding() {
    for (input, expected) in [
        ("\"  John Smith  \" <j.smith@example.com>", "John Smith"),
        (
            "\" =?utf-8?q?John_Smith?= \" <j.smith@example.com>",
            "John Smith",
        ),
        ("j.smith@example.com (  John Smith  )", "John Smith"),
        (
            "\"John\\\"\\\\\r\n\tSmith\" <j.smith@example.com>",
            "John\"\\ Smith",
        ),
        (
            "=?utf-8?q?=22Joe=22_=5C_Name?= <joe@example.com>",
            "\"Joe\" \\ Name",
        ),
        (
            "\"=?utf-8?q?=22Joe=22_=5C_Name?=\" <joe@example.com>",
            "\"Joe\" \\ Name",
        ),
        ("=?utf-8?q?Ren=C3=A9e?= <rene@example.com>", "Ren\u{e9}e"),
        (
            "=?utf-8?q?Ren?= \r\n\t=?utf-8?q?=C3=A9e?= <rene@example.com>",
            "Ren\u{e9}e",
        ),
    ] {
        let mailbox: MailboxAddress = input.parse().unwrap();
        assert_eq!(mailbox.display_name(), expected, "{input:?}");
    }
}

#[test]
fn trailing_comment_names_are_not_phrase_unquoted() {
    for local in ["user", "user@example.com", "user@[IPv6: ::1]"] {
        let input = format!("{local} ( \"a\\(b\\)\"\t(nested) ) (ignored)");
        let mailbox: MailboxAddress = input.parse().unwrap();
        assert_eq!(mailbox.display_name(), "\"a\\(b\\)\" (nested)");
    }
    let mailbox = MailboxAddress::parse(b"user@example.com (=?utf-8?q?Ren=C3=A9e?=)").unwrap();
    assert_eq!(mailbox.display_name(), "Ren\u{e9}e");
    let mailbox = MailboxAddress::parse(b"<user@example.com> (not a name)").unwrap();
    assert_eq!(mailbox.display_name(), "");
}

#[test]
fn quoted_local_parts_keep_quotes_and_actual_at_position() {
    for input in [
        "\"a@b\"@example.com",
        "Name <\"a@b\"@example.com>",
        "\"a@b\"@example.com (Name)",
    ] {
        let mailbox: MailboxAddress = input.parse().unwrap();
        assert_eq!(mailbox.address().local_part(), "\"a@b\"");
        assert_eq!(mailbox.address().domain(), Some("example.com"));
    }
    for input in [
        b"\"bad\r\nlocal\"@example.com".as_slice(),
        b"Name <\"bad\\\rlocal\"@example.com>",
    ] {
        assert_eq!(
            MailboxAddress::parse(input).unwrap_err().kind(),
            Kind::AddrSpec(AddrKind::InvalidQuotedCharacter)
        );
    }
}

#[test]
fn routes_reuse_domain_parsing_and_explicit_idna() {
    let mailbox: MailboxAddress =
        "User <@Relay (c) . Example,,@b\u{fc}cher.example,@[IPv6: ::1]:user@XN--BCHER-KVA.EXAMPLE>"
            .parse()
            .unwrap();
    assert_eq!(
        mailbox.route().as_slice(),
        &["Relay.Example", "b\u{fc}cher.example", "[IPv6:::1]"]
    );
    assert_eq!(
        mailbox.route().to_ascii().unwrap(),
        "@relay.example,@xn--bcher-kva.example,@[IPv6:::1]"
    );
    assert_eq!(mailbox.address().domain(), Some("XN--BCHER-KVA.EXAMPLE"));
    assert_eq!(
        mailbox.address().with_unicode_domain().unwrap(),
        "user@b\u{fc}cher.example"
    );
    assert_eq!(mailbox.display_name(), "User");
}

#[test]
fn literal_domains_and_international_local_parts_stay_unchanged() {
    for domain in ["[xn--e1afmk1afmk0c]", "[IPv6:::1]", "[Tag:xn--Abc.X--abc]"] {
        let input = format!("Name <\u{e9}@{domain}>");
        let mailbox: MailboxAddress = input.parse().unwrap();
        assert_eq!(mailbox.address().domain(), Some(domain));
        assert_eq!(
            mailbox.address().with_ascii_domain().unwrap(),
            format!("\u{e9}@{domain}")
        );
        assert!(mailbox.address().is_international());
    }
}

#[test]
fn canonical_rfc7103_repairs_are_mode_dependent() {
    for (input, name, address) in [
        ("<<<user2@example.org>>>", "", "user2@example.org"),
        (
            "User 2 <<<user2@example.org>",
            "User 2",
            "user2@example.org",
        ),
        (
            "User 2 <user2@example.org>>>",
            "User 2",
            "user2@example.org",
        ),
        ("<another@example.net", "", "another@example.net"),
        ("second@example.org>", "", "second@example.org"),
        ("jeff>", "", "jeff"),
        ("\"Joe <joe@example.com>", "Joe", "joe@example.com"),
        (
            "user@example.com <other@example.com>",
            "user@example.com",
            "other@example.com",
        ),
        (
            "[Invalid Sender] <sender@example.com>",
            "[Invalid Sender]",
            "sender@example.com",
        ),
        (
            "Tom Doe [C\u{f6}rp \u{d6}ne] <tom@example.com>",
            "Tom Doe [C\u{f6}rp \u{d6}ne]",
            "tom@example.com",
        ),
        (
            "Odd) Name <odd@example.com>",
            "Odd) Name",
            "odd@example.com",
        ),
    ] {
        for mode in [Mode::Loose, Mode::Looser] {
            let mailbox =
                MailboxAddress::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
            assert_eq!(mailbox.display_name(), name, "{input:?}");
            assert_eq!(mailbox.address().as_str(), address);
        }
        assert!(
            MailboxAddress::parse_with_options(input.as_bytes(), &options(Mode::Strict)).is_err(),
            "{input:?}"
        );
    }
}

#[test]
fn looser_local_part_repairs_are_shared() {
    for (input, expected) in [
        (
            "First Last <webmaster\\@custom-domain.com@mail-host.com>",
            "webmaster%40custom-domain.com@mail-host.com",
        ),
        (
            "Name <first..last.@example.com>",
            "first..last.@example.com",
        ),
    ] {
        let mailbox =
            MailboxAddress::parse_with_options(input.as_bytes(), &options(Mode::Looser)).unwrap();
        assert_eq!(mailbox.address().as_str(), expected);
        for mode in [Mode::Strict, Mode::Loose] {
            assert!(MailboxAddress::parse_with_options(input.as_bytes(), &options(mode)).is_err());
        }
    }
}

#[test]
fn domain_requirement_distinguishes_bare_from_angle_local_addresses() {
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let mut options = options(mode);
        options.address.allow_local_only = false;
        for input in ["local", "local (Name)"] {
            assert_eq!(
                MailboxAddress::parse_with_options(input.as_bytes(), &options)
                    .unwrap_err()
                    .kind(),
                Kind::AddrSpec(AddrKind::MissingDomain)
            );
        }
        for input in ["<local>", "Name <local>", "<@route:local>"] {
            let mailbox = MailboxAddress::parse_with_options(input.as_bytes(), &options).unwrap();
            assert_eq!(mailbox.address().as_str(), "local");
        }
    }
    assert!(MailboxAddress::parse(b"<>").is_err());
}

#[test]
fn unquoted_commas_are_configurable_even_in_strict_mode() {
    for mode in [Mode::Strict, Mode::Loose] {
        let mut options = options(mode);
        options.allow_unquoted_commas = false;
        let input = b"Worthington, Warren <warren@worthington.com>";
        let (mailbox, consumed) =
            MailboxAddress::parse_prefix_with_options(input, &options).unwrap();
        assert_eq!(mailbox.address().as_str(), "Worthington");
        assert_eq!(&input[consumed..], b", Warren <warren@worthington.com>");
        assert!(MailboxAddress::parse_with_options(input, &options).is_err());
        assert!(
            MailboxAddress::parse_with_options(
                b"\"Worthington, Warren\" <warren@worthington.com>",
                &options
            )
            .is_ok()
        );
    }
}

#[test]
fn prefix_parsing_leaves_enclosing_delimiters() {
    for input in [
        b"Name <a@b> (c),next@x".as_slice(),
        b"a@b (Name);tail",
        b"local (Name),other",
        b"<a@b, other@x>",
    ] {
        let (mailbox, consumed) = MailboxAddress::parse_prefix(input).unwrap();
        assert!(!mailbox.address().as_str().is_empty());
        assert!(matches!(input[consumed], b',' | b';'));
        let error = MailboxAddress::parse(input).unwrap_err();
        assert_eq!(error.kind(), Kind::TrailingInput);
        assert_eq!(error.offset(), consumed);
    }
    let (mailbox, consumed) = MailboxAddress::parse_prefix(b"a@b <other@x>,tail").unwrap();
    assert_eq!(mailbox.display_name(), "a@b");
    assert_eq!(mailbox.address().as_str(), "other@x");
    assert_eq!(consumed, 13);
}

#[test]
fn groups_lists_and_invalid_syntax_are_not_silently_accepted() {
    for input in [
        "",
        " \r\n",
        "(comment)",
        "<>",
        "Name <",
        "a@",
        "a@ (c)",
        "Name <@route>",
        "Name <@route,",
        "Name <@:a@b>",
        "Name <@route:>",
        "Name <a@b> garbage",
        "a@b,c@d",
        "Name <a@b>, Other <c@d>",
        "two localwords",
        "local>garbage",
        "local>>",
        " \"",
        "\"unterminated phrase with no mailbox",
        "a@b (unterminated",
    ] {
        assert!(
            MailboxAddress::parse(input.as_bytes()).is_err(),
            "{input:?}"
        );
    }
    for input in ["Group:", "Group:a@b;", "Group:Name <a@b>, Other <c@d>;"] {
        assert_eq!(
            MailboxAddress::parse(input.as_bytes()).unwrap_err().kind(),
            Kind::GroupNotAllowed
        );
    }
}

#[test]
fn errors_refer_to_original_mailbox_byte_offsets() {
    for (input, kind, offset) in [
        (
            b" Name <a@>".as_slice(),
            Kind::AddrSpec(AddrKind::InvalidDomain),
            9,
        ),
        (
            b" Name <a@[x>",
            Kind::AddrSpec(AddrKind::IncompleteDomainLiteral),
            9,
        ),
        (b" Name <@route a@b>", Kind::MissingRouteColon, 14),
        (
            b" Name <@route,(x",
            Kind::Route(mimecraft::DomainListParseErrorKind::IncompleteComment),
            14,
        ),
        (b" Name <a@b> (x", Kind::IncompleteComment, 12),
        (b" a@b (x", Kind::AddrSpec(AddrKind::IncompleteComment), 5),
    ] {
        let error = MailboxAddress::parse(input).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
    }
    let strict = options(Mode::Strict);
    for (input, kind, offset) in [
        (b" Name <<a@b>".as_slice(), Kind::ExcessiveAngleBracket, 7),
        (b" Name <a@b>>", Kind::ExcessiveAngleBracket, 11),
        (b" Name <a@b", Kind::MissingClosingAngleBracket, 10),
        (b" a@b>", Kind::UnexpectedClosingAngleBracket, 4),
        (b" \"Name <a@b>", Kind::IncompleteQuotedString, 1),
    ] {
        let error = MailboxAddress::parse_with_options(input, &strict).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.offset(), offset);
    }
    let input = "Ren\u{e9}e <a@\u{fc} (x";
    assert_eq!(
        MailboxAddress::parse(input.as_bytes())
            .unwrap_err()
            .offset(),
        input.find('(').unwrap()
    );
}

#[test]
fn local_word_latin1_recovery_and_strict_offsets_survive_wrapping() {
    let input = b"Name <caf\xc3\xa9\xff@example.com>";
    let mailbox = MailboxAddress::parse(input).unwrap();
    assert_eq!(mailbox.address().local_part(), "caf\u{c3}\u{a9}\u{ff}");
    assert!(mailbox.address().had_latin1_fallback());
    assert!(mailbox.display_name_issues().is_empty());
    let error = MailboxAddress::parse_with_options(input, &options(Mode::Strict)).unwrap_err();
    assert_eq!(error.kind(), Kind::AddrSpec(AddrKind::InvalidUtf8));
    assert_eq!(error.offset(), 11);
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        assert_eq!(
            MailboxAddress::parse_with_options(b"Name <a@\xff>", &options(mode))
                .unwrap_err()
                .kind(),
            Kind::AddrSpec(AddrKind::InvalidUtf8)
        );
    }
}

#[test]
fn name_recovery_is_reported_independently_of_address_mode() {
    let mailbox =
        MailboxAddress::parse_with_options(b"Andr\xe9 <a@b>", &options(Mode::Strict)).unwrap();
    assert_eq!(mailbox.display_name(), "Andr\u{e9}");
    assert!(matches!(
        mailbox.display_name_issues(),
        [DecodeIssue::UnlabelledText {
            selected: Charset::LATIN1,
            ..
        }]
    ));
    assert!(!mailbox.address().had_latin1_fallback());
    let mailbox = MailboxAddress::parse(b"\"=?unknown?q?Name?=\" <a@b>").unwrap();
    assert_eq!(mailbox.display_name(), "Name");
    match &mailbox.display_name_issues()[0] {
        DecodeIssue::UnsupportedCharset { range, .. } => assert_eq!(range.clone(), 0..18),
        issue => panic!("{issue:?}"),
    }
    let mailbox = MailboxAddress::parse(b"=?utf-8?B?/w==?= <a@b>").unwrap();
    assert_eq!(mailbox.display_name(), "\u{fffd}");
    assert!(matches!(
        mailbox.display_name_issues(),
        [DecodeIssue::InvalidBytes { .. }]
    ));
}

#[test]
fn preferred_name_charset_and_encoded_word_mode_are_forwarded() {
    let mut options = MailboxParseOptions::default();
    options.name_decoding.fallback_charset = Some(Charset::from_label("windows-1252").unwrap());
    let mailbox = MailboxAddress::parse_with_options(b"Price\x80 <a@b>", &options).unwrap();
    assert_eq!(mailbox.display_name(), "Price\u{20ac}");
    let input = b"prefix=?utf-8?q?Ren=C3=A9e?= <a@b>";
    let loose = MailboxAddress::parse_with_options(input, &options).unwrap();
    options.name_decoding.compliance_mode = ComplianceMode::Strict;
    let strict = MailboxAddress::parse_with_options(input, &options).unwrap();
    assert_eq!(loose.display_name(), "prefixRen\u{e9}e");
    assert_eq!(strict.display_name(), "prefix=?utf-8?q?Ren=C3=A9e?=");
}

#[test]
fn decoded_names_are_not_authoring_sanitized() {
    let mailbox = MailboxAddress::parse(b"=?utf-8?q?a=0D=0Ab=00?= <a@b>").unwrap();
    assert_eq!(mailbox.display_name(), "a\r\nb\0");
}

#[test]
fn shared_comment_scanning_handles_escape_parity_and_deep_nesting() {
    let input = format!("a@b ({}name{})", "(".repeat(1000), ")".repeat(1000));
    let mailbox: MailboxAddress = input.parse().unwrap();
    assert_eq!(mailbox.display_name().len(), 2004);
    for count in 0..12 {
        let input = format!("a@b (name{})", "\\".repeat(count));
        assert_eq!(
            MailboxAddress::parse(input.as_bytes()).is_ok(),
            count % 2 == 0
        );
    }
}

#[test]
fn all_truncations_and_arbitrary_bytes_have_bounded_progress_and_errors() {
    let samples = [
        "Ren\u{e9}e <@route,@b\u{fc}cher.example:\"\u{e9}@x\"@example>",
        "\"a\\\"b\" <a@[IPv6:::1]> (nested (comment))",
        "\"missing <a@b",
        "=?utf-8?q?name?= <a@b>",
    ];
    let check = |input: &[u8]| {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            let options = options(mode);
            match MailboxAddress::parse_prefix_with_options(input, &options) {
                Ok((mailbox, consumed)) => {
                    assert!(consumed > 0 && consumed <= input.len());
                    assert!(!mailbox.address().as_str().is_empty());
                    assert!(
                        mailbox
                            .address()
                            .as_str()
                            .is_char_boundary(mailbox.address().local_part().len())
                    );
                    if consumed == input.len() {
                        assert!(MailboxAddress::parse_with_options(input, &options).is_ok());
                    } else {
                        assert_eq!(
                            MailboxAddress::parse_with_options(input, &options)
                                .unwrap_err()
                                .kind(),
                            Kind::TrailingInput
                        );
                    }
                }
                Err(error) => assert!(error.offset() <= input.len(), "{input:?}: {error}"),
            }
        }
    };
    for sample in samples {
        for end in 0..=sample.len() {
            check(&sample.as_bytes()[..end]);
        }
    }
    let alphabet = b"abc .@,:;<>()[ ]\\\"\t\r\n\0\x7f\xc3\xa9\xff";
    let mut seed = 7u64;
    for len in 0..100 {
        for _ in 0..40 {
            let input: Vec<_> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    alphabet[(seed >> 32) as usize % alphabet.len()]
                })
                .collect();
            check(&input);
        }
    }
}

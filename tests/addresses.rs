// Adapted from MimeKit's GroupAddressTests.cs and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    AddrSpecParseErrorKind, Address, AddressComplianceMode as Mode, AddressList,
    AddressParseErrorKind as Kind, AddressParseOptions, GroupAddress, MailboxAddress,
    MailboxParseErrorKind, rfc2047::DecodeIssue,
};

fn options(mode: Mode) -> AddressParseOptions {
    let mut options = AddressParseOptions::default();
    options.mailbox.address.compliance_mode = mode;
    options
}

fn nested_options(mode: Mode) -> AddressParseOptions {
    AddressParseOptions {
        max_group_depth: 3,
        ..options(mode)
    }
}

fn group(address: &Address) -> &GroupAddress {
    match address {
        Address::Group(group) => group,
        Address::Mailbox(_) => panic!("expected a group"),
    }
}

fn mailboxes(list: &AddressList) -> Vec<&str> {
    list.mailboxes()
        .map(|mailbox| mailbox.address().as_str())
        .collect()
}

#[test]
fn default_options_and_variant_dispatch() {
    let options = AddressParseOptions::default();
    assert_eq!(options.max_group_depth, 1);
    assert_eq!(options.mailbox.address.compliance_mode, Mode::Loose);
    let address: Address = "Name <user@example.com>".parse().unwrap();
    assert!(matches!(address, Address::Mailbox(_)));
    assert_eq!(address.display_name(), "Name");
    let address: Address = "Friends: user@example.com;".parse().unwrap();
    assert_eq!(address.display_name(), "Friends");
    assert_eq!(group(&address).members().len(), 1);
}

#[test]
fn canonical_group_and_following_mailbox() {
    let input = "Agents of Shield: Skye <skye@shield.gov>, Leo Fitz <fitz@shield.gov>, May <may@shield.gov>;";
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let parsed = GroupAddress::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
        assert_eq!(parsed.display_name(), "Agents of Shield");
        assert_eq!(
            mailboxes(parsed.members()),
            ["skye@shield.gov", "fitz@shield.gov", "may@shield.gov"]
        );
        let input = format!("{input}, Fury <fury@shield.gov>");
        let list = AddressList::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            group(&list.as_slice()[0]).display_name(),
            "Agents of Shield"
        );
        assert_eq!(
            mailboxes(&list),
            [
                "skye@shield.gov",
                "fitz@shield.gov",
                "may@shield.gov",
                "fury@shield.gov"
            ]
        );
        assert_eq!(
            GroupAddress::parse(input.as_bytes()).unwrap_err().kind(),
            Kind::TrailingInput
        );
    }
}

#[test]
fn empty_and_nested_empty_names() {
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        for input in [
            "Empty:;",
            "\"\":;",
            ":;",
            "Empty: (comment);",
            "Empty:: \t:;",
        ] {
            let parsed =
                GroupAddress::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
            assert!(parsed.members().is_empty(), "{input}");
            assert_eq!(parsed.members().mailboxes().count(), 0);
        }
        let parsed =
            GroupAddress::parse_with_options(b"Outer: \"\": a@b;;", &nested_options(mode)).unwrap();
        let inner = group(&parsed.members().as_slice()[0]);
        assert_eq!(inner.display_name(), "");
        assert_eq!(mailboxes(inner.members()), ["a@b"]);
    }
}

#[test]
fn name_decoding_and_recovery_are_shared_with_mailboxes() {
    let parsed: GroupAddress = "\" =?utf-8?q?=22Ren=C3=A9e=22_=5C_Name?= \":a@b;"
        .parse()
        .unwrap();
    assert_eq!(parsed.display_name(), "\"Ren\u{e9}e\" \\ Name");
    assert!(parsed.display_name_issues().is_empty());
    let parsed = GroupAddress::parse(b"\"=?unknown?q?Name?=\":a@b;").unwrap();
    assert_eq!(parsed.display_name(), "Name");
    assert!(matches!(
        parsed.display_name_issues(),
        [DecodeIssue::UnsupportedCharset { .. }]
    ));
    let parsed = GroupAddress::parse(b"Andr\xe9:a@b;").unwrap();
    assert_eq!(parsed.display_name(), "Andr\u{e9}");
    assert!(matches!(
        parsed.display_name_issues(),
        [DecodeIssue::UnlabelledText { .. }]
    ));
    let parsed = GroupAddress::parse(b"=?utf-8?q?a=0D=0Ab?=:a@b;").unwrap();
    assert_eq!(parsed.display_name(), "a\r\nb");
}

#[test]
fn group_name_options_are_forwarded() {
    let mut options = AddressParseOptions::default();
    options.mailbox.name_decoding.fallback_charset =
        Some(mimecraft::charsets::Charset::from_label("windows-1252").unwrap());
    let parsed = GroupAddress::parse_with_options(b"Price\x80:a@b;", &options).unwrap();
    assert_eq!(parsed.display_name(), "Price\u{20ac}");
    options.mailbox.name_decoding.compliance_mode = mimecraft::rfc2047::ComplianceMode::Strict;
    let parsed = GroupAddress::parse_with_options(b"x=?utf-8?q?Name?=:a@b;", &options).unwrap();
    assert_eq!(parsed.display_name(), "x=?utf-8?q?Name?=");
}

#[test]
fn prefix_consumption_is_explicit() {
    let options = nested_options(Mode::Loose);
    for input in [
        b"G:a@b; (tail),next@x".as_slice(),
        b"G:Inner:a@b;; (tail);outer",
        b"Name <a@b> (tail),next@x",
    ] {
        let (_, consumed) = Address::parse_prefix_with_options(input, &options).unwrap();
        assert!(matches!(input[consumed], b',' | b';'));
        let error = Address::parse_with_options(input, &options).unwrap_err();
        assert_eq!(error.kind(), Kind::TrailingInput);
        assert_eq!(error.offset(), consumed);
    }
    let input = b"G:a@b; (tail),next@x";
    let (group, consumed) = GroupAddress::parse_prefix(input).unwrap();
    assert_eq!(group.display_name(), "G");
    assert_eq!(&input[consumed..], b",next@x");
}

#[test]
fn group_only_and_mailbox_only_parsers_reject_the_other_variant() {
    for input in ["a@b", "Name <a@b>", "local", "<@route:a@b>"] {
        let error = GroupAddress::parse(input.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), Kind::ExpectedGroup);
    }
    assert_eq!(
        MailboxAddress::parse(b"G:a@b;").unwrap_err().kind(),
        MailboxParseErrorKind::GroupNotAllowed
    );
}

#[test]
fn missing_terminators_depend_on_mode() {
    for input in ["G:a@b", "G:a@b, (tail)", "G:Inner:a@b", "G:Inner:a@b;"] {
        for mode in [Mode::Loose, Mode::Looser] {
            let parsed =
                GroupAddress::parse_with_options(input.as_bytes(), &nested_options(mode)).unwrap();
            assert_eq!(mailboxes(parsed.members()), ["a@b"]);
        }
        let error =
            GroupAddress::parse_with_options(input.as_bytes(), &nested_options(Mode::Strict))
                .unwrap_err();
        assert_eq!(error.kind(), Kind::MissingGroupTerminator);
        assert_eq!(error.offset(), input.len());
    }
    for input in ["G:", "G: (comment)", "G:\r\n", "G:Inner:"] {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            let error = GroupAddress::parse_with_options(input.as_bytes(), &nested_options(mode))
                .unwrap_err();
            assert_eq!(error.kind(), Kind::MissingAddress);
            assert_eq!(error.offset(), input.len());
        }
    }
}

#[test]
fn strict_separators_report_original_offsets() {
    for (input, offset) in [
        ("a@example.com b@example.com", 14),
        ("Friends: a@example.com b@example.com;", 23),
        ("<a@b> <c@d>", 6),
        ("G:a@b; c@d", 7),
    ] {
        let error =
            AddressList::parse_with_options(input.as_bytes(), &options(Mode::Strict)).unwrap_err();
        assert_eq!(error.kind(), Kind::MissingSeparator, "{input}");
        assert_eq!(error.offset(), offset, "{input}");
        for mode in [Mode::Loose, Mode::Looser] {
            assert_eq!(
                AddressList::parse_with_options(input.as_bytes(), &options(mode))
                    .unwrap()
                    .mailboxes()
                    .count(),
                2
            );
        }
    }
}

#[test]
fn repeated_and_trailing_commas_are_not_leading_commas() {
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        for input in [
            "a@b,,, (c), c@d,, (tail)",
            "G:a@b,,,c@d,,,;",
            "G:a@b;,,,c@d,,",
        ] {
            let list = AddressList::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
            assert_eq!(mailboxes(&list), ["a@b", "c@d"], "{input}");
        }
        for input in [",a@b", ",,,", "G:,a@b;", "G:,,,;"] {
            assert!(
                AddressList::parse_with_options(input.as_bytes(), &options(mode)).is_err(),
                "{input}"
            );
        }
    }
}

#[test]
fn explicit_empty_list_differs_from_empty_input() {
    assert!(AddressList::new().is_empty());
    assert_eq!(AddressList::default().len(), 0);
    for input in [b"".as_slice(), b" \t\r\n", b"(comment)"] {
        assert_eq!(
            AddressList::parse(input).unwrap_err().kind(),
            Kind::MissingAddress
        );
        assert_eq!(Address::parse(input).unwrap_err().offset(), input.len());
    }
    let list: AddressList = "Empty:;".parse().unwrap();
    assert!(!list.is_empty());
    assert_eq!(list.mailboxes().count(), 0);
}

#[test]
fn nesting_boundaries_include_outermost_and_empty_groups() {
    let safe = "group0: group1: group2: mailbox@host.com;;;";
    let overflow = "group0: group1: group2: group3: mailbox@host.com;;;;";
    let nested = b"Outer:Inner:;;";
    for error in [
        Address::parse(nested).unwrap_err(),
        GroupAddress::parse(nested).unwrap_err(),
        AddressList::parse(nested).unwrap_err(),
        Address::parse_prefix(nested).unwrap_err(),
        GroupAddress::parse_prefix(nested).unwrap_err(),
    ] {
        assert_eq!(error.kind(), Kind::GroupDepthExceeded);
        assert_eq!(error.offset(), 11);
    }
    assert!(
        GroupAddress::parse_with_options(safe.as_bytes(), &nested_options(Mode::Loose)).is_ok()
    );
    let error = GroupAddress::parse_with_options(overflow.as_bytes(), &nested_options(Mode::Loose))
        .unwrap_err();
    assert_eq!(error.kind(), Kind::GroupDepthExceeded);
    assert_eq!(error.offset(), overflow.find("group3:").unwrap() + 6);
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let mut options = options(mode);
        assert!(AddressList::parse_with_options(b"A:;,B:a@b;,C:;", &options).is_ok());
        assert_eq!(
            GroupAddress::parse_with_options(nested, &options)
                .unwrap_err()
                .kind(),
            Kind::GroupDepthExceeded
        );
        options.max_group_depth = 0;
        assert!(AddressList::parse_with_options(b"a@b", &options).is_ok());
        assert_eq!(
            GroupAddress::parse_with_options(b"Empty:;", &options)
                .unwrap_err()
                .kind(),
            Kind::GroupDepthExceeded
        );
        options.max_group_depth = 1;
        assert!(AddressList::parse_with_options(b"A:;,B:a@b;,C:;", &options).is_ok());
        assert_eq!(
            GroupAddress::parse_with_options(b"Outer:Inner:;;", &options)
                .unwrap_err()
                .kind(),
            Kind::GroupDepthExceeded
        );
        options.max_group_depth = 4;
        assert!(GroupAddress::parse_with_options(overflow.as_bytes(), &options).is_ok());
    }
}

#[test]
fn maximum_configurable_nesting_parses_clones_traverses_and_drops() {
    let options = AddressParseOptions {
        max_group_depth: u8::MAX,
        ..Default::default()
    };
    let input = format!("{}a@b{}", "g:".repeat(255), ";".repeat(255));
    let list = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    assert_eq!(mailboxes(&list), ["a@b"]);
    let cloned = list.clone();
    drop(list);
    assert_eq!(mailboxes(&cloned), ["a@b"]);
    drop(cloned);
    let input = format!("{}a@b{}", "g:".repeat(256), ";".repeat(256));
    let error = AddressList::parse_with_options(input.as_bytes(), &options).unwrap_err();
    assert_eq!(error.kind(), Kind::GroupDepthExceeded);
    assert_eq!(error.offset(), 511);
    let input = "g:".repeat(20_000);
    assert_eq!(
        AddressList::parse(input.as_bytes()).unwrap_err().kind(),
        Kind::GroupDepthExceeded
    );
}

#[test]
fn recursive_mailbox_order_matches_source_and_iterator_is_fused() {
    let list = AddressList::parse_with_options(
        b"Before <before@x>, Outer:Outer1 <outer1@x>, Inner:Inner1 <inner1@x>, Inner2 <inner2@x>;, Empty:;, Outer2 <outer2@x>;, After <after@x>",
        &nested_options(Mode::Loose),
    ).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(
        mailboxes(&list),
        [
            "before@x", "outer1@x", "inner1@x", "inner2@x", "outer2@x", "after@x"
        ]
    );
    let mut iter = list.mailboxes();
    assert_eq!(iter.next().unwrap().display_name(), "Before");
    let mut cloned = iter.clone();
    assert_eq!(iter.next().unwrap().display_name(), "Outer1");
    assert_eq!(cloned.next().unwrap().display_name(), "Outer1");
    assert_eq!(iter.by_ref().count(), 4);
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
    assert_eq!(cloned.count(), 4);
    let empty = AddressList::new();
    let mut iter = empty.mailboxes();
    assert!(iter.next().is_none());
    assert!(iter.next().is_none());
}

#[test]
fn list_edits_and_owned_borrowed_iteration_preserve_duplicates() {
    let mut list: AddressList = "a@b,G:c@d;,a@b".parse().unwrap();
    list.as_mut_vec().swap(0, 1);
    assert_eq!((&list).into_iter().next().unwrap().display_name(), "G");
    assert_eq!(mailboxes(&list), ["c@d", "a@b", "a@b"]);
    let collected: AddressList = list.clone().into_iter().collect();
    assert_eq!(mailboxes(&collected), mailboxes(&list));
    let from_vec = AddressList::from(collected.into_vec());
    assert_eq!(mailboxes(&from_vec), mailboxes(&list));
    list.as_mut_vec().clear();
    assert!(list.is_empty());
}

#[test]
fn unquoted_name_commas_keep_mailbox_ambiguity_options() {
    let input = b"local, Name <n@x>";
    let list = AddressList::parse(input).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list.as_slice()[0].display_name(), "local, Name");
    let mut options = AddressParseOptions::default();
    options.mailbox.allow_unquoted_commas = false;
    let list = AddressList::parse_with_options(input, &options).unwrap();
    assert_eq!(mailboxes(&list), ["local", "n@x"]);
    options.mailbox.address.allow_local_only = false;
    assert!(AddressList::parse_with_options(input, &options).is_err());
    assert!(AddressList::parse_with_options(b"G:<local>;", &options).is_ok());
    assert!(AddressList::parse_with_options(b"G:local;", &options).is_err());
}

#[test]
fn delimiters_inside_names_comments_literals_and_routes_do_not_split_lists() {
    let input =
        b"\"Name, : ;\" <@route1,@route2:\"a,b;:\"@[tag:a,b;]>, a@b (Name, : ; (nested)), G:c@d;";
    let list = AddressList::parse(input).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(mailboxes(&list), ["\"a,b;:\"@[tag:a,b;]", "a@b", "c@d"]);
    assert_eq!(list.as_slice()[0].display_name(), "Name, : ;");
    assert_eq!(list.as_slice()[1].display_name(), "Name, : ; (nested)");
    let Address::Mailbox(first) = &list.as_slice()[0] else {
        panic!("mailbox expected");
    };
    assert_eq!(first.route().as_slice(), &["route1", "route2"]);
}

#[test]
fn canonical_list_repairs_share_mailbox_compatibility() {
    for (input, expected) in [
        (
            "<third@example.net, fourth@example.net>",
            vec!["third@example.net", "fourth@example.net"],
        ),
        (
            "\"Joe <joe@example.com>, Bob <bob@example.com>",
            vec!["joe@example.com", "bob@example.com"],
        ),
        ("<<<a@b>>>, c@d>", vec!["a@b", "c@d"]),
    ] {
        assert_eq!(
            mailboxes(&AddressList::parse(input.as_bytes()).unwrap()),
            expected
        );
        assert!(AddressList::parse_with_options(input.as_bytes(), &options(Mode::Strict)).is_err());
    }
    let input = b"G:First Last <webmaster\\@custom-domain.com@mail-host.com>;";
    let list = AddressList::parse_with_options(input, &options(Mode::Looser)).unwrap();
    assert_eq!(
        mailboxes(&list),
        ["webmaster%40custom-domain.com@mail-host.com"]
    );
    assert!(AddressList::parse(input).is_err());
}

#[test]
fn addr_spec_followed_by_colon_is_not_a_group_name() {
    for input in ["a@b:c@d;", "<a@b>:<c@d>;"] {
        let list: AddressList = input.parse().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(group(&list.as_slice()[1]).display_name(), "");
        assert_eq!(mailboxes(&list), ["a@b", "c@d"]);
        assert!(AddressList::parse_with_options(input.as_bytes(), &options(Mode::Strict)).is_err());
    }
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let list = AddressList::parse_with_options(b"\"a@b\":c@d;", &options(mode)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(group(&list.as_slice()[0]).display_name(), "a@b");
    }
}

#[test]
fn malformed_members_are_not_silently_discarded() {
    for input in [
        "good@x, bad@, other@x",
        "G:good@x, bad@, other@x;",
        "good@x, \"broken, Other <other@x>",
        "good@x, (broken, other@x",
        "good@x; other@x",
        "G:good@x;;",
        "G:good@x, Inner:bad@;;",
    ] {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            assert!(
                AddressList::parse_with_options(input.as_bytes(), &nested_options(mode)).is_err(),
                "{input:?}, {mode:?}"
            );
        }
    }
    let mut options = options(Mode::Strict);
    options.mailbox.allow_unquoted_commas = false;
    options.mailbox.address.allow_local_only = false;
    assert!(
        AddressList::parse_with_options(
            b"Good <good@x>, \"Last Name, First Name\" (no address), Other <other@x>",
            &options
        )
        .is_err()
    );
}

#[test]
fn nested_errors_keep_whole_input_byte_offsets() {
    let options = nested_options(Mode::Loose);
    let input = "Ren\u{e9}e: Outer:a@\u{fc}, bad@;";
    let error = AddressList::parse_with_options(input.as_bytes(), &options).unwrap_err();
    assert_eq!(
        error.kind(),
        Kind::Mailbox(MailboxParseErrorKind::AddrSpec(
            AddrSpecParseErrorKind::InvalidDomain
        ))
    );
    assert_eq!(error.offset(), input.len() - 1);
    let input = b"Before <a@b>, G:Inner:a@b, Name <x@\xff>;;";
    let error = AddressList::parse_with_options(input, &options).unwrap_err();
    assert_eq!(
        error.kind(),
        Kind::Mailbox(MailboxParseErrorKind::AddrSpec(
            AddrSpecParseErrorKind::InvalidUtf8
        ))
    );
    assert_eq!(
        error.offset(),
        input.iter().position(|&byte| byte == 255).unwrap()
    );
    let input = b"G:Inner:a@b (unclosed";
    let error = AddressList::parse_with_options(input, &options).unwrap_err();
    assert_eq!(
        error.offset(),
        input.iter().position(|&byte| byte == b'(').unwrap()
    );
}

#[test]
fn byte_recovery_and_domain_spelling_survive_group_membership() {
    let list = AddressList::parse(b"G:Name <caf\xe9@XN--BCHER-KVA.EXAMPLE>;").unwrap();
    let mailbox = list.mailboxes().next().unwrap();
    assert_eq!(mailbox.address().local_part(), "caf\u{e9}");
    assert_eq!(mailbox.address().domain(), Some("XN--BCHER-KVA.EXAMPLE"));
    assert!(mailbox.address().had_latin1_fallback());
    assert_eq!(
        mailbox.address().with_unicode_domain().unwrap(),
        "caf\u{e9}@b\u{fc}cher.example"
    );
    assert!(
        AddressList::parse_with_options(b"G:Name <caf\xe9@x>;", &options(Mode::Strict)).is_err()
    );
    let input = "G:\u{e9}@b\u{fc}cher.example, \u{3b8}@\u{3b5}.example;, \u{44f}@example";
    let list: AddressList = input.parse().unwrap();
    assert_eq!(
        mailboxes(&list),
        [
            "\u{e9}@b\u{fc}cher.example",
            "\u{3b8}@\u{3b5}.example",
            "\u{44f}@example"
        ]
    );
}

#[test]
fn truncations_and_random_bytes_make_progress_or_report_bounded_errors() {
    let check = |input: &[u8]| {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            let options = options(mode);
            match Address::parse_prefix_with_options(input, &options) {
                Ok((_, consumed)) => {
                    assert!(consumed > 0 && consumed <= input.len());
                    assert_eq!(
                        Address::parse_with_options(input, &options).is_ok(),
                        consumed == input.len()
                    );
                }
                Err(error) => assert!(error.offset() <= input.len(), "{input:?}: {error}"),
            }
            match AddressList::parse_with_options(input, &options) {
                Ok(list) => {
                    assert!(!list.is_empty());
                    for mailbox in list.mailboxes() {
                        assert!(!mailbox.address().as_str().is_empty());
                    }
                }
                Err(error) => assert!(error.offset() <= input.len(), "{input:?}: {error}"),
            }
        }
    };
    for sample in [
        "G:Inner:\"\u{e9}@x\"@b\u{fc}cher.example;, a@b;, Empty:;",
        "\"=?utf-8?q?G?=\":Name <@relay:a@[::1]>, Empty:;;",
    ] {
        for end in 0..=sample.len() {
            check(&sample.as_bytes()[..end]);
        }
    }
    let alphabet = b"abc.:;,@<>()[ ]\\\"\r\n\t\0\xc3\xa9\xff";
    let mut seed = 42u64;
    for len in 0..100 {
        for _ in 0..30 {
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

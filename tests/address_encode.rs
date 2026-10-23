// Adapted from MimeKit's MailboxAddressTests.cs, GroupAddressTests.cs,
// and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::error::Error;

use mimecraft::{
    AddrSpec, AddrSpecParseOptions, Address, AddressComplianceMode, AddressEncodeErrorKind,
    AddressList, AddressParseOptions, GroupAddress, MailboxAddress, MailboxParseOptions,
    charsets::Charset,
    rfc2047::{self, EncodeOptions},
};

fn mailbox(name: &str, address: &str) -> MailboxAddress {
    MailboxAddress::new(name, address.parse().unwrap())
}

#[test]
fn mailbox_values_encode_names_and_angle_local_only_addresses() {
    for (name, address, expected) in [
        ("", "local", "<local>"),
        ("", "user@EXAMPLE.COM", "user@example.com"),
        ("Alice Smith", "a@b", "Alice Smith <a@b>"),
        ("Doe, Jane", "a@b", "\"Doe, Jane\" <a@b>"),
        (
            "A \"quoted\" \\ name",
            "a@b",
            "\"A \\\"quoted\\\" \\\\ name\" <a@b>",
        ),
        (
            "",
            "\"quoted local\"@example.com",
            "\"quoted local\"@example.com",
        ),
        ("", "\"a\tb\"@example.com", "\"a\tb\"@example.com"),
        ("Ren\u{e9}e", "a@b", "=?utf-8?b?UmVuw6ll?= <a@b>"),
    ] {
        let original = mailbox(name, address);
        let encoded = original.to_ascii().unwrap();
        assert_eq!(encoded, expected);
        assert!(encoded.is_ascii());
        let parsed: MailboxAddress = encoded.parse().unwrap();
        assert_eq!(parsed.display_name(), original.display_name());
        assert_eq!(
            parsed.address().local_part(),
            original.address().local_part()
        );
        assert_eq!(Address::from(original).to_ascii().unwrap(), expected);
    }
    let options = MailboxParseOptions {
        address: AddrSpecParseOptions {
            allow_local_only: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let encoded = mailbox("", "local").to_ascii().unwrap();
    assert!(MailboxAddress::parse_with_options(encoded.as_bytes(), &options).is_ok());
}

#[test]
fn address_domains_and_routes_use_checked_ascii_without_mutation() {
    let mut value = mailbox("", "user@b\u{fc}cher.example");
    *value.route_mut() = [
        "",
        "Relay.EXAMPLE",
        " \t",
        "b\u{fc}cher.example",
        "[IPv6:::1]",
        "Relay.EXAMPLE",
    ]
    .into_iter()
    .collect();
    let displayed = value.to_string();
    let encoded = value.to_ascii().unwrap();
    assert_eq!(
        encoded,
        "<@relay.example,@xn--bcher-kva.example,@[IPv6:::1],@relay.example:user@xn--bcher-kva.example>"
    );
    let parsed: MailboxAddress = encoded.parse().unwrap();
    assert_eq!(
        parsed.route().as_slice(),
        &[
            "relay.example",
            "xn--bcher-kva.example",
            "[IPv6:::1]",
            "relay.example"
        ]
    );
    assert_eq!(value.to_string(), displayed);

    *value.route_mut() = ["", " \t", "\u{a0}"].into_iter().collect();
    assert_eq!(value.to_ascii().unwrap(), "user@xn--bcher-kva.example");
    assert_eq!(
        mailbox("", "user@[not-an-ip:25]").to_ascii().unwrap(),
        "user@[not-an-ip:25]"
    );
}

#[test]
fn empty_and_punctuated_group_names_have_round_trippable_syntax() {
    for (name, expected) in [
        ("", "\"\": ;"),
        ("Friends", "Friends: ;"),
        ("A, B: C;", "\"A, B: C;\": ;"),
        ("Ren\u{e9}e", "=?utf-8?b?UmVuw6ll?=: ;"),
    ] {
        let group = GroupAddress::new(name, AddressList::new());
        let encoded = group.to_ascii().unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(encoded.parse::<GroupAddress>().unwrap(), group);
        assert_eq!(Address::from(group).to_ascii().unwrap(), expected);
    }
}

#[test]
fn nested_members_empty_groups_and_duplicates_keep_their_order() {
    let list: AddressList = vec![
        mailbox("", "before@x").into(),
        GroupAddress::new(
            "Outer",
            vec![
                mailbox("First", "first@x").into(),
                GroupAddress::new("", AddressList::new()).into(),
                GroupAddress::new("", vec![mailbox("", "inner@x").into()].into()).into(),
                mailbox("First", "first@x").into(),
            ]
            .into(),
        )
        .into(),
        mailbox("", "after@x").into(),
    ]
    .into();
    let encoded = list.to_ascii().unwrap();
    assert_eq!(
        encoded,
        "before@x, Outer: First <first@x>, \"\": ;, \"\": inner@x;, First <first@x>;, after@x"
    );
    let options = AddressParseOptions {
        max_group_depth: 2,
        ..Default::default()
    };
    assert_eq!(
        AddressList::parse_with_options(encoded.as_bytes(), &options).unwrap(),
        list
    );
    assert_eq!(AddressList::new().to_ascii().unwrap(), "");
}

#[test]
fn name_controls_and_encoded_word_lookalikes_are_encoded_not_sanitized() {
    for name in [
        "A\r\nB\t\0C",
        "=?utf-8?q?Not_decoded?=",
        "A  B",
        "A \"\\\" B",
    ] {
        let value = mailbox(name, "a@b");
        let encoded = value.to_ascii().unwrap();
        assert!(encoded.is_ascii());
        assert!(!encoded.contains(['\r', '\n', '\0']));
        assert_eq!(
            encoded.parse::<MailboxAddress>().unwrap().display_name(),
            name
        );
        let group = GroupAddress::new(name, vec![value.into()].into());
        let encoded = group.to_ascii().unwrap();
        assert!(!encoded.contains(['\r', '\n', '\0']));
        assert_eq!(encoded.parse::<GroupAddress>().unwrap(), group);
    }
}

#[test]
fn charset_and_word_limits_apply_to_every_display_name_without_folding() {
    let options = EncodeOptions {
        charset: Charset::LATIN1,
        max_word_length: 30,
    };
    let name = "Andr\u{e9} ".repeat(30).trim().to_owned();
    let value = mailbox(&name, "a@b");
    let expected_name = rfc2047::encode_phrase_with_options(&name, &options).unwrap();
    assert!(expected_name.split(' ').all(|word| word.len() <= 30));
    assert_eq!(
        value.to_ascii_with_options(&options).unwrap(),
        format!("{expected_name} <a@b>")
    );
    let group = GroupAddress::new(&name, vec![value.into()].into());
    let list: AddressList = vec![group.clone().into()].into();
    let expected = format!("{expected_name}: {expected_name} <a@b>;");
    assert_eq!(group.to_ascii_with_options(&options).unwrap(), expected);
    assert_eq!(
        Address::from(group)
            .to_ascii_with_options(&options)
            .unwrap(),
        expected
    );
    let encoded = list.to_ascii_with_options(&options).unwrap();
    assert_eq!(encoded, expected);
    assert!(encoded.len() > 998);
    assert!(!encoded.contains(['\r', '\n']));
    assert_eq!(encoded.parse::<AddressList>().unwrap(), list);
}

#[test]
fn invalid_options_fail_even_without_names_or_members() {
    for max_word_length in [0, 76, usize::MAX] {
        let options = EncodeOptions {
            max_word_length,
            ..Default::default()
        };
        let value = mailbox("", "a@b");
        let group = GroupAddress::new("", AddressList::new());
        for result in [
            value.to_ascii_with_options(&options),
            group.to_ascii_with_options(&options),
            Address::from(value).to_ascii_with_options(&options),
            Address::from(group).to_ascii_with_options(&options),
            AddressList::new().to_ascii_with_options(&options),
        ] {
            let error = result.unwrap_err();
            assert!(error.path().is_empty());
            assert_eq!(
                error.kind(),
                &AddressEncodeErrorKind::Name(rfc2047::EncodeError::InvalidWordLength {
                    requested: max_word_length
                })
            );
            assert!(error.source().is_some());
        }
    }
}

#[test]
fn international_local_parts_are_never_silently_downgraded() {
    for address in [
        "caf\u{e9}@example.com",
        "\"caf\u{e9}\"@b\u{fc}cher.example",
        "caf\u{e9}",
    ] {
        let value = mailbox("", address);
        let error = value.to_ascii().unwrap_err();
        assert_eq!(
            error.kind(),
            &AddressEncodeErrorKind::InternationalLocalPart
        );
        assert!(error.path().is_empty());
        assert!(error.source().is_none());
        assert_eq!(value.address().as_str(), address);
    }
}

#[test]
fn compatibility_only_local_syntax_is_rejected_for_authoring() {
    let options = AddrSpecParseOptions {
        compliance_mode: AddressComplianceMode::Looser,
        ..Default::default()
    };
    for text in ["bad..local@example.com", "bad.@example.com"] {
        let address = AddrSpec::parse_with_options(text.as_bytes(), &options).unwrap();
        let error = MailboxAddress::new("", address).to_ascii().unwrap_err();
        assert!(matches!(error.kind(), AddressEncodeErrorKind::AddrSpec(_)));
        assert!(error.source().is_some());
    }
}

#[test]
fn invalid_domains_and_edited_routes_report_explicit_errors() {
    for domain in [
        "-bad.example",
        "bad\u{ff20}example.com",
        "bad\u{2009}example.com",
    ] {
        let value = mailbox("", &format!("a@{domain}"));
        let error = value.to_ascii().unwrap_err();
        assert!(matches!(error.kind(), AddressEncodeErrorKind::Domain(_)));
        assert!(error.source().is_some());
    }
    for domain in [
        "bad domain",
        "evil\r\nBcc:other@example.com",
        "[broken",
        "[ok]junk",
    ] {
        let mut value = mailbox("Good", "a@b");
        *value.route_mut() = ["", "relay.example", domain].into_iter().collect();
        let before = value.to_string();
        let error = value.to_ascii().unwrap_err();
        let AddressEncodeErrorKind::Route(source) = error.kind() else {
            panic!("expected route failure, got {error}");
        };
        assert_eq!(source.domain_index(), 2);
        assert!(error.source().is_some());
        assert_eq!(value.to_string(), before);
    }
}

#[test]
fn nested_failure_paths_include_lists_groups_and_original_route_indexes() {
    let mut invalid = mailbox("", "a@b");
    *invalid.route_mut() = ["", "relay.example", "bad domain"].into_iter().collect();
    let inner = GroupAddress::new(
        "Inner",
        vec![
            GroupAddress::new("Empty", AddressList::new()).into(),
            invalid.into(),
        ]
        .into(),
    );
    let outer = GroupAddress::new(
        "Outer",
        vec![mailbox("", "ok@x").into(), inner.into()].into(),
    );
    let error = outer.to_ascii().unwrap_err();
    assert_eq!(error.path(), &[1, 1]);
    let list: AddressList = vec![
        GroupAddress::new("Before", vec![mailbox("", "before@x").into()].into()).into(),
        outer.into(),
    ]
    .into();
    let error = list.to_ascii().unwrap_err();
    assert_eq!(error.path(), &[1, 1, 1]);
    assert!(error.to_string().contains("[1, 1, 1]"));
    let AddressEncodeErrorKind::Route(source) = error.kind() else {
        panic!("expected route failure");
    };
    assert_eq!(source.domain_index(), 2);
}

#[test]
fn charset_and_word_size_failures_identify_the_failing_group_or_mailbox() {
    let options = EncodeOptions {
        charset: Charset::ASCII,
        ..Default::default()
    };
    let group = GroupAddress::new("caf\u{e9}", AddressList::new());
    let error = group.to_ascii_with_options(&options).unwrap_err();
    assert!(error.path().is_empty());
    assert!(matches!(
        error.kind(),
        AddressEncodeErrorKind::Name(rfc2047::EncodeError::Charset(_))
    ));
    assert!(error.source().unwrap().source().is_some());
    let list: AddressList = vec![mailbox("", "a@b").into(), group.into()].into();
    assert_eq!(
        list.to_ascii_with_options(&options).unwrap_err().path(),
        &[1]
    );

    let options = EncodeOptions {
        max_word_length: 1,
        ..Default::default()
    };
    let list: AddressList =
        vec![GroupAddress::new("G", vec![mailbox("Ren\u{e9}e", "a@b").into()].into()).into()]
            .into();
    let error = list.to_ascii_with_options(&options).unwrap_err();
    assert_eq!(error.path(), &[0, 0]);
    assert!(matches!(
        error.kind(),
        AddressEncodeErrorKind::Name(rfc2047::EncodeError::WordTooLong { .. })
    ));
}

#[test]
fn maximum_parsed_nesting_encodes_and_reports_the_deepest_failure() {
    let options = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let input = format!("{}a@b{}", "g:".repeat(255), ";".repeat(255));
    let list = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    let encoded = list.to_ascii().unwrap();
    assert_eq!(
        encoded,
        format!("{}a@b{}", "g: ".repeat(255), ";".repeat(255))
    );
    assert_eq!(
        AddressList::parse_with_options(encoded.as_bytes(), &options).unwrap(),
        list
    );
    let input = format!("{}caf\u{e9}@b{}", "g:".repeat(255), ";".repeat(255));
    let list = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    let error = list.to_ascii().unwrap_err();
    assert_eq!(error.path(), vec![0; 256]);
    assert_eq!(
        error.kind(),
        &AddressEncodeErrorKind::InternationalLocalPart
    );
}

#[test]
fn encoding_preserves_source_values_and_diagnostics_on_success_and_failure() {
    let mut value = MailboxAddress::parse(b"Andr\xe9 <a@b>").unwrap();
    let issues = value.display_name_issues().to_vec();
    assert!(!issues.is_empty());
    let displayed = value.to_string();
    assert_eq!(
        value.to_ascii().unwrap().parse::<MailboxAddress>().unwrap(),
        value
    );
    assert_eq!(value.display_name_issues(), issues);
    assert_eq!(value.to_string(), displayed);
    value.set_address(AddrSpec::parse(b"caf\xe9@b").unwrap());
    assert!(value.address().had_latin1_fallback());
    assert!(value.to_ascii().is_err());
    assert!(value.address().had_latin1_fallback());
    assert_eq!(value.display_name_issues(), issues);
}

// Adapted from MimeKit's MailboxAddressTests.cs and GroupAddressTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{AddrSpec, Address, AddressList, GroupAddress, MailboxAddress};

fn mailbox(name: &str, address: &str) -> MailboxAddress {
    MailboxAddress::new(name, address.parse().unwrap())
}

#[test]
fn constructors_and_setters_trim_unicode_name_whitespace() {
    let mut mailbox = mailbox(" \tJohn Smith\u{a0} ", "j.smith@example.com");
    assert_eq!(mailbox.display_name(), "John Smith");
    assert!(mailbox.display_name_issues().is_empty());
    assert!(mailbox.route().is_empty());
    mailbox.set_display_name("  Jane Smith  ");
    assert_eq!(mailbox.display_name(), "Jane Smith");
    mailbox.set_display_name(" \t\u{a0}\r\n");
    assert_eq!(mailbox.display_name(), "");
    assert_eq!(mailbox.address().as_str(), "j.smith@example.com");

    let mut group = GroupAddress::new("\u{2003} Friends \t", AddressList::new());
    assert_eq!(group.display_name(), "Friends");
    assert!(group.display_name_issues().is_empty());
    assert!(group.members().is_empty());
    group.set_display_name("\t Colleagues \r\n");
    assert_eq!(group.display_name(), "Colleagues");
    group.set_display_name("");
    assert_eq!(group.display_name(), "");
}

#[test]
fn assigned_names_are_text_not_header_syntax_or_sanitized_output() {
    for name in [
        "=?utf-8?q?Ren=C3=A9e?=",
        "\"Quoted\" \\ Name (comment)",
        "Name\r\nWith\tControls\0",
        "Ren\u{e9}e",
    ] {
        let mut mailbox = mailbox(name, "a@b");
        let mut group = GroupAddress::new(name, AddressList::new());
        assert_eq!(mailbox.display_name(), name);
        assert_eq!(group.display_name(), name);
        mailbox.set_display_name(name);
        group.set_display_name(name);
        assert_eq!(mailbox.display_name(), name);
        assert_eq!(group.display_name(), name);
        assert!(mailbox.display_name_issues().is_empty());
        assert!(group.display_name_issues().is_empty());
    }
}

#[test]
fn construction_preserves_typed_address_spelling_and_diagnostics() {
    let address = AddrSpec::parse(b"caf\xe9@XN--BCHER-KVA.EXAMPLE").unwrap();
    let mailbox = MailboxAddress::new("", address);
    assert_eq!(
        mailbox.address().as_str(),
        "caf\u{e9}@XN--BCHER-KVA.EXAMPLE"
    );
    assert!(mailbox.address().had_latin1_fallback());
    assert!(mailbox.display_name_issues().is_empty());
    assert_eq!(self::mailbox("", "local").address().as_str(), "local");
}

#[test]
fn replacing_names_clears_only_their_own_diagnostics() {
    let mut mailbox = MailboxAddress::parse(b"Andr\xe9 <@relay:caf\xe9@x>").unwrap();
    assert!(!mailbox.display_name_issues().is_empty());
    mailbox.set_display_name("Andr\u{e9}");
    assert!(mailbox.display_name_issues().is_empty());
    assert!(mailbox.address().had_latin1_fallback());
    assert_eq!(mailbox.route().as_slice(), &["relay"]);

    let member = MailboxAddress::parse(b"Ren\xe9 <caf\xe9@x>").unwrap();
    let mut group = GroupAddress::parse(b"Andr\xe9:;").unwrap();
    group.members_mut().as_mut_vec().push(member.into());
    assert!(!group.display_name_issues().is_empty());
    group.set_display_name("Andr\u{e9}");
    assert!(group.display_name_issues().is_empty());
    let member = group.members().mailboxes().next().unwrap();
    assert!(!member.display_name_issues().is_empty());
    assert!(member.address().had_latin1_fallback());
}

#[test]
fn replacing_address_preserves_name_and_route_diagnostics() {
    let mut mailbox = MailboxAddress::parse(b"Andr\xe9 <@relay:caf\xe9@x>").unwrap();
    let issues = mailbox.display_name_issues().to_vec();
    mailbox.set_address("new@XN--BCHER-KVA.EXAMPLE".parse().unwrap());
    assert_eq!(mailbox.address().as_str(), "new@XN--BCHER-KVA.EXAMPLE");
    assert!(!mailbox.address().had_latin1_fallback());
    assert_eq!(mailbox.display_name(), "Andr\u{e9}");
    assert_eq!(mailbox.display_name_issues(), issues);
    assert_eq!(mailbox.route().as_slice(), &["relay"]);

    mailbox.set_address(AddrSpec::parse(b"caf\xe9@x").unwrap());
    assert!(mailbox.address().had_latin1_fallback());
    let replace = |mailbox: &mut MailboxAddress, input: &[u8]| {
        mailbox.set_address(AddrSpec::parse(input)?);
        Ok::<(), mimecraft::AddrSpecParseError>(())
    };
    for invalid in [b"".as_slice(), b"bad@", b"user@host extra"] {
        assert!(replace(&mut mailbox, invalid).is_err());
        assert_eq!(mailbox.address().as_str(), "caf\u{e9}@x");
        assert!(mailbox.address().had_latin1_fallback());
        assert_eq!(mailbox.display_name_issues(), issues);
    }
}

#[test]
fn route_edits_preserve_other_fields_and_remain_unvalidated() {
    let mut mailbox = MailboxAddress::parse(b"Andr\xe9 <caf\xe9@x>").unwrap();
    let issues = mailbox.display_name_issues().to_vec();
    *mailbox.route_mut() = ["relay.example", "relay.example"].into_iter().collect();
    mailbox.route_mut().as_mut_vec().push("not a domain".into());
    assert_eq!(
        mailbox.route().as_slice(),
        &["relay.example", "relay.example", "not a domain"]
    );
    assert!(mailbox.route().to_ascii().is_err());
    assert_eq!(mailbox.display_name_issues(), issues);
    assert!(mailbox.address().had_latin1_fallback());
    mailbox.route_mut().as_mut_vec().clear();
    assert!(mailbox.route().is_empty());
}

#[test]
fn enum_name_edits_handle_both_variants_without_redecoding() {
    let mailbox = MailboxAddress::parse(b"Andr\xe9 <a@b>").unwrap();
    let group = GroupAddress::parse(b"Andr\xe9:a@b;").unwrap();
    for mut address in [Address::from(mailbox), Address::from(group)] {
        address.set_display_name("  =?utf-8?q?Name?=  ");
        assert_eq!(address.display_name(), "=?utf-8?q?Name?=");
        match address {
            Address::Mailbox(mailbox) => {
                assert!(mailbox.display_name_issues().is_empty());
                assert_eq!(mailbox.address().as_str(), "a@b");
            }
            Address::Group(group) => {
                assert!(group.display_name_issues().is_empty());
                assert_eq!(
                    group
                        .members()
                        .mailboxes()
                        .next()
                        .unwrap()
                        .address()
                        .as_str(),
                    "a@b"
                );
            }
        }
    }
}

#[test]
fn group_construction_and_edits_preserve_order_duplicates_and_empty_groups() {
    let first = mailbox("First", "first@x");
    let inner = GroupAddress::new("Inner", vec![first.clone().into()].into());
    let mut group = GroupAddress::new("Outer", vec![first.into(), inner.into()].into());
    group
        .members_mut()
        .as_mut_vec()
        .insert(1, GroupAddress::new("Empty", AddressList::new()).into());
    group
        .members_mut()
        .as_mut_vec()
        .push(mailbox("Last", "last@x").into());
    let addresses: Vec<_> = group
        .members()
        .mailboxes()
        .map(|m| m.address().as_str())
        .collect();
    assert_eq!(addresses, ["first@x", "first@x", "last@x"]);
    assert_eq!(group.members().len(), 4);
    group.members_mut().as_mut_vec().remove(0);
    assert_eq!(group.members().mailboxes().count(), 2);
    *group.members_mut() = AddressList::new();
    assert!(group.members().is_empty());
    assert_eq!(group.display_name(), "Outer");
}

#[test]
fn cloned_groups_and_mailboxes_can_be_edited_independently() {
    let group = GroupAddress::parse(b"Friends:Andr\xe9 <@relay:caf\xe9@x>;").unwrap();
    let mut cloned = group.clone();
    cloned.set_display_name("Colleagues");
    let Address::Mailbox(member) = &mut cloned.members_mut().as_mut_vec()[0] else {
        panic!("expected a mailbox");
    };
    member.set_display_name("New");
    member.set_address("new@y".parse().unwrap());
    member.route_mut().as_mut_vec().clear();
    let original = group.members().mailboxes().next().unwrap();
    assert_eq!(group.display_name(), "Friends");
    assert_eq!(original.display_name(), "Andr\u{e9}");
    assert!(!original.display_name_issues().is_empty());
    assert!(original.address().had_latin1_fallback());
    assert_eq!(original.route().as_slice(), &["relay"]);
    assert_eq!(
        cloned
            .members()
            .mailboxes()
            .next()
            .unwrap()
            .address()
            .as_str(),
        "new@y"
    );
}

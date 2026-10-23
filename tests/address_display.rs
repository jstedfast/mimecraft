// Adapted from MimeKit's MailboxAddressTests.cs, GroupAddressTests.cs,
// and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fmt::{self, Write};

use mimecraft::{Address, AddressList, AddressParseOptions, GroupAddress, MailboxAddress};

fn mailbox(name: &str, address: &str) -> MailboxAddress {
    MailboxAddress::new(name, address.parse().unwrap())
}

#[test]
fn mailboxes_quote_names_but_leave_unnamed_addresses_bare() {
    for (name, address, expected) in [
        ("", "hammer", "hammer"),
        ("", "hammer@example.com", "hammer@example.com"),
        (
            "Alice",
            "alice@example.com",
            "\"Alice\" <alice@example.com>",
        ),
        (
            " \tAlice Smith ",
            "alice@example.com",
            "\"Alice Smith\" <alice@example.com>",
        ),
        (
            "",
            "\"quoted local\"@example.com",
            "\"quoted local\"@example.com",
        ),
        (
            "Ren\u{e9}e",
            "caf\u{e9}@b\u{fc}cher.example",
            "\"Ren\u{e9}e\" <caf\u{e9}@b\u{fc}cher.example>",
        ),
        (
            "",
            "user@XN--BCHER-KVA.EXAMPLE",
            "user@XN--BCHER-KVA.EXAMPLE",
        ),
    ] {
        let mailbox = mailbox(name, address);
        assert_eq!(mailbox.to_string(), expected);
        assert_eq!(Address::from(mailbox).to_string(), expected);
    }
}

#[test]
fn names_escape_quotes_and_backslashes_without_decoding_again() {
    let name = "A \"quoted\" \\ name";
    let mailbox = mailbox(name, "a@b");
    let displayed = mailbox.to_string();
    assert_eq!(displayed, "\"A \\\"quoted\\\" \\\\ name\" <a@b>");
    let parsed: MailboxAddress = displayed.parse().unwrap();
    assert_eq!(parsed.display_name(), name);
    assert_eq!(parsed.address().as_str(), "a@b");
    assert_eq!(
        self::mailbox("=?utf-8?q?Name?=", "a@b").to_string(),
        "\"=?utf-8?q?Name?=\" <a@b>"
    );
}

#[test]
fn routes_force_angles_and_skip_only_blank_domains() {
    let mut mailbox = mailbox("Rusty McRouterson", "rusty@final-destination.com");
    *mailbox.route_mut() = ["comcast.net", "forward.com", "geek.net"]
        .into_iter()
        .collect();
    assert_eq!(
        mailbox.to_string(),
        "\"Rusty McRouterson\" <@comcast.net,@forward.com,@geek.net:rusty@final-destination.com>"
    );
    mailbox.set_display_name("");
    assert_eq!(
        mailbox.to_string(),
        "<@comcast.net,@forward.com,@geek.net:rusty@final-destination.com>"
    );
    *mailbox.route_mut() = ["", " \t", "\u{a0}"].into_iter().collect();
    assert_eq!(mailbox.to_string(), "rusty@final-destination.com");
    mailbox.set_display_name("Rusty");
    assert_eq!(
        mailbox.to_string(),
        "\"Rusty\" <rusty@final-destination.com>"
    );
    *mailbox.route_mut() = [
        "",
        "Relay.EXAMPLE",
        " ",
        "b\u{fc}cher.example",
        "Relay.EXAMPLE",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        mailbox.to_string(),
        "\"Rusty\" <@Relay.EXAMPLE,@b\u{fc}cher.example,@Relay.EXAMPLE:rusty@final-destination.com>"
    );
}

#[test]
fn groups_use_unquoted_names_and_keep_empty_members() {
    let empty = GroupAddress::new("Empty", AddressList::new());
    assert_eq!(empty.to_string(), "Empty: ;");
    let unnamed = GroupAddress::new("", AddressList::new());
    assert_eq!(unnamed.to_string(), ": ;");
    let group = GroupAddress::new("", vec![mailbox("", "jeff@example.com").into()].into());
    assert_eq!(group.to_string(), ": jeff@example.com;");
    assert_eq!(Address::from(group).to_string(), ": jeff@example.com;");

    let group = GroupAddress::new("Group \"with\" \\ punctuation: , ;", AddressList::new());
    assert_eq!(group.to_string(), "Group \"with\" \\ punctuation: , ;: ;");
}

#[test]
fn nested_groups_and_lists_preserve_boundaries_and_duplicate_order() {
    let inner = GroupAddress::new(
        "Inner",
        vec![
            mailbox("First Inner", "inner1@x").into(),
            mailbox("Second Inner", "inner2@x").into(),
        ]
        .into(),
    );
    let outer = GroupAddress::new(
        "Outer",
        vec![
            mailbox("", "first@x").into(),
            GroupAddress::new("Empty", AddressList::new()).into(),
            inner.into(),
            mailbox("", "first@x").into(),
        ]
        .into(),
    );
    assert_eq!(
        outer.to_string(),
        "Outer: first@x, Empty: ;, Inner: \"First Inner\" <inner1@x>, \"Second Inner\" <inner2@x>;, first@x;"
    );
    let list: AddressList = vec![
        mailbox("", "before@x").into(),
        outer.into(),
        mailbox("", "after@x").into(),
    ]
    .into();
    assert_eq!(
        list.to_string(),
        "before@x, Outer: first@x, Empty: ;, Inner: \"First Inner\" <inner1@x>, \"Second Inner\" <inner2@x>;, first@x;, after@x"
    );
    assert_eq!(AddressList::new().to_string(), "");
}

#[test]
fn display_preserves_controls_and_does_not_fold_or_validate() {
    let mut mailbox = mailbox("A\r\nB\t\0C", "user@example.com");
    mailbox
        .route_mut()
        .as_mut_vec()
        .push("bad domain\r\n".into());
    assert_eq!(
        mailbox.to_string(),
        "\"A\r\nB\t\0C\" <@bad domain\r\n:user@example.com>"
    );
    let group = GroupAddress::new("G\r\nH\0", vec![mailbox.into()].into());
    assert_eq!(
        group.to_string(),
        "G\r\nH\0: \"A\r\nB\t\0C\" <@bad domain\r\n:user@example.com>;"
    );
    let long = "x".repeat(1000);
    assert_eq!(
        self::mailbox(&long, "a@b").to_string(),
        format!("\"{long}\" <a@b>")
    );
}

#[test]
fn parsed_names_display_decoded_text_without_changing_diagnostics() {
    let list = AddressList::parse(b"=?unknown?q?Friends?=:Andr\xe9 <caf\xe9@x>;").unwrap();
    let Address::Group(group) = &list.as_slice()[0] else {
        panic!("expected group");
    };
    let issues = group.display_name_issues().to_vec();
    let member = group.members().mailboxes().next().unwrap();
    let name_issues = member.display_name_issues().to_vec();
    assert!(!issues.is_empty());
    assert!(!name_issues.is_empty());
    assert!(member.address().had_latin1_fallback());
    assert_eq!(list.to_string(), "Friends: \"Andr\u{e9}\" <caf\u{e9}@x>;");
    assert_eq!(group.display_name_issues(), issues);
    assert_eq!(member.display_name_issues(), name_issues);
    assert!(member.address().had_latin1_fallback());
}

#[test]
fn maximum_parsed_group_depth_formats_without_recursive_traversal() {
    let options = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let input = format!("{}a@b{}", "g:".repeat(255), ";".repeat(255));
    let expected = format!("{}a@b{}", "g: ".repeat(255), ";".repeat(255));
    let list = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    assert_eq!(list.to_string(), expected);
    assert_eq!(list.as_slice()[0].to_string(), expected);
    let Address::Group(group) = &list.as_slice()[0] else {
        panic!("expected group");
    };
    assert_eq!(group.to_string(), expected);
}

#[test]
fn formatter_write_errors_propagate_from_all_variants() {
    struct LimitedWriter(usize);
    impl Write for LimitedWriter {
        fn write_str(&mut self, text: &str) -> fmt::Result {
            if text.len() > self.0 {
                return Err(fmt::Error);
            }
            self.0 -= text.len();
            Ok(())
        }
    }
    let mailbox = mailbox("Alice", "a@b");
    let group = GroupAddress::new("Friends", vec![mailbox.clone().into()].into());
    let address = Address::from(group.clone());
    let list: AddressList = vec![mailbox.clone().into(), address.clone()].into();
    for value in [&mailbox as &dyn fmt::Display, &group, &address, &list] {
        let expected = value.to_string();
        for capacity in 0..expected.len() {
            assert!(write!(&mut LimitedWriter(capacity), "{value}").is_err());
        }
        assert!(write!(&mut LimitedWriter(expected.len()), "{value}").is_ok());
    }
}

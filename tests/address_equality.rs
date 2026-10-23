// Adapted from MimeKit's MailboxAddressTests.cs, GroupAddressTests.cs,
// and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fmt::Debug,
    hash::{Hash, Hasher},
};

use mimecraft::{Address, AddressList, AddressParseOptions, GroupAddress, MailboxAddress};

fn mailbox(name: &str, address: &str) -> MailboxAddress {
    MailboxAddress::new(name, address.parse().unwrap())
}

fn hash(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

fn assert_equivalent<T: Eq + Hash + Debug>(left: &T, right: &T) {
    assert_eq!(left, right);
    assert_eq!(right, left);
    assert_eq!(hash(left), hash(right));
}

#[test]
fn mailbox_equality_uses_exact_name_and_address_text() {
    let original = mailbox("User", "user@example.com");
    assert_equivalent(&original, &original.clone());
    assert_equivalent(&original, &mailbox(" User ", "user (comment) @example.com"));
    for other in [
        mailbox("user", "user@example.com"),
        mailbox("", "user@example.com"),
        mailbox("User", "USER@example.com"),
        mailbox("User", "user@EXAMPLE.COM"),
        mailbox("User", "user"),
        mailbox("User", "\"user\"@example.com"),
    ] {
        assert_ne!(original, other);
    }
    assert_ne!(mailbox("Ren\u{e9}", "a@b"), mailbox("Rene\u{301}", "a@b"));
}

#[test]
fn idna_equivalent_spellings_are_not_implicitly_equal() {
    let ascii = mailbox("User", "user@xn--bcher-kva.example");
    let unicode = mailbox("User", "user@b\u{fc}cher.example");
    assert_ne!(ascii, unicode);
    assert_eq!(
        ascii.address().with_ascii_domain().unwrap(),
        unicode.address().with_ascii_domain().unwrap()
    );
    assert_eq!(
        ascii.address().with_unicode_domain().unwrap(),
        unicode.address().with_unicode_domain().unwrap()
    );
}

#[test]
fn differing_routes_do_not_break_equality_or_hash_contract() {
    let plain = mailbox("User", "user@example.com");
    let mut routed = plain.clone();
    *routed.route_mut() = ["relay.example", "invalid domain"].into_iter().collect();
    assert_ne!(plain.to_string(), routed.to_string());
    assert_equivalent(&plain, &routed);
    let mut set = HashSet::new();
    assert!(set.insert(plain.clone()));
    assert!(!set.insert(routed.clone()));
    assert!(set.contains(&routed));
    let mut map = HashMap::new();
    map.insert(plain, 1);
    assert_eq!(map.insert(routed, 2), Some(1));
    assert_eq!(map.len(), 1);
}

#[test]
fn decoding_diagnostics_do_not_change_mailbox_identity() {
    let recovered = MailboxAddress::parse(b"Andr\xe9 <caf\xe9@x>").unwrap();
    let clean = mailbox("Andr\u{e9}", "caf\u{e9}@x");
    assert!(!recovered.display_name_issues().is_empty());
    assert!(recovered.address().had_latin1_fallback());
    assert!(clean.display_name_issues().is_empty());
    assert!(!clean.address().had_latin1_fallback());
    assert_equivalent(&recovered, &clean);
    let encoded = MailboxAddress::parse(b"=?unknown?q?Andr=E9?= <caf\xe9@x>").unwrap();
    assert_equivalent(&encoded, &clean);
    assert_equivalent(&recovered, &encoded);
    let mut cleared = recovered.clone();
    cleared.set_display_name("Andr\u{e9}");
    assert_equivalent(&recovered, &cleared);
    assert_eq!(HashSet::from([recovered, clean, encoded, cleared]).len(), 1);
}

#[test]
fn group_equality_ignores_diagnostics_but_keeps_names_and_members() {
    let recovered = GroupAddress::parse(b"Andr\xe9:Name <caf\xe9@x>;").unwrap();
    let members: AddressList = vec![mailbox("Name", "caf\u{e9}@x").into()].into();
    let clean = GroupAddress::new("Andr\u{e9}", members.clone());
    assert!(!recovered.display_name_issues().is_empty());
    assert_equivalent(&recovered, &clean);
    assert_ne!(clean, GroupAddress::new("ANDR\u{c9}", members));
    assert_ne!(clean, GroupAddress::new("Andr\u{e9}", AddressList::new()));
    let mut routed = clean.clone();
    let Address::Mailbox(member) = &mut routed.members_mut().as_mut_vec()[0] else {
        panic!("expected mailbox");
    };
    member.route_mut().as_mut_vec().push("relay".into());
    assert_equivalent(&clean, &routed);
}

#[test]
fn enum_variants_remain_distinct() {
    let mailbox = Address::from(mailbox("Same", "a@b"));
    let group = Address::from(GroupAddress::new("Same", AddressList::new()));
    assert_ne!(mailbox, group);
    assert_equivalent(&mailbox, &mailbox.clone());
    assert_equivalent(&group, &group.clone());
    assert_eq!(HashSet::from([mailbox, group]).len(), 2);
}

#[test]
fn canonical_ordered_lists_compare_equal() {
    let make_list = || {
        let members = ["phil", "joe", "alex", "bob"]
            .into_iter()
            .map(|address| mailbox("", address).into())
            .collect();
        AddressList::from(vec![
            GroupAddress::new("Local recipients", members).into(),
            mailbox("Joey", "joey@friends.com").into(),
            mailbox("Chandler", "chandler@friends.com").into(),
        ])
    };
    let first = make_list();
    let second = make_list();
    assert_equivalent(&first, &second);
    assert_ne!(first, AddressList::new());
    assert_equivalent(&AddressList::new(), &AddressList::default());
    assert_eq!(HashSet::from([first, second]).len(), 1);
}

#[test]
fn list_order_duplicates_and_group_boundaries_matter() {
    let parse = |input: &str| input.parse::<AddressList>().unwrap();
    let lists = [
        parse("a@b, c@d"),
        parse("c@d, a@b"),
        parse("a@b, c@d, a@b"),
        parse("G:a@b, c@d;"),
        parse("G:a@b;, c@d"),
        parse("a@b, Empty:;, c@d"),
        parse("a@b"),
        AddressList::new(),
        parse("Empty:;"),
    ];
    for (index, left) in lists.iter().enumerate() {
        for right in &lists[index + 1..] {
            assert_ne!(left, right);
        }
    }
    assert_eq!(HashSet::from(lists).len(), 9);
    assert_ne!(parse("G:a@b, c@d;"), parse("G:c@d, a@b;"));
}

#[test]
fn nested_names_and_contents_participate_in_equality() {
    let members: AddressList = vec![mailbox("", "a@b").into()].into();
    let inner = GroupAddress::new("Inner", members.clone());
    let outer = GroupAddress::new("Outer", vec![inner.clone().into()].into());
    assert_equivalent(&outer, &outer.clone());
    assert_ne!(outer, GroupAddress::new("Outer", members));
    let different = GroupAddress::new("Different", inner.members().clone());
    assert_ne!(
        outer,
        GroupAddress::new("Outer", vec![different.into()].into())
    );
}

#[test]
fn maximum_parsed_depth_equality_and_hash_include_deepest_member() {
    let options = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let input = format!("{}a@b{}", "g:".repeat(255), ";".repeat(255));
    let left = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    let mut right = left.clone();
    assert_equivalent(&left, &right);
    let mut address = &mut right.as_mut_vec()[0];
    loop {
        match address {
            Address::Mailbox(mailbox) => {
                mailbox.set_display_name("Changed");
                break;
            }
            Address::Group(group) => address = &mut group.members_mut().as_mut_vec()[0],
        }
    }
    assert_ne!(left, right);
}

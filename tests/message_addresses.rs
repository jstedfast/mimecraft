// Adapted from MimeKit's UnitTests/MimeMessageTests.cs and AddressParserTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    Address, AddressComplianceMode, AddressHeaderFormatError, AddressHeaderFormatOptions,
    AddressList, AddressParseErrorKind, AddressParseOptions, Header, HeaderList, MailboxAddress,
    MessageAddresses, MimeContent, MimeMessage, MimePart, TextPart, encodings::NewLineFormat,
};

const FIELDS: [&str; 10] = [
    "From",
    "Resent-From",
    "Reply-To",
    "Resent-Reply-To",
    "To",
    "Resent-To",
    "Cc",
    "Resent-Cc",
    "Bcc",
    "Resent-Bcc",
];

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn message(raw: &str) -> MimeMessage {
    MimeMessage::from_parts(headers(raw.as_bytes()), None)
}

fn snapshot<R>(message: &MimeMessage<R>) -> Vec<Vec<u8>> {
    message
        .headers()
        .iter()
        .map(|header| header.raw().to_vec())
        .collect()
}

fn get<R>(message: &MimeMessage<R>, field: &str) -> MessageAddresses {
    match field {
        "From" => message.from(),
        "Resent-From" => message.resent_from(),
        "Reply-To" => message.reply_to(),
        "Resent-Reply-To" => message.resent_reply_to(),
        "To" => message.to(),
        "Resent-To" => message.resent_to(),
        "Cc" => message.cc(),
        "Resent-Cc" => message.resent_cc(),
        "Bcc" => message.bcc(),
        "Resent-Bcc" => message.resent_bcc(),
        _ => unreachable!(),
    }
}

fn get_with_options<R>(
    message: &MimeMessage<R>,
    field: &str,
    options: &AddressParseOptions,
) -> MessageAddresses {
    match field {
        "From" => message.from_with_options(options),
        "Resent-From" => message.resent_from_with_options(options),
        "Reply-To" => message.reply_to_with_options(options),
        "Resent-Reply-To" => message.resent_reply_to_with_options(options),
        "To" => message.to_with_options(options),
        "Resent-To" => message.resent_to_with_options(options),
        "Cc" => message.cc_with_options(options),
        "Resent-Cc" => message.resent_cc_with_options(options),
        "Bcc" => message.bcc_with_options(options),
        "Resent-Bcc" => message.resent_bcc_with_options(options),
        _ => unreachable!(),
    }
}

fn set<R>(
    message: &mut MimeMessage<R>,
    field: &str,
    value: &AddressList,
) -> Result<(), AddressHeaderFormatError> {
    match field {
        "From" => message.set_from(value),
        "Resent-From" => message.set_resent_from(value),
        "Reply-To" => message.set_reply_to(value),
        "Resent-Reply-To" => message.set_resent_reply_to(value),
        "To" => message.set_to(value),
        "Resent-To" => message.set_resent_to(value),
        "Cc" => message.set_cc(value),
        "Resent-Cc" => message.set_resent_cc(value),
        "Bcc" => message.set_bcc(value),
        "Resent-Bcc" => message.set_resent_bcc(value),
        _ => unreachable!(),
    }
}

fn set_with_options<R>(
    message: &mut MimeMessage<R>,
    field: &str,
    value: &AddressList,
    options: &AddressHeaderFormatOptions,
) -> Result<(), AddressHeaderFormatError> {
    match field {
        "From" => message.set_from_with_options(value, options),
        "Resent-From" => message.set_resent_from_with_options(value, options),
        "Reply-To" => message.set_reply_to_with_options(value, options),
        "Resent-Reply-To" => message.set_resent_reply_to_with_options(value, options),
        "To" => message.set_to_with_options(value, options),
        "Resent-To" => message.set_resent_to_with_options(value, options),
        "Cc" => message.set_cc_with_options(value, options),
        "Resent-Cc" => message.set_resent_cc_with_options(value, options),
        "Bcc" => message.set_bcc_with_options(value, options),
        "Resent-Bcc" => message.set_resent_bcc_with_options(value, options),
        _ => unreachable!(),
    }
}

fn mailboxes(list: &AddressList) -> Vec<&str> {
    list.mailboxes()
        .map(|mailbox| mailbox.address().as_str())
        .collect()
}

fn strict_options() -> AddressParseOptions {
    let mut options = AddressParseOptions::default();
    options.mailbox.address.compliance_mode = AddressComplianceMode::Strict;
    options.mailbox.address.allow_local_only = false;
    options
}

#[test]
fn missing_and_comment_only_fields_are_empty_without_stream_traits() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    for field in FIELDS {
        let parsed = get(&message, field);
        assert!(parsed.addresses.is_empty());
        assert!(parsed.issues.is_empty());
        for value in [
            "",
            " \t",
            "(comment)",
            "(nested (comment))\r\n\t(again)",
            "(escaped \\))",
        ] {
            let header = Header::parse(format!("{field}: {value}\r\n").as_bytes()).unwrap();
            message.headers_mut().replace_all(header).unwrap();
            let before = snapshot(&message);
            let parsed = get(&message, field);
            assert!(parsed.addresses.is_empty(), "{field}: {value}");
            assert!(parsed.issues.is_empty(), "{field}: {value}");
            assert_eq!(snapshot(&message), before);
        }
    }
    // Message-level empty fields do not relax the standalone author-facing parser.
    assert!(AddressList::parse(b" (comment)\r\n").is_err());
}

#[test]
fn getters_aggregate_occurrences_preserving_groups_order_and_duplicates() {
    for field in FIELDS {
        let message = message(&format!(
            "{}: first@example.com\r\nX-Other: untouched\r\n{field}: Empty:;, Friends: second@example.com;\r\n{field}: first@example.com\r\n",
            field.to_ascii_lowercase()
        ));
        let before = snapshot(&message);
        let parsed = get(&message, field);
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.addresses.len(), 4);
        assert_eq!(
            mailboxes(&parsed.addresses),
            [
                "first@example.com",
                "second@example.com",
                "first@example.com"
            ]
        );
        assert!(
            matches!(&parsed.addresses.as_slice()[1], Address::Group(group) if group.members().is_empty())
        );
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn internal_recovery_matches_mimekit_quoted_name_fixture() {
    let options = strict_options();
    let value = "first@example.com, \"Last, First\" (no address), last@example.com";
    assert!(AddressList::parse_with_options(value.as_bytes(), &options).is_err());
    for field in FIELDS {
        let message = message(&format!("{field}: {value}\r\n"));
        let parsed = get_with_options(&message, field, &options);
        assert_eq!(
            mailboxes(&parsed.addresses),
            ["first@example.com", "last@example.com"]
        );
        assert_eq!(parsed.issues.len(), 1);
        assert_eq!(parsed.issues[0].header_index, 0);
    }
}

#[test]
fn malformed_entries_do_not_discard_earlier_or_later_addresses() {
    for field in FIELDS {
        let message = message(&format!(
            "X-Other: one\r\n{field}: @, first@example.com, @, last@example.com\r\nX-Other: two\r\n{field}: @\r\n{field}: later@example.com\r\n"
        ));
        let before = snapshot(&message);
        let parsed = get(&message, field);
        assert_eq!(
            mailboxes(&parsed.addresses),
            ["first@example.com", "last@example.com", "later@example.com"]
        );
        assert_eq!(
            parsed
                .issues
                .iter()
                .map(|issue| issue.header_index)
                .collect::<Vec<_>>(),
            [1, 1, 3]
        );
        for issue in &parsed.issues {
            assert!(
                issue.error.offset()
                    <= message
                        .headers()
                        .iter()
                        .nth(issue.header_index)
                        .unwrap()
                        .raw_value()
                        .len()
            );
        }
        let invalid = message.headers().get_all(field).nth(1).unwrap();
        assert_eq!(
            parsed.issues[2].error,
            AddressList::parse(invalid.raw_value()).unwrap_err()
        );
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn resynchronization_does_not_promote_addresses_inside_quotes_or_comments() {
    for rejected in [
        "@ \"hidden@example.com, hidden2@example.com\"",
        "@ (hidden@example.com, (hidden2@example.com))",
        "@ \"escaped\\\", hidden@example.com\"",
        "@ (escaped\\), hidden@example.com)",
    ] {
        let message = message(&format!(
            "To: first@example.com, {rejected}, last@example.com\r\n"
        ));
        let parsed = message.to();
        assert_eq!(
            mailboxes(&parsed.addresses),
            ["first@example.com", "last@example.com"],
            "{rejected}"
        );
        assert_eq!(parsed.issues.len(), 1);
    }
    for rejected in [
        "@ \"hidden@example.com, last@example.com",
        "@ (hidden@example.com, last@example.com",
    ] {
        let message = message(&format!("To: first@example.com, {rejected}\r\n"));
        let parsed = message.to();
        assert_eq!(mailboxes(&parsed.addresses), ["first@example.com"]);
        assert_eq!(parsed.issues.len(), 1);
    }
}

#[test]
fn internal_group_recovery_leaves_the_terminator_for_the_group() {
    let options = strict_options();
    let message = message("To: Friends: \"bad, name\" (no address);, next@example.com\r\n");
    let parsed = message.to_with_options(&options);
    assert_eq!(parsed.addresses.len(), 2);
    assert!(
        matches!(&parsed.addresses.as_slice()[0], Address::Group(group) if group.members().is_empty())
    );
    assert_eq!(mailboxes(&parsed.addresses), ["next@example.com"]);
    assert_eq!(parsed.issues.len(), 1);
}

#[test]
fn groups_recover_members_and_nested_groups_with_bounded_depth() {
    let options = AddressParseOptions {
        max_group_depth: 2,
        ..Default::default()
    };
    let message = message(
        "To: Outer: first@example.com, @, Inner: @, second@example.com;, last@example.com;, tail@example.com\r\n",
    );
    let parsed = message.to_with_options(&options);
    assert_eq!(
        mailboxes(&parsed.addresses),
        [
            "first@example.com",
            "second@example.com",
            "last@example.com",
            "tail@example.com"
        ]
    );
    assert_eq!(parsed.issues.len(), 2);
    let Address::Group(outer) = &parsed.addresses.as_slice()[0] else {
        panic!("outer group")
    };
    assert_eq!(outer.members().len(), 3);
    assert!(
        matches!(&outer.members().as_slice()[1], Address::Group(group) if group.display_name() == "Inner")
    );

    let disabled = AddressParseOptions {
        max_group_depth: 0,
        ..Default::default()
    };
    let parsed = message.to_with_options(&disabled);
    assert!(
        parsed
            .issues
            .iter()
            .any(|issue| issue.error.kind() == AddressParseErrorKind::GroupDepthExceeded)
    );
    assert!(
        parsed
            .addresses
            .as_slice()
            .iter()
            .all(|address| matches!(address, Address::Mailbox(_)))
    );
}

#[test]
fn unterminated_groups_survive_internal_parsing_even_in_strict_mode() {
    for value in [
        "Group:",
        "Group: (comment)",
        "Group: first@example.com",
        "Group: @",
    ] {
        let message = message(&format!("To: {value}\r\n"));
        let parsed = message.to_with_options(&strict_options());
        assert_eq!(parsed.addresses.len(), 1, "{value}");
        assert!(
            matches!(&parsed.addresses.as_slice()[0], Address::Group(group) if group.display_name() == "Group")
        );
        assert!(
            parsed
                .issues
                .iter()
                .any(|issue| issue.error.kind() == AddressParseErrorKind::MissingGroupTerminator)
        );
    }
}

#[test]
fn failed_list_productions_follow_mimekit_discard_rules() {
    let options = strict_options();
    let message = message("To: first@example.com second@example.com\r\nTo: later@example.com\r\n");
    let parsed = message.to_with_options(&options);
    assert_eq!(mailboxes(&parsed.addresses), ["later@example.com"]);
    assert_eq!(
        parsed.issues[0].error.kind(),
        AddressParseErrorKind::MissingSeparator
    );

    let message = self::message("To: first@example.com, (unclosed\r\nTo: later@example.com\r\n");
    let parsed = message.to();
    assert_eq!(mailboxes(&parsed.addresses), ["later@example.com"]);
    assert_eq!(parsed.issues.len(), 1);
    assert_eq!(parsed.issues[0].header_index, 0);

    let message =
        self::message("To: Group: first@example.com, (unclosed\r\nTo: later@example.com\r\n");
    let parsed = message.to();
    assert_eq!(mailboxes(&parsed.addresses), ["later@example.com"]);
    assert!(
        matches!(&parsed.addresses.as_slice()[0], Address::Group(group) if group.members().is_empty())
    );
    assert_eq!(parsed.issues.len(), 2);
}

#[test]
fn options_and_decoding_diagnostics_reach_each_header_and_group() {
    for field in FIELDS {
        let message = message(&format!(
            "{field}: local\r\n{field}: =?utf-8?q?Ren=C3=A9e?=: =?utf-8?b?/w==?= <@relay.example:agent@example.com>;\r\n"
        ));
        let loose = get(&message, field);
        assert_eq!(mailboxes(&loose.addresses), ["local", "agent@example.com"]);
        let parsed = get_with_options(&message, field, &strict_options());
        assert_eq!(parsed.issues.len(), 1);
        let Address::Group(group) = &parsed.addresses.as_slice()[0] else {
            panic!("group")
        };
        assert_eq!(group.display_name(), "Ren\u{e9}e");
        let mailbox = group.members().mailboxes().next().unwrap();
        assert!(!mailbox.display_name_issues().is_empty());
        assert_eq!(mailbox.route().as_slice(), &["relay.example"]);
    }
}

#[test]
fn many_rejected_entries_and_deep_groups_terminate_with_bounded_nesting() {
    let message = message(&format!("To: {}last@example.com\r\n", "@,".repeat(2000)));
    let parsed = message.to();
    assert_eq!(parsed.issues.len(), 2000);
    assert_eq!(mailboxes(&parsed.addresses), ["last@example.com"]);
    let options = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let message = self::message(&format!(
        "To: {}a@example.com{}\r\n",
        "G:".repeat(255),
        ";".repeat(255)
    ));
    let parsed = message.to_with_options(&options);
    assert!(parsed.issues.is_empty());
    assert_eq!(mailboxes(&parsed.addresses), ["a@example.com"]);
    let message = self::message(&format!(
        "To: {}a@example.com{}\r\n",
        "G:".repeat(256),
        ";".repeat(256)
    ));
    assert!(
        message
            .to_with_options(&options)
            .issues
            .iter()
            .any(|issue| issue.error.kind() == AddressParseErrorKind::GroupDepthExceeded)
    );
}

#[test]
fn edits_are_detached_until_assignment_and_raw_changes_are_visible() {
    for field in FIELDS {
        let mut message = message(&format!(
            "{field}: first@example.com\r\n{field}: second@example.com\r\n"
        ));
        let mut parsed = get(&message, field);
        parsed
            .addresses
            .as_mut_vec()
            .push("third@example.com".parse().unwrap());
        assert_eq!(get(&message, field).addresses.len(), 2);
        set(&mut message, field, &parsed.addresses).unwrap();
        assert_eq!(get(&message, field).addresses, parsed.addresses);
        assert_eq!(message.headers().get_all(field).count(), 1);
        message
            .headers_mut()
            .replace_all(Header::parse(format!("{field}: raw@example.com").as_bytes()).unwrap())
            .unwrap();
        assert_eq!(
            mailboxes(&get(&message, field).addresses),
            ["raw@example.com"]
        );
        message.headers_mut().clear();
        assert!(get(&message, field).addresses.is_empty());
    }
}

#[test]
fn setters_replace_at_first_slot_preserve_unrelated_fields_and_empty_groups() {
    let value: AddressList = "Empty:;, first@example.com, first@example.com"
        .parse()
        .unwrap();
    for field in FIELDS {
        let mut message = message(&format!(
            "X-Before: one\n{}: @\nX-Middle: two\n{field}: other@example.com\n",
            field.to_ascii_lowercase()
        ));
        set(&mut message, field, &value).unwrap();
        assert_eq!(
            snapshot(&message),
            [
                b"X-Before: one\n".to_vec(),
                value.to_header(field).unwrap().raw().to_vec(),
                b"X-Middle: two\n".to_vec(),
            ]
        );
        assert_eq!(get(&message, field).addresses, value);
    }
}

#[test]
fn empty_assignment_removes_every_occurrence_and_ignores_unused_options() {
    let options = AddressHeaderFormatOptions {
        max_line_length: 0,
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    for field in FIELDS {
        let mut message = message(&format!(
            "X-Other: keep\r\n{field}: @\r\n{field}: (unclosed\r\n"
        ));
        set_with_options(&mut message, field, &AddressList::new(), &options).unwrap();
        assert_eq!(snapshot(&message), [b"X-Other: keep\r\n".to_vec()]);
        assert!(get(&message, field).issues.is_empty());
        set(&mut message, field, &AddressList::new()).unwrap();
        assert_eq!(snapshot(&message), [b"X-Other: keep\r\n".to_vec()]);
    }
}

#[test]
fn equal_lists_reformat_to_preserve_nested_route_edits() {
    let first: AddressList = "Group: Agent <@first.example:agent@example.com>;"
        .parse()
        .unwrap();
    let second: AddressList = "Group: Agent <@second.example:agent@example.com>;"
        .parse()
        .unwrap();
    assert_eq!(first, second);
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set(&mut message, field, &first).unwrap();
        set(&mut message, field, &second).unwrap();
        assert_eq!(
            get(&message, field)
                .addresses
                .mailboxes()
                .next()
                .unwrap()
                .route()
                .as_slice(),
            &["second.example"]
        );
    }
}

#[test]
fn formatting_later_invalid_members_or_options_is_atomic() {
    let valid: AddressList = "first@example.com".parse().unwrap();
    let mut invalid = valid.clone();
    invalid
        .as_mut_vec()
        .push(MailboxAddress::new("", "caf\u{e9}@example.com".parse().unwrap()).into());
    for field in FIELDS {
        let mut message = message(&format!(
            "{field}: old@example.com\r\n{field}: duplicate@example.com\r\n"
        ));
        let before = snapshot(&message);
        assert!(matches!(
            set(&mut message, field, &invalid),
            Err(AddressHeaderFormatError::Address(_))
        ));
        assert_eq!(snapshot(&message), before);
        let options = AddressHeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        };
        assert!(set_with_options(&mut message, field, &valid, &options).is_err());
        assert_eq!(snapshot(&message), before);
        let long: AddressList = format!("{}@example.com", "a".repeat(100)).parse().unwrap();
        assert!(matches!(
            set(&mut message, field, &long),
            Err(AddressHeaderFormatError::LineTooLong { .. })
        ));
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn checked_formatting_encodes_names_domains_and_injection_with_exact_limits() {
    let mailbox = MailboxAddress::new(
        &format!("{}\r\nX-Injected: value", "Ren\u{e9}e ".repeat(30)),
        "agent@b\u{fc}cher.example".parse().unwrap(),
    );
    let value = AddressList::from(vec![mailbox.clone().into()]);
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set_with_options(&mut message, field, &value, &options).unwrap();
        let raw = message.headers().get(field).unwrap().raw();
        assert!(raw.is_ascii());
        assert!(!raw.contains(&b'\r'));
        let lines: Vec<_> = std::str::from_utf8(raw).unwrap().lines().collect();
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 60));
        assert!(lines[1..].iter().all(|line| line.starts_with('\t')));
        assert_eq!(headers(raw).len(), 1);
        let parsed = get(&message, field);
        let actual = parsed.addresses.mailboxes().next().unwrap();
        assert_eq!(actual.display_name(), mailbox.display_name());
        assert_eq!(actual.address().as_str(), "agent@xn--bcher-kva.example");
    }
    assert_eq!(
        value.mailboxes().next().unwrap().address().as_str(),
        "agent@b\u{fc}cher.example"
    );
}

#[test]
fn all_fields_are_independent_and_bcc_is_not_hidden_when_writing() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    for (index, field) in FIELDS.iter().enumerate() {
        set(
            &mut message,
            field,
            &format!("user{index}@example.com").parse().unwrap(),
        )
        .unwrap();
    }
    assert!(!message.headers().contains("Sender"));
    assert!(!message.headers().contains("Resent-Sender"));
    let before = snapshot(&message);
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(snapshot(&loaded), before);
    assert!(loaded.headers().contains("Bcc"));
    assert!(loaded.headers().contains("Resent-Bcc"));
    for (index, field) in FIELDS.iter().enumerate() {
        assert_eq!(
            mailboxes(&get(&loaded, field).addresses),
            [format!("user{index}@example.com")]
        );
        set(&mut message, field, &AddressList::new()).unwrap();
    }
    assert!(message.headers().is_empty());
}

#[test]
fn reply_and_resent_fields_do_not_infer_other_addresses() {
    let message = message("From: author@example.com\r\nTo: recipient@example.com\r\n");
    for field in FIELDS
        .into_iter()
        .filter(|field| !matches!(*field, "From" | "To"))
    {
        assert!(get(&message, field).addresses.is_empty());
    }
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("address metadata must not read body sources")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("address metadata must not seek body sources")
    }
}

#[test]
fn address_metadata_does_not_inspect_body_headers_or_sources() {
    let raw = FIELDS
        .iter()
        .map(|field| format!("{field}: body@example.com\r\n"))
        .collect::<String>();
    let part = MimePart::from_parts(headers(raw.as_bytes()), MimeContent::new(NoIo));
    let mut message = MimeMessage::with_body(part.into());
    let before = message.body().unwrap().headers().clone();
    for field in FIELDS {
        assert!(get(&message, field).addresses.is_empty());
        set(&mut message, field, &"message@example.com".parse().unwrap()).unwrap();
        assert_eq!(
            mailboxes(&get(&message, field).addresses),
            ["message@example.com"]
        );
        set(&mut message, field, &AddressList::new()).unwrap();
    }
    assert_eq!(
        message.body().unwrap().headers().iter().collect::<Vec<_>>(),
        before.iter().collect::<Vec<_>>()
    );
}

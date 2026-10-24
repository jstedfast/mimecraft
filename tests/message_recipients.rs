// Adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    AddressComplianceMode, AddressParseOptions, Header, HeaderList, MessageRecipients, MimeContent,
    MimeMessage, MimePart, TextPart,
};

fn message(raw: &str) -> MimeMessage {
    MimeMessage::from_parts(HeaderList::load(&mut raw.as_bytes()).unwrap(), None)
}

fn addresses(result: &MessageRecipients) -> Vec<&str> {
    result
        .mailboxes
        .iter()
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
fn upstream_recipient_order_excludes_senders_and_preserves_duplicates() {
    let mut message = message(concat!(
        "Sender: Sender <sender@example.com>\r\n",
        "From: Author <author@example.com>\r\n",
        "Reply-To: reply@example.com\r\n",
        "To: First <to@example.com>, Duplicate <to@example.com>\r\n",
        "Cc: First <cc@example.com>, Duplicate <cc@example.com>\r\n",
        "Bcc: First <bcc@example.com>, Duplicate <bcc@example.com>\r\n",
    ));
    let parsed = message.recipients().unwrap();
    assert!(!parsed.uses_resent_headers);
    assert!(parsed.issues.is_empty());
    assert_eq!(
        addresses(&parsed),
        [
            "to@example.com",
            "to@example.com",
            "cc@example.com",
            "cc@example.com",
            "bcc@example.com",
            "bcc@example.com",
        ]
    );
    assert_eq!(parsed.mailboxes[0].display_name(), "First");
    assert_eq!(parsed.mailboxes[1].display_name(), "Duplicate");

    // Now test the same ordering after setting the Resent-* headers.
    for raw in [
        "Resent-Sender: sender@example.com",
        "Resent-From: author@example.com",
        "Resent-Reply-To: reply@example.com",
        "Resent-To: First <resent-to@example.com>, Duplicate <resent-to@example.com>",
        "Resent-Cc: First <resent-cc@example.com>, Duplicate <resent-cc@example.com>",
        "Resent-Bcc: First <resent-bcc@example.com>, Duplicate <resent-bcc@example.com>",
    ] {
        message
            .headers_mut()
            .add(Header::parse(raw.as_bytes()).unwrap());
    }
    let parsed = message.recipients().unwrap();
    assert!(parsed.uses_resent_headers);
    assert!(parsed.issues.is_empty());
    assert_eq!(
        addresses(&parsed),
        [
            "resent-to@example.com",
            "resent-to@example.com",
            "resent-cc@example.com",
            "resent-cc@example.com",
            "resent-bcc@example.com",
            "resent-bcc@example.com",
        ]
    );
}

#[test]
fn interleaved_headers_are_grouped_by_field_and_groups_flatten_in_member_order() {
    let options = AddressParseOptions {
        max_group_depth: 2,
        ..Default::default()
    };
    let message = message(concat!(
        "Bcc: blind@example.com\r\n",
        "to: first@example.com\r\n",
        "Cc: copy@example.com\r\n",
        "To: Empty:;, Outer: second@example.com, Inner: third@example.com;;\r\n",
        "CC: copy-again@example.com\r\n",
        "To: first@example.com\r\n",
    ));
    let parsed = message.recipients_with_options(&options).unwrap();
    assert_eq!(
        addresses(&parsed),
        [
            "first@example.com",
            "second@example.com",
            "third@example.com",
            "first@example.com",
            "copy@example.com",
            "copy-again@example.com",
            "blind@example.com",
        ]
    );
    assert!(parsed.issues.is_empty());
}

#[test]
fn resent_selection_uses_recovered_address_count_not_header_or_mailbox_count() {
    for (from, resent, issue_count) in [
        (None, false, 0),
        (Some(""), false, 0),
        (Some("(comment)"), false, 0),
        (Some("@"), false, 1),
        (Some("Empty:;"), true, 0),
        (Some("local"), true, 0),
        (Some("@, author@example.com"), true, 1),
    ] {
        let mut message = message(concat!(
            "To: original@example.com\r\n",
            "Resent-To: resent@example.com\r\n",
        ));
        if let Some(from) = from {
            message
                .headers_mut()
                .add(Header::parse(format!("Resent-From: {from}\r\n").as_bytes()).unwrap());
        }
        let parsed = message.recipients().unwrap();
        assert_eq!(parsed.uses_resent_headers, resent, "{from:?}");
        assert_eq!(parsed.issues.len(), issue_count, "{from:?}");
        assert_eq!(
            addresses(&parsed),
            if resent {
                vec!["resent@example.com"]
            } else {
                vec!["original@example.com"]
            }
        );
    }
}

#[test]
fn empty_resent_recipients_do_not_fall_back_to_original_recipients() {
    for selector in ["Resent-Sender: sender@example.com", "Resent-From: Empty:;"] {
        let message = message(&format!(
            "{selector}\r\nTo: original@example.com\r\nResent-To: (empty)\r\nResent-Cc: Empty:;\r\n"
        ));
        let parsed = message.recipients().unwrap();
        assert!(parsed.uses_resent_headers);
        assert!(parsed.mailboxes.is_empty());
        assert!(parsed.issues.is_empty());
    }
}

#[test]
fn invalid_first_resent_sender_is_an_error_even_with_valid_later_selectors() {
    for invalid in ["", "(empty)", "@", "Group: sender@example.com;"] {
        let message = message(&format!(
            "Resent-Sender: {invalid}\r\nResent-Sender: valid@example.com\r\nResent-From: author@example.com\r\nTo: to@example.com\r\n"
        ));
        let before = message.headers().clone();
        assert_eq!(
            message.recipients().unwrap_err(),
            message.resent_sender().unwrap_err()
        );
        assert_eq!(message.headers(), &before);
    }
}

#[test]
fn valid_resent_sender_short_circuits_from_and_unselected_fields() {
    let message = message(concat!(
        "Resent-Sender: sender@example.com, @\r\n",
        "Resent-From: @\r\n",
        "From: @\r\n",
        "Sender: @\r\n",
        "Reply-To: @\r\n",
        "To: @\r\n",
        "Cc: @\r\n",
        "Bcc: @\r\n",
        "Resent-Reply-To: @\r\n",
        "Resent-To: selected@example.com\r\n",
    ));
    let parsed = message.recipients().unwrap();
    assert!(parsed.uses_resent_headers);
    assert_eq!(addresses(&parsed), ["selected@example.com"]);
    assert!(parsed.issues.is_empty());
    assert!(!message.resent_from().issues.is_empty());

    let message = self::message(concat!(
        "From: @\r\nSender: @\r\nReply-To: @\r\n",
        "Resent-To: @\r\nResent-Cc: @\r\nResent-Bcc: @\r\n",
        "To: selected@example.com\r\n",
    ));
    let parsed = message.recipients().unwrap();
    assert!(!parsed.uses_resent_headers);
    assert_eq!(addresses(&parsed), ["selected@example.com"]);
    assert!(parsed.issues.is_empty());
}

#[test]
fn explicit_options_apply_to_selector_mailboxes_and_all_selected_fields() {
    let mut message = message(concat!(
        "Resent-From: local\r\n",
        "Resent-To: forwarded@example.com\r\n",
        "To: local, valid@example.com\r\nCc: local\r\nBcc: local\r\n",
    ));
    assert!(message.recipients().unwrap().uses_resent_headers);
    let parsed = message.recipients_with_options(&strict_options()).unwrap();
    assert!(!parsed.uses_resent_headers);
    assert_eq!(addresses(&parsed), ["valid@example.com"]);
    assert_eq!(parsed.issues.len(), 4);

    message
        .headers_mut()
        .add(Header::parse(b"Resent-Sender: local\r\n").unwrap());
    assert!(message.recipients().unwrap().uses_resent_headers);
    assert_eq!(
        message
            .recipients_with_options(&strict_options())
            .unwrap_err(),
        message
            .resent_sender_with_options(&strict_options().mailbox)
            .unwrap_err()
    );
}

#[test]
fn diagnostics_keep_original_indices_and_offsets_in_evaluation_order() {
    let message = message(concat!(
        "Bcc: @, blind@example.com\r\n",
        "To: @, first@example.com, @\r\n",
        "X-Other: untouched\r\n",
        "Cc: @, copy@example.com\r\n",
        "Resent-From: @\r\n",
    ));
    let before = message.headers().clone();
    let parsed = message.recipients().unwrap();
    let expected_issues = [
        message.resent_from(),
        message.to(),
        message.cc(),
        message.bcc(),
    ]
    .into_iter()
    .flat_map(|parsed| parsed.issues)
    .collect::<Vec<_>>();
    assert_eq!(parsed.issues, expected_issues);
    assert_eq!(
        parsed
            .issues
            .iter()
            .map(|issue| issue.header_index)
            .collect::<Vec<_>>(),
        [4, 1, 1, 3, 0]
    );
    for issue in &parsed.issues {
        let raw = message
            .headers()
            .iter()
            .nth(issue.header_index)
            .unwrap()
            .raw_value();
        assert_eq!(raw[issue.error.offset()], b'@');
    }
    assert_eq!(
        addresses(&parsed),
        ["first@example.com", "copy@example.com", "blind@example.com"]
    );
    assert_eq!(message.headers(), &before);
}

#[test]
fn all_resent_blocks_are_aggregated_instead_of_selecting_the_latest() {
    let message = message(concat!(
        "Resent-From: @\r\nResent-To: first@example.com\r\nResent-Cc: copy@example.com\r\n",
        "Resent-From: author@example.com\r\nResent-To: second@example.com\r\n",
        "Resent-Bcc: blind@example.com\r\n",
    ));
    let parsed = message.recipients().unwrap();
    assert!(parsed.uses_resent_headers);
    assert_eq!(parsed.issues.len(), 1);
    assert_eq!(parsed.issues[0].header_index, 0);
    assert_eq!(
        addresses(&parsed),
        [
            "first@example.com",
            "second@example.com",
            "copy@example.com",
            "blind@example.com"
        ]
    );
}

#[test]
fn mailbox_spelling_routes_and_decode_diagnostics_survive_flattening() {
    let message = message(concat!(
        "To: Group: =?utf-8?b?/w==?= <@relay.example:User@Example.COM>;\r\n",
        "Cc: Duplicate <user@example.com>, \u{e9}@b\u{fc}cher.example\r\n",
        "Bcc: Duplicate <User@Example.COM>\r\n",
    ));
    let parsed = message.recipients().unwrap();
    assert_eq!(
        addresses(&parsed),
        [
            "User@Example.COM",
            "user@example.com",
            "\u{e9}@b\u{fc}cher.example",
            "User@Example.COM",
        ]
    );
    assert_eq!(parsed.mailboxes[0].route().as_slice(), ["relay.example"]);
    assert!(!parsed.mailboxes[0].display_name_issues().is_empty());
    assert!(parsed.issues.is_empty());
    assert_eq!(
        parsed.mailboxes[0],
        message.to().addresses.mailboxes().next().unwrap().clone()
    );
}

#[test]
fn returned_mailboxes_are_detached_and_later_raw_edits_change_selection() {
    let mut message = message("To: Original <first@example.com>\r\n");
    let before = message.headers().clone();
    let mut parsed = message.recipients().unwrap();
    parsed.mailboxes[0].set_display_name("Edited");
    parsed.mailboxes.clear();
    assert_eq!(message.headers(), &before);
    assert_eq!(
        message.recipients().unwrap().mailboxes[0].display_name(),
        "Original"
    );
    message
        .headers_mut()
        .add(Header::parse(b"Resent-From: author@example.com\r\n").unwrap());
    message
        .headers_mut()
        .add(Header::parse(b"Resent-To: later@example.com\r\n").unwrap());
    assert_eq!(
        addresses(&message.recipients().unwrap()),
        ["later@example.com"]
    );
    message.headers_mut().remove_all("Resent-From");
    assert_eq!(
        addresses(&message.recipients().unwrap()),
        ["first@example.com"]
    );
}

struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("recipient collection must not read the body");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("recipient collection must not seek the body");
    }
}

#[test]
fn collection_needs_no_stream_traits_and_ignores_body_headers_and_sources() {
    let empty: MimeMessage<()> = MimeMessage::new();
    let parsed = empty.recipients().unwrap();
    assert!(parsed.mailboxes.is_empty());
    assert!(parsed.issues.is_empty());
    assert!(!parsed.uses_resent_headers);

    let mut part = MimePart::new(MimeContent::new(NoIo));
    part.headers_mut()
        .add(Header::parse(b"Resent-Sender: @\r\n").unwrap());
    part.headers_mut()
        .add(Header::parse(b"To: body@example.com\r\n").unwrap());
    let mut message = MimeMessage::with_body(part.into());
    message
        .set_to(&"message@example.com".parse().unwrap())
        .unwrap();
    let parsed = message.recipients().unwrap();
    assert_eq!(addresses(&parsed), ["message@example.com"]);
    assert!(parsed.issues.is_empty());
}

#[test]
fn borrowed_sources_stay_positioned_and_bcc_is_not_removed_during_serialization() {
    let mut source = Cursor::new(b"body".to_vec());
    source.set_position(2);
    let mut part = MimePart::new(MimeContent::new(&mut source));
    part.headers_mut()
        .add(Header::parse(b"Content-Type: text/plain\r\n").unwrap());
    let mut message = MimeMessage::with_body(part.into());
    message
        .set_bcc(&"blind@example.com".parse().unwrap())
        .unwrap();
    let before = message.headers().clone();
    assert_eq!(
        addresses(&message.recipients().unwrap()),
        ["blind@example.com"]
    );
    assert_eq!(message.headers(), &before);
    drop(message);
    assert_eq!(source.position(), 2);

    let mut message = MimeMessage::with_body(TextPart::new("plain", "body").unwrap().into());
    message
        .set_bcc(&"blind@example.com".parse().unwrap())
        .unwrap();
    let before = message.headers().clone();
    assert_eq!(message.recipients().unwrap().mailboxes.len(), 1);
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(message.headers(), &before);
    let loaded = MimeMessage::load(&mut output.as_slice()).unwrap();
    assert!(loaded.headers().contains("Bcc"));
    assert_eq!(
        addresses(&loaded.recipients().unwrap()),
        ["blind@example.com"]
    );
}

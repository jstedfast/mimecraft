// Adapted from MimeKit's UnitTests/MimeMessageTests.cs and MailboxAddressTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    AddrSpecParseOptions, AddressHeaderFormatError, AddressHeaderFormatOptions, Header, HeaderList,
    MailboxAddress, MailboxParseError, MailboxParseOptions, MimeContent, MimeMessage, MimePart,
    TextPart, encodings::NewLineFormat,
};

const FIELDS: [&str; 2] = ["Sender", "Resent-Sender"];

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn snapshot<R>(message: &MimeMessage<R>) -> Vec<Vec<u8>> {
    message
        .headers()
        .iter()
        .map(|header| header.raw().to_vec())
        .collect()
}

fn mailbox() -> MailboxAddress {
    MailboxAddress::parse(b"Agent <agent@example.com>").unwrap()
}

fn get<R>(
    message: &MimeMessage<R>,
    field: &str,
) -> Result<Option<MailboxAddress>, MailboxParseError> {
    match field {
        "Sender" => message.sender(),
        "Resent-Sender" => message.resent_sender(),
        _ => unreachable!(),
    }
}

fn get_with_options<R>(
    message: &MimeMessage<R>,
    field: &str,
    options: &MailboxParseOptions,
) -> Result<Option<MailboxAddress>, MailboxParseError> {
    match field {
        "Sender" => message.sender_with_options(options),
        "Resent-Sender" => message.resent_sender_with_options(options),
        _ => unreachable!(),
    }
}

fn set<R>(
    message: &mut MimeMessage<R>,
    field: &str,
    value: Option<&MailboxAddress>,
) -> Result<(), AddressHeaderFormatError> {
    match field {
        "Sender" => message.set_sender(value),
        "Resent-Sender" => message.set_resent_sender(value),
        _ => unreachable!(),
    }
}

fn set_with_options<R>(
    message: &mut MimeMessage<R>,
    field: &str,
    value: Option<&MailboxAddress>,
    options: &AddressHeaderFormatOptions,
) -> Result<(), AddressHeaderFormatError> {
    match field {
        "Sender" => message.set_sender_with_options(value, options),
        "Resent-Sender" => message.set_resent_sender_with_options(value, options),
        _ => unreachable!(),
    }
}

#[test]
fn missing_senders_remain_absent_without_inference_or_stream_traits() {
    let mut message: MimeMessage<()> = MimeMessage::from_parts(
        headers(b"From: author@example.com\r\nResent-From: forwarder@example.com\r\n"),
        None,
    );
    let before = snapshot(&message);
    for field in FIELDS {
        assert_eq!(get(&message, field), Ok(None));
        set(&mut message, field, None).unwrap();
    }
    assert_eq!(snapshot(&message), before);
}

#[test]
fn getters_parse_one_mailbox_prefix_from_only_the_first_header() {
    for field in FIELDS {
        let raw = format!(
            "{}: (before) Agent <agent@example.com> (after), <ignored@example.com>\r\n{field}: invalid:\r\n",
            field.to_ascii_lowercase()
        );
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        assert_eq!(get(&message, field), Ok(Some(mailbox())));
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn invalid_first_values_preserve_parser_errors_and_do_not_fall_back() {
    for field in FIELDS {
        for value in [
            "",
            "Group: member@example.com;",
            "Agent <agent@example.com> (open",
        ] {
            let raw = format!("{field}: {value}\r\n{field}: Agent <agent@example.com>\r\n");
            let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
            let raw_value = message.headers().get(field).unwrap().raw_value();
            let expected = MailboxAddress::parse_prefix(raw_value).unwrap_err();
            assert_eq!(get(&message, field), Err(expected));
        }
    }
}

#[test]
fn explicit_mailbox_options_control_compatibility_parsing() {
    let options = MailboxParseOptions {
        address: AddrSpecParseOptions {
            allow_local_only: false,
            ..Default::default()
        },
        ..Default::default()
    };
    for field in FIELDS {
        let message: MimeMessage =
            MimeMessage::from_parts(headers(format!("{field}: local\r\n").as_bytes()), None);
        assert_eq!(
            get(&message, field).unwrap().unwrap().address().as_str(),
            "local"
        );
        let raw = message.headers().get(field).unwrap().raw_value();
        let expected = MailboxAddress::parse_prefix_with_options(raw, &options).unwrap_err();
        assert_eq!(get_with_options(&message, field, &options), Err(expected));
        let absent: MimeMessage<()> = MimeMessage::new();
        assert_eq!(get_with_options(&absent, field, &options), Ok(None));
    }
}

#[test]
fn getters_decode_names_and_preserve_source_routes_and_diagnostics() {
    for field in FIELDS {
        let raw = format!("{field}: =?utf-8?q?Ren=C3=A9e?= <@relay.example:agent@example.com>\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let actual = get(&message, field).unwrap().unwrap();
        assert_eq!(actual.display_name(), "Ren\u{e9}e");
        assert_eq!(actual.route().as_slice(), &["relay.example"]);
        assert!(actual.display_name_issues().is_empty());

        let raw = format!("{field}: =?utf-8?b?/w==?= <agent@example.com>\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let raw = message.headers().get(field).unwrap().raw_value();
        let expected = MailboxAddress::parse_prefix(raw).unwrap().0;
        let actual = get(&message, field).unwrap().unwrap();
        assert!(!actual.display_name_issues().is_empty());
        assert_eq!(actual.display_name(), expected.display_name());
        assert_eq!(actual.display_name_issues(), expected.display_name_issues());
    }
}

#[test]
fn returned_mailboxes_are_detached_until_explicitly_assigned() {
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        let original = mailbox();
        set(&mut message, field, Some(&original)).unwrap();
        let before = snapshot(&message);
        let mut edited = get(&message, field).unwrap().unwrap();
        edited.set_display_name("Delivery Agent");
        assert_eq!(snapshot(&message), before);
        assert_eq!(get(&message, field), Ok(Some(original.clone())));
        set(&mut message, field, Some(&edited)).unwrap();
        assert_eq!(get(&message, field), Ok(Some(edited)));
        assert_eq!(original, mailbox());
    }
}

#[test]
fn assignments_reformat_equal_mailboxes_and_collapse_duplicate_headers() {
    for field in FIELDS {
        let raw = format!(
            "X-Before: one\n{}: (comment) Agent\n\t<agent@example.com>\nX-Middle: two\n{field}: other@example.com\n",
            field.to_ascii_lowercase()
        );
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        assert_eq!(get(&message, field), Ok(Some(mailbox())));
        set(&mut message, field, Some(&mailbox())).unwrap();
        assert_eq!(
            snapshot(&message),
            [
                b"X-Before: one\n".to_vec(),
                format!("{field}: Agent <agent@example.com>\r\n").into_bytes(),
                b"X-Middle: two\n".to_vec(),
            ]
        );
    }
}

#[test]
fn route_only_edits_are_not_lost_to_mailbox_equality() {
    let first = MailboxAddress::parse(b"Agent <@first.example:agent@example.com>").unwrap();
    let second = MailboxAddress::parse(b"Agent <@second.example:agent@example.com>").unwrap();
    assert_eq!(first, second);
    assert_ne!(first.route(), second.route());
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set(&mut message, field, Some(&first)).unwrap();
        set(&mut message, field, Some(&second)).unwrap();
        assert_eq!(
            get(&message, field).unwrap().unwrap().route(),
            second.route()
        );
        assert_eq!(
            message.headers().get(field).unwrap().raw(),
            format!("{field}: Agent <@second.example:agent@example.com>\r\n").as_bytes()
        );
    }
}

#[test]
fn setters_encode_names_and_idna_without_mutating_the_input() {
    let mailbox = MailboxAddress::new("Ren\u{e9}e", "agent@b\u{fc}cher.example".parse().unwrap());
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set(&mut message, field, Some(&mailbox)).unwrap();
        let raw = message.headers().get(field).unwrap().raw();
        assert!(raw.is_ascii());
        let actual = get(&message, field).unwrap().unwrap();
        assert_eq!(actual.display_name(), mailbox.display_name());
        assert_eq!(actual.address().as_str(), "agent@xn--bcher-kva.example");
        assert_eq!(mailbox.address().as_str(), "agent@b\u{fc}cher.example");
    }
}

#[test]
fn name_line_breaks_are_encoded_not_written_as_extra_headers() {
    let mailbox = MailboxAddress::new(
        "Agent\r\nBcc: injected@example.com",
        "agent@example.com".parse().unwrap(),
    );
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set(&mut message, field, Some(&mailbox)).unwrap();
        let raw = message.headers().get(field).unwrap().raw();
        let parsed = headers(raw);
        assert_eq!(parsed.len(), 1);
        assert!(!parsed.contains("Bcc"));
        assert_eq!(
            get(&message, field).unwrap().unwrap().display_name(),
            mailbox.display_name()
        );
    }
}

#[test]
fn formatting_failures_are_atomic_even_for_an_equal_mailbox() {
    for field in FIELDS {
        let raw = format!(
            "X-Before: one\r\n{field}: Agent <agent@example.com>\r\n{field}: duplicate@example.com\r\n"
        );
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        for options in [
            AddressHeaderFormatOptions {
                max_line_length: 59,
                ..Default::default()
            },
            AddressHeaderFormatOptions {
                new_line_format: NewLineFormat::Mixed,
                ..Default::default()
            },
        ] {
            assert!(set_with_options(&mut message, field, Some(&mailbox()), &options).is_err());
            assert_eq!(snapshot(&message), before);
        }
        let international = MailboxAddress::new("", "caf\u{e9}@example.com".parse().unwrap());
        assert!(matches!(
            set(&mut message, field, Some(&international)),
            Err(AddressHeaderFormatError::Address(_))
        ));
        assert_eq!(snapshot(&message), before);
        let long = MailboxAddress::new(
            "",
            format!("{}@example.com", "a".repeat(100)).parse().unwrap(),
        );
        assert!(matches!(
            set(&mut message, field, Some(&long)),
            Err(AddressHeaderFormatError::LineTooLong { .. })
        ));
        assert_eq!(snapshot(&message), before);
        let options = AddressHeaderFormatOptions {
            max_line_length: 160,
            ..Default::default()
        };
        set_with_options(&mut message, field, Some(&long), &options).unwrap();
        assert_eq!(get(&message, field), Ok(Some(long)));
    }
}

#[test]
fn custom_formatting_honors_newlines_and_exact_physical_limits() {
    let mailbox = MailboxAddress::new(
        &"Ren\u{e9}e Agent ".repeat(20),
        "agent@example.com".parse().unwrap(),
    );
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    for field in FIELDS {
        let mut message: MimeMessage = MimeMessage::new();
        set_with_options(&mut message, field, Some(&mailbox), &options).unwrap();
        let raw = message.headers().get(field).unwrap().raw();
        assert!(raw.is_ascii());
        assert!(!raw.contains(&b'\r'));
        let text = std::str::from_utf8(raw).unwrap();
        assert!(text.ends_with('\n'));
        assert!(text.lines().count() > 1);
        for (index, line) in text.lines().enumerate() {
            assert!(line.len() <= 60);
            if index > 0 {
                assert!(line.starts_with('\t'));
            }
        }
        assert_eq!(get(&message, field), Ok(Some(mailbox.clone())));
    }
}

#[test]
fn removing_senders_ignores_unused_options_and_removes_all_occurrences_only() {
    let options = AddressHeaderFormatOptions {
        max_line_length: 0,
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    for field in FIELDS {
        let raw = format!(
            "X-Before: one\r\n{field}: invalid:\r\nX-Middle: two\r\n{field}: other@example.com\r\n"
        );
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        set_with_options(&mut message, field, None, &options).unwrap();
        assert_eq!(get(&message, field), Ok(None));
        assert_eq!(
            snapshot(&message),
            [b"X-Before: one\r\n".to_vec(), b"X-Middle: two\r\n".to_vec()]
        );
    }
}

#[test]
fn raw_edits_and_removal_of_first_duplicates_are_visible_immediately() {
    for field in FIELDS {
        let raw = format!("{field}: first@example.com\r\n{field}: second@example.com\r\n");
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        assert_eq!(
            get(&message, field).unwrap().unwrap().address().as_str(),
            "first@example.com"
        );
        message.headers_mut().remove(field);
        assert_eq!(
            get(&message, field).unwrap().unwrap().address().as_str(),
            "second@example.com"
        );
        message
            .headers_mut()
            .replace_all(Header::parse(format!("{field}: third@example.com").as_bytes()).unwrap())
            .unwrap();
        assert_eq!(
            get(&message, field).unwrap().unwrap().address().as_str(),
            "third@example.com"
        );
        message.headers_mut().clear();
        assert_eq!(get(&message, field), Ok(None));
    }
}

#[test]
fn sender_fields_remain_independent_through_message_serialization() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    let sender = mailbox();
    let resent = MailboxAddress::parse(b"Forwarder <forwarder@example.com>").unwrap();
    message.set_sender(Some(&sender)).unwrap();
    message.set_resent_sender(Some(&resent)).unwrap();
    let before = snapshot(&message);
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert!(wire.ends_with(b"\r\n\r\nhello"));
    assert_eq!(snapshot(&message), before);
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(snapshot(&loaded), before);
    assert_eq!(loaded.sender(), Ok(Some(sender)));
    assert_eq!(loaded.resent_sender(), Ok(Some(resent.clone())));
    message.set_sender(None).unwrap();
    assert_eq!(message.resent_sender(), Ok(Some(resent)));
    assert!(!message.headers().contains("From"));
    assert!(!message.headers().contains("Resent-From"));
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("sender metadata must not read body sources");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("sender metadata must not seek body sources");
    }
}

#[test]
fn sender_metadata_ignores_body_headers_and_sources() {
    let part = MimePart::from_parts(
        headers(b"Content-Type: invalid\r\nSender: body@example.com\r\nResent-Sender: body@example.com\r\n"),
        MimeContent::new(NoIo),
    );
    let mut message = MimeMessage::with_body(part.into());
    let before: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    for field in FIELDS {
        assert_eq!(get(&message, field), Ok(None));
        set(&mut message, field, Some(&mailbox())).unwrap();
        assert_eq!(get(&message, field), Ok(Some(mailbox())));
        set(&mut message, field, None).unwrap();
    }
    let after: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    assert_eq!(after, before);
}

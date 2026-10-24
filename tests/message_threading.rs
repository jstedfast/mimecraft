// Adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    Header, HeaderList, MessageId, MessageIdParseError, MessageIdParseErrorKind, MimeContent,
    MimeMessage, MimePart, References, TextPart,
};

const ID_FIELDS: [&str; 3] = ["Message-Id", "Resent-Message-Id", "In-Reply-To"];

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

fn get<R>(message: &MimeMessage<R>, field: &str) -> Result<Option<MessageId>, MessageIdParseError> {
    match field {
        "Message-Id" => message.message_id(),
        "Resent-Message-Id" => message.resent_message_id(),
        "In-Reply-To" => message.in_reply_to(),
        _ => unreachable!(),
    }
}

fn set<R>(
    message: &mut MimeMessage<R>,
    field: &str,
    value: &str,
) -> Result<(), MessageIdParseError> {
    match field {
        "Message-Id" => message.set_message_id(value),
        "Resent-Message-Id" => message.set_resent_message_id(value),
        "In-Reply-To" => message.set_in_reply_to(Some(value)),
        _ => unreachable!(),
    }
}

fn reference_values<R>(message: &MimeMessage<R>) -> Vec<String> {
    message
        .references()
        .map(|id| id.unwrap().into_string())
        .collect()
}

#[test]
fn absent_headers_have_no_defaults_and_need_no_stream_traits() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    for field in ID_FIELDS {
        assert_eq!(get(&message, field), Ok(None));
    }
    assert!(message.references().next().is_none());
    message.set_in_reply_to(None).unwrap();
    message.set_references(std::iter::empty::<&str>()).unwrap();
    assert!(message.headers().is_empty());
}

#[test]
fn getters_use_only_first_header_and_stop_after_first_id() {
    for field in ID_FIELDS {
        let raw = format!(
            "{}: (before) <first@x> (unterminated\r\n{field}: <second@x>\r\n",
            field.to_ascii_lowercase()
        );
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        assert_eq!(get(&message, field).unwrap().unwrap().as_str(), "first@x");
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn invalid_first_values_are_errors_not_missing_or_later_duplicates() {
    for field in ID_FIELDS {
        let raw = format!("{field}: <>\r\n{field}: <valid@x>\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let value = message.headers().get(field).unwrap().raw_value();
        let expected = if field == "In-Reply-To" {
            References::new(value).next().unwrap().unwrap_err()
        } else {
            MessageId::parse_prefix(value).unwrap_err()
        };
        assert_eq!(expected.kind(), MessageIdParseErrorKind::EmptyId);
        assert_eq!(get(&message, field), Err(expected));
    }
    for field in ["Message-Id", "Resent-Message-Id"] {
        let message: MimeMessage =
            MimeMessage::from_parts(headers(format!("{field}:\r\n").as_bytes()), None);
        assert_eq!(
            get(&message, field).unwrap_err().kind(),
            MessageIdParseErrorKind::MissingId
        );
    }
}

#[test]
fn in_reply_to_scans_bracketed_candidates_instead_of_a_prefix() {
    let mut message: MimeMessage = MimeMessage::new();
    for value in [
        "",
        "bare@x",
        "(<comment@x>)",
        "\"quoted <hidden@x>\"",
        "=?utf-8?q?=3Cencoded=40x=3E?=",
    ] {
        message
            .headers_mut()
            .replace_all(Header::parse(format!("In-Reply-To: {value}").as_bytes()).unwrap())
            .unwrap();
        assert_eq!(message.in_reply_to(), Ok(None), "{value}");
    }
    message
        .headers_mut()
        .replace_all(
            Header::parse(b"In-Reply-To: prose (<hidden@x>) \"<also-hidden@x>\" <first@x> <>")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(message.in_reply_to().unwrap().unwrap().as_str(), "first@x");
    message
        .headers_mut()
        .replace_all(Header::parse(b"In-Reply-To: prose <> <valid@x>").unwrap())
        .unwrap();
    assert_eq!(
        message.in_reply_to().unwrap_err().kind(),
        MessageIdParseErrorKind::EmptyId
    );
}

#[test]
fn reference_iterator_preserves_order_duplicates_errors_and_recovery() {
    let message: MimeMessage = MimeMessage::from_parts(
        headers(
            b"References: prose <root@x> <> <parent@x> <root@x>\r\nReferences: <ignored@x>\r\n",
        ),
        None,
    );
    let before = snapshot(&message);
    let raw = message.headers().get("References").unwrap().raw_value();
    let mut expected = References::new(raw);
    let mut actual = message.references();
    assert_eq!(actual.next(), expected.next());
    let error = actual.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), MessageIdParseErrorKind::EmptyId);
    assert_eq!(Some(Err(error)), expected.next());
    assert_eq!(actual.next().unwrap().unwrap().as_str(), "parent@x");
    assert_eq!(actual.next().unwrap().unwrap().as_str(), "root@x");
    assert!(actual.next().is_none());
    assert!(actual.next().is_none());
    assert_eq!(snapshot(&message), before);
}

#[test]
fn raw_replacement_removal_and_clear_are_immediately_visible() {
    let mut message: MimeMessage = MimeMessage::new();
    for field in ID_FIELDS {
        set(&mut message, field, "initial@x").unwrap();
        assert_eq!(get(&message, field).unwrap().unwrap().as_str(), "initial@x");
        message
            .headers_mut()
            .replace_all(Header::parse(format!("{field}: <changed@x>").as_bytes()).unwrap())
            .unwrap();
        assert_eq!(get(&message, field).unwrap().unwrap().as_str(), "changed@x");
        message.headers_mut().remove_all(field);
        assert_eq!(get(&message, field), Ok(None));
    }
    message.set_references(["initial@x"]).unwrap();
    assert_eq!(reference_values(&message), ["initial@x"]);
    message
        .headers_mut()
        .replace_all(Header::parse(b"References: <changed@x>").unwrap())
        .unwrap();
    assert_eq!(reference_values(&message), ["changed@x"]);
    for field in ID_FIELDS {
        set(&mut message, field, "present@x").unwrap();
    }
    message.headers_mut().clear();
    for field in ID_FIELDS {
        assert_eq!(get(&message, field), Ok(None));
    }
    assert!(message.references().next().is_none());
}

#[test]
fn assignments_normalize_compatibility_values_and_emit_canonical_headers() {
    for (value, expected) in [
        (" \t<abc@Example.COM>\r\n", "abc@Example.COM"),
        ("<a b@example.com>", "ab@example.com"),
        ("<a@@example.com.>", "a@example.com"),
        ("local", "local"),
        ("<local@>", "local@"),
        ("(paren@example.com", "(paren@example.com"),
        ("\"x\r\n y\"@example.com", "\"x y\"@example.com"),
        ("caf\u{e9}@example.com", "caf\u{e9}@example.com"),
    ] {
        for field in ID_FIELDS {
            let mut message: MimeMessage = MimeMessage::new();
            set(&mut message, field, value).unwrap();
            assert_eq!(
                get(&message, field).unwrap().unwrap().as_str(),
                expected,
                "{value}"
            );
            assert_eq!(
                message.headers().get(field).unwrap().raw(),
                format!("{field}: <{expected}>\r\n").as_bytes()
            );
            assert_eq!(message.headers().len(), 1);
        }
    }
}

#[test]
fn equal_normalized_assignments_preserve_raw_bytes_and_duplicates() {
    for field in ID_FIELDS {
        let raw = format!(
            "X-Before: untouched\n{}: (comment)\n\t<same@x> <another@x>\nX-Middle: untouched\n{field}: <duplicate@x>\n",
            field.to_ascii_lowercase()
        );
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        set(&mut message, field, "\t<same@x> ").unwrap();
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn changed_or_invalid_identifiers_replace_at_first_slot_and_remove_duplicates() {
    for field in ID_FIELDS {
        for old in ["<old@x>", "<>"] {
            let raw = format!(
                "X-Before: one\r\n{field}: {old}\r\nX-Middle: two\r\n{}: <duplicate@x>\r\nX-After: three\r\n",
                field.to_ascii_lowercase()
            );
            let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
            set(&mut message, field, "new@x").unwrap();
            assert_eq!(
                snapshot(&message),
                [
                    b"X-Before: one\r\n".to_vec(),
                    format!("{field}: <new@x>\r\n").into_bytes(),
                    b"X-Middle: two\r\n".to_vec(),
                    b"X-After: three\r\n".to_vec(),
                ]
            );
        }
    }
}

#[test]
fn invalid_assignments_cannot_inject_headers_or_partially_edit() {
    let mut message: MimeMessage = MimeMessage::new();
    for field in ID_FIELDS {
        set(&mut message, field, "ab@x").unwrap();
    }
    message.set_references(["root@x"]).unwrap();
    let before = snapshot(&message);
    for value in [
        "",
        "<>",
        "a b@x",
        "a@x>",
        "a\0@x",
        "a@x\r\nBcc: injected@x",
        "a@x\nBcc: injected@x",
        "a@x\rBcc: injected@x",
        "a@x\r\n\r\nInjected body",
        "<one@x> <two@x>",
    ] {
        let expected = MessageId::from_value(value).unwrap_err();
        for field in ID_FIELDS {
            assert_eq!(set(&mut message, field, value), Err(expected));
            assert_eq!(snapshot(&message), before);
        }
        assert_eq!(message.set_references(["valid@x", value]), Err(expected));
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn removing_in_reply_to_removes_all_occurrences_only() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"Message-Id: <self@x>\r\nIn-Reply-To: <>\r\nReferences: <root@x>\r\nin-reply-to: <parent@x>\r\n"),
        None,
    );
    message.set_in_reply_to(None).unwrap();
    assert_eq!(message.in_reply_to(), Ok(None));
    assert_eq!(
        snapshot(&message),
        [
            b"Message-Id: <self@x>\r\n".to_vec(),
            b"References: <root@x>\r\n".to_vec(),
        ]
    );
}

#[test]
fn references_replacement_retains_id_order_and_duplicates_not_duplicate_headers() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"X-Before: one\r\nreferences: <>\r\nX-Middle: two\r\nReferences: <ignored@x>\r\nIn-Reply-To: <parent@x>\r\n"),
        None,
    );
    message
        .set_references(["<root@x>", "parent@x", "root@x"])
        .unwrap();
    assert_eq!(reference_values(&message), ["root@x", "parent@x", "root@x"]);
    assert_eq!(
        snapshot(&message),
        [
            b"X-Before: one\r\n".to_vec(),
            b"References: <root@x> <parent@x> <root@x>\r\n".to_vec(),
            b"X-Middle: two\r\n".to_vec(),
            b"In-Reply-To: <parent@x>\r\n".to_vec(),
        ]
    );
}

#[test]
fn empty_reference_sequence_removes_all_occurrences() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"References: <>\r\nIn-Reply-To: <parent@x>\r\nreferences: <other@x>\r\n"),
        None,
    );
    message.set_references(Vec::<String>::new()).unwrap();
    assert!(message.references().next().is_none());
    assert_eq!(
        snapshot(&message),
        [b"In-Reply-To: <parent@x>\r\n".to_vec()]
    );
}

#[test]
fn reference_wrapping_counts_brackets_spaces_and_field_prefix() {
    let first = "a".repeat(28);
    for length in [32, 33, 34] {
        let second = "b".repeat(length);
        let mut message: MimeMessage = MimeMessage::new();
        message
            .set_references([first.as_str(), second.as_str(), "c"])
            .unwrap();
        let expected = if length <= 33 {
            format!("References: <{first}> <{second}>\r\n\t<c>\r\n")
        } else {
            format!("References: <{first}>\r\n\t<{second}> <c>\r\n")
        };
        let actual = message.headers().get("References").unwrap().raw();
        assert_eq!(actual, expected.as_bytes());
        assert!(expected.split("\r\n").all(|line| line.len() <= 78));
        assert_eq!(
            reference_values(&message),
            [first.as_str(), second.as_str(), "c"]
        );
    }
}

#[test]
fn reference_wrapping_counts_utf8_bytes_not_characters() {
    let first = "\u{e9}".repeat(28);
    for second in ["abcde", "abcdef"] {
        let mut message: MimeMessage = MimeMessage::new();
        message.set_references([first.as_str(), second]).unwrap();
        let expected = if second.len() == 5 {
            format!("References: <{first}> <{second}>\r\n")
        } else {
            format!("References: <{first}>\r\n\t<{second}>\r\n")
        };
        assert_eq!(
            message.headers().get("References").unwrap().raw(),
            expected.as_bytes()
        );
        assert_eq!(reference_values(&message), [first.as_str(), second]);
    }
}

#[test]
fn indivisible_identifiers_are_never_split_or_truncated() {
    let long = format!("{}@example.com", "a".repeat(1100));
    let mut message: MimeMessage = MimeMessage::new();
    for field in ID_FIELDS {
        set(&mut message, field, &long).unwrap();
        assert_eq!(
            message.headers().get(field).unwrap().raw(),
            format!("{field}: <{long}>\r\n").as_bytes()
        );
    }
    message
        .set_references([long.as_str(), "next@x", long.as_str()])
        .unwrap();
    assert_eq!(
        message.headers().get("References").unwrap().raw(),
        format!("References: <{long}>\r\n\t<next@x>\r\n\t<{long}>\r\n").as_bytes()
    );
    assert_eq!(
        reference_values(&message),
        [long.as_str(), "next@x", long.as_str()]
    );
}

#[test]
fn compatible_getter_values_can_be_assigned_and_reloaded() {
    // These compatibility forms occur in real mail. Getter values must remain
    // assignable without changing the normalized identifier on a wire roundtrip.
    for raw in [
        "<a@example.com>",
        "<(paren@example.com>",
        "<\"x\r\n y\"@example.com>",
        "garbage <a@example.com>",
        "<a@b@example.com>",
        "<local@>",
        "<[a@example.com]>",
        "<a.@example.com.>",
        "<a@@example.com>",
        "<a b@example.com>",
        "<1234@local.machine.example> (comment)",
        "a@example.com",
    ] {
        let source: MimeMessage = MimeMessage::from_parts(
            headers(format!("Message-Id: {raw}\r\nResent-Message-Id: {raw}\r\nIn-Reply-To: {raw}\r\nReferences: {raw} <z@x>\r\n").as_bytes()),
            None,
        );
        let mut message: MimeMessage = MimeMessage::new();
        for field in ID_FIELDS {
            if let Some(id) = get(&source, field).unwrap() {
                set(&mut message, field, id.as_str()).unwrap();
                assert_eq!(get(&message, field).unwrap(), Some(id), "{raw}: {field}");
            }
        }
        let references = source.references().collect::<Result<Vec<_>, _>>().unwrap();
        message
            .set_references(references.iter().map(MessageId::as_str))
            .unwrap();
        let mut wire = Vec::new();
        message.write_to(&mut wire).unwrap();
        let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
        for field in ID_FIELDS {
            assert_eq!(get(&loaded, field), get(&message, field), "{raw}: {field}");
        }
        assert_eq!(
            reference_values(&loaded),
            reference_values(&message),
            "{raw}"
        );
    }
}

#[test]
fn threaded_message_roundtrips_with_body_and_unrelated_headers_unchanged() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    message
        .headers_mut()
        .add(Header::parse(b"Subject: reply").unwrap());
    message.set_message_id("reply@x").unwrap();
    message.set_resent_message_id("resent@x").unwrap();
    message.set_in_reply_to(Some("parent@x")).unwrap();
    message.set_references(["root@x", "parent@x"]).unwrap();
    let before = snapshot(&message);
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert!(wire.ends_with(b"\r\n\r\nhello"));
    assert_eq!(snapshot(&message), before);
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(snapshot(&loaded), before);
    assert_eq!(loaded.message_id().unwrap().unwrap().as_str(), "reply@x");
    assert_eq!(
        loaded.resent_message_id().unwrap().unwrap().as_str(),
        "resent@x"
    );
    assert_eq!(loaded.in_reply_to().unwrap().unwrap().as_str(), "parent@x");
    assert_eq!(reference_values(&loaded), ["root@x", "parent@x"]);
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("threading metadata must not read body sources");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("threading metadata must not seek body sources");
    }
}

#[test]
fn metadata_never_inspects_body_headers_or_sources() {
    let part = MimePart::from_parts(
        headers(b"Content-Type: invalid\r\nMessage-Id: <body@x>\r\nReferences: <body@x>\r\n"),
        MimeContent::new(NoIo),
    );
    let mut message = MimeMessage::with_body(part.into());
    let body_headers: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    assert_eq!(message.message_id(), Ok(None));
    assert!(message.references().next().is_none());
    for field in ID_FIELDS {
        set(&mut message, field, "outer@x").unwrap();
        assert_eq!(get(&message, field).unwrap().unwrap().as_str(), "outer@x");
    }
    message.set_references(["outer@x"]).unwrap();
    assert_eq!(reference_values(&message), ["outer@x"]);
    message.set_in_reply_to(None).unwrap();
    message.set_references([] as [&str; 0]).unwrap();
    let after: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    assert_eq!(after, body_headers);
}

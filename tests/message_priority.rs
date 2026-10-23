// Adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::Cursor;

use mimecraft::{
    Header, HeaderId, HeaderList, HeaderParser, MessageImportance, MessagePriority,
    MessagePriorityParseError, MimeContent, MimeMessage, MimePart, TextPart, XMessagePriority,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

#[test]
fn importance_values_parse_format_and_create_headers() {
    for (value, token) in [
        (MessageImportance::Low, "low"),
        (MessageImportance::Normal, "normal"),
        (MessageImportance::High, "high"),
    ] {
        assert_eq!(MessageImportance::parse(token), Ok(value));
        assert_eq!(token.parse(), Ok(value));
        assert_eq!(
            MessageImportance::parse(&format!("\u{a0}{}\u{2003}", token.to_ascii_uppercase())),
            Ok(value)
        );
        assert_eq!(value.as_str(), token);
        assert_eq!(value.to_string(), token);
        assert_eq!(value.to_header().id(), HeaderId::Importance);
        assert_eq!(
            value.to_header().raw(),
            format!("Importance: {token}\r\n").as_bytes()
        );
    }
    assert_eq!(MessageImportance::default(), MessageImportance::Normal);
}

#[test]
fn priority_values_parse_format_and_create_headers() {
    for (value, token) in [
        (MessagePriority::NonUrgent, "non-urgent"),
        (MessagePriority::Normal, "normal"),
        (MessagePriority::Urgent, "urgent"),
    ] {
        assert_eq!(MessagePriority::parse(token), Ok(value));
        assert_eq!(token.parse(), Ok(value));
        assert_eq!(
            MessagePriority::parse(&format!("\r\n\t{}\u{a0}", token.to_ascii_uppercase())),
            Ok(value)
        );
        assert_eq!(value.as_str(), token);
        assert_eq!(value.to_string(), token);
        assert_eq!(value.to_header().id(), HeaderId::Priority);
        assert_eq!(
            value.to_header().raw(),
            format!("Priority: {token}\r\n").as_bytes()
        );
    }
    assert_eq!(MessagePriority::default(), MessagePriority::Normal);
}

#[test]
fn unknown_word_values_are_not_silently_normalized() {
    for value in [
        "",
        " \t\r\n",
        "other",
        "(comment) normal",
        "normal (comment)",
        "\"normal\"",
        "normal\0",
    ] {
        assert_eq!(
            MessageImportance::parse(value),
            Err(MessagePriorityParseError::Importance)
        );
        assert_eq!(
            MessagePriority::parse(value),
            Err(MessagePriorityParseError::Priority)
        );
    }
    assert!(MessageImportance::parse("urgent").is_err());
    assert!(MessagePriority::parse("high").is_err());
    assert!(MessageImportance::parse("=?utf-8?q?high?=").is_err());
    assert!(MessagePriority::parse("=?utf-8?q?urgent?=").is_err());
}

#[test]
fn x_priority_values_parse_format_and_create_headers() {
    for (value, number, token) in [
        (XMessagePriority::Highest, 1, "1 (Highest)"),
        (XMessagePriority::High, 2, "2 (High)"),
        (XMessagePriority::Normal, 3, "3 (Normal)"),
        (XMessagePriority::Low, 4, "4 (Low)"),
        (XMessagePriority::Lowest, 5, "5 (Lowest)"),
    ] {
        assert_eq!(XMessagePriority::parse(token.as_bytes()), Ok(value));
        assert_eq!(token.parse(), Ok(value));
        assert_eq!(value.number(), number);
        assert_eq!(value.as_str(), token);
        assert_eq!(value.to_string(), token);
        assert_eq!(value.to_header().id(), HeaderId::XPriority);
        assert_eq!(
            value.to_header().raw(),
            format!("X-Priority: {token}\r\n").as_bytes()
        );
    }
    assert_eq!(XMessagePriority::default(), XMessagePriority::Normal);
}

#[test]
fn x_priority_clamps_checked_numeric_prefixes_and_ignores_suffix_bytes() {
    for number in 0u32..=1000 {
        let parsed = XMessagePriority::parse(number.to_string().as_bytes()).unwrap();
        assert_eq!(u32::from(parsed.number()), number.clamp(1, 5));
    }
    for (input, expected) in [
        (
            b" \t\r\n0002 (not the level)".as_slice(),
            XMessagePriority::High,
        ),
        (b"1garbage", XMessagePriority::Highest),
        (b"3.5", XMessagePriority::Normal),
        (b"4\xff\0", XMessagePriority::Low),
        (b"2147483647", XMessagePriority::Lowest),
    ] {
        assert_eq!(XMessagePriority::parse(input), Ok(expected));
    }
    assert_eq!(
        XMessagePriority::parse(format!("{}2", "0".repeat(128)).as_bytes()),
        Ok(XMessagePriority::High)
    );
}

#[test]
fn x_priority_rejects_overflow_signs_and_non_numeric_prefixes() {
    for value in [
        b"".as_slice(),
        b" \t\r\n",
        b"garbage",
        b"-1",
        b"+1",
        b"(High) 2",
        b"\x0b2",
        b"\x0c2",
        b"\xc2\xa02",
        b"\xef\xbc\x92",
        b"2147483648",
        b"999999999999999999999",
        b"=?utf-8?q?2?=",
    ] {
        assert_eq!(
            XMessagePriority::parse(value),
            Err(MessagePriorityParseError::XPriority),
            "{value:?}"
        );
    }
}

#[test]
fn missing_headers_and_normal_setters_do_not_require_io_or_add_fields() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    assert_eq!(message.importance(), Ok(MessageImportance::Normal));
    assert_eq!(message.priority(), Ok(MessagePriority::Normal));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));

    // As in MimeKit's tests, setting absent values to Normal changes nothing.
    message.set_importance(MessageImportance::Normal);
    message.set_priority(MessagePriority::Normal);
    message.set_x_priority(XMessagePriority::Normal);
    assert!(message.headers().is_empty());

    message.set_importance(MessageImportance::High);
    assert_eq!(message.priority(), Ok(MessagePriority::Normal));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));
    message.set_priority(MessagePriority::NonUrgent);
    message.set_x_priority(XMessagePriority::Lowest);
    assert_eq!(message.headers().len(), 3);
    assert_eq!(message.importance(), Ok(MessageImportance::High));
    assert_eq!(message.priority(), Ok(MessagePriority::NonUrgent));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Lowest));
}

#[test]
fn decoded_word_headers_and_raw_x_priority_follow_distinct_paths() {
    let mut message: MimeMessage<()> = MimeMessage::from_parts(
        headers(
            b"Importance: =?utf-8?q?hIgH?=\r\nPriority: \r\n\tURGENT \r\nX-Priority: 2\xff\r\n",
        ),
        None,
    );
    let original = message.headers().clone();
    assert_eq!(message.importance(), Ok(MessageImportance::High));
    assert_eq!(message.priority(), Ok(MessagePriority::Urgent));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::High));
    assert_eq!(message.headers(), &original);

    message
        .headers_mut()
        .replace_all(Header::parse(b"Priority: =?utf-8?q?non-urgent?=").unwrap())
        .unwrap();
    message
        .headers_mut()
        .replace_all(Header::parse(b"X-Priority: =?utf-8?q?2?=").unwrap())
        .unwrap();
    assert_eq!(message.priority(), Ok(MessagePriority::NonUrgent));
    assert_eq!(
        message.x_priority(),
        Err(MessagePriorityParseError::XPriority)
    );
    message.headers_mut().clear();
    assert_eq!(message.importance(), Ok(MessageImportance::Normal));
    assert_eq!(message.priority(), Ok(MessagePriority::Normal));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));
}

#[test]
fn matching_setters_preserve_first_values_duplicate_headers_and_allocations() {
    let mut message: MimeMessage<()> = MimeMessage::from_parts(
        headers(b"iMpOrTaNcE: HIGH\nImportance: low\nPriority: urgent\nPriority: non-urgent\nX-Priority: 0 (custom)\nX-Priority: 5\n"),
        None,
    );
    let original = message.headers().clone();
    let pointers: Vec<_> = message
        .headers()
        .iter()
        .map(|header| header.raw().as_ptr())
        .collect();
    message.set_importance(MessageImportance::High);
    message.set_priority(MessagePriority::Urgent);
    message.set_x_priority(XMessagePriority::Highest);
    assert_eq!(message.headers(), &original);
    assert_eq!(
        message
            .headers()
            .iter()
            .map(|header| header.raw().as_ptr())
            .collect::<Vec<_>>(),
        pointers
    );
}

#[test]
fn invalid_first_values_are_errors_and_explicit_setters_repair_duplicates() {
    let mut message: MimeMessage<()> = MimeMessage::from_parts(
        headers(b"Subject: keep\nImportance: invalid\nX-Unrelated: \xff\nPriority: invalid\nImportance: high\nX-Priority: 2147483648\nPriority: urgent\nX-Priority: 2\n"),
        None,
    );
    let unrelated = message.headers().get("X-Unrelated").unwrap().raw().as_ptr();
    assert_eq!(
        message.importance(),
        Err(MessagePriorityParseError::Importance)
    );
    assert_eq!(message.priority(), Err(MessagePriorityParseError::Priority));
    assert_eq!(
        message.x_priority(),
        Err(MessagePriorityParseError::XPriority)
    );
    message.set_importance(MessageImportance::Normal);
    message.set_priority(MessagePriority::Normal);
    message.set_x_priority(XMessagePriority::Normal);
    assert_eq!(
        message.headers(),
        &headers(b"Subject: keep\nImportance: normal\r\nX-Unrelated: \xff\nPriority: normal\r\nX-Priority: 3 (Normal)\r\n")
    );
    assert_eq!(
        message.headers().get("X-Unrelated").unwrap().raw().as_ptr(),
        unrelated
    );
    assert_eq!(message.importance(), Ok(MessageImportance::Normal));
    assert_eq!(message.priority(), Ok(MessagePriority::Normal));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));
}

#[test]
fn empty_present_headers_are_not_treated_as_missing() {
    let mut message: MimeMessage<()> = MimeMessage::from_parts(
        headers(b"Importance:\r\nPriority: \t\r\nX-Priority: \t\r\n"),
        None,
    );
    assert_eq!(
        message.importance(),
        Err(MessagePriorityParseError::Importance)
    );
    assert_eq!(message.priority(), Err(MessagePriorityParseError::Priority));
    assert_eq!(
        message.x_priority(),
        Err(MessagePriorityParseError::XPriority)
    );
    message.set_importance(MessageImportance::Normal);
    message.set_priority(MessagePriority::Normal);
    message.set_x_priority(XMessagePriority::Normal);
    assert_eq!(
        message.headers(),
        &headers(b"Importance: normal\r\nPriority: normal\r\nX-Priority: 3 (Normal)\r\n")
    );
}

#[test]
fn raw_header_replacement_and_removal_are_immediately_visible() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    message.set_importance(MessageImportance::High);
    message.set_priority(MessagePriority::Urgent);
    message.set_x_priority(XMessagePriority::Highest);
    message
        .headers_mut()
        .replace_all(MessageImportance::Low.to_header())
        .unwrap();
    message
        .headers_mut()
        .replace_all(MessagePriority::NonUrgent.to_header())
        .unwrap();
    message
        .headers_mut()
        .replace_all(XMessagePriority::Lowest.to_header())
        .unwrap();
    assert_eq!(message.importance(), Ok(MessageImportance::Low));
    assert_eq!(message.priority(), Ok(MessagePriority::NonUrgent));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Lowest));
    message.set_importance(MessageImportance::High);
    message.set_priority(MessagePriority::Urgent);
    message.set_x_priority(XMessagePriority::Highest);
    assert_eq!(message.importance(), Ok(MessageImportance::High));
    assert_eq!(message.priority(), Ok(MessagePriority::Urgent));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Highest));
    message.headers_mut().remove_all("Importance");
    message.headers_mut().remove_all("Priority");
    message.headers_mut().remove_all("X-Priority");
    assert_eq!(message.importance(), Ok(MessageImportance::Normal));
    assert_eq!(message.priority(), Ok(MessagePriority::Normal));
    assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));
}

#[test]
fn message_metadata_does_not_inspect_or_change_body_metadata_or_sources() {
    let mut source = Cursor::new(b"payload".to_vec());
    source.set_position(4);
    {
        let mut part = MimePart::new(MimeContent::new(&mut source));
        part.headers_mut().add(MessageImportance::Low.to_header());
        part.headers_mut()
            .add(MessagePriority::NonUrgent.to_header());
        part.headers_mut().add(XMessagePriority::Lowest.to_header());
        let original = part.headers().clone();
        let mut message = MimeMessage::with_body(part.into());
        assert_eq!(message.importance(), Ok(MessageImportance::Normal));
        assert_eq!(message.priority(), Ok(MessagePriority::Normal));
        assert_eq!(message.x_priority(), Ok(XMessagePriority::Normal));
        message.set_importance(MessageImportance::High);
        message.set_priority(MessagePriority::Urgent);
        message.set_x_priority(XMessagePriority::Highest);
        let part = message.leaf_parts().next().unwrap();
        assert_eq!(part.headers(), &original);
        assert_eq!(part.content().get_ref().position(), 4);
    }
    assert_eq!(source.position(), 4);
    assert_eq!(source.get_ref(), b"payload");
}

#[test]
fn changed_levels_serialize_canonically_and_survive_message_loading() {
    for (importance, priority, x_priority) in [
        (
            MessageImportance::Low,
            MessagePriority::NonUrgent,
            XMessagePriority::Lowest,
        ),
        (
            MessageImportance::Normal,
            MessagePriority::Normal,
            XMessagePriority::Normal,
        ),
        (
            MessageImportance::High,
            MessagePriority::Urgent,
            XMessagePriority::High,
        ),
    ] {
        let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
        message.set_importance(MessageImportance::High);
        message.set_priority(MessagePriority::Urgent);
        message.set_x_priority(XMessagePriority::Highest);
        message.set_importance(importance);
        message.set_priority(priority);
        message.set_x_priority(x_priority);
        assert_eq!(
            message.headers().get("Importance").unwrap(),
            &importance.to_header()
        );
        assert_eq!(
            message.headers().get("Priority").unwrap(),
            &priority.to_header()
        );
        assert_eq!(
            message.headers().get("X-Priority").unwrap(),
            &x_priority.to_header()
        );
        let mut output = Vec::new();
        message.write_to(&mut output).unwrap();
        let loaded = MimeMessage::load(&mut output.as_slice()).unwrap();
        assert_eq!(loaded.importance(), Ok(importance));
        assert_eq!(loaded.priority(), Ok(priority));
        assert_eq!(loaded.x_priority(), Ok(x_priority));
    }
}

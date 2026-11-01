// Adapted from MimeKit's TnefMessageTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    embedded::{IID, object, wrap},
    *,
};
use mimecraft::tnef::{
    TnefDateTime, TnefDateTimeKind, TnefMessage as Message, TnefRecipientType,
    TnefValueConversionError as ConversionError,
};
use std::{borrow::Cow, io::Cursor, path::Path};

fn string(tag: Tag, value: &str) -> Vec<u8> {
    let raw = if tag.value_type() == Type::UNICODE {
        unicode(&format!("{value}\0"))
    } else {
        [value.as_bytes(), &[0]].concat()
    };
    object(tag.id(), tag.value_type(), &raw)
}

fn integer(tag: Tag, value: i32) -> Vec<u8> {
    fixed(tag.id(), tag.value_type(), &value.to_le_bytes())
}

fn attr(input: &mut Vec<u8>, tag: Attribute, payload: &[u8]) {
    let start = input.len();
    attribute(input, tag, payload);
    if matches!(
        tag,
        Attribute::ATTACH_RENDER_DATA
            | Attribute::ATTACH_TITLE
            | Attribute::ATTACH_DATA
            | Attribute::ATTACH_CREATE_DATE
            | Attribute::ATTACH_MODIFY_DATE
            | Attribute::ATTACH_TRANSPORT_FILENAME
            | Attribute::ATTACH_META_FILE
    ) {
        input[start] = 2;
    }
}

fn empty() -> Vec<u8> {
    vec![0x78, 0x9f, 0x3e, 0x22, 0x34, 0x12]
}

fn render(kind: u16, flags: u32) -> Vec<u8> {
    [
        kind.to_le_bytes().as_slice(),
        &(-1i32).to_le_bytes(),
        &[0; 4],
        &flags.to_le_bytes(),
    ]
    .concat()
}

fn attachment(input: &mut Vec<u8>, name: &str, data: &[u8]) {
    attr(input, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
    attr(
        input,
        Attribute::ATTACH_TITLE,
        format!("{name}\0").as_bytes(),
    );
    attr(input, Attribute::ATTACH_DATA, data);
}

fn date(hour: u16) -> Vec<u8> {
    [2024u16, 3, 15, hour, 30, 0, 5]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn owner(name: &str, address: &str) -> Vec<u8> {
    let name = format!("{name}\0").into_bytes();
    let address = format!("{address}\0").into_bytes();
    [
        &((name.len() as u16).to_le_bytes())[..],
        &name,
        &((address.len() as u16).to_le_bytes())[..],
        &address,
    ]
    .concat()
}

// attFrom is a one-off TRP followed by a null TRP ([MS-OXTNEF] 2.1.3.3.1).
fn from(name: &str, address: &str) -> Vec<u8> {
    let name = format!("{name}\0").into_bytes();
    let address = format!("{address}\0").into_bytes();
    [
        &4u16.to_le_bytes()[..],
        &((8 + name.len() + address.len()) as u16).to_le_bytes(),
        &(name.len() as u16).to_le_bytes(),
        &(address.len() as u16).to_le_bytes(),
        &name,
        &address,
        &[0; 8],
    ]
    .concat()
}

fn entry_id(name: &str, kind: &str, address: &str) -> Vec<u8> {
    let mut bytes = vec![0; 4];
    bytes.extend_from_slice(&[
        0x81, 0x2b, 0x1f, 0xa4, 0xbe, 0xa3, 0x10, 0x19, 0x9d, 0x6e, 0, 0xdd, 1, 0x0f, 0x54, 2, 0,
        0, 0, 0x80,
    ]);
    for value in [name, kind, address] {
        bytes.extend(unicode(&format!("{value}\0")));
    }
    bytes
}

fn load(input: &[u8]) -> Message {
    let mut reader = TnefReader::new(Chunked { input, chunk: 1 });
    let message = reader
        .load_message_with_logger(&mut |issue| panic!("unexpected issue: {issue:?}"))
        .unwrap();
    assert!(reader.is_finished());
    assert!(!reader.is_faulted());
    message
}

#[test]
fn upstream_legacy_message_attributes_and_dates() {
    let mut input = empty();
    attr(
        &mut input,
        Attribute::TNEF_VERSION,
        &0x10000u32.to_le_bytes(),
    );
    attr(&mut input, Attribute::OEM_CODEPAGE, &1252u32.to_le_bytes());
    attr(
        &mut input,
        Attribute::MESSAGE_CLASS,
        b"IPM.Microsoft Mail.Note\0",
    );
    attr(
        &mut input,
        Attribute::FROM,
        &from("Alice", "SMTP:alice@example.com"),
    );
    attr(&mut input, Attribute::SUBJECT, b"Hello\0");
    attr(&mut input, Attribute::MESSAGE_ID, b"0A1bFF\0");
    attr(&mut input, Attribute::DATE_SENT, &date(10));
    attr(&mut input, Attribute::DATE_RECEIVED, &date(11));
    attr(&mut input, Attribute::PRIORITY, &1u16.to_le_bytes());
    attr(&mut input, Attribute::MESSAGE_STATUS, &[0x20 | 0x80]);
    attr(&mut input, Attribute::REQUEST_RESPONSE, &1u16.to_le_bytes());
    let owner_offset = input.len() as u64;
    attr(
        &mut input,
        Attribute::OWNER,
        &owner("Bob", "SMTP:bob@example.com"),
    );
    attr(&mut input, Attribute::AID_OWNER, &42i32.to_le_bytes());
    let message = load(&input);
    assert_eq!(message.codepage(), 1252);
    assert_eq!(message.legacy_key(), 0x1234);
    assert_eq!(
        message.message_class().unwrap().as_deref(),
        Some("IPM.Note")
    );
    assert_eq!(message.subject().unwrap().as_deref(), Some("Hello"));
    let p = message.properties();
    assert_eq!(
        p.get_bytes(Tag::SEARCH_KEY).unwrap(),
        Some([0x0a, 0x1b, 0xff].as_slice())
    );
    assert_eq!(p.get_i32(Tag::IMPORTANCE).unwrap(), Some(2));
    assert_eq!(p.get_i32(Tag::MESSAGE_FLAGS).unwrap(), Some(0x13));
    assert_eq!(p.get_bool(Tag::RESPONSE_REQUESTED).unwrap(), Some(true));
    assert_eq!(p.get_i32(Tag::OWNER_APPT_ID).unwrap(), Some(42));
    for (tag, hour) in [
        (Tag::CLIENT_SUBMIT_TIME, 10),
        (Tag::MESSAGE_DELIVERY_TIME, 11),
    ] {
        let expected =
            TnefDateTime::new(2024, 3, 15, hour, 30, 0, TnefDateTimeKind::Unspecified).unwrap();
        assert_eq!(p.get_datetime(tag).unwrap(), Some(expected));
        assert_eq!(
            p.get(tag).unwrap().value(),
            Some(&Value::DateTime(expected))
        );
    }
    for (tag, expected) in [
        (Tag::SENDER_NAME_W, "Alice"),
        (Tag::SENDER_ADDRTYPE_W, "SMTP"),
        (Tag::SENDER_EMAIL_ADDRESS_W, "alice@example.com"),
        (Tag::SENT_REPRESENTING_NAME_W, "Bob"),
        (Tag::SENT_REPRESENTING_ADDRTYPE_W, "SMTP"),
        (Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W, "bob@example.com"),
    ] {
        assert_eq!(p.get_text(tag).unwrap().as_deref(), Some(expected));
    }
    assert_eq!(
        p.get_bytes(Tag::SENDER_ENTRY_ID).unwrap(),
        Some(entry_id("Alice", "SMTP", "alice@example.com").as_slice())
    );
    assert_eq!(
        p.get_bytes(Tag::SENT_REPRESENTING_ENTRY_ID).unwrap(),
        Some(entry_id("Bob", "SMTP", "bob@example.com").as_slice())
    );
    assert_eq!(p.get_text(Tag::RCVD_REPRESENTING_NAME_W).unwrap(), None);
    for tag in [
        Tag::SENT_REPRESENTING_NAME_W,
        Tag::SENT_REPRESENTING_ADDRTYPE_W,
        Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W,
        Tag::SENT_REPRESENTING_ENTRY_ID,
    ] {
        assert_eq!(p.get(tag).unwrap().info().offset, owner_offset);
    }
}

#[test]
fn upstream_class_translation_and_priority_vectors() {
    for (legacy, expected) in [
        (
            "Microsoft Mail v3.0 IPM.Microsoft Mail.Read Receipt",
            Some("Report.IPM.Note.IPNRN"),
        ),
        (
            "IPM.Microsoft Mail.Non-Delivery",
            Some("Report.IPM.Note.NDR"),
        ),
        (
            "IPM.Microsoft Schedule.MtgRespN",
            Some("IPM.Schedule.Meeting.Resp.Neg"),
        ),
        (
            "IPM.Microsoft Schedule.MtgRespA",
            Some("IPM.Schedule.Meeting.Resp.Tent"),
        ),
        (
            "IPM.Microsoft Schedule.MtgRespP",
            Some("IPM.Schedule.Meeting.Resp.Pos"),
        ),
        (
            "IPM.Microsoft Schedule.MtgReq",
            Some("IPM.Schedule.Meeting.Request"),
        ),
        (
            "IPM.Microsoft Schedule.MtgCncl",
            Some("IPM.Schedule.Meeting.Canceled"),
        ),
        ("IPM.Note.Custom", Some("IPM.Note.Custom")),
        (
            "Microsoft Mail v3.0 Unknown",
            Some("Microsoft Mail v3.0 Unknown"),
        ),
        (
            "  IPM.Microsoft Schedule.MtgReq  ",
            Some("IPM.Schedule.Meeting.Request"),
        ),
        ("\tIPM.Note \r\n", Some("IPM.Note")),
        ("   ", None),
    ] {
        let input = document(Attribute::MESSAGE_CLASS, format!("{legacy}\0").as_bytes());
        // The reader reports nonprintable/empty classes; the loader still trims them.
        let message = Message::load(input.as_slice()).unwrap();
        assert_eq!(
            message.message_class().unwrap().as_deref(),
            expected,
            "{legacy:?}"
        );
    }
    for value in 1u16..=3 {
        let message = load(&document(Attribute::PRIORITY, &value.to_le_bytes()));
        assert_eq!(
            message.properties().get_i32(Tag::IMPORTANCE).unwrap(),
            Some(3 - i32::from(value))
        );
    }
}

#[test]
fn upstream_owner_mapping_waits_for_trimmed_final_class() {
    for (class, response) in [
        ("  IPM.Schedule.Meeting.Resp.Pos  ", true),
        ("ipm.schedule.meeting.resp.neg", true),
        (" ", false),
    ] {
        let mut input = document(Attribute::OWNER, &owner("Bob", "SMTP:bob@example.com"));
        attr(
            &mut input,
            Attribute::MAPI_PROPERTIES,
            &properties(&[
                string(Tag::MESSAGE_CLASS_W, class),
                integer(Tag::INTERNET_CODEPAGE, 65001),
            ]),
        );
        let message = load(&input);
        assert_eq!(message.codepage(), 65001);
        assert_eq!(
            message
                .properties()
                .get_text(Tag::MESSAGE_CLASS_W)
                .unwrap()
                .as_deref(),
            Some(class)
        );
        assert_eq!(
            message.message_class().unwrap().as_deref(),
            (!class.trim().is_empty()).then_some(class.trim())
        );
        let tag = if response {
            Tag::RCVD_REPRESENTING_NAME_W
        } else {
            Tag::SENT_REPRESENTING_NAME_W
        };
        assert_eq!(
            message.properties().get_text(tag).unwrap().as_deref(),
            Some("Bob")
        );
        let property = message.properties().get(tag).unwrap();
        assert_eq!(property.info().offset, 6);
        assert_eq!(
            property.charset(),
            mimecraft::charsets::Charset::from_label("windows-1252").unwrap()
        );
    }
    let mut input = document(Attribute::OWNER, &owner("Bob", "SMTP:bob@example.com"));
    attr(
        &mut input,
        Attribute::MESSAGE_CLASS,
        b"IPM.Microsoft Schedule.MtgRespP\0",
    );
    let message = load(&input);
    assert_eq!(
        message
            .properties()
            .get_bytes(Tag::RCVD_REPRESENTING_ENTRY_ID)
            .unwrap(),
        Some(entry_id("Bob", "SMTP", "bob@example.com").as_slice())
    );
}

#[test]
fn upstream_misc_legacy_attributes_and_rendering() {
    let mut input = document(Attribute::FROM, &from("Carol", "carol@example.com"));
    for tag in [
        Attribute::DATE_START,
        Attribute::DATE_END,
        Attribute::DATE_MODIFIED,
    ] {
        attr(&mut input, tag, &date(9));
    }
    attr(
        &mut input,
        Attribute::ORIGINAL_MESSAGE_CLASS,
        b"IPM.Microsoft Mail.Note\0",
    );
    attr(&mut input, Attribute::DELEGATE, &[0, 0, 0, 0, 1, 2, 3, 4]);
    attr(&mut input, Attribute::MESSAGE_STATUS, &[7]);
    attr(&mut input, Attribute::ATTACH_RENDER_DATA, &render(2, 1));
    attr(
        &mut input,
        Attribute::ATTACH_TRANSPORT_FILENAME,
        b"transport.txt\0",
    );
    attr(&mut input, Attribute::ATTACH_META_FILE, &[1, 2, 3, 4]);
    attr(&mut input, Attribute::ATTACH_MODIFY_DATE, &date(9));
    attr(&mut input, Attribute::ATTACH_CREATE_DATE, &date(10));
    let message = load(&input);
    let p = message.properties();
    assert_eq!(p.get_text(Tag::SENDER_ADDRTYPE_W).unwrap(), None);
    assert_eq!(
        p.get_bytes(Tag::SENDER_ENTRY_ID).unwrap(),
        Some(entry_id("Carol", "", "carol@example.com").as_slice())
    );
    assert_eq!(p.get_i32(Tag::MESSAGE_FLAGS).unwrap(), Some(12));
    assert_eq!(
        p.get_text(Tag::ORIG_MESSAGE_CLASS_W).unwrap().as_deref(),
        Some("IPM.Note")
    );
    assert_eq!(
        p.get_bytes(Tag::RCVD_REPRESENTING_ENTRY_ID).unwrap(),
        Some([0, 0, 0, 0, 1, 2, 3, 4].as_slice())
    );
    for tag in [Tag::START_DATE, Tag::END_DATE, Tag::LAST_MODIFICATION_TIME] {
        assert_eq!(p.get_datetime(tag).unwrap().unwrap().hour(), 9);
    }
    let p = message.attachments()[0].properties();
    assert_eq!(
        p.get_text(Tag::ATTACH_TRANSPORT_NAME_W).unwrap().as_deref(),
        Some("transport.txt")
    );
    assert_eq!(
        p.get_bytes(Tag::ATTACH_RENDERING).unwrap(),
        Some([1, 2, 3, 4].as_slice())
    );
    assert_eq!(
        p.get_datetime(Tag::LAST_MODIFICATION_TIME)
            .unwrap()
            .unwrap()
            .hour(),
        9
    );
    assert_eq!(
        p.get_datetime(Tag::CREATION_TIME).unwrap().unwrap().hour(),
        10
    );
    assert_eq!(p.get_i32(Tag::RENDERING_POSITION).unwrap(), Some(-1));
    assert_eq!(
        p.get_bytes(Tag::ATTACH_TAG).unwrap(),
        Some([0x2a, 0x86, 0x48, 0x86, 0xf7, 0x14, 3, 10, 3, 2, 1].as_slice())
    );
    assert_eq!(
        p.get_bytes(Tag::ATTACH_ENCODING).unwrap(),
        Some([0x2a, 0x86, 0x48, 0x86, 0xf7, 0x14, 3, 11, 1].as_slice())
    );
}

#[test]
fn upstream_invalid_legacy_values_remain_diagnosed_not_fabricated() {
    let mut input = document(Attribute::PRIORITY, &7u16.to_le_bytes());
    attr(&mut input, Attribute::MESSAGE_ID, b"ABC\0");
    attr(&mut input, Attribute::CONVERSATION_ID, b"not hex\0");
    attr(&mut input, Attribute::MESSAGE_STATUS, &[]);
    attr(&mut input, Attribute::FROM, &[1, 0, 0, 0]);
    attr(&mut input, Attribute::OWNER, &[0xff, 0, 1, 2]);
    attr(&mut input, Attribute::SUBJECT, b"still loaded\0");
    let mut issues = Vec::new();
    let message = TnefReader::new(input.as_slice())
        .load_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert_eq!(message.subject().unwrap().as_deref(), Some("still loaded"));
    for tag in [
        Tag::IMPORTANCE,
        Tag::SEARCH_KEY,
        Tag::MESSAGE_FLAGS,
        Tag::SENDER_ENTRY_ID,
        Tag::SENT_REPRESENTING_ENTRY_ID,
    ] {
        assert!(message.properties().get_by_id(tag.id()).is_none());
    }
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == Violation::InvalidAttributeValue)
            .count(),
        5
    );
    let mut input = document(Attribute::MESSAGE_ID, b"0G\0");
    let mut bad_type = from("Alice", "SMTP:alice@example.com");
    bad_type[0] = 1;
    let mut bad_length = from("Alice", "SMTP:alice@example.com");
    bad_length[4] = 255;
    for bytes in [bad_type, bad_length] {
        attr(&mut input, Attribute::FROM, &bytes);
    }
    for bytes in [vec![1], vec![1, 0, b'A', 10, 0, b'B']] {
        attr(&mut input, Attribute::OWNER, &bytes);
    }
    issues.clear();
    let message = TnefReader::new(input.as_slice())
        .load_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert!(message.properties().is_empty());
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == Violation::InvalidAttributeValue)
            .count(),
        5
    );
}

#[test]
fn upstream_truncated_date_does_not_reuse_previous_date() {
    let mut input = document(Attribute::DATE_SENT, &date(10));
    attr(&mut input, Attribute::DATE_RECEIVED, &[0; 4]);
    let mut issues = Vec::new();
    let message = TnefReader::new(input.as_slice())
        .load_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert!(
        message
            .properties()
            .get_datetime(Tag::CLIENT_SUBMIT_TIME)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        message
            .properties()
            .get_datetime(Tag::MESSAGE_DELIVERY_TIME)
            .unwrap(),
        None
    );
    assert!(
        issues
            .iter()
            .any(|i| i.violation == Violation::InvalidAttributeValue)
    );
}

#[test]
fn upstream_mapi_precedence_and_duplicate_order() {
    let mut input = document(Attribute::SUBJECT, b"Legacy subject\0");
    attr(
        &mut input,
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            string(Tag::SUBJECT_W, "MAPI subject"),
            integer(Tag::IMPORTANCE, 0),
            string(Tag::SUBJECT_W, "second"),
            string(Tag::INTERNET_MESSAGE_ID_W, " <id@example.com> "),
        ]),
    );
    attr(&mut input, Attribute::PRIORITY, &1u16.to_le_bytes());
    attr(&mut input, Attribute::SUBJECT, b"later legacy\0");
    let message = load(&input);
    assert_eq!(
        message.subject().unwrap(),
        Some(Cow::Borrowed("MAPI subject"))
    );
    assert_eq!(
        message.internet_message_id().unwrap(),
        Some(Cow::Borrowed(" <id@example.com> "))
    );
    assert_eq!(
        message.properties().get_i32(Tag::IMPORTANCE).unwrap(),
        Some(0)
    );
    let subjects: Vec<_> = message
        .properties()
        .as_slice()
        .iter()
        .filter(|p| p.tag().id() == Tag::SUBJECT_W.id())
        .map(|p| p.to_text().unwrap())
        .collect();
    assert_eq!(subjects, ["MAPI subject", "second"]);
}

#[test]
fn upstream_bodies_select_first_mapi_and_apply_final_codepages() {
    let rtf = super::bodies::compressed(b"{\\rtf1 Hello}");
    let mut input = document(Attribute::BODY, b"legacy\0");
    attr(
        &mut input,
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            string(Tag::BODY_W, "MAPI body\0\0"),
            string(Tag::BODY_W, "Second body"),
            object(
                Tag::BODY_HTML_B.id(),
                Type::BINARY,
                "<p>caf\u{e9}</p>".as_bytes(),
            ),
            object(Tag::RTF_COMPRESSED.id(), Type::BINARY, &rtf),
            integer(Tag::INTERNET_CODEPAGE, 65001),
        ]),
    );
    let message = load(&input);
    assert_eq!(message.codepage(), 65001);
    assert_eq!(message.text_body().unwrap().text().unwrap(), "MAPI body");
    assert_eq!(message.text_body().unwrap().len(), 18);
    assert_eq!(
        message.html_body().unwrap().text().unwrap(),
        "<p>caf\u{e9}</p>"
    );
    assert_eq!(
        message.rtf_body().unwrap().text().unwrap(),
        "{\\rtf1 Hello}"
    );
    assert_eq!(message.rtf_body().unwrap().as_bytes(), rtf);
    assert_eq!(message.properties().len(), 1);
    for oem in [false, true] {
        let mut input = empty();
        if oem {
            attr(&mut input, Attribute::OEM_CODEPAGE, &1252i32.to_le_bytes());
        }
        let body: &[u8] = if oem { b"Caf\xe9\0" } else { b"Caf\xc3\xa9\0" };
        attr(&mut input, Attribute::BODY, body);
        attr(
            &mut input,
            Attribute::MAPI_PROPERTIES,
            &properties(&[integer(Tag::INTERNET_CODEPAGE, 65001)]),
        );
        let message = load(&input);
        assert_eq!(message.codepage(), if oem { 1252 } else { 65001 });
        assert_eq!(message.text_body().unwrap().text().unwrap(), "Caf\u{e9}");
    }
}

#[test]
fn valueless_and_unsupported_body_properties_are_retained() {
    let empty_body = [Tag::BODY_W.0.to_le_bytes(), 0u32.to_le_bytes()].concat();
    let mut input = document(Attribute::BODY, b"legacy");
    attr(
        &mut input,
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            empty_body,
            integer(Tag::new(Tag::BODY_A.id(), Type::LONG), 7),
            object(Tag::BODY_A.id(), Type::STRING8, &[]),
        ]),
    );
    let message = Message::load(input.as_slice()).unwrap();
    assert!(message.text_body().unwrap().is_empty());
    assert_eq!(message.properties().len(), 2);
    assert_eq!(message.properties()[0].value_count(), 0);
}

#[test]
fn upstream_recipients_across_tables_retain_rows_and_flags() {
    let first = properties(&[
        string(Tag::DISPLAY_NAME_W, "Alice"),
        string(Tag::EMAIL_ADDRESS_W, "alice@example.com"),
        string(Tag::ADDRTYPE_W, "SMTP"),
        integer(Tag::RECIPIENT_TYPE, 1),
    ]);
    let second = properties(&[
        string(Tag::DISPLAY_NAME_W, "Bob"),
        integer(Tag::RECIPIENT_TYPE, 0x10000003),
    ]);
    let mut input = empty();
    for row in [first, second] {
        attr(
            &mut input,
            Attribute::RECIPIENT_TABLE,
            &[&1u32.to_le_bytes()[..], &row].concat(),
        );
    }
    let message = load(&input);
    assert_eq!(message.recipients().len(), 2);
    assert_eq!(
        message.recipients()[0].recipient_type().unwrap(),
        TnefRecipientType::To
    );
    assert_eq!(
        message.recipients()[0].email_address().unwrap().as_deref(),
        Some("alice@example.com")
    );
    assert_eq!(
        message.recipients()[1].recipient_type().unwrap(),
        TnefRecipientType::Bcc
    );
    assert_eq!(
        message.recipients()[1].display_name().unwrap().as_deref(),
        Some("Bob")
    );
}

#[test]
fn upstream_attachment_grouping_precedence_and_empty_presence() {
    let mut input = empty();
    // No attAttachRendData: the first attachment starts implicitly.
    attr(&mut input, Attribute::ATTACH_TITLE, b"orphan.txt\0");
    attr(&mut input, Attribute::ATTACH_DATA, b"legacy");
    attr(
        &mut input,
        Attribute::ATTACHMENT,
        &properties(&[
            object(Tag::ATTACH_DATA_BIN.id(), Type::BINARY, b"MAPI"),
            object(Tag::ATTACH_DATA_BIN.id(), Type::BINARY, b"ignored"),
            string(Tag::ATTACH_LONG_FILENAME_W, "mapiname.txt"),
        ]),
    );
    attr(&mut input, Attribute::ATTACH_DATA, b"later legacy");
    attachment(&mut input, "empty.bin", &[]);
    attr(&mut input, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
    let message = load(&input);
    assert_eq!(message.attachments().len(), 3);
    let first = &message.attachments()[0];
    assert_eq!(first.file_name().unwrap().as_deref(), Some("mapiname.txt"));
    assert_eq!(first.as_bytes(), Some(b"MAPI".as_slice()));
    assert!(
        first
            .properties()
            .get_by_id(Tag::ATTACH_DATA_BIN.id())
            .is_none()
    );
    assert_eq!(message.attachments()[1].as_bytes(), Some([].as_slice()));
    assert_eq!(message.attachments()[2].as_bytes(), None);
}

#[test]
fn upstream_embedded_messages_and_late_binary_method() {
    let inner = document(Attribute::SUBJECT, b"Inner\0");
    for kind in [Type::BINARY, Type::OBJECT] {
        let mut input = empty();
        attr(
            &mut input,
            Attribute::ATTACHMENT,
            &properties(&[
                object(
                    Tag::ATTACH_DATA_BIN.id(),
                    kind,
                    &[IID.as_slice(), &inner].concat(),
                ),
                integer(Tag::ATTACH_METHOD, 5),
            ]),
        );
        let attachment = load(&input).attachments()[0].clone();
        assert!(attachment.is_embedded_message());
        assert_eq!(attachment.as_bytes(), Some(inner.as_slice()));
        for _ in 0..2 {
            assert_eq!(
                attachment
                    .load_embedded_message()
                    .unwrap()
                    .subject()
                    .unwrap()
                    .as_deref(),
                Some("Inner")
            );
        }
    }
    let input = wrap(&wrap(&inner));
    let message = load(&input);
    let child = message.attachments()[0].load_embedded_message().unwrap();
    assert_eq!(
        child.attachments()[0]
            .load_embedded_message()
            .unwrap()
            .subject()
            .unwrap()
            .as_deref(),
        Some("Inner")
    );
}

#[test]
fn attachment_and_byte_limits_return_errors_instead_of_partial_messages() {
    let mut input = empty();
    for i in 0..4 {
        attachment(&mut input, &format!("file{i}.bin"), &[i]);
    }
    let options = TnefReaderOptions {
        max_attachments: 4,
        ..Default::default()
    };
    assert_eq!(
        TnefReader::with_options(input.as_slice(), &options)
            .unwrap()
            .load_message()
            .unwrap()
            .attachments()
            .len(),
        4
    );
    for limit in [0, 2] {
        let options = TnefReaderOptions {
            max_attachments: limit,
            ..Default::default()
        };
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        assert!(matches!(
            reader.load_message_with_logger(&mut |issue| {
                issues.push(issue);
                Ok(())
            }),
            Err(Error::AttachmentLimitExceeded)
        ));
        assert!(!reader.is_faulted());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::TooManyAttachments);
        assert_eq!(issues[0].attribute_tag, Some(Attribute::ATTACH_RENDER_DATA));
        while reader.read_next().unwrap() {}
    }
    let mut input = empty();
    attachment(&mut input, "small.bin", &[0; 100]);
    attachment(&mut input, "large.bin", &[0; 100]);
    let options = TnefReaderOptions {
        max_value_length: 100,
        max_total_data_bytes: 248,
        ..Default::default()
    };
    assert_eq!(
        TnefReader::with_options(input.as_slice(), &options)
            .unwrap()
            .load_message()
            .unwrap()
            .attachments()
            .len(),
        2
    );
    for options in [
        TnefReaderOptions {
            max_total_data_bytes: 247,
            ..Default::default()
        },
        TnefReaderOptions {
            max_value_length: 99,
            ..Default::default()
        },
    ] {
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        assert!(matches!(
            reader.load_message_with_logger(&mut |issue| {
                issues.push(issue);
                Ok(())
            }),
            Err(Error::ValueLimitExceeded)
        ));
        assert!(!reader.is_faulted());
        assert_eq!(
            issues.last().unwrap().violation,
            Violation::DataSizeLimitExceeded
        );
    }
}

#[test]
fn nested_limits_and_diagnostic_offsets_survive_detachment() {
    let inner = document(Attribute::SUBJECT, b"Inner\0");
    for kind in [Type::BINARY, Type::OBJECT] {
        let middle = document(
            Attribute::ATTACHMENT,
            &properties(&[
                object(
                    Tag::ATTACH_DATA_BIN.id(),
                    kind,
                    &[IID.as_slice(), &inner].concat(),
                ),
                integer(Tag::ATTACH_METHOD, 5),
            ]),
        );
        let input = wrap(&middle);
        let options = TnefReaderOptions {
            max_nesting_depth: 1,
            ..Default::default()
        };
        let message = TnefReader::with_options(input.as_slice(), &options)
            .unwrap()
            .load_message()
            .unwrap();
        let mut issues = Vec::new();
        assert!(matches!(
            message.attachments()[0].load_embedded_message_with_logger(&mut |issue| {
                issues.push(issue);
                Ok(())
            }),
            Err(Error::NestingLimitExceeded)
        ));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::NestingTooDeep);
        assert_eq!(issues[0].depth, 1);
        assert_eq!(issues[0].attribute_tag, Some(Attribute::ATTACHMENT));
        assert_eq!(
            issues[0].property_tag,
            Some(Tag::new(Tag::ATTACH_DATA_BIN.id(), kind))
        );
        assert!(issues[0].stream_offset > 16);
    }
}

#[test]
fn borrowed_message_loading_uses_parent_logger_and_budget() {
    let inner = document(Attribute::BODY, b"hello");
    let input = wrap(&inner);
    let options = TnefReaderOptions {
        max_total_data_bytes: 4,
        ..Default::default()
    };
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let mut pr = reader.open_properties_with_logger(&mut logger).unwrap();
    assert!(pr.read_next_property().unwrap());
    let mut child = pr.open_embedded_message().unwrap();
    assert!(matches!(
        child.load_message(),
        Err(Error::ValueLimitExceeded)
    ));
    assert!(!child.is_faulted());
    drop(child);
    drop(pr);
    drop(logger);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].depth, 1);
    assert_eq!(issues[0].violation, Violation::DataSizeLimitExceeded);
}

#[test]
fn invalid_headers_started_readers_and_source_ownership() {
    for input in [vec![], vec![1, 2, 3, 4, 0, 0], empty()[..5].to_vec()] {
        assert!(matches!(
            Message::load(input.as_slice()),
            Err(Error::InvalidMessageHeader)
        ));
    }
    let input = document(Attribute::SUBJECT, b"Hello\0");
    let mut reader = TnefReader::new(input.as_slice());
    assert!(reader.read_next().unwrap());
    assert!(matches!(
        reader.load_message(),
        Err(Error::ReaderAlreadyStarted)
    ));
    assert_eq!(reader.read_value_string().unwrap(), "Hello");
    let mut source = Cursor::new(&input);
    assert_eq!(
        Message::load(&mut source)
            .unwrap()
            .subject()
            .unwrap()
            .as_deref(),
        Some("Hello")
    );
    assert_eq!(source.position(), input.len() as u64);
    source.set_position(0);
    assert_eq!(
        Message::load(&mut source)
            .unwrap()
            .subject()
            .unwrap()
            .as_deref(),
        Some("Hello")
    );
    assert_eq!(load(&empty()).internet_message_id().unwrap(), None);
}

#[test]
fn source_and_logger_failures_fault_without_successful_models() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("source failure"))
        }
    }
    let mut reader = TnefReader::new(Broken);
    assert!(matches!(reader.load_message(), Err(Error::Io(_))));
    assert!(reader.is_faulted());
    assert!(matches!(reader.load_message(), Err(Error::Faulted)));
    let input = document(Attribute::BODY, b"complete");
    let mut reader = TnefReader::new(input[..input.len() - 1].chain(Broken));
    assert!(matches!(reader.load_message(), Err(Error::Io(_))));
    assert!(reader.is_faulted());
    let input = document(Attribute::MESSAGE_ID, b"0G\0");
    let mut reader = TnefReader::new(input.as_slice());
    assert!(matches!(
        reader.load_message_with_logger(&mut |_| Err(io::Error::other("logger failure"))),
        Err(Error::Io(_))
    ));
    assert!(reader.is_faulted());
    let mut input = document(Attribute::BODY, b"complete");
    *input.last_mut().unwrap() ^= 1;
    let mut reader = TnefReader::new(input.as_slice());
    assert!(matches!(
        reader.load_message_with_logger(&mut |_| Err(io::Error::other("checksum"))),
        Err(Error::Io(_))
    ));
    assert!(reader.is_faulted());
}

#[test]
fn embedded_loading_is_lazy_and_snapshots_charset_at_attachment_completion() {
    let inner = document(Attribute::BODY, b"caf\xe9");
    let data = properties(&[object(
        Tag::ATTACH_DATA_OBJ.id(),
        Type::OBJECT,
        &[IID.as_slice(), &inner].concat(),
    )]);
    let mut input = empty();
    attr(&mut input, Attribute::ATTACHMENT, &data);
    attr(&mut input, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
    attr(&mut input, Attribute::ATTACHMENT, &data);
    let codepage_offset = input.len() as u64;
    attr(&mut input, Attribute::OEM_CODEPAGE, &1251u32.to_le_bytes());
    let mut issues = Vec::new();
    let message = TnefReader::new(input.as_slice())
        .load_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(
        issues[0].violation,
        Violation::MessageAttributeAfterAttachment
    );
    assert_eq!(issues[0].stream_offset, codepage_offset);
    assert_eq!(message.codepage(), 1251);
    for (attachment, (page, text)) in message
        .attachments()
        .iter()
        .zip([(1252, "caf\u{e9}"), (1251, "caf\u{439}")])
    {
        let child = attachment.load_embedded_message().unwrap();
        assert_eq!(child.codepage(), page);
        assert_eq!(child.text_body().unwrap().text().unwrap(), text);
    }
    let message = load(&wrap(b"not a TNEF header"));
    assert!(message.attachments()[0].is_embedded_message());
    assert!(matches!(
        message.attachments()[0].load_embedded_message(),
        Err(Error::InvalidMessageHeader)
    ));
}

#[test]
fn attachment_and_depth_limit_logger_failures_fault_the_reader() {
    let inner = document(Attribute::SUBJECT, b"Inner\0");
    let input = wrap(&inner);
    for (options, expected) in [
        (
            TnefReaderOptions {
                max_attachments: 0,
                ..Default::default()
            },
            Violation::TooManyAttachments,
        ),
        (
            TnefReaderOptions {
                max_nesting_depth: 0,
                ..Default::default()
            },
            Violation::NestingTooDeep,
        ),
    ] {
        let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
        let mut issues = Vec::new();
        assert!(matches!(
            reader.load_message_with_logger(&mut |issue| {
                issues.push(issue);
                Err(io::Error::other("limit logger failure"))
            }),
            Err(Error::Io(_))
        ));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, expected);
        assert!(reader.is_faulted());
    }
    let options = TnefReaderOptions {
        max_attachments: 0,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(input.as_slice(), &options).unwrap();
    while reader
        .read_next_with_logger(&mut |issue| panic!("{issue:?}"))
        .unwrap()
    {}
    assert!(!reader.is_faulted());
}

#[test]
fn loader_does_not_hide_metadata_conversion_failures() {
    let input = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[integer(Tag::new(Tag::SUBJECT_W.id(), Type::LONG), 7)]),
    );
    assert_eq!(load(&input).subject(), Err(ConversionError::InvalidType));
    let input = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[string(
            Tag::new(Tag::INTERNET_CODEPAGE.id(), Type::UNICODE),
            "bad",
        )]),
    );
    assert!(matches!(
        Message::load(input.as_slice()),
        Err(Error::ValueConversion(ConversionError::InvalidType))
    ));
    let mut input = document(Attribute::OWNER, &owner("Bob", "SMTP:bob@example.com"));
    attr(
        &mut input,
        Attribute::MAPI_PROPERTIES,
        &properties(&[integer(Tag::new(Tag::MESSAGE_CLASS_W.id(), Type::LONG), 7)]),
    );
    assert!(matches!(
        Message::load(input.as_slice()),
        Err(Error::ValueConversion(ConversionError::InvalidType))
    ));
}

#[test]
fn partial_content_requires_logged_truncation_and_preserves_actual_bytes() {
    let mut input = document(Attribute::BODY, b"truncated");
    input.truncate(input.len() - 5);
    let mut issues = Vec::new();
    let message = TnefReader::new(input.as_slice())
        .load_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert_eq!(message.text_body().unwrap().as_bytes(), b"trunca");
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.violation == Violation::TruncatedStream)
            .count(),
        1
    );
}

fn normalized(bytes: Vec<u8>, path: &Path) -> Vec<u8> {
    let text = matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("cfg" | "dat" | "htm" | "ini" | "src" | "html" | "rtf" | "txt")
    ) || matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("AUTHORS" | "README")
    );
    if !text {
        return bytes;
    }
    let mut filter = mimecraft::filters::Dos2UnixFilter::with_ensure_new_line(true);
    let mut output = Vec::new();
    filter.filter(&bytes, &mut output);
    filter.finish(&mut output);
    output
}

fn count_embedded_messages(message: &Message) -> usize {
    message
        .attachments()
        .iter()
        .filter(|attachment| attachment.is_embedded_message())
        .map(|attachment| {
            let child = attachment.load_embedded_message().unwrap();
            assert!(child.message_class().unwrap().is_some());
            1 + count_embedded_messages(&child)
        })
        .sum()
}

#[test]
fn upstream_corpus_messages_match_expected_names_and_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut fixtures: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tnef"))
        .collect();
    fixtures.sort();
    assert_eq!(fixtures.len(), 19);
    for path in fixtures {
        let input = std::fs::read(&path).unwrap();
        let mut issues = Vec::new();
        let message = TnefReader::new(input.as_slice())
            .load_message_with_logger(&mut |issue| {
                issues.push(issue);
                Ok(())
            })
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let names = std::fs::read_to_string(path.with_extension("list")).unwrap();
        // Every named attachment in the expectation list must be present.
        for name in names
            .lines()
            .filter(|name| !name.starts_with("body.") && *name != "Untitled Attachment")
        {
            assert!(
                message
                    .attachments()
                    .iter()
                    .any(|a| a.file_name().unwrap().as_deref() == Some(name)),
                "{}: {name}",
                path.display()
            );
        }
        let folder = path.with_extension("");
        // The bodies must match the expected (decoded) content.
        for (listed, expected, body, decoded) in [
            ("body.rtf", "message.rtf", message.rtf_body(), true),
            ("body.html", "message.html", message.html_body(), false),
        ] {
            if names.lines().any(|name| name == listed) {
                let body = body.expect("expected body present");
                let target = folder.join(expected);
                if target.exists() {
                    let actual = if decoded {
                        let mut bytes = Vec::new();
                        body.open_decoded_read().read_to_end(&mut bytes).unwrap();
                        bytes
                    } else {
                        body.as_bytes().to_vec()
                    };
                    assert_eq!(
                        normalized(actual, &target),
                        normalized(std::fs::read(&target).unwrap(), &target),
                        "{}",
                        target.display()
                    );
                }
            }
        }
        let embedded = message
            .attachments()
            .iter()
            .filter(|attachment| attachment.is_embedded_message())
            .count();
        // Untitled attachments have numbered "Untitled Attachment.N" references.
        let mut untitled = 0;
        // The attachment content must match the expected content.
        for attachment in message.attachments() {
            if attachment.is_embedded_message() {
                continue;
            }
            let Some(name) = attachment.file_name().unwrap() else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let name = if name == "Untitled Attachment" {
                untitled += 1;
                format!("Untitled Attachment.{untitled}")
            } else {
                name.into_owned()
            };
            let target = folder.join(name);
            if target.exists() {
                assert_eq!(
                    normalized(attachment.as_bytes().unwrap_or_default().to_vec(), &target),
                    normalized(std::fs::read(&target).unwrap(), &target),
                    "{}",
                    target.display()
                );
            }
        }
        assert_eq!(
            embedded,
            if path.file_stem().unwrap() == "christmas" {
                2
            } else {
                0
            }
        );
        assert_eq!(count_embedded_messages(&message), embedded);
        if path.file_stem().unwrap() == "christmas" {
            assert!(issues.is_empty(), "{issues:?}");
        }
    }
}

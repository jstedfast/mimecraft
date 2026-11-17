// Adapted from MimeKit's TnefMessageConverterTests header/address/body cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::{
    ContentEncoding, MimeEntity, MimeMessage, MimeParser,
    tnef::{TnefConversionError, TnefConversionLossKind as Loss},
};

#[path = "advanced_conversion.rs"]
mod advanced;
#[path = "inline_conversion.rs"]
mod inline;
#[path = "rtf_conversion.rs"]
mod rtf;
#[path = "rtf_images.rs"]
mod rtf_images;

fn message(fields: &[Vec<u8>]) -> Message {
    load(&document(Attribute::MAPI_PROPERTIES, &properties(fields)))
}

fn named(index: u16, field: &str, values: &[&str]) -> Vec<u8> {
    let kind = if values.len() == 1 {
        Type::UNICODE
    } else {
        Type(Type::UNICODE.0 | Type::MULTI_VALUED.0)
    };
    let mut bytes = Tag::new(0x8000 + index, kind).0.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0x86, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    dword(&mut bytes, 1);
    variable(&mut bytes, &unicode(&format!("{field}\0")));
    dword(&mut bytes, values.len() as i32);
    for value in values {
        variable(&mut bytes, &unicode(&format!("{value}\0")));
    }
    bytes
}

fn raw_body(entity: &MimeEntity) -> &[u8] {
    let MimeEntity::Part(part) = entity else {
        panic!("expected MIME leaf");
    };
    part.content().get_ref().get_ref()
}

fn converted(fields: &[Vec<u8>]) -> MimeMessage {
    let result = message(fields).to_mime_message().unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    result.message
}

#[test]
fn upstream_received_headers_precede_mapped_and_named_headers() {
    let message = converted(&[
        string(Tag::SUBJECT_W, "Subject"),
        string(
            Tag::TRANSPORT_MESSAGE_HEADERS_W,
            "\r\nReceived: from a.example.com\r\nSubject: ignored\r\nX-Transport: ignored\r\nReceived: from c.example.com\r\n\tby a.example.com\r\n",
        ),
        named(1, "X-Custom", &["custom value"]),
    ]);
    let headers: Vec<_> = message.headers().iter().collect();
    assert_eq!(headers[0].field(), Some("Received"));
    assert_eq!(headers[1].field(), Some("Received"));
    assert_eq!(
        headers[1].raw_value(),
        b" from c.example.com\r\n\tby a.example.com\r\n"
    );
    assert_eq!(headers[2].field(), Some("Subject"));
    assert_eq!(headers[3].field(), Some("X-Custom"));
    assert!(!message.headers().contains("X-Transport"));
    assert_eq!(message.subject().unwrap().text, "Subject");
}

#[test]
fn upstream_named_headers_preserve_multivalues_and_do_not_override_structure() {
    let result = message(&[
        string(Tag::SUBJECT_W, "Subject"),
        named(1, "X-Custom", &["custom value"]),
        named(2, "X-Multi", &["one", "two"]),
        named(3, "sUbJeCt", &["Duplicate subject"]),
        named(4, "Content-Type", &["multipart/mixed; boundary=x"]),
        named(5, "MIME-Version", &["1.0"]),
        named(6, "X-MS-Exchange-Organization-SCL", &["1"]),
        named(7, "X-Microsoft-Exchange-Forest-Thing", &["1"]),
        named(8, "Bad Field", &["value"]),
        named(9, "X-Body", &["safe\r\nInjected: no"]),
    ])
    .to_mime_message()
    .unwrap();
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidHeader);
    let mut message = result.message;
    assert_eq!(
        message
            .headers()
            .get_all("X-Multi")
            .map(|header| header.decode_value().text)
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(message.headers().get_all("Subject").count(), 1);
    for field in [
        "Content-Type",
        "MIME-Version",
        "X-MS-Exchange-Organization-SCL",
        "X-Microsoft-Exchange-Forest-Thing",
        "Bad Field",
        "Injected",
    ] {
        assert!(!message.headers().contains(field), "{field}");
    }
    let mut bytes = Vec::new();
    message.write_to(&mut bytes).unwrap();
    let parsed = MimeParser::new(bytes.as_slice()).parse_message().unwrap();
    assert!(!parsed.headers().contains("Injected"));
}

#[test]
fn named_address_headers_encode_names_without_hiding_mailbox_or_group_syntax() {
    use mimecraft::{Address, AddressList};

    for field in [
        "fRoM",
        "To",
        "Cc",
        "Bcc",
        "Sender",
        "Reply-To",
        "Resent-From",
        "Resent-To",
        "Resent-Cc",
        "Resent-Bcc",
        "Resent-Sender",
        "Resent-Reply-To",
        "Disposition-Notification-To",
    ] {
        let values = [
            "\"Doe, J\u{f6}hn\" <john@example.com>",
            "\"Gr\u{fc}ppe\": Ren\u{e9}e <renee@example.com>;",
        ];
        let mut message = converted(&[named(1, field, &values)]);
        let mut wire = Vec::new();
        message.write_to(&mut wire).unwrap();
        assert!(wire.is_ascii());
        let parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
        let headers: Vec<_> = parsed.headers().get_all(field).collect();
        assert_eq!(headers.len(), 2, "{field}");
        let addresses = AddressList::parse(headers[0].raw_value()).unwrap();
        let mailbox = addresses.mailboxes().next().unwrap();
        assert_eq!(mailbox.display_name(), "Doe, J\u{f6}hn", "{field}");
        assert_eq!(mailbox.address().to_string(), "john@example.com", "{field}");
        let addresses = AddressList::parse(headers[1].raw_value()).unwrap();
        let Address::Group(group) = &addresses.as_slice()[0] else {
            panic!("{field}: group syntax was lost");
        };
        assert_eq!(group.display_name(), "Gr\u{fc}ppe");
        let mailbox = group.members().mailboxes().next().unwrap();
        assert_eq!(mailbox.display_name(), "Ren\u{e9}e");
        assert_eq!(mailbox.address().to_string(), "renee@example.com");
    }
}

#[test]
fn named_address_header_fallback_and_formatting_failures_are_reported() {
    let mut result = message(&[
        named(1, "From", &["<bad@@>"]),
        named(2, "To", &["J\u{f6}hn <caf\u{e9}@example.com>"]),
        named(3, "Cc", &[" \r\n "]),
        named(4, "Reply-To", &[" J\u{f6}hn\r\n <john@example.com> "]),
    ])
    .to_mime_message()
    .unwrap();
    assert_eq!(result.losses.len(), 2, "{:?}", result.losses);
    assert!(
        result
            .losses
            .iter()
            .all(|loss| loss.kind == Loss::InvalidHeader)
    );
    assert_eq!(
        result
            .message
            .headers()
            .get("From")
            .unwrap()
            .decode_value()
            .text,
        "<bad@@>"
    );
    assert!(!result.message.headers().contains("To"));
    assert_eq!(
        result
            .message
            .headers()
            .get("Cc")
            .unwrap()
            .decode_value()
            .text,
        ""
    );
    let mut wire = Vec::new();
    result.message.write_to(&mut wire).unwrap();
    let parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    let addresses =
        mimecraft::AddressList::parse(parsed.headers().get("Reply-To").unwrap().raw_value())
            .unwrap();
    let mailbox = addresses.mailboxes().next().unwrap();
    assert_eq!(mailbox.display_name(), "J\u{f6}hn");
    assert_eq!(mailbox.address().to_string(), "john@example.com");
}

#[test]
fn upstream_long_subject_conversion_respects_parser_budgets_and_round_trips() {
    use mimecraft::{
        HeaderTooLong, MimeEntityParseOptions, MimePartWriteOptions, encodings::NewLineFormat,
    };

    let subject = "\u{e9}".repeat(6000);
    let mut input = empty();
    attr(&mut input, Attribute::OEM_CODEPAGE, &1252u32.to_le_bytes());
    let mut value = vec![0xe9; 6000];
    value.push(0);
    attr(&mut input, Attribute::SUBJECT, &value);
    let mut result = load(&input).to_mime_message().unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    assert_eq!(result.message.subject().unwrap().text, subject);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let options = MimePartWriteOptions {
            new_line_format: format,
            ..Default::default()
        };
        let mut wire = Vec::new();
        result
            .message
            .write_to_with_options(&mut wire, &options)
            .unwrap();
        // A generated RFC 2047 header can exceed the parser's default input budget.
        assert!(wire.len() > MimeEntityParseOptions::default().max_header_length);
        let error = MimeParser::new(wire.as_slice())
            .parse_message()
            .unwrap_err();
        assert!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<HeaderTooLong>()
                .is_some()
        );
        let mut parsed = MimeParser::with_options(
            wire.as_slice(),
            MimeEntityParseOptions {
                max_header_length: wire.len(),
                ..Default::default()
            },
        )
        .unwrap()
        .parse_message()
        .unwrap();
        assert_eq!(parsed.subject().unwrap().text, subject);
        let mut second = Vec::new();
        parsed.write_to_with_options(&mut second, &options).unwrap();
        assert_eq!(second, wire);
    }
}

#[test]
fn upstream_originator_precedence_and_case_insensitive_sender_suppression() {
    for (sender, expected) in [("assistant@example.com", true), ("BOSS@example.com", false)] {
        let message = converted(&[
            string(Tag::SENT_REPRESENTING_NAME_W, "Boss"),
            string(Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W, "boss@example.com"),
            string(Tag::SENDER_NAME_W, "Assistant"),
            string(Tag::SENDER_EMAIL_ADDRESS_W, sender),
        ]);
        let from = message.from();
        let mailbox = from.addresses.mailboxes().next().unwrap();
        assert_eq!(mailbox.display_name(), "Boss");
        assert_eq!(mailbox.address().to_string(), "boss@example.com");
        assert_eq!(message.sender().unwrap().is_some(), expected);
        assert_eq!(
            message.headers().iter().next().unwrap().field(),
            Some("From")
        );
    }
    let message = converted(&[
        string(Tag::SENDER_NAME_W, "Alice"),
        string(Tag::SENDER_ADDRTYPE_W, "EX"),
        string(Tag::SENDER_EMAIL_ADDRESS_W, "/O=EXAMPLE/CN=ALICE"),
        string(Tag::SENDER_SMTP_ADDRESS_W, "alice@example.com"),
    ]);
    assert_eq!(
        message
            .from()
            .addresses
            .mailboxes()
            .next()
            .unwrap()
            .address()
            .to_string(),
        "alice@example.com"
    );
    assert!(message.sender().unwrap().is_none());
}

#[test]
fn upstream_recipients_and_non_smtp_loss_keep_table_order() {
    let rows = [
        (1, "to@example.com", "SMTP"),
        (2, "cc@example.com", "SMTP"),
        (3, "bcc@example.com", "SMTP"),
        (0, "ignored@example.com", "SMTP"),
        (1, "/O=EXAMPLE/CN=ALICE", "EX"),
    ];
    let mut table = (rows.len() as u32).to_le_bytes().to_vec();
    for (kind, address, addrtype) in rows {
        table.extend(properties(&[
            integer(Tag::RECIPIENT_TYPE, kind),
            string(Tag::EMAIL_ADDRESS_W, address),
            string(Tag::ADDRTYPE_W, addrtype),
        ]));
    }
    let result = load(&document(Attribute::RECIPIENT_TABLE, &table))
        .to_mime_message()
        .unwrap();
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::UnparsableRecipient);
    for (addresses, expected) in [
        (result.message.to(), "to@example.com"),
        (result.message.cc(), "cc@example.com"),
        (result.message.bcc(), "bcc@example.com"),
    ] {
        assert_eq!(
            addresses
                .addresses
                .mailboxes()
                .map(|mailbox| mailbox.address().to_string())
                .collect::<Vec<_>>(),
            [expected]
        );
    }
}

#[test]
fn search_key_and_received_by_fallback_do_not_replace_an_existing_table() {
    let fields = [
        string(Tag::SENDER_SEARCH_KEY, "sMtP:sender@example.com\0"),
        string(Tag::RECEIVED_BY_EMAIL_ADDRESS_W, "delivered@example.com"),
    ];
    let message = converted(&fields);
    assert_eq!(
        message
            .from()
            .addresses
            .mailboxes()
            .next()
            .unwrap()
            .address()
            .to_string(),
        "sender@example.com"
    );
    assert_eq!(
        message
            .to()
            .addresses
            .mailboxes()
            .next()
            .unwrap()
            .address()
            .to_string(),
        "delivered@example.com"
    );
    let mut bytes = document(Attribute::MAPI_PROPERTIES, &properties(&fields));
    let row = properties(&[integer(Tag::RECIPIENT_TYPE, 0)]);
    attr(
        &mut bytes,
        Attribute::RECIPIENT_TABLE,
        &[1u32.to_le_bytes().as_slice(), &row].concat(),
    );
    let result = load(&bytes).to_mime_message().unwrap();
    assert!(result.message.to().addresses.is_empty());
}

#[test]
fn upstream_subject_date_priority_and_no_fabricated_headers() {
    let bare = converted(&[string(Tag::BODY_W, "Hello")]);
    assert!(bare.headers().is_empty());
    assert_eq!(raw_body(bare.body().unwrap()), b"Hello");
    for subject in [None, Some(""), Some("Visible subject")] {
        let mut fields = vec![
            string(Tag::SUBJECT_PREFIX_W, "Re: "),
            string(Tag::NORMALIZED_SUBJECT_W, "Normalized"),
            integer(Tag::IMPORTANCE, 0),
            integer(Tag::PRIORITY, -1),
            integer(Tag::SENSITIVITY, 2),
            fixed(
                Tag::CLIENT_SUBMIT_TIME.id(),
                Type::SYS_TIME,
                &TnefDateTime::new(2024, 1, 2, 3, 4, 5, TnefDateTimeKind::Utc)
                    .unwrap()
                    .to_file_time()
                    .unwrap()
                    .to_le_bytes(),
            ),
        ];
        if let Some(subject) = subject {
            fields.push(string(Tag::SUBJECT_W, subject));
        }
        let message = converted(&fields);
        assert_eq!(
            message.subject().unwrap().text,
            subject.unwrap_or("Re: Normalized")
        );
        assert_eq!(
            message
                .headers()
                .get("Sensitivity")
                .unwrap()
                .decode_value()
                .text,
            "Private"
        );
        assert!(
            message
                .headers()
                .get("Date")
                .unwrap()
                .decode_value()
                .text
                .ends_with("+0000")
        );
        assert_eq!(
            message
                .headers()
                .get("Importance")
                .unwrap()
                .decode_value()
                .text,
            "low"
        );
        assert_eq!(
            message
                .headers()
                .get("Priority")
                .unwrap()
                .decode_value()
                .text,
            "non-urgent"
        );
    }
}

#[test]
fn upstream_ids_and_references_are_validated_before_authoring() {
    let result = message(&[
        string(Tag::INTERNET_MESSAGE_ID_W, "<bad\0id@example.com>"),
        string(Tag::TNEF_CORRELATION_KEY, "<fallback@example.com>\0"),
        string(Tag::IN_REPLY_TO_ID_W, " <parent@example.com> "),
        string(
            Tag::INTERNET_REFERENCES_W,
            "<root@example.com> <bad\0id@example.com> <> <parent@example.com>",
        ),
    ])
    .to_mime_message()
    .unwrap();
    // Rust's References iterator also reports the empty identifier; MimeKit's
    // EnumerateReferences skips it. Both retain the two valid references.
    assert_eq!(result.losses.len(), 3);
    assert!(
        result
            .losses
            .iter()
            .all(|loss| loss.kind == Loss::InvalidMessageId)
    );
    assert_eq!(
        result.message.message_id().unwrap().unwrap().as_str(),
        "fallback@example.com"
    );
    assert_eq!(
        result.message.in_reply_to().unwrap().unwrap().as_str(),
        "parent@example.com"
    );
    assert_eq!(
        result
            .message
            .references()
            .map(|id| id.unwrap().into_string())
            .collect::<Vec<_>>(),
        ["root@example.com", "parent@example.com"]
    );
}

#[test]
fn body_alternatives_attachments_and_owned_output_round_trip() {
    let mut bytes = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            string(Tag::BODY_W, "caf\u{e9}"),
            string(Tag::BODY_HTML_W, "<b>caf\u{e9}</b>"),
        ]),
    );
    attachment(&mut bytes, "one.bin", b"\0\xffabc");
    attachment(&mut bytes, "two.bin", b"second");
    let source = load(&bytes);
    let result = source.to_mime_message().unwrap();
    assert_eq!(source.text_body().unwrap().text().unwrap(), "caf\u{e9}");
    drop(source);
    assert!(result.losses.is_empty());
    let mut message = result.message;
    let MimeEntity::Multipart(mixed) = message.body().unwrap() else {
        panic!("mixed");
    };
    assert_eq!(mixed.children().len(), 3);
    let MimeEntity::Multipart(alternative) = &mixed.children()[0] else {
        panic!("alternative");
    };
    assert_eq!(
        alternative.content_type().unwrap().unwrap().mime_type(),
        "multipart/alternative"
    );
    assert_eq!(raw_body(&alternative.children()[0]), "caf\u{e9}".as_bytes());
    assert_eq!(
        raw_body(&alternative.children()[1]),
        "<b>caf\u{e9}</b>".as_bytes()
    );
    assert_eq!(raw_body(&mixed.children()[1]), b"\0\xffabc");
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let mut parsed = MimeParser::new(wire.as_slice()).parse_message().unwrap();
    let MimeEntity::Multipart(mixed) = parsed.body_mut().unwrap() else {
        panic!("mixed");
    };
    for (child, expected) in mixed.children_mut()[1..]
        .iter_mut()
        .zip([b"\0\xffabc".as_slice(), b"second"])
    {
        let MimeEntity::Part(part) = child else {
            panic!("leaf");
        };
        let mut decoded = Vec::new();
        part.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, expected);
    }
}

#[test]
fn upstream_attachment_without_body_still_uses_mixed_and_losses_keep_indices() {
    let mut bytes = empty();
    attr(&mut bytes, Attribute::ATTACH_RENDER_DATA, &render(1, 0));
    attachment(&mut bytes, "one.bin", b"data");
    let result = load(&bytes).to_mime_message().unwrap();
    assert_eq!(result.losses[0].kind, Loss::AttachmentWithoutContent);
    assert_eq!(result.losses[0].attachment_index, Some(0));
    let MimeEntity::Multipart(mixed) = result.message.body().unwrap() else {
        panic!("mixed");
    };
    assert_eq!(mixed.children().len(), 1);
    assert_eq!(raw_body(&mixed.children()[0]), b"data");
}

#[test]
fn upstream_html_meta_charset_overrides_only_when_usable_and_in_head() {
    for (meta, expected) in [
        ("<meta charset=windows-1251>", "windows-1251"),
        (
            "<meta http-equiv=Content-Type content=\"text/html; charset=windows-1251\">",
            "windows-1251",
        ),
        ("<meta charset=utf-8>", "windows-1252"),
        ("<meta charset=x-unknown>", "windows-1252"),
        ("</head><meta charset=windows-1251>", "windows-1252"),
        ("<body><meta charset=windows-1251>", "windows-1252"),
    ] {
        let mut html = format!("<html><head>{meta}</head><body>").into_bytes();
        html.extend_from_slice(b"\xcf\xf0\xe8\xe2\xe5\xf2</body></html>");
        let message = converted(&[
            object(Tag::BODY_HTML_B.id(), Type::BINARY, &html),
            integer(Tag::INTERNET_CODEPAGE, 1252),
        ]);
        let MimeEntity::Part(part) = message.body().unwrap() else {
            panic!("leaf");
        };
        assert_eq!(
            part.content_type().unwrap().unwrap().charset(),
            Some(expected),
            "{meta}"
        );
        assert_eq!(part.content().get_ref().get_ref(), &html);
        assert_ne!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::SevenBit
        );
    }
}

#[test]
fn unknown_rtf_and_invalid_skeleton_are_reported_instead_of_silently_discarded() {
    let mut unknown = vec![0; 16];
    unknown[..4].copy_from_slice(&12i32.to_le_bytes());
    let result = message(&[
        string(Tag::BODY_W, "plain"),
        object(Tag::RTF_COMPRESSED.id(), Type::BINARY, &unknown),
        object(Tag::MIME_SKELETON.id(), Type::BINARY, b"not restored"),
    ])
    .to_mime_message()
    .unwrap();
    assert_eq!(
        result
            .losses
            .iter()
            .map(|loss| loss.kind)
            .collect::<Vec<_>>(),
        [Loss::InvalidRtfBody, Loss::InvalidMimeSkeleton]
    );
    assert_eq!(raw_body(result.message.body().unwrap()), b"plain");
}

#[test]
fn invalid_typed_properties_return_errors_not_missing_headers() {
    let source = message(&[integer(Tag::new(Tag::SUBJECT_W.id(), Type::LONG), 42)]);
    assert!(matches!(
        source.to_mime_message(),
        Err(TnefConversionError::Property(_))
    ));
}

#[test]
fn upstream_flag_request_and_non_string_internet_headers() {
    let mut flag = Tag::new(0x8001, Type::UNICODE).0.to_le_bytes().to_vec();
    flag.extend_from_slice(&[8, 32, 6, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    dword(&mut flag, 0);
    dword(&mut flag, 0x8530);
    dword(&mut flag, 1);
    variable(&mut flag, &unicode("Follow up\0"));

    let mut number = Tag::new(0x8002, Type::LONG).0.to_le_bytes().to_vec();
    number.extend_from_slice(&[0x86, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    dword(&mut number, 1);
    variable(&mut number, &unicode("X-Number\0"));
    dword(&mut number, 42);
    let result = message(&[
        flag,
        number,
        named(3, "X-Message-Flag", &["must not replace"]),
        named(4, "X-Empty", &[]),
    ])
    .to_mime_message()
    .unwrap();
    assert_eq!(
        result
            .message
            .headers()
            .get("X-Message-Flag")
            .unwrap()
            .decode_value()
            .text,
        "Follow up"
    );
    assert_eq!(
        result.message.headers().get_all("X-Message-Flag").count(),
        1
    );
    assert!(!result.message.headers().contains("X-Number"));
    assert!(!result.message.headers().contains("X-Empty"));
    assert_eq!(result.losses.len(), 2);
    assert!(
        result
            .losses
            .iter()
            .all(|loss| loss.kind == Loss::InvalidHeader)
    );
}

#[test]
fn malformed_unicode_bodies_fail_without_partial_output() {
    for bytes in [b"\0\xd8".as_slice(), b"a"] {
        let source = message(&[object(Tag::BODY_W.id(), Type::UNICODE, bytes)]);
        assert!(matches!(
            source.to_mime_message(),
            Err(TnefConversionError::Body(_))
        ));
    }
}

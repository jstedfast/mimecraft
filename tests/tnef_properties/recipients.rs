// Adapted from MimeKit's TnefMessageTests.TestRecipients and TnefPropertySetTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use super::*;
use mimecraft::tnef::{
    TnefPropertySet, TnefRecipient, TnefRecipientType as RecipientType,
    TnefValueConversionError as ConversionError,
};

fn string(tag: Tag, value: &str) -> Vec<u8> {
    let bytes = if tag.value_type() == Type::UNICODE {
        unicode(&format!("{value}\0"))
    } else {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    let mut item = text(tag.value_type(), &bytes);
    item[..4].copy_from_slice(&tag.0.to_le_bytes());
    item
}

fn table(rows: &[Vec<Vec<u8>>]) -> Vec<u8> {
    let mut payload = (rows.len() as i32).to_le_bytes().to_vec();
    for row in rows {
        payload.extend(properties(row));
    }
    document(Attribute::RECIPIENT_TABLE, &payload)
}

fn read_recipients(data: &[u8], chunk: usize) -> Vec<TnefRecipient> {
    let mut reader = TnefReader::new(Chunked { input: data, chunk });
    let mut logger = |issue| panic!("unexpected issue: {issue:?}");
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let rows = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_rows_as_property_sets()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    rows.into_iter().map(TnefRecipient::from).collect()
}

#[test]
fn upstream_recipient_rows_preserve_order_fields_and_resent_flags() {
    let data = table(&[
        vec![
            string(Tag::DISPLAY_NAME_W, "Alice"),
            string(Tag::EMAIL_ADDRESS_W, "alice@example.com"),
            string(Tag::ADDRTYPE_W, "SMTP"),
            fixed(Tag::RECIPIENT_TYPE.id(), Type::LONG, &1i32.to_le_bytes()),
        ],
        vec![
            string(Tag::DISPLAY_NAME_W, "Bob"),
            // The high bits of PidTagRecipientType are flags.
            fixed(
                Tag::RECIPIENT_TYPE.id(),
                Type::LONG,
                &0x10000003i32.to_le_bytes(),
            ),
        ],
    ]);
    for chunk in [1, 7, 4096] {
        let recipients = read_recipients(&data, chunk);
        assert_eq!(recipients.len(), 2);
        let alice = &recipients[0];
        assert_eq!(alice.recipient_type(), Ok(RecipientType::To));
        assert_eq!(alice.display_name().unwrap().as_deref(), Some("Alice"));
        assert_eq!(
            alice.email_address().unwrap().as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(alice.address_type().unwrap().as_deref(), Some("SMTP"));
        assert_eq!(alice.properties().len(), 4);
        let bob = &recipients[1];
        assert_eq!(bob.recipient_type(), Ok(RecipientType::Bcc));
        assert_eq!(bob.display_name().unwrap().as_deref(), Some("Bob"));
        assert_eq!(bob.email_address(), Ok(None));
        assert_eq!(bob.address_type(), Ok(None));
        assert_eq!(
            bob.properties().get_i32(Tag::RECIPIENT_TYPE),
            Ok(Some(0x10000003))
        );
    }
}

#[test]
fn recipient_types_roundtrip_unknown_values_and_strip_only_high_flags() {
    for (number, expected) in [
        (0, RecipientType::Originator),
        (1, RecipientType::To),
        (2, RecipientType::Cc),
        (3, RecipientType::Bcc),
        (4, RecipientType::Unknown(4)),
        (0x0fffffff, RecipientType::Unknown(0x0fffffff)),
    ] {
        assert_eq!(RecipientType::from(number), expected);
        assert_eq!(i32::from(expected), number);
        for flags in [0, 0x10000000, 0x80000000, 0xf0000000] {
            let raw = number as u32 | flags;
            let data = table(&[vec![fixed(
                Tag::RECIPIENT_TYPE.id(),
                Type::LONG,
                &raw.to_le_bytes(),
            )]]);
            let recipients = read_recipients(&data, 1);
            assert_eq!(recipients[0].recipient_type(), Ok(expected));
            assert_eq!(
                recipients[0].properties().get_i32(Tag::RECIPIENT_TYPE),
                Ok(Some(raw as i32))
            );
        }
    }
    for number in [i32::MIN, -1, i32::MAX, 0x10000003] {
        assert_eq!(RecipientType::from(number), RecipientType::Unknown(number));
        assert_eq!(i32::from(RecipientType::from(number)), number);
    }
}

#[test]
fn missing_fields_default_without_inventing_names_or_addresses() {
    let recipient = TnefRecipient::from(TnefPropertySet::new());
    assert_eq!(recipient.recipient_type(), Ok(RecipientType::To));
    assert_eq!(recipient.display_name(), Ok(None));
    assert_eq!(recipient.email_address(), Ok(None));
    assert_eq!(recipient.address_type(), Ok(None));
    assert!(recipient.properties().is_empty());
    let recipients = read_recipients(&table(&[vec![], vec![]]), 1);
    assert_eq!(recipients, [recipient.clone(), recipient]);
}

#[test]
fn name_priority_is_independent_of_wire_order_and_empty_strings_do_not_fall_back() {
    let data = table(&[
        vec![
            string(Tag::DISPLAY_NAME_A, "third"),
            string(Tag::TRANSMITABLE_DISPLAY_NAME_W, "second"),
            string(Tag::RECIPIENT_DISPLAY_NAME_A, " first "),
        ],
        vec![
            string(Tag::DISPLAY_NAME_W, "third"),
            string(Tag::TRANSMITABLE_DISPLAY_NAME_A, "second"),
        ],
        vec![string(Tag::DISPLAY_NAME_A, "third")],
        vec![
            string(Tag::DISPLAY_NAME_W, "third"),
            string(Tag::RECIPIENT_DISPLAY_NAME_W, ""),
        ],
        vec![
            string(Tag::RECIPIENT_DISPLAY_NAME_A, "first duplicate"),
            string(Tag::RECIPIENT_DISPLAY_NAME_W, "second duplicate"),
        ],
    ]);
    for chunk in [1, 7, 4096] {
        let recipients = read_recipients(&data, chunk);
        for (recipient, expected) in
            recipients
                .iter()
                .zip([" first ", "second", "third", "", "first duplicate"])
        {
            assert_eq!(
                recipient.display_name().unwrap(),
                Some(Cow::Borrowed(expected))
            );
            assert!(matches!(
                recipient.display_name().unwrap(),
                Some(Cow::Borrowed(_))
            ));
        }
        assert_eq!(recipients[4].properties().len(), 2);
    }
}

#[test]
fn smtp_address_overrides_email_but_not_stored_address_type() {
    let data = table(&[
        vec![
            string(Tag::EMAIL_ADDRESS_W, "/O=Example/CN=Alice"),
            string(Tag::ADDRTYPE_A, "EX"),
            string(Tag::SMTP_ADDRESS_A, " Alice@Example.COM "),
        ],
        vec![
            string(Tag::EMAIL_ADDRESS_A, "/O=Example/CN=Bob"),
            string(Tag::ADDRTYPE_W, "EX"),
        ],
        vec![
            string(Tag::EMAIL_ADDRESS_W, "fallback@example.com"),
            string(Tag::SMTP_ADDRESS_W, ""),
        ],
        vec![
            string(Tag::SMTP_ADDRESS_W, "first@example.com"),
            string(Tag::SMTP_ADDRESS_A, "second@example.com"),
        ],
    ]);
    let recipients = read_recipients(&data, 7);
    assert_eq!(
        recipients[0].email_address().unwrap().as_deref(),
        Some(" Alice@Example.COM ")
    );
    assert_eq!(recipients[0].address_type().unwrap().as_deref(), Some("EX"));
    assert_eq!(
        recipients[1].email_address().unwrap().as_deref(),
        Some("/O=Example/CN=Bob")
    );
    assert_eq!(recipients[1].address_type().unwrap().as_deref(), Some("EX"));
    assert_eq!(recipients[2].email_address().unwrap().as_deref(), Some(""));
    assert_eq!(
        recipients[3].email_address().unwrap().as_deref(),
        Some("first@example.com")
    );
}

#[test]
fn incompatible_high_priority_fields_are_errors_not_missing_values() {
    let data = table(&[
        vec![
            fixed(
                Tag::RECIPIENT_DISPLAY_NAME_W.id(),
                Type::LONG,
                &7i32.to_le_bytes(),
            ),
            string(Tag::TRANSMITABLE_DISPLAY_NAME_W, "do not use"),
        ],
        vec![
            fixed(Tag::TRANSMITABLE_DISPLAY_NAME_W.id(), Type::NULL, &[]),
            string(Tag::DISPLAY_NAME_W, "do not use"),
        ],
        vec![
            fixed(Tag::SMTP_ADDRESS_W.id(), Type::LONG, &7i32.to_le_bytes()),
            string(Tag::EMAIL_ADDRESS_W, "do not use"),
        ],
        vec![fixed(Tag::ADDRTYPE_W.id(), Type::LONG, &7i32.to_le_bytes())],
        vec![
            string(Tag::new(Tag::RECIPIENT_TYPE.id(), Type::UNICODE), "2"),
            fixed(Tag::RECIPIENT_TYPE.id(), Type::LONG, &2i32.to_le_bytes()),
        ],
    ]);
    let recipients = read_recipients(&data, 1);
    assert_eq!(
        recipients[0].display_name(),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(
        recipients[1].display_name(),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(
        recipients[2].email_address(),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(
        recipients[3].address_type(),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(
        recipients[4].recipient_type(),
        Err(ConversionError::InvalidType)
    );
    assert_eq!(recipients[0].recipient_type(), Ok(RecipientType::To));
}

#[test]
fn empty_and_multivalued_fields_preserve_scalar_errors() {
    let mut empty = fixed(Tag::RECIPIENT_DISPLAY_NAME_W.id(), Type::UNICODE, &[]);
    dword(&mut empty, 0);
    let mut multi = fixed(
        Tag::SMTP_ADDRESS_W.id(),
        Type(Type::UNICODE.0 | 0x1000),
        &[],
    );
    dword(&mut multi, 1);
    variable(&mut multi, &unicode("first@example.com\0"));
    let mut types = fixed(Tag::RECIPIENT_TYPE.id(), Type(Type::LONG.0 | 0x1000), &[]);
    dword(&mut types, 1);
    dword(&mut types, 2);
    let data = table(&[
        vec![empty, string(Tag::DISPLAY_NAME_W, "do not use")],
        vec![multi, string(Tag::EMAIL_ADDRESS_W, "do not use")],
        vec![types],
    ]);
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let rows = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_rows_as_property_sets()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].violation, Violation::InvalidValueCount);
    let recipients: Vec<_> = rows.into_iter().map(TnefRecipient::from).collect();
    assert_eq!(recipients[0].display_name(), Err(ConversionError::NoValue));
    assert_eq!(
        recipients[1].email_address(),
        Err(ConversionError::MultiValued)
    );
    assert_eq!(
        recipients[2].recipient_type(),
        Err(ConversionError::MultiValued)
    );
}

#[test]
fn recipient_types_reuse_numeric_conversion_without_treating_failure_as_to() {
    let data = table(&[
        vec![fixed(
            Tag::RECIPIENT_TYPE.id(),
            Type::I2,
            &2i32.to_le_bytes(),
        )],
        vec![fixed(
            Tag::RECIPIENT_TYPE.id(),
            Type::CURRENCY,
            &35000i64.to_le_bytes(),
        )],
        vec![fixed(
            Tag::RECIPIENT_TYPE.id(),
            Type::BOOLEAN,
            &0i32.to_le_bytes(),
        )],
        vec![fixed(
            Tag::RECIPIENT_TYPE.id(),
            Type::DOUBLE,
            &f64::INFINITY.to_le_bytes(),
        )],
    ]);
    let recipients = read_recipients(&data, 1);
    assert_eq!(recipients[0].recipient_type(), Ok(RecipientType::Cc));
    assert_eq!(recipients[1].recipient_type(), Ok(RecipientType::Bcc));
    assert_eq!(
        recipients[2].recipient_type(),
        Ok(RecipientType::Originator)
    );
    assert_eq!(
        recipients[3].recipient_type(),
        Err(ConversionError::OutOfRange)
    );
}

#[test]
fn ownership_roundtrips_preserve_storage_named_properties_and_cloned_lifetime() {
    let data = table(&[vec![
        string(Tag::DISPLAY_NAME_W, "Alice"),
        named(0, &0x8501i32.to_le_bytes()),
        string(Tag::DISPLAY_NAME_A, "duplicate"),
    ]]);
    let mut recipients = read_recipients(&data, 1);
    let recipient = recipients.pop().unwrap();
    let storage = recipient.properties().as_slice().as_ptr();
    let owned = recipient.into_properties();
    assert_eq!(owned.as_slice().as_ptr(), storage);
    let recipient = TnefRecipient::from(owned);
    assert_eq!(recipient.properties().as_slice().as_ptr(), storage);
    let name = TnefNameId {
        guid: [0x42; 16],
        name: TnefName::Id(0x8501),
    };
    assert_eq!(
        recipient.properties().get_named(&name).unwrap().to_i32(),
        Ok(7)
    );
    assert_eq!(recipient.properties().len(), 3);
    let cloned = recipient.clone();
    assert_eq!(recipient, cloned);
    drop(recipient);
    drop(data);
    for _ in 0..2 {
        assert_eq!(cloned.display_name().unwrap().as_deref(), Some("Alice"));
        assert_eq!(
            cloned.properties().get_named(&name).unwrap().to_i32(),
            Ok(7)
        );
    }
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TnefRecipient>();
}

#[test]
fn binary_fields_use_captured_charset_after_later_codepage_changes() {
    let mut legacy = text(Type::BINARY, b"Caf\xe9\0\0");
    legacy[..4].copy_from_slice(
        &Tag::new(Tag::DISPLAY_NAME_W.id(), Type::BINARY)
            .0
            .to_le_bytes(),
    );
    let mut data = table(&[vec![legacy]]);
    attribute(
        &mut data,
        Attribute::MAPI_PROPERTIES,
        &properties(&[fixed(
            Tag::INTERNET_CODEPAGE.id(),
            Type::LONG,
            &65001i32.to_le_bytes(),
        )]),
    );
    let utf8 = table(&[vec![string(
        Tag::new(Tag::DISPLAY_NAME_W.id(), Type::BINARY),
        "Caf\u{e9}",
    )]]);
    data.extend_from_slice(&utf8[6..]);
    let mut reader = TnefReader::new(data.as_slice());
    let mut logger = |issue| panic!("unexpected issue: {issue:?}");
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let first = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_rows_as_property_sets()
        .unwrap()
        .pop()
        .unwrap();
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    let second = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_rows_as_property_sets()
        .unwrap()
        .pop()
        .unwrap();
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    drop(reader);
    let first = TnefRecipient::from(first);
    let second = TnefRecipient::from(second);
    assert_eq!(first.display_name().unwrap().as_deref(), Some("Caf\u{e9}"));
    assert_eq!(second.display_name().unwrap().as_deref(), Some("Caf\u{e9}"));
    assert_eq!(
        first.properties()[0].as_bytes(),
        Ok(b"Caf\xe9\0\0".as_slice())
    );
}

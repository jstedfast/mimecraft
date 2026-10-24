// Adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    DateTimeOffset, DateTimeOffsetError, Header, HeaderList, MimeContent, MimeMessage, MimePart,
    MimeVersion, TextPart,
};

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

fn date() -> DateTimeOffset {
    DateTimeOffset::new(2007, 6, 28, 12, 47, 52, -300).unwrap()
}

fn get_date<R>(
    message: &MimeMessage<R>,
    field: &str,
) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
    match field {
        "Date" => message.date(),
        "Resent-Date" => message.resent_date(),
        _ => unreachable!(),
    }
}

fn set_date<R>(message: &mut MimeMessage<R>, field: &str, value: DateTimeOffset) {
    match field {
        "Date" => message.set_date(value),
        "Resent-Date" => message.set_resent_date(value),
        _ => unreachable!(),
    }
}

#[test]
fn absent_metadata_has_no_default_and_serialization_does_not_generate_it() {
    let message: MimeMessage<()> = MimeMessage::new();
    assert_eq!(message.date(), Ok(None));
    assert_eq!(message.resent_date(), Ok(None));
    assert_eq!(message.mime_version(), Ok(None));
    let mut message: MimeMessage = MimeMessage::new();
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert_eq!(wire, b"\r\n");
    assert!(message.headers().is_empty());
}

#[test]
fn date_getters_reuse_legacy_parsing_without_changing_raw_bytes() {
    for (raw, expected) in [
        ("Thu, 28 Jun 2007 12:47:52 -0500", date()),
        (
            "Tue, 9 Jun 92 03:45:24 JST",
            DateTimeOffset::new(1992, 6, 9, 3, 45, 24, 540).unwrap(),
        ),
        (
            "Sat Mar 24 21:23:03 EDT 2007",
            DateTimeOffset::new(2007, 3, 24, 21, 23, 3, -240).unwrap(),
        ),
        (
            "28 Jun 2007",
            DateTimeOffset::new(2007, 6, 28, 0, 0, 0, 0).unwrap(),
        ),
    ] {
        for field in ["Date", "Resent-Date"] {
            let raw = format!(
                "{}: {raw}\r\n{field}: invalid\r\n",
                field.to_ascii_lowercase()
            );
            let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
            let before = snapshot(&message);
            assert_eq!(get_date(&message, field), Ok(Some(expected)));
            assert_eq!(snapshot(&message), before);
        }
    }
}

#[test]
fn invalid_first_dates_are_errors_not_absence_or_later_values() {
    for field in ["Date", "Resent-Date"] {
        for raw in ["", "invalid", "31 Feb 2007"] {
            let raw = format!("{field}: {raw}\r\n{field}: {date}\r\n", date = date());
            let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
            assert_eq!(
                get_date(&message, field),
                Err(DateTimeOffsetError::InvalidSyntax)
            );
        }
    }
}

#[test]
fn date_setters_format_and_update_independently() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    let original = DateTimeOffset::new(1997, 6, 28, 12, 47, 52, -300).unwrap();
    message.set_date(original);
    message.set_resent_date(date());
    assert_eq!(message.date(), Ok(Some(original)));
    assert_eq!(message.resent_date(), Ok(Some(date())));
    assert_eq!(
        snapshot(&message),
        [
            b"Date: Sat, 28 Jun 1997 12:47:52 -0500\r\n".to_vec(),
            b"Resent-Date: Thu, 28 Jun 2007 12:47:52 -0500\r\n".to_vec(),
        ]
    );
    assert_eq!(message.mime_version(), Ok(None));
}

#[test]
fn equal_date_assignments_preserve_bytes_but_collapse_singleton_duplicates() {
    for field in ["Date", "Resent-Date"] {
        let raw = format!(
            "{}: Thu, 28 Jun 2007\n\t12:47:52 EST (zone)\nX-Middle: unchanged\n",
            field.to_ascii_lowercase()
        );
        let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        set_date(&mut message, field, date());
        assert_eq!(snapshot(&message), before);
        message
            .headers_mut()
            .add(Header::parse(format!("{field}: invalid\n").as_bytes()).unwrap());
        let before = snapshot(&message);
        set_date(&mut message, field, date());
        if field == "Date" {
            assert_eq!(
                snapshot(&message),
                [
                    format!("Date: {}\r\n", date()).into_bytes(),
                    b"X-Middle: unchanged\n".to_vec(),
                ]
            );
        } else {
            assert_eq!(snapshot(&message), before);
        }
    }
}

#[test]
fn same_instant_with_different_offset_is_an_explicit_date_edit() {
    let utc = DateTimeOffset::new(2007, 6, 28, 17, 47, 52, 0).unwrap();
    assert_eq!(utc.unix_timestamp(), date().unix_timestamp());
    assert_ne!(utc, date());
    for field in ["Date", "Resent-Date"] {
        let mut message: MimeMessage = MimeMessage::new();
        set_date(&mut message, field, date());
        set_date(&mut message, field, utc);
        assert_eq!(get_date(&message, field), Ok(Some(utc)));
        assert_eq!(
            message.headers().get(field).unwrap().raw(),
            format!("{field}: Thu, 28 Jun 2007 17:47:52 +0000\r\n").as_bytes()
        );
    }
}

#[test]
fn date_replacements_repair_invalid_values_and_preserve_first_slot() {
    for field in ["Date", "Resent-Date"] {
        for old in ["invalid", "28 Jun 2007"] {
            let raw =
                format!("X-Before: one\r\n{field}: {old}\r\nX-Middle: two\r\n{field}: {old}\r\n");
            let mut message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
            set_date(&mut message, field, date());
            assert_eq!(
                snapshot(&message),
                [
                    b"X-Before: one\r\n".to_vec(),
                    format!("{field}: {}\r\n", date()).into_bytes(),
                    b"X-Middle: two\r\n".to_vec(),
                ]
            );
        }
    }
}

#[test]
fn authored_dates_do_not_hide_compatibility_parser_limits_in_a_cache() {
    let historical = DateTimeOffset::new(1800, 1, 1, 0, 0, 0, 0).unwrap();
    let unusual_zone = DateTimeOffset::new(2007, 6, 28, 12, 47, 52, -780).unwrap();
    for field in ["Date", "Resent-Date"] {
        let mut message: MimeMessage = MimeMessage::new();
        set_date(&mut message, field, historical);
        assert_eq!(
            message.headers().get(field).unwrap().raw(),
            format!("{field}: {historical}\r\n").as_bytes()
        );
        assert_eq!(
            get_date(&message, field),
            Err(DateTimeOffsetError::InvalidSyntax)
        );
        set_date(&mut message, field, unusual_zone);
        // The checked type allows -13 hours, while the legacy reader recovers
        // zones outside its narrower range as UTC. Never return cached input.
        let parsed = get_date(&message, field).unwrap().unwrap();
        assert_eq!(parsed.offset_minutes(), 0);
        assert_eq!(parsed.hour(), unusual_zone.hour());
        assert_eq!(
            message.headers().get(field).unwrap().raw(),
            format!("{field}: {unusual_zone}\r\n").as_bytes()
        );
    }
}

#[test]
fn version_getter_accepts_comments_components_and_first_header_only() {
    for (raw, expected) in [
        ("(before) 01 (major).\r\n\t00 (minor)", "1.0"),
        ("1.0.0", "1.0.0"),
        ("1.0.0.0", "1.0.0.0"),
        ("2147483647.0", "2147483647.0"),
    ] {
        let raw = format!("mime-version: {raw}\r\nMIME-Version: invalid\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let before = snapshot(&message);
        assert_eq!(
            message.mime_version().unwrap().unwrap().to_string(),
            expected
        );
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn invalid_version_errors_retain_raw_value_offsets_without_fallback() {
    for raw in [
        "",
        "one.zero",
        "1",
        "1.0.0.0.0",
        "1.2147483648",
        "1.0 (open",
        "=?utf-8?q?1.0?=",
    ] {
        let raw = format!("MIME-Version: {raw}\r\nMIME-Version: 1.0\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(raw.as_bytes()), None);
        let value = message.headers().get("MIME-Version").unwrap().raw_value();
        assert_eq!(
            message.mime_version(),
            Err(MimeVersion::parse(value).unwrap_err())
        );
    }
}

#[test]
fn equal_version_preserves_bytes_but_component_counts_are_significant() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"mime-version: (comment) 01.\n\t00\nX-Middle: untouched\n"),
        None,
    );
    let before = snapshot(&message);
    message.set_mime_version(MimeVersion::V1_0);
    assert_eq!(snapshot(&message), before);
    message
        .headers_mut()
        .add(Header::parse(b"MIME-Version: 2.0\n").unwrap());
    message.set_mime_version(MimeVersion::V1_0);
    assert_eq!(
        snapshot(&message),
        [
            b"MIME-Version: 1.0\r\n".to_vec(),
            b"X-Middle: untouched\n".to_vec(),
        ]
    );
    for raw in [
        "1.0.0",
        "1.0.0.0",
        "1.0",
        "2.0",
        "2147483647.2147483647.2147483647.2147483647",
    ] {
        let value = MimeVersion::parse(raw.as_bytes()).unwrap();
        message.set_mime_version(value);
        assert_eq!(message.mime_version(), Ok(Some(value)));
        assert_eq!(
            snapshot(&message),
            [
                format!("MIME-Version: {raw}\r\n").into_bytes(),
                b"X-Middle: untouched\n".to_vec(),
            ]
        );
    }
}

#[test]
fn version_setter_repairs_invalid_headers_at_first_slot() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(
            b"X-Before: one\r\nMIME-Version: invalid\r\nX-Middle: two\r\nmime-version: 1.0\r\n",
        ),
        None,
    );
    message.set_mime_version(MimeVersion::V1_0);
    assert_eq!(
        snapshot(&message),
        [
            b"X-Before: one\r\n".to_vec(),
            b"MIME-Version: 1.0\r\n".to_vec(),
            b"X-Middle: two\r\n".to_vec(),
        ]
    );
}

#[test]
fn direct_header_edits_and_removals_are_visible_without_cache_synchronization() {
    let mut message: MimeMessage = MimeMessage::new();
    for field in ["Date", "Resent-Date"] {
        set_date(&mut message, field, date());
        assert_eq!(get_date(&message, field), Ok(Some(date())));
        message
            .headers_mut()
            .replace_all(Header::parse(format!("{field}: 28 Jun 2007").as_bytes()).unwrap())
            .unwrap();
        assert_eq!(get_date(&message, field).unwrap().unwrap().hour(), 0);
        message.headers_mut().remove_all(field);
        assert_eq!(get_date(&message, field), Ok(None));
    }
    message.set_mime_version(MimeVersion::V1_0);
    assert_eq!(message.mime_version(), Ok(Some(MimeVersion::V1_0)));
    message
        .headers_mut()
        .replace_all(Header::parse(b"MIME-Version: 2.0").unwrap())
        .unwrap();
    assert_eq!(message.mime_version().unwrap().unwrap().major(), 2);
    message.headers_mut().remove_all("MIME-Version");
    assert_eq!(message.mime_version(), Ok(None));
    message.set_date(date());
    message.set_resent_date(date());
    message.set_mime_version(MimeVersion::V1_0);
    message.headers_mut().clear();
    assert_eq!(message.date(), Ok(None));
    assert_eq!(message.resent_date(), Ok(None));
    assert_eq!(message.mime_version(), Ok(None));
}

#[test]
fn explicit_metadata_roundtrips_with_body_and_other_headers_unchanged() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    message
        .headers_mut()
        .add(Header::parse(b"Subject: example").unwrap());
    message.set_date(date());
    message.set_resent_date(date());
    message.set_mime_version(MimeVersion::V1_0);
    let before = snapshot(&message);
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert!(wire.ends_with(b"\r\n\r\nhello"));
    assert_eq!(snapshot(&message), before);
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(snapshot(&loaded), before);
    assert_eq!(loaded.date(), Ok(Some(date())));
    assert_eq!(loaded.resent_date(), Ok(Some(date())));
    assert_eq!(loaded.mime_version(), Ok(Some(MimeVersion::V1_0)));
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("metadata access must not read body sources");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("metadata access must not seek body sources");
    }
}

#[test]
fn metadata_does_not_inspect_body_headers_or_sources() {
    let part = MimePart::from_parts(
        headers(b"Content-Type: invalid\r\nDate: invalid\r\nResent-Date: invalid\r\nMIME-Version: invalid\r\n"),
        MimeContent::new(NoIo),
    );
    let mut message = MimeMessage::with_body(part.into());
    let before: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    assert_eq!(message.date(), Ok(None));
    assert_eq!(message.resent_date(), Ok(None));
    assert_eq!(message.mime_version(), Ok(None));
    message.set_date(date());
    message.set_resent_date(date());
    message.set_mime_version(MimeVersion::V1_0);
    assert_eq!(message.date(), Ok(Some(date())));
    assert_eq!(message.resent_date(), Ok(Some(date())));
    assert_eq!(message.mime_version(), Ok(Some(MimeVersion::V1_0)));
    let after: Vec<_> = message.body().unwrap().headers().iter().cloned().collect();
    assert_eq!(after, before);
}

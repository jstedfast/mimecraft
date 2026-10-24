// Adapted from MimeKit's UnitTests/MimePartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentEncoding, Header, HeaderList, InvalidContentDuration, MimeContent, MimeEntity,
    MimeMessage, MimePart, TextPart, UnstructuredHeaderFormatOptions,
    charsets::Charset,
    encodings::NewLineFormat,
    rfc2047::{ComplianceMode, DecodeOptions, EncodeOptions},
};

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("metadata access must not read content")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("metadata access must not seek content")
    }
}

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn part(raw: &[u8]) -> MimePart<NoIo> {
    MimePart::from_parts(headers(raw), MimeContent::new(NoIo))
}

#[test]
fn description_absence_empty_text_assignment_and_removal_follow_upstream() {
    let mut part = part(b"");
    assert!(part.content_description().is_none());
    part.set_content_description(Some("This is a sample description."))
        .unwrap();
    assert_eq!(
        part.content_description().unwrap().text,
        "This is a sample description."
    );
    part.set_content_description(None).unwrap();
    assert!(part.content_description().is_none());
    assert!(!part.headers().contains("Content-Description"));

    part.set_content_description(Some(" \r\n\t")).unwrap();
    assert_eq!(part.content_description().unwrap().text, "");
    assert_eq!(
        part.headers().get("Content-Description").unwrap().raw(),
        b"Content-Description:\r\n"
    );
    part.headers_mut().clear();
    assert!(part.content_description().is_none());
}

#[test]
fn description_uses_first_header_and_retains_tolerant_decoding_diagnostics() {
    for value in [
        b" =?unknown?q?caf=E9?=\r\n".as_slice(),
        b" =?utf-8?q?caf=C3=A9=FF?=\r\n",
        b" \xe9\r\n",
        b" malformed =?utf-8?x?hello?=\r\n",
        b" x =?utf-8?q?=0D=0Ay?=\r\n",
        b" one\r\n\t two  \r\n",
        b" \0\x01\r\n",
    ] {
        let mut raw = b"cOnTeNt-DeScRiPtIoN:".to_vec();
        raw.extend_from_slice(value);
        raw.extend_from_slice(b"Content-Description: ignored\r\n");
        let part = part(&raw);
        let before = part.headers().clone();
        let expected = before.get("Content-Description").unwrap().decode_value();
        assert_eq!(part.content_description(), Some(expected));
        assert_eq!(part.headers(), &before);
    }
    let part = part(b"Content-Description: =?unknown?q?caf=E9?=\n");
    let decoded = part.content_description().unwrap();
    assert_eq!(decoded.text, "caf\u{e9}");
    assert!(!decoded.issues.is_empty());
}

#[test]
fn explicit_description_decode_options_control_recovery() {
    let part = part(b"Content-Description: before=?utf-8?q?caf=C3=A9?=after \x80\r\n");
    let options = DecodeOptions {
        compliance_mode: ComplianceMode::Strict,
        fallback_charset: Some(Charset::LATIN1),
    };
    let decoded = part.content_description_with_options(&options).unwrap();
    assert_eq!(decoded.text, "before=?utf-8?q?caf=C3=A9?=after \u{80}");
    assert!(!decoded.issues.is_empty());
    assert!(
        part.content_description()
            .unwrap()
            .text
            .starts_with("beforecaf\u{e9}after")
    );
}

#[test]
fn description_assignment_collapses_duplicates_and_preserves_other_records() {
    let mut part = part(
        b"X-Before: one\ncontent-description \t: old\nX-Middle: two\nCONTENT-DESCRIPTION: duplicate\nX-After: three\n",
    );
    part.set_content_description(Some(" \r\nnew\r\n value "))
        .unwrap();
    assert_eq!(
        part.headers(),
        &headers(
            b"X-Before: one\nContent-Description: new value\r\nX-Middle: two\nX-After: three\n"
        )
    );
    part.set_content_description(Some("new value")).unwrap();
    assert_eq!(part.headers().get_all("Content-Description").count(), 1);
    let mut detached = part.content_description().unwrap();
    detached.text.push_str(" changed");
    assert_eq!(part.content_description().unwrap().text, "new value");
}

#[test]
fn description_formatting_failures_are_atomic_and_removal_ignores_options() {
    let mut part = part(b"Content-Description: first\nX-Keep: raw\nContent-Description: second\n");
    let before = part.headers().clone();
    for options in [
        UnstructuredHeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        },
        UnstructuredHeaderFormatOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        UnstructuredHeaderFormatOptions {
            encoding: EncodeOptions {
                charset: Charset::ASCII,
                ..Default::default()
            },
            ..Default::default()
        },
    ] {
        assert!(
            part.set_content_description_with_options(Some("caf\u{e9}"), &options)
                .is_err()
        );
        assert_eq!(part.headers(), &before);
    }
    let invalid = UnstructuredHeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    part.set_content_description_with_options(None, &invalid)
        .unwrap();
    assert_eq!(part.headers(), &headers(b"X-Keep: raw\n"));
    assert!(
        part.set_content_description_with_options(Some(""), &invalid)
            .is_err()
    );
    assert_eq!(part.headers(), &headers(b"X-Keep: raw\n"));
}

#[test]
fn description_folding_and_encoded_word_lookalikes_round_trip() {
    let mut part = part(b"content-description: old\nContent-Description: duplicate\n");
    let value = "literal =?utf-8?q?description?= caf\u{e9} ".repeat(20);
    let options = UnstructuredHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    part.set_content_description_with_options(Some(&value), &options)
        .unwrap();
    assert_eq!(part.content_description().unwrap().text, value.trim());
    assert_eq!(part.headers().get_all("Content-Description").count(), 1);
    let raw = part.headers().get("Content-Description").unwrap().raw();
    assert!(!raw.contains(&b'\r'));
    assert!(
        std::str::from_utf8(raw)
            .unwrap()
            .lines()
            .all(|line| line.len() <= 60)
    );
}

#[test]
fn description_and_duration_observe_raw_edits_without_caches() {
    let mut part = part(
        b"Content-Description: first\nContent-Description: second\nContent-Duration: 500\nContent-Duration: 600\n",
    );
    assert_eq!(part.content_description().unwrap().text, "first");
    assert_eq!(part.content_duration(), Ok(Some(500)));
    part.headers_mut().remove("Content-Description");
    part.headers_mut().remove("Content-Duration");
    assert_eq!(part.content_description().unwrap().text, "second");
    assert_eq!(part.content_duration(), Ok(Some(600)));
    part.headers_mut()
        .replace_all(Header::parse(b"Content-Description: third").unwrap())
        .unwrap();
    part.headers_mut()
        .replace_all(Header::parse(b"Content-Duration: 700").unwrap())
        .unwrap();
    assert_eq!(part.content_description().unwrap().text, "third");
    assert_eq!(part.content_duration(), Ok(Some(700)));
    part.headers_mut().clear();
    assert!(part.content_description().is_none());
    assert_eq!(part.content_duration(), Ok(None));
}

#[test]
fn duration_accepts_nonnegative_int32_seconds_after_header_decoding() {
    for (value, expected) in [
        ("0", 0),
        ("500", 500),
        ("000500", 500),
        ("2147483647", i32::MAX),
        (" \t500 \t", 500),
        ("\u{a0}500\u{2003}", 500),
        ("\x0b500\x0c", 500),
        ("=?utf-8?q?500?=", 500),
        ("=?utf-8?q?5=0D=0A00?=", 500),
        ("500\0\0", 500),
        ("500 \t\x0b\x0c\0", 500),
        ("500\0 ", 500),
    ] {
        let part = part(format!("Content-Duration: {value}\r\n").as_bytes());
        let before = part.headers().clone();
        assert_eq!(part.content_duration(), Ok(Some(expected)), "{value:?}");
        assert_eq!(part.headers(), &before);
    }
    let part = part(format!("Content-Duration: {}500\n", "0".repeat(4096)).as_bytes());
    assert_eq!(part.content_duration(), Ok(Some(500)));
}

#[test]
fn invalid_durations_do_not_fall_back_to_absence_or_later_duplicates() {
    for value in [
        "",
        " ",
        "+500",
        "-500",
        "-0",
        "2147483648",
        "4294967295",
        "999999999999999999999999999",
        "5.00",
        "5,000",
        "5e2",
        "500 seconds",
        "(comment) 500",
        "500 (comment)",
        "5 00",
        "5\r\n\t00",
        "\u{0665}\u{0660}\u{0660}",
        "\u{ff15}\u{ff10}\u{ff10}",
        "\0",
        "5\0 00",
        "500\u{a0}\0",
        "garbage",
        "=?utf-8?q?+500?=",
    ] {
        let part =
            part(format!("Content-Duration: {value}\r\nContent-Duration: 123\r\n").as_bytes());
        let before = part.headers().clone();
        assert_eq!(
            part.content_duration(),
            Err(InvalidContentDuration),
            "{value:?}"
        );
        assert_eq!(part.headers(), &before);
    }
    let part = part(b"Content-Duration: 50\xff0\n");
    assert_eq!(part.content_duration(), Err(InvalidContentDuration));
}

#[test]
fn duration_assignment_and_removal_match_upstream() {
    let mut part = part(b"X-Keep: raw\n");
    assert_eq!(part.content_duration(), Ok(None));
    for value in [500, 500, 0, i32::MAX] {
        part.set_content_duration(Some(value)).unwrap();
        assert_eq!(part.content_duration(), Ok(Some(value)));
        assert_eq!(
            part.headers().get("Content-Duration").unwrap().raw(),
            format!("Content-Duration: {value}\r\n").as_bytes()
        );
    }
    part.set_content_duration(None).unwrap();
    assert_eq!(part.content_duration(), Ok(None));
    assert_eq!(part.headers(), &headers(b"X-Keep: raw\n"));
}

#[test]
fn equal_duration_assignments_canonicalize_and_collapse_duplicates() {
    let mut part = part(
        b"X-Before: one\ncontent-duration \t: 000500\nX-Middle: two\nContent-Duration: bad\nX-After: three\n",
    );
    assert_eq!(part.content_duration(), Ok(Some(500)));
    part.set_content_duration(Some(500)).unwrap();
    assert_eq!(
        part.headers(),
        &headers(b"X-Before: one\nContent-Duration: 500\r\nX-Middle: two\nX-After: three\n")
    );
    part.headers_mut()
        .add(Header::parse(b"Content-Duration: duplicate").unwrap());
    part.set_content_duration(None).unwrap();
    assert_eq!(
        part.headers(),
        &headers(b"X-Before: one\nX-Middle: two\nX-After: three\n")
    );
}

#[test]
fn negative_duration_assignments_are_atomic_even_with_invalid_existing_metadata() {
    let mut part = part(b"Content-Duration: invalid\nX-Keep: raw\nContent-Duration: 123\n");
    let before = part.headers().clone();
    for value in [-1, i32::MIN] {
        assert_eq!(
            part.set_content_duration(Some(value)),
            Err(InvalidContentDuration)
        );
        assert_eq!(part.headers(), &before);
    }
    part.set_content_duration(Some(0)).unwrap();
    assert_eq!(part.content_duration(), Ok(Some(0)));
    assert_eq!(part.headers().get_all("Content-Duration").count(), 1);
}

#[test]
fn metadata_accessors_need_no_source_traits_and_do_not_touch_streams() {
    fn edit<R>(part: &mut MimePart<R>) {
        part.set_content_description(Some("description")).unwrap();
        part.set_content_duration(Some(500)).unwrap();
        assert_eq!(part.content_description().unwrap().text, "description");
        assert_eq!(part.content_duration(), Ok(Some(500)));
    }
    edit(&mut part(
        b"Content-Type: invalid\nContent-Transfer-Encoding: unknown\n",
    ));

    let mut source = Cursor::new(b"YWJj".to_vec());
    source.set_position(3);
    {
        let mut part = MimePart::new(MimeContent::with_encoding(
            &mut source,
            ContentEncoding::Base64,
        ));
        let original = part.headers().get("Content-Type").unwrap().clone();
        part.set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
        edit(&mut part);
        assert_eq!(part.headers().get("Content-Type"), Some(&original));
        assert_eq!(part.content().encoding(), ContentEncoding::Base64);
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::QuotedPrintable
        );
    }
    assert_eq!(source.position(), 3);
    assert_eq!(source.into_inner(), b"YWJj");
}

#[test]
fn metadata_round_trips_through_text_part_and_message_without_header_injection() {
    let mut text = TextPart::new("plain", "body").unwrap();
    text.part_mut()
        .set_content_description(Some("news\r\nBcc: not-a-header"))
        .unwrap();
    text.part_mut().set_content_duration(Some(12)).unwrap();
    let mut message = MimeMessage::with_body(text.into());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert!(!loaded.headers().contains("Content-Description"));
    assert!(!loaded.headers().contains("Content-Duration"));
    assert!(!loaded.headers().contains("Bcc"));
    let MimeEntity::Part(part) = loaded.body().unwrap() else {
        panic!("expected leaf")
    };
    assert_eq!(
        part.content_description().unwrap().text,
        "newsBcc: not-a-header"
    );
    assert_eq!(part.content_duration(), Ok(Some(12)));
    assert!(!part.headers().contains("Bcc"));
    assert_eq!(part.content().get_ref().get_ref(), b"body");
}

#[test]
fn invalid_duration_does_not_reject_loading_or_change_raw_serialization() {
    let raw =
        b"Subject: tolerant\r\nContent-Type: text/plain\r\nContent-Duration: -invalid\r\n\r\nbody";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected leaf")
    };
    assert_eq!(part.content_duration(), Err(InvalidContentDuration));
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    assert_eq!(wire, raw);
}

#[test]
fn duration_error_implements_standard_error_traits() {
    let error: &dyn std::error::Error = &InvalidContentDuration;
    assert!(error.source().is_none());
    assert_eq!(
        error.to_string(),
        "content duration must be an integer from 0 through 2147483647 seconds"
    );
}

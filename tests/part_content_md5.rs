// Adapted from MimeKit's UnitTests/MimePartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MimeContent, MimeEntity, MimeMessage, MimePart, TextPart,
    UnstructuredHeaderFormatError, UnstructuredHeaderFormatOptions,
    charsets::Charset,
    encodings::NewLineFormat,
    rfc2047::{ComplianceMode, DecodeOptions, EncodeOptions},
};

struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("metadata must not read the content");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("metadata must not seek the content");
    }
}

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn part(raw: &[u8]) -> MimePart<NoIo> {
    MimePart::from_parts(headers(raw), MimeContent::new(NoIo))
}

#[test]
fn absence_assignment_removal_and_raw_edits_follow_upstream() {
    let mut part = part(b"");
    assert!(part.content_md5().is_none());
    part.set_content_md5(Some(" XYZ ")).unwrap();
    assert_eq!(part.content_md5().unwrap().text, "XYZ");
    assert_eq!(
        part.headers().get("Content-MD5").unwrap().raw(),
        b"Content-Md5: XYZ\r\n"
    );
    part.set_content_md5(Some("XYZ")).unwrap();
    assert_eq!(part.content_md5().unwrap().text, "XYZ");
    part.set_content_md5(None).unwrap();
    assert!(!part.headers().contains("Content-MD5"));
    assert!(part.content_md5().is_none());
    part.headers_mut()
        .add(Header::parse(b"CONTENT-MD5: changed\r\n").unwrap());
    assert_eq!(part.content_md5().unwrap().text, "changed");
    part.headers_mut().remove("Content-MD5");
    assert!(part.content_md5().is_none());
    part.set_content_md5(Some("XYZ")).unwrap();
    part.headers_mut().clear();
    assert!(part.content_md5().is_none());
}

#[test]
fn first_header_wins_even_when_empty_or_not_a_checksum() {
    for (value, expected) in [
        ("", ""),
        (" \t", ""),
        ("XYZ", "XYZ"),
        ("not base64!", "not base64!"),
        ("8criUiOQmpfifOuOmYFtEQ==", "8criUiOQmpfifOuOmYFtEQ=="),
        ("\u{a0}value\u{2003}", "value"),
        ("=?utf-8?q?XYZ?=", "XYZ"),
        ("=?utf-8?q?X=0D=0AYZ?=", "XYZ"),
        ("first\r\n\tsecond", "first\tsecond"),
    ] {
        let mut part = part(format!("cOnTeNt-Md5: {value}\r\nContent-MD5: later\r\n").as_bytes());
        let before = part.headers().clone();
        assert_eq!(part.content_md5().unwrap().text, expected, "{value:?}");
        let mut detached = part.content_md5().unwrap();
        detached.text.push_str("modified");
        assert_eq!(part.content_md5().unwrap().text, expected);
        assert_eq!(part.headers(), &before);
        part.headers_mut().remove("Content-MD5");
        assert_eq!(part.content_md5().unwrap().text, "later");
    }
}

#[test]
fn decoding_preserves_recovery_diagnostics_and_explicit_options() {
    for value in [
        b" =?unknown?q?caf=E9?=\r\n".as_slice(),
        b" =?utf-8?q?caf=C3=A9=FF?=\r\n",
        b" \xe9\r\n",
        b" malformed =?utf-8?x?hello?=\r\n",
        b" \0\x01\r\n",
    ] {
        let mut raw = b"Content-MD5:".to_vec();
        raw.extend_from_slice(value);
        let part = part(&raw);
        let before = part.headers().clone();
        assert_eq!(
            part.content_md5(),
            Some(before.get("Content-MD5").unwrap().decode_value())
        );
        assert_eq!(part.headers(), &before);
    }
    let recovered = part(b"Content-MD5: =?unknown?q?caf=E9?=\r\n")
        .content_md5()
        .unwrap();
    assert_eq!(recovered.text, "caf\u{e9}");
    assert!(!recovered.issues.is_empty());
    let part = part(b"Content-MD5: before=?utf-8?q?caf=C3=A9?=after \x80\r\n");
    let decoded = part
        .content_md5_with_options(&DecodeOptions {
            compliance_mode: ComplianceMode::Strict,
            fallback_charset: Some(Charset::LATIN1),
        })
        .unwrap();
    assert_eq!(decoded.text, "before=?utf-8?q?caf=C3=A9?=after \u{80}");
    assert!(!decoded.issues.is_empty());
    assert!(
        part.content_md5()
            .unwrap()
            .text
            .starts_with("beforecaf\u{e9}after")
    );
}

#[test]
fn equal_assignment_reformats_at_the_first_slot_and_collapses_duplicates() {
    let mut part =
        part(b"X-Before: a\ncontent-md5 \t: XYZ\nX-Middle: b\nCONTENT-MD5: second\nX-After: c\n");
    part.set_content_md5(Some("XYZ")).unwrap();
    assert_eq!(
        part.headers(),
        &headers(b"X-Before: a\nContent-Md5: XYZ\r\nX-Middle: b\nX-After: c\n")
    );
    part.headers_mut()
        .add(Header::parse(b"Content-MD5: third\r\n").unwrap());
    part.set_content_md5(None).unwrap();
    assert_eq!(
        part.headers(),
        &headers(b"X-Before: a\nX-Middle: b\nX-After: c\n")
    );
    part.set_content_md5(Some(" \r\n\t")).unwrap();
    assert_eq!(part.content_md5().unwrap().text, "");
    assert_eq!(
        part.headers().get("Content-MD5").unwrap().raw(),
        b"Content-Md5:\r\n"
    );
}

#[test]
fn formatting_is_atomic_and_removal_does_not_validate_options() {
    let mut part = part(b"Content-MD5: first\r\nX-Keep: a\r\nContent-MD5: second\r\n");
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
            part.set_content_md5_with_options(Some("caf\u{e9}"), &options)
                .is_err()
        );
        assert_eq!(part.headers(), &before);
    }
    let invalid = UnstructuredHeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    for value in ["", "first"] {
        assert!(matches!(
            part.set_content_md5_with_options(Some(value), &invalid),
            Err(UnstructuredHeaderFormatError::InvalidLineLength(_))
        ));
        assert_eq!(part.headers(), &before);
    }
    part.set_content_md5_with_options(None, &invalid).unwrap();
    assert_eq!(part.headers(), &headers(b"X-Keep: a\r\n"));
    part.set_content_md5_with_options(None, &invalid).unwrap();
    assert_eq!(part.headers(), &headers(b"X-Keep: a\r\n"));
}

#[test]
fn authoring_encodes_controls_and_literal_encoded_words_without_header_injection() {
    for (value, expected) in [
        ("XYZ\r\nX-Injected: value", "XYZX-Injected: value"),
        ("a\0b\u{1}c", "a\0b\u{1}c"),
        ("=?utf-8?q?literal?=", "=?utf-8?q?literal?="),
    ] {
        let mut part = part(b"");
        part.set_content_md5(Some(value)).unwrap();
        let raw = part.headers().get("Content-MD5").unwrap().raw();
        assert!(raw.is_ascii());
        assert!(!raw.contains(&0));
        assert!(!raw.contains(&1));
        let reloaded = headers(raw);
        assert_eq!(reloaded.len(), 1);
        assert!(!reloaded.contains("X-Injected"));
        assert_eq!(part.content_md5().unwrap().text, expected);
        assert_eq!(
            reloaded.get("Content-MD5").unwrap().decode_value().text,
            expected
        );
    }
}

#[test]
fn explicit_formatting_folds_long_values_and_round_trips_unicode() {
    let value = "literal =?utf-8?q?checksum?= caf\u{e9} ".repeat(20);
    let mut part = part(b"Content-MD5: old\r\nContent-MD5: duplicate\r\n");
    let options = UnstructuredHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    part.set_content_md5_with_options(Some(&value), &options)
        .unwrap();
    assert_eq!(part.content_md5().unwrap().text, value.trim());
    assert_eq!(part.headers().get_all("Content-MD5").count(), 1);
    let raw = part.headers().get("Content-MD5").unwrap().raw();
    assert!(!raw.contains(&b'\r'));
    assert!(raw.split(|&b| b == b'\n').all(|line| line.len() <= 60));
    assert!(raw.split(|&b| b == b'\n').count() > 2);
}

#[test]
fn metadata_has_no_source_trait_bounds_and_does_not_change_borrowed_content() {
    fn edit_without_bounds<R>(part: &mut MimePart<R>) {
        part.set_content_md5(Some("XYZ")).unwrap();
        assert_eq!(part.content_md5().unwrap().text, "XYZ");
        part.set_content_md5_with_options(Some("ABC"), &Default::default())
            .unwrap();
        assert_eq!(
            part.content_md5_with_options(&Default::default())
                .unwrap()
                .text,
            "ABC"
        );
    }
    let mut source = Cursor::new(b"body".to_vec());
    source.set_position(2);
    {
        let mut content = MimeContent::with_encoding(&mut source, ContentEncoding::Base64);
        content.set_new_line_format(Some(NewLineFormat::Mixed));
        let mut part = MimePart::new(content);
        edit_without_bounds(&mut part);
        assert_eq!(part.content().encoding(), ContentEncoding::Base64);
        assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Mixed));
    }
    assert_eq!(source.position(), 2);
    assert_eq!(source.get_ref(), b"body");
    edit_without_bounds(&mut part(b""));
}

#[test]
fn text_part_exposes_metadata_without_refreshing_it_after_body_changes() {
    let mut text = TextPart::new("plain", "original").unwrap();
    text.part_mut().set_content_md5(Some("XYZ")).unwrap();
    text.part_mut()
        .set_content(MimeContent::new(Cursor::new(b"changed".to_vec())));
    assert_eq!(text.part().content_md5().unwrap().text, "XYZ");
}

#[test]
fn loading_and_serialization_preserve_unverified_md5_metadata() {
    let raw = b"Subject: test\r\nContent-Type: text/plain\r\n\
        cOnTeNt-MD5: not a checksum\r\nContent-MD5: later\r\n\r\nbody";
    let mut message = MimeMessage::load(&mut raw.as_slice()).unwrap();
    assert!(!message.headers().contains("Content-MD5"));
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        unreachable!()
    };
    assert_eq!(part.content_md5().unwrap().text, "not a checksum");
    let before = part.headers().clone();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, raw);
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        unreachable!()
    };
    assert_eq!(part.headers(), &before);
    part.set_content_md5(Some("supplied")).unwrap();
    output.clear();
    message.write_to(&mut output).unwrap();
    let parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
    let MimeEntity::Part(part) = parsed.body().unwrap() else {
        unreachable!()
    };
    assert_eq!(part.content_md5().unwrap().text, "supplied");
    assert_eq!(part.headers().get_all("Content-MD5").count(), 1);
    assert_eq!(part.content().get_ref().get_ref(), b"body");
}

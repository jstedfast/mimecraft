// Adapted from MimeKit's UnitTests/MimeMessageTests.cs and HeaderTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Seek, SeekFrom};

use mimecraft::{
    Header, HeaderList, MimeContent, MimeMessage, MimePart, TextPart,
    UnstructuredHeaderFormatOptions,
    charsets::Charset,
    encodings::NewLineFormat,
    rfc2047::{ComplianceMode, DecodeOptions, EncodeOptions},
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

#[test]
fn absent_and_empty_subjects_are_distinct_without_stream_traits() {
    let mut message: MimeMessage<()> = MimeMessage::new();
    assert!(message.subject().is_none());
    message.set_subject(" \r\n\t").unwrap();
    let subject = message.subject().unwrap();
    assert_eq!(subject.text, "");
    assert!(subject.issues.is_empty());
    assert_eq!(
        message.headers().get("Subject").unwrap().raw(),
        b"Subject:\r\n"
    );
    message.headers_mut().remove_all("Subject");
    assert!(message.subject().is_none());
}

#[test]
fn getter_uses_first_header_with_tolerant_decoding_and_raw_value_diagnostics() {
    for value in [
        b" =?unknown?q?caf=E9?=\r\n".as_slice(),
        b" =?utf-8?q?caf=C3=A9=FF?=\r\n",
        b" \xe9\r\n",
        b" malformed =?utf-8?x?hello?=\r\n",
        b" x =?utf-8?q?=0D=0Ay?=\r\n",
        b" one\r\n\t two  \r\n",
    ] {
        let mut raw = b"sUbJeCt:".to_vec();
        raw.extend_from_slice(value);
        raw.extend_from_slice(b"Subject: ignored\r\n");
        let message: MimeMessage = MimeMessage::from_parts(headers(&raw), None);
        let before = snapshot(&message);
        let expected = message.headers().get("Subject").unwrap().decode_value();
        assert_eq!(message.subject(), Some(expected));
        assert_eq!(snapshot(&message), before);
    }
}

#[test]
fn explicit_options_control_boundaries_and_fallback_charset() {
    let message: MimeMessage = MimeMessage::from_parts(
        headers(b"Subject: before=?utf-8?q?caf=C3=A9?=after \x80\r\n"),
        None,
    );
    let options = DecodeOptions {
        compliance_mode: ComplianceMode::Strict,
        fallback_charset: Some(Charset::LATIN1),
    };
    let decoded = message.subject_with_options(&options).unwrap();
    assert_eq!(decoded.text, "before=?utf-8?q?caf=C3=A9?=after \u{80}");
    assert!(!decoded.issues.is_empty());
    assert!(
        message
            .subject()
            .unwrap()
            .text
            .starts_with("beforecaf\u{e9}after")
    );
}

#[test]
fn assignment_canonicalizes_first_slot_and_removes_later_duplicates() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"X-Before: one\nsUbJeCt \t: old\nX-Middle: two\nSUBJECT: duplicate\nsubject: third\nX-After: three\n"),
        None,
    );
    message.set_subject(" \r\nnew\r\n value ").unwrap();
    assert_eq!(
        snapshot(&message),
        [
            b"X-Before: one\n".to_vec(),
            b"Subject: new value\r\n".to_vec(),
            b"X-Middle: two\n".to_vec(),
            b"X-After: three\n".to_vec(),
        ]
    );
    assert_eq!(message.subject().unwrap().text, "new value");
}

#[test]
fn equal_values_still_reformat_and_returned_text_is_detached() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"subject: =?utf-8?q?hello?=\nSubject: duplicate\n"),
        None,
    );
    let mut edited = message.subject().unwrap();
    edited.text.push_str(" changed");
    assert_eq!(message.subject().unwrap().text, "hello");
    message.set_subject("hello").unwrap();
    assert_eq!(
        message.headers().get("Subject").unwrap().raw(),
        b"Subject: hello\r\n"
    );
    assert_eq!(message.headers().get_all("Subject").count(), 1);
    message.set_subject(&edited.text).unwrap();
    assert_eq!(message.subject().unwrap().text, "hello changed");
}

#[test]
fn empty_assignment_collapses_duplicates_without_removing_subject() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"subject:\nX-Keep: unchanged\nSUBJECT: duplicate\n"),
        None,
    );
    message.set_subject(" \r\n\t").unwrap();
    assert_eq!(
        snapshot(&message),
        [b"Subject:\r\n".to_vec(), b"X-Keep: unchanged\n".to_vec()]
    );
    assert_eq!(message.subject().unwrap().text, "");
}

#[test]
fn formatting_failure_keeps_all_original_headers() {
    let mut message: MimeMessage = MimeMessage::from_parts(
        headers(b"X-Before: one\r\nSubject: first\r\nSubject: second\r\n"),
        None,
    );
    let before = snapshot(&message);
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
            message
                .set_subject_with_options("caf\u{e9}", &options)
                .is_err()
        );
        assert_eq!(snapshot(&message), before);
    }
    message.headers_mut().remove_all("Subject");
    let before = snapshot(&message);
    let options = UnstructuredHeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    assert!(message.set_subject_with_options("", &options).is_err());
    assert_eq!(snapshot(&message), before);
}

#[test]
fn custom_formatting_and_encoded_lookalikes_round_trip() {
    let mut message: MimeMessage =
        MimeMessage::from_parts(headers(b"subject: old\nSUBJECT: duplicate\n"), None);
    let options = UnstructuredHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    let value = "literal =?utf-8?q?subject?= ".repeat(20);
    message.set_subject_with_options(&value, &options).unwrap();
    assert_eq!(message.headers().get_all("Subject").count(), 1);
    assert_eq!(message.subject().unwrap().text, value.trim());
    let raw = message.headers().get("Subject").unwrap().raw();
    assert!(!raw.contains(&b'\r'));
    assert!(
        std::str::from_utf8(raw)
            .unwrap()
            .lines()
            .all(|line| line.len() <= 60)
    );
}

#[test]
fn raw_edits_and_removing_first_header_are_immediately_visible() {
    let mut message: MimeMessage =
        MimeMessage::from_parts(headers(b"Subject: first\r\nSubject: second\r\n"), None);
    message.headers_mut().remove("Subject");
    assert_eq!(message.subject().unwrap().text, "second");
    message
        .headers_mut()
        .replace_all(Header::parse(b"Subject: third").unwrap())
        .unwrap();
    assert_eq!(message.subject().unwrap().text, "third");
    message.headers_mut().clear();
    assert!(message.subject().is_none());
}

#[test]
fn subject_serialization_does_not_inject_headers_or_generate_metadata() {
    let mut message = MimeMessage::with_body(TextPart::new("plain", "hello").unwrap().into());
    message
        .set_subject("news\r\nBcc: injected@example.com")
        .unwrap();
    let before = snapshot(&message);
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(snapshot(&loaded), before);
    assert_eq!(
        loaded.subject().unwrap().text,
        "newsBcc: injected@example.com"
    );
    assert!(!loaded.headers().contains("Bcc"));
    assert!(!loaded.headers().contains("Message-Id"));
    assert!(!loaded.headers().contains("Date"));
    assert!(!loaded.headers().contains("MIME-Version"));
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("Subject access must not read body sources")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("Subject access must not seek body sources")
    }
}

#[test]
fn subject_access_ignores_body_headers_and_sources() {
    let part = MimePart::from_parts(
        headers(b"Subject: body subject\r\n"),
        MimeContent::new(NoIo),
    );
    let mut message = MimeMessage::with_body(part.into());
    let before = message.body().unwrap().headers().clone();
    assert!(message.subject().is_none());
    message.set_subject("message subject").unwrap();
    assert_eq!(message.subject().unwrap().text, "message subject");
    assert_eq!(message.body().unwrap().headers(), &before);
}

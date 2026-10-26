// Adapted from MimeKit's UnitTests/MimePartTests.cs and MimeEntity.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    AttachmentEditError, ContentDisposition, ContentHeaderParseError, Header, HeaderFormatError,
    HeaderFormatOptions, HeaderList, MessagePart, MimeContent, MimeEntity, MimeMessage, MimePart,
    Multipart, TextFormat, TextPart, encodings::NewLineFormat,
};

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("attachment flags must not read content")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("attachment flags must not seek content")
    }
}

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn leaf() -> MimeEntity<NoIo> {
    MimePart::from_parts(HeaderList::new(), MimeContent::new(NoIo)).into()
}

fn entities(raw: &[u8]) -> [MimeEntity<NoIo>; 3] {
    [
        MimePart::from_parts(headers(raw), MimeContent::new(NoIo)).into(),
        Multipart::from_parts(
            headers(raw),
            vec![leaf()],
            b"preamble".to_vec(),
            b"epilogue".to_vec(),
        )
        .into(),
        MessagePart::from_parts(
            headers(raw),
            Some(MimeMessage::from_parts(
                headers(b"Content-Disposition: attachment\n"),
                Some(leaf()),
            )),
        )
        .into(),
    ]
}

fn get_concrete<R>(entity: &MimeEntity<R>) -> Result<bool, ContentHeaderParseError> {
    match entity {
        MimeEntity::Part(part) => part.is_attachment(),
        MimeEntity::Multipart(part) => part.is_attachment(),
        MimeEntity::Message(part) => part.is_attachment(),
    }
}

fn set_concrete<R>(entity: &mut MimeEntity<R>, value: bool) -> Result<(), AttachmentEditError> {
    match entity {
        MimeEntity::Part(part) => part.set_is_attachment(value),
        MimeEntity::Multipart(part) => part.set_is_attachment(value),
        MimeEntity::Message(part) => part.set_is_attachment(value),
    }
}

fn set_concrete_with_options<R>(
    entity: &mut MimeEntity<R>,
    value: bool,
    options: &HeaderFormatOptions,
) -> Result<(), AttachmentEditError> {
    match entity {
        MimeEntity::Part(part) => part.set_is_attachment_with_options(value, options),
        MimeEntity::Multipart(part) => part.set_is_attachment_with_options(value, options),
        MimeEntity::Message(part) => part.set_is_attachment_with_options(value, options),
    }
}

fn disposition<R>(entity: &MimeEntity<R>) -> ContentDisposition {
    entity
        .headers()
        .get("Content-Disposition")
        .unwrap()
        .parse_content_disposition()
        .unwrap()
}

fn parameters(value: &ContentDisposition) -> Vec<(String, String)> {
    value
        .parameters()
        .iter()
        .map(|p| (p.name().to_owned(), p.value().to_owned()))
        .collect()
}

#[test]
fn missing_dispositions_follow_upstream_flag_transitions_on_all_types() {
    for mut entity in entities(b"Content-Type: application/octet-stream; name=file.txt\n") {
        let before = entity.headers().clone();
        assert!(!entity.is_attachment().unwrap());
        assert!(!get_concrete(&entity).unwrap());
        entity.set_is_attachment(false).unwrap();
        assert_eq!(entity.headers(), &before);
        set_concrete(&mut entity, true).unwrap();
        assert!(entity.is_attachment().unwrap());
        assert!(get_concrete(&entity).unwrap());
        assert_eq!(
            entity.headers().as_slice()[1].raw(),
            b"Content-Disposition: attachment\r\n"
        );
        set_concrete(&mut entity, false).unwrap();
        assert!(!entity.is_attachment().unwrap());
        assert_eq!(
            entity.headers().as_slice()[1].raw(),
            b"Content-Disposition: inline\r\n"
        );
        entity.set_is_attachment(true).unwrap();
        assert!(get_concrete(&entity).unwrap());
        entity.headers_mut().remove_all("Content-Disposition");
        set_concrete(&mut entity, false).unwrap();
        assert_eq!(entity.headers(), &before);
    }
}

#[test]
fn matching_flags_preserve_raw_bytes_duplicates_and_extension_tokens() {
    let invalid = HeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    for (first, value) in [
        ("(before) AtTaChMeNt (after); filename=report.txt", true),
        ("inline; filename=report.txt", false),
        ("X-Custom; filename=report.txt", false),
    ] {
        let raw = format!(
            "X-Before: untouched\ncontent-disposition: {first}\nContent-Disposition: invalid syntax\nX-After: untouched\n"
        );
        for mut entity in entities(raw.as_bytes()) {
            let before = entity.headers().clone();
            assert_eq!(get_concrete(&entity).unwrap(), value);
            entity.set_is_attachment(value).unwrap();
            set_concrete(&mut entity, value).unwrap();
            entity
                .set_is_attachment_with_options(value, &invalid)
                .unwrap();
            set_concrete_with_options(&mut entity, value, &invalid).unwrap();
            assert_eq!(entity.headers(), &before);
        }
    }
    for mut entity in entities(b"X-Keep: raw\n") {
        let before = entity.headers().clone();
        entity
            .set_is_attachment_with_options(false, &invalid)
            .unwrap();
        set_concrete_with_options(&mut entity, false, &invalid).unwrap();
        assert_eq!(entity.headers(), &before);
    }
}

#[test]
fn changed_flags_preserve_first_parameters_and_replace_duplicates_at_the_first_slot() {
    for (token, requested, expected) in [
        ("inline", true, "attachment"),
        ("X-Custom", true, "attachment"),
        ("AtTaChMeNt", false, "inline"),
    ] {
        let raw = format!(
            "X-Before: raw\ncontent-disposition \t: {token}; filename*=utf-8''caf%C3%A9.txt;\n size=42; creation-date=\"Tue, 14 Sep 2021 12:34:56 +0000\"; x-custom=\"keep me\"\nMalformed record\nCONTENT-DISPOSITION: inline; filename=ignored.txt\nX-After: raw\n"
        );
        for mut entity in entities(raw.as_bytes()) {
            let old = disposition(&entity);
            let mut expected_disposition = old.clone();
            expected_disposition.set_is_attachment(requested);
            entity.set_is_attachment(requested).unwrap();
            assert_eq!(get_concrete(&entity).unwrap(), requested);
            let actual = disposition(&entity);
            assert_eq!(actual.disposition(), expected);
            assert_eq!(parameters(&actual), parameters(&old));
            assert_eq!(actual.file_name(), Some("caf\u{e9}.txt"));
            assert_eq!(entity.headers().len(), 4);
            assert_eq!(entity.headers().as_slice()[0].raw(), b"X-Before: raw\n");
            assert_eq!(
                entity.headers().as_slice()[1].raw(),
                expected_disposition.to_header().unwrap().raw()
            );
            assert_eq!(entity.headers().as_slice()[2].raw(), b"Malformed record\n");
            assert_eq!(entity.headers().as_slice()[3].raw(), b"X-After: raw\n");
        }
    }
}

#[test]
fn changed_headers_use_checked_parameter_encoding_and_requested_line_format() {
    let raw = b"Content-Disposition: inline; filename*=utf-8''caf%C3%A9-abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz.txt; size=12; x=hello\n";
    for max_line_length in [60, 78, 998] {
        for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            for always_quote_parameters in [false, true] {
                let options = HeaderFormatOptions {
                    max_line_length,
                    new_line_format,
                    always_quote_parameters,
                    ..Default::default()
                };
                for mut entity in entities(raw) {
                    let original = disposition(&entity);
                    set_concrete_with_options(&mut entity, true, &options).unwrap();
                    assert_eq!(parameters(&disposition(&entity)), parameters(&original));
                    let output = entity.headers().get("Content-Disposition").unwrap().raw();
                    assert!(output.is_ascii());
                    for line in output.split(|&b| b == b'\n') {
                        assert!(line.strip_suffix(b"\r").unwrap_or(line).len() <= max_line_length);
                    }
                    if new_line_format == NewLineFormat::Unix {
                        assert!(!output.contains(&b'\r'));
                    } else {
                        assert!(output.ends_with(b"\r\n"));
                    }
                    let mut expected = original;
                    expected.set_is_attachment(true);
                    assert_eq!(
                        output,
                        expected.to_header_with_options(&options).unwrap().raw()
                    );
                    entity
                        .set_is_attachment_with_options(false, &options)
                        .unwrap();
                    assert_eq!(parameters(&disposition(&entity)), parameters(&expected));
                    assert!(!get_concrete(&entity).unwrap());
                }
            }
        }
    }
}

#[test]
fn invalid_first_dispositions_return_typed_errors_without_repair_or_mutation() {
    let invalid_options = HeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    for first in ["", "\"attachment\"", "attachment invalid", "(unclosed"] {
        let raw =
            format!("X-Keep: raw\nContent-Disposition: {first}\nCONTENT-DISPOSITION: attachment\n");
        for mut entity in entities(raw.as_bytes()) {
            let before = entity.headers().clone();
            let error = before
                .get("Content-Disposition")
                .unwrap()
                .parse_content_disposition()
                .unwrap_err();
            assert_eq!(entity.is_attachment(), Err(error));
            assert_eq!(get_concrete(&entity), Err(error));
            for value in [false, true] {
                let expected = AttachmentEditError::ContentDisposition(error);
                assert_eq!(entity.set_is_attachment(value), Err(expected.clone()));
                assert_eq!(set_concrete(&mut entity, value), Err(expected.clone()));
                assert_eq!(
                    entity.set_is_attachment_with_options(value, &invalid_options),
                    Err(expected.clone())
                );
                assert_eq!(entity.headers(), &before);
                assert_eq!(expected.source().unwrap().to_string(), error.to_string());
                assert_eq!(expected.to_string(), error.to_string());
            }
        }
    }
}

#[test]
fn formatting_failures_leave_absent_or_existing_dispositions_untouched() {
    for options in [
        HeaderFormatOptions {
            max_line_length: 0,
            ..Default::default()
        },
        HeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        },
        HeaderFormatOptions {
            max_line_length: 999,
            ..Default::default()
        },
        HeaderFormatOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
    ] {
        for (raw, value) in [
            ("X-Keep: raw\n", true),
            (
                "Content-Disposition: inline; filename=keep.txt\nContent-Disposition: attachment\n",
                true,
            ),
            (
                "Content-Disposition: attachment; filename=keep.txt\nContent-Disposition: inline\n",
                false,
            ),
        ] {
            for mut entity in entities(raw.as_bytes()) {
                let before = entity.headers().clone();
                let mut expected = if before.contains("Content-Disposition") {
                    disposition(&entity)
                } else {
                    ContentDisposition::default()
                };
                expected.set_is_attachment(value);
                let error = expected.to_header_with_options(&options).unwrap_err();
                assert_eq!(
                    set_concrete_with_options(&mut entity, value, &options),
                    Err(AttachmentEditError::HeaderFormat(error.clone()))
                );
                let actual = entity
                    .set_is_attachment_with_options(value, &options)
                    .unwrap_err();
                assert_eq!(actual, AttachmentEditError::HeaderFormat(error.clone()));
                assert_eq!(actual.source().unwrap().to_string(), error.to_string());
                assert_eq!(actual.to_string(), error.to_string());
                assert_eq!(entity.headers(), &before);
            }
        }
    }

    let raw = format!("Content-Disposition: inline; {}=value\n", "x".repeat(100));
    for mut entity in entities(raw.as_bytes()) {
        let before = entity.headers().clone();
        assert!(matches!(
            entity.set_is_attachment(true),
            Err(AttachmentEditError::HeaderFormat(
                HeaderFormatError::LineTooLong { .. }
            ))
        ));
        assert_eq!(entity.headers(), &before);
        entity.set_is_attachment(false).unwrap();
        assert_eq!(entity.headers(), &before);
    }
}

#[test]
fn tolerant_parameter_recovery_is_shared_with_existing_disposition_editing() {
    let raw = b"Content-Disposition: inline; filename=first.txt; filename=second.txt; size=42\n";
    for mut entity in entities(raw) {
        let parsed = disposition(&entity);
        assert!(!parsed.parameters().issues().is_empty());
        let mut expected = parsed.clone();
        expected.set_is_attachment(true);
        entity.set_is_attachment(true).unwrap();
        assert_eq!(parameters(&disposition(&entity)), parameters(&parsed));
        assert_eq!(
            entity.headers().get("Content-Disposition").unwrap().raw(),
            expected.to_header().unwrap().raw()
        );
        assert!(!parsed.parameters().issues().is_empty());
    }
}

#[test]
fn raw_header_edits_are_visible_and_do_not_leave_cached_flags() {
    for mut entity in entities(b"Content-Disposition: attachment\nContent-Disposition: inline\n") {
        assert!(get_concrete(&entity).unwrap());
        entity.headers_mut().remove("Content-Disposition");
        assert!(!entity.is_attachment().unwrap());
        entity
            .headers_mut()
            .replace_all(Header::parse(b"Content-Disposition: ATTACHMENT").unwrap())
            .unwrap();
        assert!(get_concrete(&entity).unwrap());
        entity.headers_mut().clear();
        assert!(!get_concrete(&entity).unwrap());
        entity.set_is_attachment(false).unwrap();
        assert!(entity.headers().is_empty());
    }
}

#[test]
fn node_flags_do_not_modify_nested_metadata_or_need_source_traits() {
    for mut entity in entities(b"Content-Type: invalid\n") {
        set_concrete(&mut entity, true).unwrap();
        match &entity {
            MimeEntity::Multipart(part) => {
                assert!(!part.children()[0].is_attachment().unwrap());
                assert!(part.children()[0].headers().is_empty());
                assert_eq!(part.preamble(), b"preamble");
                assert_eq!(part.epilogue(), b"epilogue");
                assert_eq!(entity.attachments().count(), 0);
            }
            MimeEntity::Message(part) => {
                let message = part.message().unwrap();
                assert_eq!(
                    message.headers(),
                    &headers(b"Content-Disposition: attachment\n")
                );
                assert!(message.body().unwrap().headers().is_empty());
                assert_eq!(entity.attachments().count(), 1);
            }
            MimeEntity::Part(_) => assert_eq!(entity.attachments().count(), 1),
        }
        assert_eq!(
            entity.headers().get("Content-Type").unwrap().raw(),
            b"Content-Type: invalid\n"
        );
    }
    let mut wrapper = MessagePart::new(MimeMessage::<()>::new());
    wrapper.set_is_attachment(true).unwrap();
    assert!(wrapper.is_attachment().unwrap());
    let mut container: Multipart<()> = Multipart::with_boundary("mixed", "boundary").unwrap();
    container.set_is_attachment(true).unwrap();
    assert!(container.is_attachment().unwrap());
    container.children_mut().push(wrapper.into());
    let mut entity: MimeEntity<()> = container.into();
    entity.set_is_attachment(false).unwrap();
    assert!(!entity.is_attachment().unwrap());
    assert_eq!(entity.attachments().count(), 1);
}

#[test]
fn text_selection_and_attachment_enumeration_observe_flag_changes() {
    let mut text = TextPart::new("plain", "body").unwrap();
    assert!(!text.part().is_attachment().unwrap());
    text.part_mut().set_is_attachment(true).unwrap();
    let mut entity: MimeEntity = text.into();
    assert!(entity.text_body_part(TextFormat::Plain).unwrap().is_none());
    assert_eq!(entity.attachments().count(), 1);
    entity.set_is_attachment(false).unwrap();
    assert!(entity.text_body_part(TextFormat::Plain).unwrap().is_some());
    assert_eq!(entity.attachments().count(), 0);
}

#[test]
fn edits_leave_borrowed_source_position_encoding_and_bytes_unchanged() {
    let mut source = Cursor::new(b"Ym9keQ==".to_vec());
    source.set_position(3);
    {
        let content = MimeContent::with_encoding(&mut source, mimecraft::ContentEncoding::Base64);
        let mut part = MimePart::from_parts(headers(b"Content-Type: text/plain\n"), content);
        part.set_is_attachment(true).unwrap();
        assert!(part.is_attachment().unwrap());
        part.set_is_attachment(false).unwrap();
        assert_eq!(
            part.content().encoding(),
            mimecraft::ContentEncoding::Base64
        );
    }
    assert_eq!(source.position(), 3);
    assert_eq!(source.into_inner(), b"Ym9keQ==");
}

#[test]
fn serialized_flags_round_trip_and_malformed_dispositions_do_not_reject_loading() {
    let mut text = TextPart::new("plain", "body").unwrap();
    text.part_mut().set_is_attachment(true).unwrap();
    let mut message = MimeMessage::with_body(text.into());
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    let mut loaded = MimeMessage::load(&mut output.as_slice()).unwrap();
    assert_eq!(loaded.attachments().count(), 1);
    loaded.body_mut().unwrap().set_is_attachment(false).unwrap();
    assert_eq!(loaded.attachments().count(), 0);

    let raw = b"Subject: tolerant\r\nContent-Type: text/plain\r\nContent-Disposition: \"attachment\"\r\n\r\nbody";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    assert!(message.body_mut().unwrap().set_is_attachment(true).is_err());
    output.clear();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, raw);
}

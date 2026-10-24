// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, DispositionNotificationError, DispositionNotificationParseOptions, Header,
    HeaderList, MessageDispositionNotification, MimeContent, MimeEntity, MimePart,
    anonymize::{self, HeaderOptions},
    encodings::NewLineFormat,
};

fn part<R: Read + Seek>(source: R, encoding: ContentEncoding) -> MimePart<R> {
    MimePart::from_parts(
        vec![Header::parse(b"Content-Type: message/disposition-notification").unwrap()].into(),
        MimeContent::with_encoding(source, encoding),
    )
}

fn notification(bytes: &[u8]) -> MessageDispositionNotification {
    MessageDispositionNotification::from_part(part(
        Cursor::new(bytes.to_vec()),
        ContentEncoding::Unspecified,
    ))
    .unwrap()
}

fn fields(bytes: &[u8]) -> HeaderList {
    HeaderList::load(&mut &bytes[..]).unwrap()
}

fn io_error(error: DispositionNotificationError, kind: io::ErrorKind) {
    assert!(error.source().is_some());
    assert!(matches!(error, DispositionNotificationError::Io(inner) if inner.kind() == kind));
}

#[test]
fn construction_checks_first_media_type_and_accepts_global_subtype() {
    let mut mdn = MessageDispositionNotification::default();
    assert!(mdn.fields().unwrap().is_empty());
    assert_eq!(
        mdn.part().headers().get("Content-Type").unwrap().raw(),
        b"Content-Type: message/disposition-notification\r\n"
    );
    mdn.part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    assert!(mdn.fields().unwrap().is_empty());
    mdn.part_mut()
        .headers_mut()
        .replace_all(
            Header::parse(b"Content-Type: MESSAGE/GLOBAL-DISPOSITION-NOTIFICATION").unwrap(),
        )
        .unwrap();
    let mut mdn = MessageDispositionNotification::from_part(mdn.into_part()).unwrap();
    assert!(mdn.fields().unwrap().is_empty());

    let missing =
        MimePart::from_parts(HeaderList::new(), MimeContent::new(Cursor::new(Vec::new())));
    assert!(matches!(
        MessageDispositionNotification::from_part(missing),
        Err(DispositionNotificationError::MissingContentType)
    ));
    let wrong = MimePart::new(MimeContent::new(Cursor::new(Vec::new())));
    assert!(matches!(
        MessageDispositionNotification::from_part(wrong),
        Err(DispositionNotificationError::NotDispositionNotification(_))
    ));
    mdn.part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    let error = mdn.fields().unwrap_err();
    assert!(matches!(
        &error,
        DispositionNotificationError::ContentType(_)
    ));
    assert!(error.source().is_some());
}

#[test]
fn canonical_fields_serialize_as_one_terminated_header_block() {
    let bytes = b"Reporting-UA: joes-pc.cs.example.com; Foomail 97.1\r\n\
Original-Recipient: rfc822;Joe_Recipient@example.com\r\n\
Final-Recipient: rfc822;Joe_Recipient@example.com\r\n\
Original-Message-ID: <199509192301.23456@example.org>\r\n\
Disposition: manual-action/MDN-sent-manually; displayed\r\n\r\n";
    let parsed = fields(bytes);
    assert_eq!(parsed.len(), 5);
    let mut mdn = MessageDispositionNotification::new();
    mdn.set_fields(&parsed).unwrap();
    assert_eq!(mdn.part().content().get_ref().get_ref(), bytes);
    assert_eq!(mdn.fields().unwrap(), parsed);
    assert_eq!(
        mdn.fields()
            .unwrap()
            .get("Disposition")
            .unwrap()
            .decode_value()
            .text,
        "manual-action/MDN-sent-manually; displayed",
    );
}

#[test]
fn raw_fields_preserve_folds_duplicates_malformed_and_non_utf8_bytes() {
    let bytes = b"bad record\r\nReporting-UA: client;\r\n version\r\nX: \xff\r\nX: two\r\n";
    let mut mdn = notification(bytes);
    let parsed = mdn.fields().unwrap();
    assert_eq!(parsed, fields(bytes));
    assert!(parsed.as_slice()[0].is_invalid());
    assert_eq!(parsed.get_all("X").count(), 2);
    mdn.set_fields(&parsed).unwrap();
    assert_eq!(mdn.fields().unwrap(), parsed);
    let mut expected = bytes.to_vec();
    expected.extend_from_slice(b"\r\n");
    assert_eq!(mdn.part().content().get_ref().get_ref(), &expected);
}

#[test]
fn only_first_block_is_returned_without_skipping_blanks_or_decoding_field_cte() {
    for bytes in [&b""[..], b"\r", b"\r\nX: ignored", b"\n\nX: ignored"] {
        assert!(notification(bytes).fields().unwrap().is_empty());
    }
    let bytes = b"Content-Transfer-Encoding: unknown\r\n\r\nX: trailing\r\n";
    let mut mdn = notification(bytes);
    let parsed = mdn.fields().unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(mdn.part().content().get_ref().get_ref(), bytes);
    mdn.set_fields(&parsed).unwrap();
    assert_eq!(mdn.fields().unwrap(), parsed);
    assert_eq!(
        mdn.part().content().get_ref().get_ref(),
        b"Content-Transfer-Encoding: unknown\r\n\r\n"
    );
    let mut mdn = notification(b"Content-Transfer-Encoding: base64\r\n\r\nWDogYQ==");
    assert_eq!(mdn.fields().unwrap().len(), 1);
}

#[test]
fn eof_without_separator_or_final_newline_is_accepted() {
    let mut mdn = notification(b"X: value");
    let parsed = mdn.fields().unwrap();
    assert_eq!(parsed.as_slice()[0].raw(), b"X: value");
    mdn.set_fields(&parsed).unwrap();
    assert_eq!(
        mdn.part().content().get_ref().get_ref(),
        b"X: value\r\n\r\n"
    );
}

#[test]
fn detached_edits_are_atomic_and_keep_outer_metadata() {
    let mut mdn = notification(b"X: old\r\n\r\ntrailing");
    mdn.part_mut()
        .set_content_transfer_encoding(ContentEncoding::Base64);
    mdn.part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Length: 999").unwrap());
    mdn.part_mut()
        .content_mut()
        .set_new_line_format(Some(NewLineFormat::Mixed));
    let headers = mdn.part().headers().clone();
    let mut parsed = mdn.fields().unwrap();
    parsed
        .replace_all(Header::parse(b"X: edited").unwrap())
        .unwrap();
    assert_eq!(
        mdn.part().content().get_ref().get_ref(),
        b"X: old\r\n\r\ntrailing"
    );
    mdn.part_mut().content_mut().get_mut().set_position(3);
    let before = mdn.part().content().get_ref().clone();
    io_error(
        mdn.set_fields_with_new_line_format(&parsed, NewLineFormat::Mixed)
            .unwrap_err(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(mdn.part().content().get_ref(), &before);
    assert_eq!(mdn.part().headers(), &headers);
    assert_eq!(
        mdn.part().content().new_line_format(),
        Some(NewLineFormat::Mixed)
    );
    mdn.set_fields_with_new_line_format(&parsed, NewLineFormat::Unix)
        .unwrap();
    assert_eq!(mdn.part().content().get_ref().get_ref(), b"X: edited\n\n");
    assert_eq!(
        mdn.part().content().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(mdn.part().content().get_ref().position(), 0);
    assert_eq!(mdn.part().content().new_line_format(), None);
    assert_eq!(mdn.part().headers(), &headers);
    mdn.set_fields(&HeaderList::new()).unwrap();
    assert_eq!(mdn.part().content().get_ref().get_ref(), b"\r\n");
    assert!(mdn.fields().unwrap().is_empty());
}

#[test]
fn stored_encoding_controls_reading_and_mutations_are_not_cached() {
    let mut mdn = MessageDispositionNotification::from_part(part(
        Cursor::new(b"WDogYQ==".to_vec()),
        ContentEncoding::Base64,
    ))
    .unwrap();
    mdn.part_mut()
        .set_content_transfer_encoding(ContentEncoding::SevenBit);
    mdn.part_mut().content_mut().get_mut().set_position(4);
    let headers = mdn.part().headers().clone();
    assert_eq!(mdn.fields().unwrap(), fields(b"X: a"));
    assert_eq!(mdn.part().content().get_ref().position(), 0);
    assert_eq!(mdn.part().headers(), &headers);
    assert_eq!(mdn.part().content().get_ref().get_ref(), b"WDogYQ==");
    mdn.part_mut()
        .set_content(MimeContent::new(Cursor::new(b"X: new".to_vec())));
    assert_eq!(mdn.fields().unwrap(), fields(b"X: new"));
}

#[test]
fn byte_limits_cover_decoded_body_including_unreturned_tail() {
    let bytes = b"X: a\r\n\r\ntail";
    let mut mdn = notification(bytes);
    let mut options = DispositionNotificationParseOptions {
        max_decoded_bytes: bytes.len(),
        ..Default::default()
    };
    assert_eq!(
        mdn.fields_with_options(&options).unwrap(),
        fields(b"X: a\r\n")
    );
    options.max_decoded_bytes -= 1;
    io_error(
        mdn.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    let mut mdn = MessageDispositionNotification::from_part(part(
        Cursor::new(b"WDogYQ=="),
        ContentEncoding::Base64,
    ))
    .unwrap();
    options.max_decoded_bytes = 4;
    assert_eq!(mdn.fields_with_options(&options).unwrap(), fields(b"X: a"));
    options.max_decoded_bytes = 3;
    io_error(
        mdn.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn folded_record_limits_and_default_16_kib_are_enforced() {
    let bytes = b"X: a\r\n b\r\n";
    let mut mdn = notification(bytes);
    let mut options = DispositionNotificationParseOptions {
        max_header_length: bytes.len(),
        ..Default::default()
    };
    assert_eq!(mdn.fields_with_options(&options).unwrap().len(), 1);
    options.max_header_length -= 1;
    io_error(
        mdn.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(mdn.part().content().get_ref().position(), 0);
    assert_eq!(
        DispositionNotificationParseOptions::default().max_header_length,
        16 * 1024
    );
    let mut bytes = b"X: ".to_vec();
    bytes.resize(16 * 1024, b'a');
    assert_eq!(notification(&bytes).fields().unwrap().len(), 1);
    bytes.push(b'a');
    io_error(
        notification(&bytes).fields().unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn global_notification_round_trips_inside_report_with_outer_transfer_encoding() {
    let wire = b"Content-Type: multipart/report; boundary=r; report-type=global-disposition-notification\r\n\r\n\
--r\r\nContent-Type: message/global-disposition-notification\r\n\r\n\
Final-Recipient: utf-8; Ren\xc3\xa9e@example.com\r\n\
Disposition: manual-action/MDN-sent-manually; displayed\r\n\r\n\
\r\n--r--\r\n";
    let MimeEntity::Multipart(mut report) = MimeEntity::load(&mut &wire[..]).unwrap() else {
        panic!()
    };
    let MimeEntity::Part(part) = report.children_mut().remove(0) else {
        panic!()
    };
    let mut mdn = MessageDispositionNotification::from_part(part).unwrap();
    let parsed = mdn.fields().unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed.get("Final-Recipient").unwrap().raw(),
        b"Final-Recipient: utf-8; Ren\xc3\xa9e@example.com\r\n"
    );
    mdn.set_fields(&parsed).unwrap();
    mdn.part_mut()
        .set_content_transfer_encoding(ContentEncoding::Base64);
    let mut entity: MimeEntity = mdn.into();
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    let MimeEntity::Part(part) = MimeEntity::load(&mut output.as_slice()).unwrap() else {
        panic!()
    };
    assert_eq!(part.content().encoding(), ContentEncoding::Base64);
    assert_eq!(
        MessageDispositionNotification::from_part(part)
            .unwrap()
            .fields()
            .unwrap(),
        parsed
    );
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_after: Option<u64>,
    fail_seek: Option<usize>,
    interrupt_once: bool,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(b"X: a\r\n\r\ntail".to_vec()),
            reads: 0,
            seeks: 0,
            fail_after: None,
            fail_seek: None,
            interrupt_once: false,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self
            .fail_after
            .is_some_and(|offset| self.bytes.position() >= offset)
        {
            return Err(io::Error::other("read failed"));
        }
        if std::mem::take(&mut self.interrupt_once) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let size = buffer.len().min(1);
        self.bytes.read(&mut buffer[..size])
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn borrowed_fragmented_source_retries_interruptions_and_rewinds() {
    let mut source = Source {
        interrupt_once: true,
        ..Source::new()
    };
    {
        let mut mdn = MessageDispositionNotification::from_part(part(
            &mut source,
            ContentEncoding::Unspecified,
        ))
        .unwrap();
        assert_eq!(mdn.fields().unwrap(), fields(b"X: a\r\n"));
    }
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 2);
}

#[test]
fn read_initial_seek_final_rewind_and_trailing_read_failures_propagate() {
    for (fail_after, fail_seek) in [
        (Some(0), None),
        (Some(8), None),
        (None, Some(1)),
        (None, Some(2)),
    ] {
        let mut source = Source {
            fail_after,
            fail_seek,
            ..Source::new()
        };
        let mut mdn = MessageDispositionNotification::from_part(part(
            &mut source,
            ContentEncoding::Unspecified,
        ))
        .unwrap();
        io_error(mdn.fields().unwrap_err(), io::ErrorKind::Other);
    }
}

#[test]
fn masked_content_source_and_trailing_read_failures_do_not_emit_fields() {
    for (fail_after, fail_seek) in [
        (Some(0), None),
        (Some(8), None),
        (None, Some(1)),
        (None, Some(2)),
    ] {
        let mut source = Source {
            fail_after,
            fail_seek,
            ..Source::new()
        };
        let mut mdn = MessageDispositionNotification::from_part(part(
            &mut source,
            ContentEncoding::Unspecified,
        ))
        .unwrap();
        let mut output = b"prefix".to_vec();
        io_error(
            anonymize::write_disposition_notification_content(&mut mdn, &mut output).unwrap_err(),
            io::ErrorKind::Other,
        );
        assert_eq!(output, b"prefix");
    }
    let mut source = Source {
        interrupt_once: true,
        ..Source::new()
    };
    let mut mdn =
        MessageDispositionNotification::from_part(part(&mut source, ContentEncoding::Unspecified))
            .unwrap();
    let mut output = Vec::new();
    anonymize::write_disposition_notification_content(&mut mdn, &mut output).unwrap();
    assert_eq!(output, b"X: x\r\n");
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 2);
}

#[test]
fn invalid_options_and_media_edits_fail_before_io_and_preserve_owned_body() {
    let mut source = Source::new();
    {
        let mut mdn = MessageDispositionNotification::from_part(part(
            &mut source,
            ContentEncoding::Unspecified,
        ))
        .unwrap();
        for options in [
            DispositionNotificationParseOptions {
                max_decoded_bytes: 0,
                ..Default::default()
            },
            DispositionNotificationParseOptions {
                max_header_length: 0,
                ..Default::default()
            },
        ] {
            io_error(
                mdn.fields_with_options(&options).unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
            io_error(
                anonymize::write_disposition_notification_content_with_options(
                    &mut mdn,
                    &mut Vec::new(),
                    &HeaderOptions::default(),
                    NewLineFormat::Dos,
                    &options,
                )
                .unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
        }
        io_error(
            anonymize::write_disposition_notification_content_with_options(
                &mut mdn,
                &mut Vec::new(),
                &HeaderOptions::default(),
                NewLineFormat::Mixed,
                &DispositionNotificationParseOptions::default(),
            )
            .unwrap_err(),
            io::ErrorKind::InvalidInput,
        );
        mdn.part_mut()
            .headers_mut()
            .replace_all(Header::parse(b"Content-Type: text/plain").unwrap())
            .unwrap();
        assert!(matches!(
            mdn.fields(),
            Err(DispositionNotificationError::NotDispositionNotification(_))
        ));
        assert!(matches!(
            anonymize::write_disposition_notification_content(&mut mdn, &mut Vec::new()),
            Err(DispositionNotificationError::NotDispositionNotification(_))
        ));
    }
    assert_eq!(source.reads, 0);
    assert_eq!(source.seeks, 0);
    let mut mdn = notification(b"X: old");
    mdn.part_mut().content_mut().get_mut().set_position(2);
    let before = mdn.part().content().get_ref().clone();
    mdn.part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    assert!(mdn.set_fields(&HeaderList::new()).is_err());
    assert_eq!(mdn.part().content().get_ref(), &before);
}

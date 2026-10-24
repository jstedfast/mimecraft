// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, DeliveryStatusError, DeliveryStatusParseOptions, Header, HeaderList,
    MessageDeliveryStatus, MimeContent, MimeEntity, MimePart,
    anonymize::{self, HeaderOptions},
    encodings::NewLineFormat,
};

fn raw_part<R: Read + Seek>(source: R, encoding: ContentEncoding) -> MimePart<R> {
    MimePart::from_parts(
        vec![Header::parse(b"Content-Type: message/delivery-status").unwrap()].into(),
        MimeContent::with_encoding(source, encoding),
    )
}

fn status(bytes: &[u8]) -> MessageDeliveryStatus {
    MessageDeliveryStatus::from_part(raw_part(
        Cursor::new(bytes.to_vec()),
        ContentEncoding::Unspecified,
    ))
    .unwrap()
}

fn group(bytes: &[u8]) -> HeaderList {
    HeaderList::load(&mut &bytes[..]).unwrap()
}

fn io_error(error: DeliveryStatusError, kind: io::ErrorKind) {
    assert!(error.source().is_some());
    assert!(matches!(error, DeliveryStatusError::Io(error) if error.kind() == kind));
}

#[test]
fn new_empty_and_checked_media_types() {
    let mut delivery = MessageDeliveryStatus::default();
    assert!(delivery.status_groups().unwrap().is_empty());
    assert_eq!(
        delivery.part().headers().get("Content-Type").unwrap().raw(),
        b"Content-Type: message/delivery-status\r\n"
    );
    for value in [
        "message/delivery-status",
        "MESSAGE/GLOBAL-DELIVERY-STATUS; charset=utf-8",
    ] {
        let mut part = raw_part(Cursor::new(Vec::new()), ContentEncoding::Unspecified);
        part.headers_mut()
            .replace_all(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap())
            .unwrap();
        assert!(MessageDeliveryStatus::from_part(part).is_ok());
    }
    let missing =
        MimePart::from_parts(HeaderList::new(), MimeContent::new(Cursor::new(Vec::new())));
    assert!(matches!(
        MessageDeliveryStatus::from_part(missing),
        Err(DeliveryStatusError::MissingContentType)
    ));
    let wrong = MimePart::new(MimeContent::new(Cursor::new(Vec::new())));
    assert!(matches!(
        MessageDeliveryStatus::from_part(wrong),
        Err(DeliveryStatusError::NotDeliveryStatus(_))
    ));
    delivery
        .part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    assert!(matches!(
        delivery.status_groups(),
        Err(DeliveryStatusError::ContentType(_))
    ));
    assert!(delivery.set_status_groups(&[]).is_err());
}

#[test]
fn empty_lines_and_final_unterminated_record_follow_header_parser() {
    for bytes in [&b""[..], b"\r", b"\n\n", b"\r\n\r\n\n\r"] {
        assert!(
            status(bytes).status_groups().unwrap().is_empty(),
            "{bytes:?}"
        );
    }
    let mut delivery = status(
        b"\r\n\nReporting-MTA: dns; example.com\r\n\r\n\n\
Final-Recipient: rfc822; user@example.com\nAction: failed\n\n\nStatus: 5.1.1",
    );
    let groups = delivery.status_groups().unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups[0].get("Reporting-MTA").unwrap().decode_value().text,
        "dns; example.com"
    );
    assert_eq!(groups[1].len(), 2);
    assert_eq!(groups[2].as_slice()[0].raw(), b"Status: 5.1.1");
    assert_eq!(delivery.part().content().get_ref().position(), 0);
}

#[test]
fn raw_groups_preserve_duplicates_folds_malformed_and_non_utf8_records() {
    let bytes =
        b"rEPorting-MTA: dns;\r\n example.com\r\nX-Test: \xff\r\nX-Test: two\r\nbad line\r\n\r\n";
    let mut delivery = status(bytes);
    let groups = delivery.status_groups().unwrap();
    assert_eq!(groups, vec![group(bytes)]);
    assert_eq!(groups[0].get_all("X-Test").count(), 2);
    assert!(groups[0].as_slice()[3].is_invalid());
    delivery.set_status_groups(&groups).unwrap();
    assert_eq!(delivery.part().content().get_ref().get_ref(), bytes);
}

#[test]
fn office365_encoded_remainder_and_tolerant_empty_payloads() {
    let mut delivery = status(
        b"Content-Transfer-Encoding: base64\r\nContent-Length: 123\r\n\r\nQWN0aW9uOiBmYWlsZWQNCg0K",
    );
    let groups = delivery.status_groups().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1], group(b"Action: failed\r\n"));
    assert_eq!(
        groups[0].get("Content-Length").unwrap().decode_value().text,
        "123"
    );
    delivery.set_status_groups(&groups).unwrap();
    assert!(
        delivery
            .part()
            .content()
            .get_ref()
            .get_ref()
            .ends_with(b"QWN0aW9uOiBmYWlsZWQNCg0K\r\n")
    );
    assert_eq!(delivery.status_groups().unwrap(), groups);
    for payload in ["", "DQo=", "!!!!"] {
        let bytes = format!("Content-Transfer-Encoding: base64\r\n\r\n{payload}");
        assert_eq!(status(bytes.as_bytes()).status_groups().unwrap().len(), 1);
    }
}

#[test]
fn transparent_encodings_continue_and_first_field_wins() {
    for encoding in ["7bit", "8bit", "binary"] {
        let bytes = format!(
            "Content-Transfer-Encoding: {encoding}\r\nContent-Transfer-Encoding: bogus\r\n\r\nAction: failed\r\n\r\n"
        );
        assert_eq!(status(bytes.as_bytes()).status_groups().unwrap().len(), 2);
    }
    let mut delivery = status(
        b"Reporting-MTA: dns; example.com\r\n\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\nAction: fail=65d\r\n\r\n",
    );
    assert_eq!(
        delivery.status_groups().unwrap()[2],
        group(b"Action: failed\r\n")
    );
}

#[test]
fn each_inner_encoding_round_trips_once_with_both_newline_formats() {
    for encoding in ["base64", "quoted-printable", "x-uuencode"] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let newline = if format == NewLineFormat::Dos {
                "\r\n"
            } else {
                "\n"
            };
            let groups = vec![
                group(format!("Reporting-MTA: dns; example.com{newline}").as_bytes()),
                group(format!("Content-Transfer-Encoding: {encoding}{newline}").as_bytes()),
                group(
                    format!("Content-Transfer-Encoding: unknown{newline}Action: failed{newline}")
                        .as_bytes(),
                ),
                group(format!("Status: 5.1.1{newline}").as_bytes()),
            ];
            let mut delivery = MessageDeliveryStatus::new();
            delivery
                .set_status_groups_with_new_line_format(&groups, format)
                .unwrap();
            assert_eq!(
                delivery.status_groups().unwrap(),
                groups,
                "{encoding} {format:?}"
            );
            let bytes = delivery.part().content().get_ref().get_ref();
            if encoding == "x-uuencode" {
                assert!(String::from_utf8_lossy(bytes).contains("begin 0644 status.txt"));
            }
            if format == NewLineFormat::Unix {
                assert!(!bytes.contains(&b'\r'));
            }
            delivery
                .set_status_groups_with_new_line_format(&groups[..2], format)
                .unwrap();
            assert_eq!(delivery.status_groups().unwrap(), groups[..2]);
        }
    }
}

#[test]
fn unknown_active_encoding_errors_without_partial_groups_or_mutation() {
    let mut delivery = status(b"Action: failed\r\n\r\nContent-Transfer-Encoding: bogus\r\n\r\n");
    delivery.part_mut().content_mut().get_mut().set_position(3);
    let error = delivery.status_groups().unwrap_err();
    assert!(matches!(&error, DeliveryStatusError::Io(inner)
        if inner.get_ref().unwrap().is::<mimecraft::UnknownContentEncoding>()));
    io_error(error, io::ErrorKind::InvalidData);
    assert_eq!(delivery.part().content().get_ref().position(), 0);

    delivery.part_mut().content_mut().get_mut().set_position(3);
    let before = delivery.part().content().get_ref().clone();
    io_error(
        delivery
            .set_status_groups(&[group(b"Content-Transfer-Encoding: bogus\r\n")])
            .unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(delivery.part().content().get_ref(), &before);
}

#[test]
fn detached_edits_replace_atomically_and_preserve_outer_headers() {
    let mut delivery = status(b"Action: failed");
    delivery
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::Base64);
    delivery
        .part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Length: 999").unwrap());
    let headers = delivery.part().headers().clone();
    let mut groups = delivery.status_groups().unwrap();
    groups[0]
        .replace_all(Header::parse(b"Action: delivered").unwrap())
        .unwrap();
    assert_eq!(
        delivery.part().content().get_ref().get_ref(),
        b"Action: failed"
    );
    delivery.part_mut().content_mut().get_mut().set_position(4);
    let before = delivery.part().content().get_ref().clone();
    io_error(
        delivery
            .set_status_groups(&[HeaderList::new()])
            .unwrap_err(),
        io::ErrorKind::InvalidInput,
    );
    io_error(
        delivery
            .set_status_groups_with_new_line_format(&groups, NewLineFormat::Mixed)
            .unwrap_err(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(delivery.part().content().get_ref(), &before);
    assert_eq!(delivery.part().headers(), &headers);
    delivery.set_status_groups(&groups).unwrap();
    assert_eq!(
        delivery.part().content().get_ref().get_ref(),
        b"Action: delivered\r\n\r\n"
    );
    assert_eq!(
        delivery.part().content().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(delivery.part().content().get_ref().position(), 0);
    assert_eq!(delivery.part().headers(), &headers);
    delivery.set_status_groups(&[]).unwrap();
    assert!(delivery.part().content().get_ref().get_ref().is_empty());
    assert!(delivery.status_groups().unwrap().is_empty());
}

#[test]
fn owned_entity_io_round_trip_preserves_outer_and_inner_encoding() {
    let groups = vec![
        group(b"Content-Transfer-Encoding: base64\r\n"),
        group(b"Action: failed\r\n"),
    ];
    let mut delivery = MessageDeliveryStatus::new();
    delivery.set_status_groups(&groups).unwrap();
    delivery
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    let mut entity: MimeEntity = delivery.into();
    let mut wire = Vec::new();
    entity.write_to(&mut wire).unwrap();
    let MimeEntity::Part(part) = MimeEntity::load(&mut wire.as_slice()).unwrap() else {
        panic!("delivery status must remain a leaf");
    };
    assert_eq!(part.content().encoding(), ContentEncoding::QuotedPrintable);
    let stored = part.content().get_ref().get_ref().clone();
    let headers = part.headers().clone();
    let mut delivery = MessageDeliveryStatus::from_part(part).unwrap();
    delivery
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::SevenBit);
    for _ in 0..2 {
        assert_eq!(delivery.status_groups().unwrap(), groups);
        assert_eq!(delivery.part().content().get_ref().position(), 0);
        assert_eq!(delivery.part().content().get_ref().get_ref(), &stored);
    }
    delivery
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    assert_eq!(delivery.into_part().headers(), &headers);
}

#[test]
fn byte_limits_are_exact_and_include_both_decoding_stages() {
    let bytes = b"Action: failed\r\n\r\n";
    let mut delivery = status(bytes);
    let mut options = DeliveryStatusParseOptions {
        max_decoded_bytes: bytes.len(),
        ..Default::default()
    };
    assert_eq!(
        delivery.status_groups_with_options(&options).unwrap().len(),
        1
    );
    options.max_decoded_bytes -= 1;
    io_error(
        delivery.status_groups_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );

    let bytes = b"Content-Transfer-Encoding: base64\r\n\r\nQWN0aW9uOiBmYWlsZWQNCg0K";
    let mut delivery = status(bytes);
    options.max_decoded_bytes = bytes.len() + b"Action: failed\r\n\r\n".len();
    assert_eq!(
        delivery.status_groups_with_options(&options).unwrap().len(),
        2
    );
    options.max_decoded_bytes -= 1;
    io_error(
        delivery.status_groups_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(delivery.part().content().get_ref().position(), 0);
}

#[test]
fn outer_decoding_limit_counts_decoded_bytes_not_encoded_wire_size() {
    let mut delivery = MessageDeliveryStatus::from_part(raw_part(
        Cursor::new(b"QWN0aW9uOiBmYWlsZWQNCg0K"),
        ContentEncoding::Base64,
    ))
    .unwrap();
    let mut options = DeliveryStatusParseOptions {
        max_decoded_bytes: b"Action: failed\r\n\r\n".len(),
        ..Default::default()
    };
    assert_eq!(
        delivery.status_groups_with_options(&options).unwrap(),
        vec![group(b"Action: failed\r\n")]
    );
    options.max_decoded_bytes -= 1;
    io_error(
        delivery.status_groups_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn global_delivery_status_inside_report_stays_a_raw_leaf() {
    let wire =
        b"Content-Type: multipart/report; boundary=r; report-type=global-delivery-status\r\n\r\n\
--r\r\nContent-Type: message/global-delivery-status\r\n\r\n\
Reporting-MTA: dns; example.com\r\n\r\n\
Final-Recipient: utf-8; Ren\xc3\xa9e@example.com\r\nStatus: 5.1.1\r\n\r\n\
\r\n--r--\r\n";
    let MimeEntity::Multipart(mut report) = MimeEntity::load(&mut &wire[..]).unwrap() else {
        panic!("expected multipart/report");
    };
    let MimeEntity::Part(part) = report.children_mut().remove(0) else {
        panic!("expected a delivery-status leaf");
    };
    let mut delivery = MessageDeliveryStatus::from_part(part).unwrap();
    let groups = delivery.status_groups().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[1].get("Final-Recipient").unwrap().raw(),
        b"Final-Recipient: utf-8; Ren\xc3\xa9e@example.com\r\n"
    );
    delivery.set_status_groups(&groups).unwrap();
    assert_eq!(delivery.status_groups().unwrap(), groups);
}

#[test]
fn header_limits_apply_to_folds_and_decoded_remainder() {
    let bytes = b"X: a\r\n b\r\n";
    let mut delivery = status(bytes);
    let mut options = DeliveryStatusParseOptions {
        max_header_length: bytes.len(),
        ..Default::default()
    };
    assert_eq!(
        delivery.status_groups_with_options(&options).unwrap().len(),
        1
    );
    options.max_header_length -= 1;
    io_error(
        delivery.status_groups_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(delivery.part().content().get_ref().position(), 0);

    let groups = vec![
        group(b"Content-Transfer-Encoding: base64\r\n"),
        group(b"Diagnostic-Code: smtp; this record is longer than the encoding field\r\n"),
    ];
    delivery.set_status_groups(&groups).unwrap();
    options.max_header_length = 40;
    io_error(
        delivery.status_groups_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_read: bool,
    fail_seek: Option<usize>,
    interrupt_once: bool,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(
                b"\r\nReporting-MTA: dns; example.com\r\n\r\nAction: failed\r\n\r\n".to_vec(),
            ),
            reads: 0,
            seeks: 0,
            fail_read: false,
            fail_seek: None,
            interrupt_once: false,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read {
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
fn fragmented_borrowed_sources_retry_interruptions_and_rewind() {
    let mut source = Source {
        interrupt_once: true,
        ..Source::new()
    };
    {
        let mut delivery =
            MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        assert_eq!(delivery.status_groups().unwrap().len(), 2);
        assert_eq!(delivery.status_groups().unwrap().len(), 2);
    }
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 4);
}

#[test]
fn source_failures_are_propagated_including_final_rewind() {
    for (fail_read, fail_seek) in [(true, None), (false, Some(1)), (false, Some(2))] {
        let mut source = Source {
            fail_read,
            fail_seek,
            ..Source::new()
        };
        let mut delivery =
            MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        io_error(delivery.status_groups().unwrap_err(), io::ErrorKind::Other);
    }
}

#[test]
fn masked_content_source_failures_produce_no_output_or_fallback() {
    for (fail_read, fail_seek) in [(true, None), (false, Some(1)), (false, Some(2))] {
        let mut source = Source {
            fail_read,
            fail_seek,
            ..Source::new()
        };
        let mut delivery =
            MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        let mut output = b"prefix".to_vec();
        io_error(
            anonymize::write_delivery_status_content(&mut delivery, &mut output).unwrap_err(),
            io::ErrorKind::Other,
        );
        assert_eq!(output, b"prefix");
    }
    let mut source = Source {
        interrupt_once: true,
        ..Source::new()
    };
    let mut delivery =
        MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
            .unwrap();
    anonymize::write_delivery_status_content(&mut delivery, &mut Vec::new()).unwrap();
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 2);
}

#[test]
fn complete_masking_source_failures_leave_only_outer_headers() {
    for (fail_read, fail_seek) in [(true, None), (false, Some(1)), (false, Some(2))] {
        let mut source = Source {
            fail_read,
            fail_seek,
            ..Source::new()
        };
        let mut delivery =
            MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        let mut output = b"prefix".to_vec();
        io_error(
            anonymize::write_delivery_status(&mut delivery, &mut output).unwrap_err(),
            io::ErrorKind::Other,
        );
        assert_eq!(
            output,
            b"prefixContent-Type: message/delivery-status\r\n\r\n"
        );
    }
    let mut source = Source {
        interrupt_once: true,
        ..Source::new()
    };
    let mut delivery =
        MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
            .unwrap();
    let mut output = Vec::new();
    anonymize::write_delivery_status(&mut delivery, &mut output).unwrap();
    assert_eq!(output, b"Content-Type: message/delivery-status\r\n\r\nReporting-MTA: xxxx xxxxxxxxxxx\r\n\r\nAction: xxxxxx\r\n");
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 2);
}

#[test]
fn invalid_options_and_raw_media_edits_fail_before_source_io() {
    let mut source = Source::new();
    {
        let mut delivery =
            MessageDeliveryStatus::from_part(raw_part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        for options in [
            DeliveryStatusParseOptions {
                max_decoded_bytes: 0,
                ..Default::default()
            },
            DeliveryStatusParseOptions {
                max_header_length: 0,
                ..Default::default()
            },
        ] {
            io_error(
                delivery.status_groups_with_options(&options).unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
            io_error(
                anonymize::write_delivery_status_content_with_options(
                    &mut delivery,
                    &mut Vec::new(),
                    &HeaderOptions::default(),
                    NewLineFormat::Dos,
                    &options,
                )
                .unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
            let mut output = Vec::new();
            io_error(
                anonymize::write_delivery_status_with_options(
                    &mut delivery,
                    &mut output,
                    &HeaderOptions::default(),
                    NewLineFormat::Dos,
                    &options,
                )
                .unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
            assert_eq!(output, b"Content-Type: message/delivery-status\r\n\r\n");
        }
        io_error(
            anonymize::write_delivery_status_content_with_options(
                &mut delivery,
                &mut Vec::new(),
                &HeaderOptions::default(),
                NewLineFormat::Mixed,
                &DeliveryStatusParseOptions::default(),
            )
            .unwrap_err(),
            io::ErrorKind::InvalidInput,
        );
        let mut output = Vec::new();
        io_error(
            anonymize::write_delivery_status_with_options(
                &mut delivery,
                &mut output,
                &HeaderOptions::default(),
                NewLineFormat::Mixed,
                &DeliveryStatusParseOptions::default(),
            )
            .unwrap_err(),
            io::ErrorKind::InvalidInput,
        );
        assert!(output.is_empty());
        delivery
            .part_mut()
            .headers_mut()
            .replace_all(Header::parse(b"Content-Type: text/plain").unwrap())
            .unwrap();
        assert!(matches!(
            delivery.status_groups(),
            Err(DeliveryStatusError::NotDeliveryStatus(_))
        ));
        assert!(matches!(
            anonymize::write_delivery_status_content(&mut delivery, &mut Vec::new()),
            Err(DeliveryStatusError::NotDeliveryStatus(_))
        ));
        assert!(matches!(
            anonymize::write_delivery_status(&mut delivery, &mut output),
            Err(DeliveryStatusError::NotDeliveryStatus(_))
        ));
        assert_eq!(output, b"Content-Type: text/plain\r\n\r\n");
    }
    assert_eq!(source.reads, 0);
    assert_eq!(source.seeks, 0);
}

// Adapted from MimeKit's MultipartReportTests.cs with Rust ownership and I/O cases.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, Header, HeaderFormatError, HeaderFormatOptions, HeaderList,
    MessageDeliveryStatus, MessageDispositionNotification, MessagePart, MimeContent, MimeEntity,
    MimeMessage, MimePart, MimePartWriteOptions, Multipart, MultipartError, MultipartReport,
    MultipartReportError, TextFormat, TextPart, encodings::NewLineFormat,
};

fn headers(bytes: &[u8]) -> HeaderList {
    HeaderList::load(&mut &bytes[..]).unwrap()
}

fn raw_part(bytes: &[u8]) -> Multipart {
    Multipart::from_parts(headers(bytes), Vec::new(), Vec::new(), Vec::new())
}

fn report() -> MultipartReport {
    MultipartReport::new("delivery-status", "report-boundary").unwrap()
}

#[test]
fn construction_trims_type_and_does_not_generate_children_or_other_headers() {
    let report: MultipartReport =
        MultipartReport::new("\u{2003}\tDelivery-Status\r\n", "explicit-boundary").unwrap();
    assert_eq!(
        report.report_type().unwrap().as_deref(),
        Some("Delivery-Status")
    );
    let part = report.multipart();
    assert_eq!(part.headers().len(), 1);
    let content_type = part.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("multipart", "report"));
    assert_eq!(content_type.boundary(), Some("explicit-boundary"));
    assert!(part.children().is_empty());
    assert!(part.preamble().is_empty());
    assert!(part.epilogue().is_empty());

    for value in [
        "",
        " \t\r\n",
        "x-unknown",
        "Unknown Report",
        "caf\u{e9}\u{1f600}",
    ] {
        let report: MultipartReport = MultipartReport::new(value, "b").unwrap();
        assert_eq!(report.report_type().unwrap().as_deref(), Some(value.trim()));
    }
    for boundary in ["", "bad\nboundary", "trailing "] {
        let error = MultipartReport::<()>::new("delivery-status", boundary).unwrap_err();
        assert!(error.source().is_some());
        assert!(matches!(
            error,
            MultipartReportError::Multipart(MultipartError::InvalidBoundary { .. })
        ));
    }
}

#[test]
fn wrapping_checks_first_media_type_without_requiring_report_type_or_boundary() {
    let raw = b"CONTENT-TYPE: MULTIPART/REPORT\nContent-Type: text/plain\nX-Keep: value\n";
    let original = headers(raw);
    let report = MultipartReport::from_multipart(raw_part(raw)).unwrap();
    assert_eq!(report.multipart().headers(), &original);
    assert_eq!(report.report_type().unwrap(), None);
    assert_eq!(report.multipart().boundary().unwrap(), None);

    let missing = MultipartReport::from_multipart(raw_part(b"X: only\n")).unwrap_err();
    assert!(missing.source().is_none());
    assert!(matches!(missing, MultipartReportError::MissingContentType));

    for media_type in [
        "text/report",
        "multipart/mixed",
        "message/delivery-status",
        "invalid",
    ] {
        let raw =
            format!("Content-Type: {media_type}\r\nContent-Type: multipart/report; boundary=b\r\n");
        let error = MultipartReport::from_multipart(raw_part(raw.as_bytes())).unwrap_err();
        if media_type == "invalid" {
            assert!(error.source().is_some());
            assert!(matches!(error, MultipartReportError::ContentType(_)));
        } else {
            assert!(error.source().is_none());
            assert!(matches!(error, MultipartReportError::NotReport(value) if value == media_type));
        }
    }
}

#[test]
fn getter_preserves_decoded_value_and_observes_raw_edits_without_caching() {
    let mut report = report();
    for (value, expected) in [
        ("multipart/report; boundary=b", None),
        ("multipart/report; boundary=b; report-type=\"\"", Some("")),
        (
            "multipart/report; boundary=b; REPORT-TYPE=\" Mixed Type \"",
            Some(" Mixed Type "),
        ),
        (
            "multipart/report; boundary=b; report-type*=utf-8''caf%C3%A9",
            Some("caf\u{e9}"),
        ),
    ] {
        report
            .multipart_mut()
            .headers_mut()
            .replace_all(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap())
            .unwrap();
        let before = report.multipart().headers().clone();
        let mut detached = report.report_type().unwrap();
        assert_eq!(detached.as_deref(), expected);
        if let Some(value) = &mut detached {
            value.clear();
        }
        assert_eq!(report.report_type().unwrap().as_deref(), expected);
        assert_eq!(report.multipart().headers(), &before);
    }
}

#[test]
fn getters_and_setters_reject_invalid_raw_media_type_even_for_no_op_values() {
    for value in [
        None,
        Some("invalid"),
        Some("multipart/mixed"),
        Some("text/report"),
    ] {
        let mut report = report();
        report
            .multipart_mut()
            .headers_mut()
            .remove_all("Content-Type");
        if let Some(value) = value {
            report.multipart_mut().headers_mut().add(
                Header::parse(
                    format!("Content-Type: {value}; report-type=delivery-status").as_bytes(),
                )
                .unwrap(),
            );
        }
        let before = report.multipart().headers().clone();
        for error in [
            report.report_type().unwrap_err(),
            report.set_report_type("delivery-status").unwrap_err(),
        ] {
            match value {
                None => assert!(matches!(error, MultipartReportError::MissingContentType)),
                Some("invalid") => assert!(matches!(error, MultipartReportError::ContentType(_))),
                _ => assert!(matches!(error, MultipartReportError::NotReport(_))),
            }
        }
        assert_eq!(report.multipart().headers(), &before);
    }
}

#[test]
fn same_value_is_a_raw_no_op_before_boundary_or_format_validation() {
    let raw = b"content-type: multipart/report;\n\tREPORT-TYPE=\"delivery-status\"\nContent-Type: text/plain\nBroken record\n";
    let mut report = MultipartReport::from_multipart(raw_part(raw)).unwrap();
    let before = report.multipart().headers().clone();
    let invalid_options = HeaderFormatOptions {
        max_line_length: 0,
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    for value in ["delivery-status", "  delivery-status \t"] {
        report
            .set_report_type_with_options(value, &invalid_options)
            .unwrap();
        assert_eq!(report.multipart().headers(), &before);
    }
    assert!(matches!(
        report.set_report_type("different"),
        Err(MultipartReportError::Multipart(
            MultipartError::MissingBoundary
        ))
    ));
    assert_eq!(report.multipart().headers(), &before);
}

#[test]
fn exact_untrimmed_match_is_unchanged_but_changed_assignments_are_trimmed() {
    let raw = b"Content-Type: multipart/report; boundary=b; report-type=\" old \"\n";
    let mut report = MultipartReport::from_multipart(raw_part(raw)).unwrap();
    let before = report.multipart().headers().clone();
    report.set_report_type(" old ").unwrap();
    assert_eq!(report.report_type().unwrap().as_deref(), Some(" old "));
    assert_eq!(report.multipart().headers(), &before);
    report.set_report_type("old").unwrap();
    assert_eq!(report.report_type().unwrap().as_deref(), Some("old"));
    report.set_report_type(" \u{2003}New Type\t ").unwrap();
    assert_eq!(report.report_type().unwrap().as_deref(), Some("New Type"));
    report.set_report_type(" \t ").unwrap();
    assert_eq!(report.report_type().unwrap().as_deref(), Some(""));
}

#[test]
fn changed_type_preserves_other_metadata_children_and_source_positions() {
    let raw = headers(b"X-First: \xff\ncontent-type: multipart/report; boundary=b; x=keep; report-type=old\nMalformed\nCONTENT-TYPE: text/plain\nX-Last: last\n");
    let mut source = Cursor::new(b"payload".to_vec());
    source.set_position(2);
    let leaf = MimePart::new(MimeContent::with_encoding(source, ContentEncoding::Binary));
    let multipart = Multipart::from_parts(
        raw.clone(),
        vec![leaf.into()],
        b"\xffpreamble".to_vec(),
        b"\0epilogue".to_vec(),
    );
    let children = multipart.children().as_ptr();
    let preamble = multipart.preamble().as_ptr();
    let epilogue = multipart.epilogue().as_ptr();
    let mut report = MultipartReport::from_multipart(multipart).unwrap();
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        always_quote_parameters: true,
        ..Default::default()
    };
    report
        .set_report_type_with_options(" disposition-notification ", &options)
        .unwrap();
    let part = report.multipart();
    let content_type = part.content_type().unwrap().unwrap();
    assert_eq!(
        content_type.parameters().get("report-type"),
        Some("disposition-notification")
    );
    assert_eq!(content_type.parameters().get("x"), Some("keep"));
    assert_eq!(content_type.boundary(), Some("b"));
    assert_eq!(part.headers().len(), 4);
    assert_eq!(part.headers().index_of("Content-Type"), Some(1));
    assert_eq!(part.headers().last_index_of("Content-Type"), Some(1));
    assert_eq!(part.headers().as_slice()[0], raw.as_slice()[0]);
    assert_eq!(part.headers().as_slice()[2], raw.as_slice()[2]);
    assert_eq!(part.headers().as_slice()[3], raw.as_slice()[4]);
    assert_eq!(
        part.headers().as_slice()[1],
        content_type.to_header_with_options(&options).unwrap()
    );
    assert_eq!(part.children().as_ptr(), children);
    assert_eq!(part.preamble().as_ptr(), preamble);
    assert_eq!(part.epilogue().as_ptr(), epilogue);
    assert_eq!(part.preamble(), b"\xffpreamble");
    assert_eq!(part.epilogue(), b"\0epilogue");
    let leaf = part.leaf_parts().next().unwrap();
    assert_eq!(leaf.content().get_ref().position(), 2);
    assert_eq!(leaf.content().encoding(), ContentEncoding::Binary);
    assert_eq!(leaf.content().get_ref().get_ref(), b"payload");

    let MimeEntity::Multipart(extracted) = MimeEntity::from(report) else {
        panic!("report should remain an ordinary multipart");
    };
    assert_eq!(extracted.children().as_ptr(), children);
    assert_eq!(extracted.preamble().as_ptr(), preamble);
    assert_eq!(extracted.epilogue().as_ptr(), epilogue);
}

#[test]
fn changed_value_requires_valid_boundary_and_failed_edits_are_atomic() {
    for (value, missing) in [
        ("multipart/report; report-type=old", true),
        (
            "multipart/report; boundary=\"bad \"; report-type=old",
            false,
        ),
    ] {
        let raw = format!("Content-Type: {value}\n");
        let mut report = MultipartReport::from_multipart(raw_part(raw.as_bytes())).unwrap();
        let before = report.multipart().headers().clone();
        report.set_report_type("old").unwrap();
        let error = report.set_report_type("new").unwrap_err();
        assert!(error.source().is_some());
        if missing {
            assert!(matches!(
                error,
                MultipartReportError::Multipart(MultipartError::MissingBoundary)
            ));
        } else {
            assert!(matches!(
                error,
                MultipartReportError::Multipart(MultipartError::InvalidBoundary { .. })
            ));
        }
        assert_eq!(report.multipart().headers(), &before);
        report.multipart_mut().set_boundary("repaired").unwrap();
        report.set_report_type("new").unwrap();
        assert_eq!(report.report_type().unwrap().as_deref(), Some("new"));
    }

    let mut report = report();
    let before = report.multipart().headers().clone();
    for options in [
        HeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        },
        HeaderFormatOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
    ] {
        let error = report
            .set_report_type_with_options("new", &options)
            .unwrap_err();
        assert!(error.source().unwrap().source().is_some());
        assert!(matches!(
            error,
            MultipartReportError::Multipart(MultipartError::HeaderFormat(_))
        ));
        assert_eq!(report.multipart().headers(), &before);
    }
    let mut content_type = report.multipart().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("x".repeat(100).as_str(), "value")
        .unwrap();
    report
        .multipart_mut()
        .set_content_type_with_options(
            &content_type,
            &HeaderFormatOptions {
                max_line_length: 998,
                ..Default::default()
            },
        )
        .unwrap();
    let before = report.multipart().headers().clone();
    assert!(matches!(
        report.set_report_type("new"),
        Err(MultipartReportError::Multipart(
            MultipartError::HeaderFormat(HeaderFormatError::LineTooLong { .. })
        ))
    ));
    assert_eq!(report.multipart().headers(), &before);
}

#[test]
fn missing_report_type_is_not_an_empty_value_and_long_values_round_trip() {
    let mut report = MultipartReport::from_multipart(raw_part(
        b"Content-Type: multipart/report; boundary=b\r\n",
    ))
    .unwrap();
    assert_eq!(report.report_type().unwrap(), None);
    report.set_report_type("").unwrap();
    assert_eq!(report.report_type().unwrap().as_deref(), Some(""));
    let value = "delivery-\u{e9}\u{1f600}".repeat(30);
    let options = HeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    report
        .set_report_type_with_options(&value, &options)
        .unwrap();
    assert_eq!(
        report.report_type().unwrap().as_deref(),
        Some(value.as_str())
    );
    let header = report.multipart().headers().get("Content-Type").unwrap();
    assert!(
        header
            .raw()
            .split(|&byte| byte == b'\n')
            .all(|line| { line.strip_suffix(b"\r").unwrap_or(line).len() <= 60 })
    );
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("report metadata must not read child sources");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("report metadata must not seek child sources");
    }
}

#[test]
fn metadata_operations_do_not_access_borrowed_child_sources() {
    let mut source = NoIo;
    let leaf = MimePart::new(MimeContent::new(&mut source));
    let mut part = Multipart::new("report", "b").unwrap();
    part.children_mut().push(leaf.into());
    let mut report = MultipartReport::from_multipart(part).unwrap();
    assert_eq!(report.report_type().unwrap(), None);
    report.set_report_type("delivery-status").unwrap();
    report.set_report_type("delivery-status").unwrap();
    let part = report.into_multipart();
    assert_eq!(part.children().len(), 1);
    drop(part);

    // Metadata operations do not even require a readable child-source type.
    let mut empty = MultipartReport::<()>::new("x-report", "b").unwrap();
    empty.set_report_type("different").unwrap();
    assert_eq!(empty.report_type().unwrap().as_deref(), Some("different"));
}

#[test]
fn report_children_are_not_validated_or_automatically_changed() {
    let mut report = report();
    report
        .multipart_mut()
        .children_mut()
        .push(TextPart::new("plain", "body").unwrap().into());
    report
        .multipart_mut()
        .children_mut()
        .push(MimePart::new(MimeContent::new(Cursor::new(b"not a report".to_vec()))).into());
    report.set_report_type("unknown-report").unwrap();
    assert_eq!(report.multipart().children().len(), 2);
    let MimeEntity::Part(second) = &report.multipart().children()[1] else {
        panic!("second child must remain untouched");
    };
    assert_eq!(second.content().get_ref().get_ref(), b"not a report");
    assert!(
        second
            .content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("application", "octet-stream")
    );
}

#[test]
fn delivery_and_disposition_reports_round_trip_through_existing_tree_io() {
    for report_type in ["delivery-status", "disposition-notification"] {
        let expected_fields = headers(b"Final-Recipient: rfc822; user@example.com\r\n");
        let notification: MimeEntity = if report_type == "delivery-status" {
            let mut status = MessageDeliveryStatus::new();
            status
                .set_status_groups(std::slice::from_ref(&expected_fields))
                .unwrap();
            status.into()
        } else {
            let mut notification = MessageDispositionNotification::new();
            notification.set_fields(&expected_fields).unwrap();
            notification.into()
        };
        let mut report: MultipartReport =
            MultipartReport::new(report_type, "report-boundary").unwrap();
        report.multipart_mut().children_mut().extend([
            TextPart::new("plain", "Human-readable report")
                .unwrap()
                .into(),
            notification,
            MessagePart::new(MimeMessage::with_body(
                TextPart::new("plain", "Original message").unwrap().into(),
            ))
            .into(),
        ]);
        report
            .multipart_mut()
            .set_preamble(b"Report preamble".to_vec());
        report
            .multipart_mut()
            .set_epilogue(b"Report epilogue".to_vec());
        let mut output = Vec::new();
        report
            .multipart_mut()
            .write_to_with_options(
                &mut output,
                &MimePartWriteOptions {
                    ensure_new_line: false,
                    ..Default::default()
                },
            )
            .unwrap();
        let MimeEntity::Multipart(part) = MimeEntity::load(&mut output.as_slice()).unwrap() else {
            panic!("loaded report should be a multipart");
        };
        let mut report = MultipartReport::from_multipart(part).unwrap();
        assert_eq!(report.report_type().unwrap().as_deref(), Some(report_type));
        assert_eq!(report.multipart().children().len(), 3);
        assert_eq!(report.multipart().epilogue(), b"Report epilogue");
        let notification = report.multipart_mut().children_mut().remove(1);
        let MimeEntity::Part(part) = notification else {
            panic!("notification should remain a leaf");
        };
        if report_type == "delivery-status" {
            let mut status = MessageDeliveryStatus::from_part(part).unwrap();
            assert_eq!(status.status_groups().unwrap(), vec![expected_fields]);
        } else {
            let mut notification = MessageDispositionNotification::from_part(part).unwrap();
            assert_eq!(notification.fields().unwrap(), expected_fields);
        }
        let entity: MimeEntity = report.into();
        let message = MimeMessage::with_body(entity);
        assert_eq!(message.body_parts().count(), 2);
        assert_eq!(message.leaf_parts().count(), 2);
        assert!(message.text_body_part(TextFormat::Plain).unwrap().is_some());
    }
}

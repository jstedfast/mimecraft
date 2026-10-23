// Adapted from MimeKit's MessageFeedbackReportTests.cs and feedback-report.txt.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, FeedbackReportError, FeedbackReportParseOptions, Header, HeaderList,
    MessageFeedbackReport, MimeContent, MimeEntity, MimeMessage, MimePart, MultipartReport,
    encodings::NewLineFormat,
};

const CANONICAL_FIELDS: &[u8] = b"Feedback-Type: abuse\r\n\
User-Agent: SomeGenerator/1.0\r\n\
Version: 1\r\n\
Original-Mail-From: <somespammer@example.net>\r\n\
Original-Rcpt-To: <user@example.com>\r\n\
Received-Date: Thu, 8 Mar 2005 14:00:00 EDT\r\n\
Source-IP: 192.0.2.2\r\n\
Authentication-Results: mail.example.com;\r\n               spf=fail smtp.mail=somespammer@example.com\r\n\
Reported-Domain: example.net\r\n\
Reported-Uri: http://example.net/earn_money.html\r\n\
Reported-Uri: mailto:user@example.com\r\n\
Removal-Recipient: user@example.com\r\n";

fn fields(bytes: &[u8]) -> HeaderList {
    HeaderList::load(&mut &bytes[..]).unwrap()
}

fn part<R: Read + Seek>(source: R, encoding: ContentEncoding) -> MimePart<R> {
    MimePart::from_parts(
        fields(b"Content-Type: message/feedback-report\r\n"),
        MimeContent::with_encoding(source, encoding),
    )
}

fn feedback(bytes: &[u8]) -> MessageFeedbackReport {
    MessageFeedbackReport::from_part(part(
        Cursor::new(bytes.to_vec()),
        ContentEncoding::Unspecified,
    ))
    .unwrap()
}

fn io_error(error: FeedbackReportError, kind: io::ErrorKind) {
    assert!(error.source().is_some());
    assert!(matches!(error, FeedbackReportError::Io(inner) if inner.kind() == kind));
}

#[test]
fn constructor_creates_only_content_type_and_empty_unencoded_body() {
    let mut report = MessageFeedbackReport::default();
    assert_eq!(
        report.part().headers(),
        &fields(b"Content-Type: message/feedback-report\r\n")
    );
    assert!(report.part().content().get_ref().get_ref().is_empty());
    assert!(report.fields().unwrap().is_empty());
    assert_eq!(
        report.part().content().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(report.part().content().new_line_format(), None);
    let mut report = MessageFeedbackReport::new();
    let expected = b"Feedback-Type: abuse\nUser-Agent: SomeGenerator/1.0\nVersion: 1\n";
    report
        .set_fields_with_new_line_format(&fields(expected), NewLineFormat::Unix)
        .unwrap();
    assert_eq!(
        report.part().content().get_ref().get_ref(),
        &[expected.as_slice(), b"\n"].concat()
    );
}

#[test]
fn wrapping_checks_first_type_and_does_not_accept_invented_global_subtype() {
    let mut report = MessageFeedbackReport::new();
    report
        .part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: MESSAGE/FEEDBACK-REPORT; x=keep").unwrap())
        .unwrap();
    report
        .part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    let headers = report.part().headers().clone();
    let mut report = MessageFeedbackReport::from_part(report.into_part()).unwrap();
    assert!(report.fields().unwrap().is_empty());
    assert_eq!(report.part().headers(), &headers);

    for value in [
        None,
        Some("invalid"),
        Some("message/global-feedback-report"),
        Some("text/feedback-report"),
    ] {
        let mut part = part(Cursor::new(Vec::new()), ContentEncoding::Unspecified);
        part.headers_mut().remove_all("Content-Type");
        if let Some(value) = value {
            part.headers_mut()
                .add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
            part.headers_mut()
                .add(Header::parse(b"Content-Type: message/feedback-report").unwrap());
        }
        let error = MessageFeedbackReport::from_part(part).unwrap_err();
        match value {
            None => {
                assert!(error.source().is_none());
                assert!(matches!(error, FeedbackReportError::MissingContentType));
            }
            Some("invalid") => {
                assert!(error.source().is_some());
                assert!(matches!(error, FeedbackReportError::ContentType(_)));
            }
            Some(value) => {
                assert!(error.source().is_none());
                assert!(
                    matches!(error, FeedbackReportError::NotFeedbackReport(actual) if actual == value)
                );
            }
        }
    }
}

#[test]
fn canonical_report_loads_as_leaf_in_multipart_and_preserves_repeated_uris() {
    let mut wire = b"From: abusedesk@example.com\r\nSubject: FW: Earn money\r\n\
Content-Type: multipart/report; boundary=r; report-type=feedback-report\r\n\r\n\
--r\r\nContent-Type: text/plain\r\n\r\nThis is an email abuse report.\r\n\
--r\r\nContent-Type: message/feedback-report\r\n\r\n"
        .to_vec();
    wire.extend_from_slice(CANONICAL_FIELDS);
    wire.extend_from_slice(
        b"\r\n--r\r\nContent-Type: message/rfc822\r\n\r\n\
From: somespammer@example.net\r\nSubject: Earn money\r\n\r\nSpam\r\n--r--\r\n",
    );
    let message = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert_eq!(message.body_parts().count(), 3);
    let (_, Some(MimeEntity::Multipart(part))) = message.into_parts() else {
        panic!("feedback report must load as a multipart");
    };
    let mut report = MultipartReport::from_multipart(part).unwrap();
    assert_eq!(
        report.report_type().unwrap().as_deref(),
        Some("feedback-report")
    );
    assert!(matches!(
        report.multipart().children()[2],
        MimeEntity::Message(_)
    ));
    let MimeEntity::Part(part) = report.multipart_mut().children_mut().remove(1) else {
        panic!("feedback fields must remain an opaque leaf until requested");
    };
    let mut feedback = MessageFeedbackReport::from_part(part).unwrap();
    let parsed = feedback.fields().unwrap();
    assert_eq!(parsed, fields(CANONICAL_FIELDS));
    assert_eq!(parsed.len(), 12);
    for (name, value) in [
        ("Feedback-Type", "abuse"),
        ("User-Agent", "SomeGenerator/1.0"),
        ("Version", "1"),
        ("Original-Mail-From", "<somespammer@example.net>"),
        ("Original-Rcpt-To", "<user@example.com>"),
        ("Received-Date", "Thu, 8 Mar 2005 14:00:00 EDT"),
        ("Source-IP", "192.0.2.2"),
        (
            "Authentication-Results",
            "mail.example.com;               spf=fail smtp.mail=somespammer@example.com",
        ),
        ("Reported-Domain", "example.net"),
        ("Removal-Recipient", "user@example.com"),
    ] {
        assert_eq!(parsed.get(name).unwrap().decode_value().text, value);
    }
    assert_eq!(
        parsed
            .get_all("Reported-Uri")
            .map(|header| header.decode_value().text)
            .collect::<Vec<_>>(),
        [
            "http://example.net/earn_money.html",
            "mailto:user@example.com"
        ]
    );

    feedback.set_fields(&parsed).unwrap();
    feedback
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::Base64);
    report
        .multipart_mut()
        .children_mut()
        .insert(1, feedback.into());
    let mut message = MimeMessage::with_body(report.into());
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    let mut loaded = MimeMessage::load(&mut output.as_slice()).unwrap();
    let Some(MimeEntity::Multipart(report)) = loaded.body_mut() else {
        panic!()
    };
    let MimeEntity::Part(part) = report.children_mut().remove(1) else {
        panic!()
    };
    assert_eq!(part.content().encoding(), ContentEncoding::Base64);
    assert_eq!(
        MessageFeedbackReport::from_part(part)
            .unwrap()
            .fields()
            .unwrap(),
        parsed
    );
}

#[test]
fn fields_are_raw_detached_and_not_semantically_validated() {
    let raw = b"bad record\r\nFeedback-Type: unknown\r\nVersion: not-a-number\r\nX: \xff\r\n\
Reported-Uri: one\r\nReported-Uri: two\r\nX-Fold: first\r\n second\r\n";
    let mut report = feedback(raw);
    let mut parsed = report.fields().unwrap();
    assert_eq!(parsed, fields(raw));
    assert!(parsed.as_slice()[0].is_invalid());
    assert_eq!(parsed.get_all("Reported-Uri").count(), 2);
    parsed.add(Header::parse(b"X-Edit: detached").unwrap());
    assert_eq!(report.fields().unwrap(), fields(raw));
    assert_eq!(report.part().content().get_ref().get_ref(), raw);
    report.set_fields(&parsed).unwrap();
    assert_eq!(report.fields().unwrap(), parsed);
    report
        .part_mut()
        .set_content(MimeContent::new(Cursor::new(b"X: replaced".to_vec())));
    assert_eq!(report.fields().unwrap(), fields(b"X: replaced"));
}

#[test]
fn only_first_block_is_parsed_and_field_encodings_are_ordinary_data() {
    for raw in [&b""[..], b"\r", b"\n", b"\r\nX: ignored", b"\n\nX: ignored"] {
        assert!(feedback(raw).fields().unwrap().is_empty());
    }
    for raw in [
        &b"X: unterminated"[..],
        b"Content-Transfer-Encoding: base64\r\n\r\nWDogYQ==",
        b"Content-Transfer-Encoding: unknown\r\n\r\nX: ignored",
    ] {
        let mut report = feedback(raw);
        let parsed = report.fields().unwrap();
        assert_eq!(parsed, fields(raw));
        assert_eq!(parsed.len(), 1);
        assert_eq!(report.part().content().get_ref().get_ref(), raw);
        report.set_fields(&parsed).unwrap();
        assert_eq!(report.fields().unwrap().len(), 1);
        assert!(
            report
                .part()
                .content()
                .get_ref()
                .get_ref()
                .ends_with(b"\r\n\r\n")
        );
    }
}

#[test]
fn stored_transfer_encoding_not_edited_header_controls_reads() {
    for (encoded, encoding) in [
        (&b"WDogYQ=="[..], ContentEncoding::Base64),
        (&b"X: =61"[..], ContentEncoding::QuotedPrintable),
    ] {
        let mut report =
            MessageFeedbackReport::from_part(part(Cursor::new(encoded), encoding)).unwrap();
        report
            .part_mut()
            .set_content_transfer_encoding(ContentEncoding::SevenBit);
        report.part_mut().content_mut().get_mut().set_position(2);
        let headers = report.part().headers().clone();
        assert_eq!(report.fields().unwrap(), fields(b"X: a"));
        assert_eq!(report.part().content().get_ref().position(), 0);
        assert_eq!(report.part().content().get_ref().get_ref(), &encoded);
        assert_eq!(report.part().headers(), &headers);
    }
}

#[test]
fn edits_replace_entire_body_atomically_without_rewriting_outer_headers() {
    let mut report = feedback(b"X: old\r\n\r\ntrailing");
    report
        .part_mut()
        .set_content_transfer_encoding(ContentEncoding::Base64);
    report
        .part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Length: 999").unwrap());
    report
        .part_mut()
        .content_mut()
        .set_new_line_format(Some(NewLineFormat::Mixed));
    report.part_mut().content_mut().get_mut().set_position(3);
    let headers = report.part().headers().clone();
    let before = report.part().content().get_ref().clone();
    let parsed = fields(b"X: new");
    io_error(
        report
            .set_fields_with_new_line_format(&parsed, NewLineFormat::Mixed)
            .unwrap_err(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(report.part().content().get_ref(), &before);
    assert_eq!(
        report.part().content().new_line_format(),
        Some(NewLineFormat::Mixed)
    );
    assert_eq!(report.part().headers(), &headers);
    report
        .set_fields_with_new_line_format(&parsed, NewLineFormat::Unix)
        .unwrap();
    assert_eq!(report.part().content().get_ref().get_ref(), b"X: new\n\n");
    assert_eq!(report.part().content().get_ref().position(), 0);
    assert_eq!(
        report.part().content().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(report.part().content().new_line_format(), None);
    assert_eq!(report.part().headers(), &headers);
    report.set_fields(&HeaderList::new()).unwrap();
    assert_eq!(report.part().content().get_ref().get_ref(), b"\r\n");
}

#[test]
fn byte_limits_include_trailing_data_and_measure_decoded_not_encoded_bytes() {
    let bytes = b"X: a\r\n\r\ntail";
    let mut report = feedback(bytes);
    let mut options = FeedbackReportParseOptions {
        max_decoded_bytes: bytes.len(),
        ..Default::default()
    };
    assert_eq!(
        report.fields_with_options(&options).unwrap(),
        fields(b"X: a\r\n")
    );
    options.max_decoded_bytes -= 1;
    io_error(
        report.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    let mut report =
        MessageFeedbackReport::from_part(part(Cursor::new(b"WDogYQ=="), ContentEncoding::Base64))
            .unwrap();
    options.max_decoded_bytes = 4;
    assert_eq!(
        report.fields_with_options(&options).unwrap(),
        fields(b"X: a")
    );
    options.max_decoded_bytes = 3;
    io_error(
        report.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(
        FeedbackReportParseOptions::default().max_decoded_bytes,
        64 * 1024 * 1024
    );
}

#[test]
fn folded_header_limits_and_default_16_kib_boundary_are_enforced() {
    let bytes = b"X: a\r\n b\r\n";
    let mut report = feedback(bytes);
    let mut options = FeedbackReportParseOptions {
        max_header_length: bytes.len(),
        ..Default::default()
    };
    assert_eq!(report.fields_with_options(&options).unwrap().len(), 1);
    options.max_header_length -= 1;
    io_error(
        report.fields_with_options(&options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(report.part().content().get_ref().position(), 0);
    assert_eq!(
        FeedbackReportParseOptions::default().max_header_length,
        16 * 1024
    );
    let mut bytes = b"X: ".to_vec();
    bytes.resize(16 * 1024, b'a');
    assert_eq!(feedback(&bytes).fields().unwrap().len(), 1);
    bytes.push(b'a');
    io_error(
        feedback(&bytes).fields().unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_after: Option<u64>,
    fail_seek: Option<usize>,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(b"X: a\r\n\r\ntail".to_vec()),
            reads: 0,
            seeks: 0,
            fail_after: None,
            fail_seek: None,
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
fn borrowed_fragmented_sources_rewind_and_all_io_failures_propagate() {
    let mut source = Source::new();
    {
        let mut report =
            MessageFeedbackReport::from_part(part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        assert_eq!(report.fields().unwrap(), fields(b"X: a\r\n"));
        let MimeEntity::Part(_) = MimeEntity::from(report) else {
            panic!()
        };
    }
    assert_eq!(source.bytes.position(), 0);
    assert_eq!(source.seeks, 2);
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
        let mut report =
            MessageFeedbackReport::from_part(part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        io_error(report.fields().unwrap_err(), io::ErrorKind::Other);
    }
}

#[test]
fn invalid_limits_and_raw_media_edits_fail_without_io_or_partial_changes() {
    let mut source = Source::new();
    {
        let mut report =
            MessageFeedbackReport::from_part(part(&mut source, ContentEncoding::Unspecified))
                .unwrap();
        for options in [
            FeedbackReportParseOptions {
                max_decoded_bytes: 0,
                ..Default::default()
            },
            FeedbackReportParseOptions {
                max_header_length: 0,
                ..Default::default()
            },
        ] {
            io_error(
                report.fields_with_options(&options).unwrap_err(),
                io::ErrorKind::InvalidInput,
            );
        }
        report
            .part_mut()
            .headers_mut()
            .replace_all(Header::parse(b"Content-Type: text/plain").unwrap())
            .unwrap();
        assert!(matches!(
            report.fields(),
            Err(FeedbackReportError::NotFeedbackReport(_))
        ));
    }
    assert_eq!((source.reads, source.seeks), (0, 0));
    for value in [None, Some("invalid"), Some("message/delivery-status")] {
        let mut report = feedback(b"X: original");
        report.part_mut().content_mut().get_mut().set_position(2);
        let before = report.part().content().get_ref().clone();
        report.part_mut().headers_mut().remove_all("Content-Type");
        if let Some(value) = value {
            report
                .part_mut()
                .headers_mut()
                .add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
        }
        let headers = report.part().headers().clone();
        for error in [
            report.fields().unwrap_err(),
            report.set_fields(&HeaderList::new()).unwrap_err(),
        ] {
            match value {
                None => assert!(matches!(error, FeedbackReportError::MissingContentType)),
                Some("invalid") => assert!(matches!(error, FeedbackReportError::ContentType(_))),
                _ => assert!(matches!(error, FeedbackReportError::NotFeedbackReport(_))),
            }
        }
        assert_eq!(report.part().content().get_ref(), &before);
        assert_eq!(report.part().headers(), &headers);
    }
}

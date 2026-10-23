// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentDisposition, ContentEncoding, ContentType, Header, HeaderFormatOptions, HeaderId,
    HeaderList, HeaderParser, MimeContent, MimePart, ParameterComplianceMode,
    ParameterParseOptions, charsets::Charset, encodings::NewLineFormat,
};

fn headers(input: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    assert_eq!(parser.parse(input, &mut records).unwrap(), input.len());
    parser.finish(&mut records).unwrap();
    records.into()
}

fn content() -> MimeContent<Cursor<Vec<u8>>> {
    let mut stream = Cursor::new(b"body bytes".to_vec());
    stream.set_position(4);
    MimeContent::new(stream)
}

#[test]
fn default_part_owns_content_without_inferring_its_transfer_encoding() {
    let mut stored =
        MimeContent::with_encoding(Cursor::new(b"aGVsbG8=\r\n"), ContentEncoding::Base64);
    stored.get_mut().set_position(3);
    stored.set_new_line_format(Some(NewLineFormat::Dos));
    let part = MimePart::new(stored);
    assert_eq!(part.headers().len(), 1);
    let header = &part.headers().as_slice()[0];
    assert_eq!(header.raw(), b"Content-Type: application/octet-stream\r\n");
    assert_eq!(header.id(), HeaderId::ContentType);
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("application", "octet-stream")
    );
    assert_eq!(part.content_disposition().unwrap(), None);
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Unspecified
    );
    assert_eq!(part.content().encoding(), ContentEncoding::Base64);
    assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Dos));
    assert_eq!(part.content().get_ref().position(), 3);
}

#[test]
fn explicit_content_type_is_formatted_not_shared_or_applied_to_body_bytes() {
    let mut value = ContentType::new("Text", "Plain").unwrap();
    value.set_charset(Some("utf-8"));
    let mut part = MimePart::with_content_type(&value, content()).unwrap();
    assert_eq!(
        part.headers().as_slice()[0].raw(),
        value.to_header_string().unwrap().as_bytes()
    );
    value.set_charset(Some("iso-8859-1"));
    assert_eq!(
        part.content_type().unwrap().unwrap().charset(),
        Some("utf-8")
    );
    assert_eq!(part.content().get_ref().position(), 4);
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, b"body bytes");
    assert_eq!(part.content().get_ref().position(), 0);
}

#[test]
fn from_parts_preserves_missing_duplicate_and_malformed_headers() {
    let raw = headers(b"X-Raw: \xff\nMalformed record\r\nContent-Type \t: Text/Plain\nCONTENT-TYPE: image/png\nSubject: no terminal newline");
    let part = MimePart::from_parts(raw.clone(), content());
    assert_eq!(part.headers(), &raw);
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("text", "plain")
    );
    assert_eq!(part.content().get_ref().position(), 4);
    let missing = MimePart::from_parts(HeaderList::new(), content());
    assert!(missing.headers().is_empty());
    assert_eq!(missing.content_type().unwrap(), None);
    assert_eq!(missing.content_disposition().unwrap(), None);
    assert_eq!(
        missing.content_transfer_encoding().unwrap(),
        ContentEncoding::Unspecified
    );
}

#[test]
fn raw_edits_are_visible_and_first_invalid_values_never_fall_back_to_duplicates() {
    let mut part = MimePart::new(content());
    let snapshot = part.content_type().unwrap().unwrap();
    part.headers_mut()
        .replace_all(Header::parse(b"content-type: image/png\n").unwrap())
        .unwrap();
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("image", "png")
    );
    assert!(snapshot.is_mime_type("application", "octet-stream"));
    part.headers_mut().remove_all("Content-Type");
    assert_eq!(part.content_type().unwrap(), None);
    part.headers_mut()
        .add(Header::parse(b"Content-Type: broken").unwrap());
    part.headers_mut()
        .add(Header::parse(b"Content-Type: text/plain").unwrap());
    assert!(part.content_type().is_err());
    part.headers_mut()
        .add(Header::parse(b"Content-Disposition: ;").unwrap());
    part.headers_mut()
        .add(Header::parse(b"Content-Disposition: attachment").unwrap());
    assert!(part.content_disposition().is_err());
    part.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap());
    part.headers_mut()
        .add(Header::parse(b"Content-Transfer-Encoding: base64").unwrap());
    assert!(part.content_transfer_encoding().is_err());
    assert_eq!(part.content().get_ref().position(), 4);
}

#[test]
fn content_metadata_options_and_diagnostics_are_preserved() {
    let part = MimePart::from_parts(headers(
        b"Content-Type: text/plain; name=two words\nContent-Disposition: attachment; filename=two words\n"
    ), content());
    assert!(part.content_type().is_ok());
    assert!(part.content_disposition().is_ok());
    let options = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    assert!(part.content_type_with_options(&options).is_err());
    assert!(part.content_disposition_with_options(&options).is_err());

    let part = MimePart::from_parts(headers(
        b"Content-Type: text/plain; name=\"\xe0\"\nContent-Disposition: attachment; filename=\"\xe0\"\n"
    ), content());
    let options = ParameterParseOptions {
        fallback_charset: Some(Charset::from_label("windows-1251").unwrap()),
        ..Default::default()
    };
    assert_eq!(
        part.content_type_with_options(&options)
            .unwrap()
            .unwrap()
            .name(),
        Some("\u{430}")
    );
    assert_eq!(
        part.content_disposition_with_options(&options)
            .unwrap()
            .unwrap()
            .file_name(),
        Some("\u{430}")
    );
    let parsed = part.content_disposition().unwrap().unwrap();
    assert!(!parsed.parameters().issues().is_empty());
    assert_eq!(
        parsed.parameters().issues(),
        part.headers()
            .get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap()
            .parameters()
            .issues()
    );
}

#[test]
fn content_type_setter_retains_first_position_and_unrelated_raw_records() {
    let raw = headers(b"X-First: raw\ncontent-type: text/html\nMalformed\r\nX-Raw: \xff\nCONTENT-TYPE: image/png\nX-End: raw\n");
    let mut part = MimePart::from_parts(raw.clone(), content());
    let value = ContentType::new("text", "plain").unwrap();
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    part.set_content_type_with_options(&value, &options)
        .unwrap();
    assert_eq!(
        part.headers().as_slice(),
        &[
            raw.as_slice()[0].clone(),
            value.to_header_with_options(&options).unwrap(),
            raw.as_slice()[2].clone(),
            raw.as_slice()[3].clone(),
            raw.as_slice()[5].clone(),
        ]
    );
    assert_eq!(part.headers().index_of("CONTENT-TYPE"), Some(1));
    assert_eq!(part.headers().last_index_of("content-type"), Some(1));
    assert_eq!(part.headers().index_of("X-End"), Some(4));
    assert_eq!(part.content().get_ref().position(), 4);
    part.headers_mut().remove_all("content-type");
    part.set_content_type(&value).unwrap();
    assert_eq!(part.headers().index_of("content-type"), Some(4));
}

#[test]
fn disposition_edits_add_replace_and_remove_all_occurrences() {
    let mut part = MimePart::new(content());
    let mut value = ContentDisposition::default();
    value.set_file_name(Some("caf\u{e9}.txt"));
    part.set_content_disposition(Some(&value)).unwrap();
    assert_eq!(part.headers().index_of("Content-Disposition"), Some(1));
    assert_eq!(
        part.content_disposition().unwrap().unwrap().file_name(),
        Some("caf\u{e9}.txt")
    );
    part.headers_mut()
        .add(Header::parse(b"CONTENT-DISPOSITION: inline\n").unwrap());
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    part.set_content_disposition_with_options(Some(&value), &options)
        .unwrap();
    assert_eq!(part.headers().get_all("content-disposition").count(), 1);
    assert_eq!(
        part.headers().as_slice()[1].raw(),
        value
            .to_header_string_with_options(&options)
            .unwrap()
            .as_bytes()
    );
    part.headers_mut()
        .add(Header::parse(b"content-disposition: broken\n").unwrap());
    let invalid_options = HeaderFormatOptions {
        max_line_length: 0,
        ..Default::default()
    };
    part.set_content_disposition_with_options(None, &invalid_options)
        .unwrap();
    assert_eq!(part.content_disposition().unwrap(), None);
    assert!(!part.headers().contains("Content-Disposition"));
    part.set_content_disposition(None).unwrap();
    assert_eq!(part.content().get_ref().position(), 4);
}

#[test]
fn metadata_format_failures_leave_the_entire_part_unchanged() {
    let mut part = MimePart::from_parts(headers(
        b"Content-Type: text/plain\ncontent-type: text/html\nContent-Disposition: inline\nCONTENT-DISPOSITION: attachment\n"
    ), content());
    let before = part.headers().clone();
    let value = ContentType::new(&"x".repeat(100), "plain").unwrap();
    assert!(part.set_content_type(&value).is_err());
    assert_eq!(part.headers(), &before);
    let value = ContentDisposition::new(&"x".repeat(100)).unwrap();
    assert!(part.set_content_disposition(Some(&value)).is_err());
    assert_eq!(part.headers(), &before);
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert!(
        part.set_content_type_with_options(&ContentType::new("text", "plain").unwrap(), &options)
            .is_err()
    );
    assert_eq!(part.headers(), &before);
    assert_eq!(part.headers().index_of("Content-Disposition"), Some(2));
    assert_eq!(part.headers().last_index_of("Content-Type"), Some(1));
    assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
    assert_eq!(part.content().get_ref().position(), 4);
    assert_eq!(part.content().get_ref().get_ref(), b"body bytes");
}

#[test]
fn transfer_encoding_reads_decoded_first_value_with_explicit_unknown_errors() {
    for (value, encoding) in [
        (" BASE64; ignored", ContentEncoding::Base64),
        (
            "=?us-ascii?q?quoted-printable?=",
            ContentEncoding::QuotedPrintable,
        ),
        ("x-uue", ContentEncoding::UuEncode),
        ("8bit", ContentEncoding::EightBit),
        ("binary", ContentEncoding::Binary),
        ("7bit", ContentEncoding::SevenBit),
    ] {
        let mut part = MimePart::new(content());
        part.headers_mut()
            .add(Header::parse(format!("Content-Transfer-Encoding: {value}").as_bytes()).unwrap());
        assert_eq!(part.content_transfer_encoding().unwrap(), encoding);
        assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
        assert_eq!(part.content().get_ref().position(), 4);
    }
    for value in ["", "unknown", "(comment) base64"] {
        let mut part = MimePart::new(content());
        part.headers_mut()
            .add(Header::parse(format!("Content-Transfer-Encoding: {value}").as_bytes()).unwrap());
        assert!(part.content_transfer_encoding().is_err());
    }
}

#[test]
fn transfer_encoding_updates_do_not_relabel_or_transcode_stored_content() {
    let raw = headers(
        b"Content-Transfer-Encoding: 7bit\nX-Middle: raw\nCONTENT-TRANSFER-ENCODING: binary\n",
    );
    let mut stored =
        MimeContent::with_encoding(Cursor::new(b"aGVsbG8=\r\n"), ContentEncoding::Base64);
    stored.get_mut().set_position(5);
    let mut part = MimePart::from_parts(raw.clone(), stored);
    for encoding in [
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        part.set_content_transfer_encoding(encoding);
        assert_eq!(part.content_transfer_encoding().unwrap(), encoding);
        assert_eq!(
            part.headers().get_all("Content-Transfer-Encoding").count(),
            1
        );
        assert_eq!(
            part.headers().as_slice()[0].raw(),
            format!(
                "Content-Transfer-Encoding: {}\r\n",
                encoding.as_str().unwrap()
            )
            .as_bytes()
        );
        assert_eq!(part.headers().as_slice()[1], raw.as_slice()[1]);
        assert_eq!(part.content().encoding(), ContentEncoding::Base64);
        assert_eq!(part.content().get_ref().position(), 5);
    }
    part.set_content_transfer_encoding(ContentEncoding::Unspecified);
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Unspecified
    );
    assert_eq!(part.headers().as_slice(), &raw.as_slice()[1..2]);
    part.set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    assert_eq!(
        part.headers().index_of("Content-Transfer-Encoding"),
        Some(1)
    );
    let mut decoded = Vec::new();
    part.content_mut()
        .open()
        .unwrap()
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, b"hello");
}

#[test]
fn replacing_and_extracting_content_preserves_ownership_positions_and_hints() {
    let mut part = MimePart::new(content());
    part.set_content_transfer_encoding(ContentEncoding::SevenBit);
    let before = part.headers().clone();
    let mut replacement = MimeContent::with_encoding(
        Cursor::new(b"new bytes".to_vec()),
        ContentEncoding::EightBit,
    );
    replacement.get_mut().set_position(2);
    replacement.set_new_line_format(Some(NewLineFormat::Mixed));
    let previous = part.set_content(replacement);
    assert_eq!(previous.encoding(), ContentEncoding::Unspecified);
    assert_eq!(previous.get_ref().position(), 4);
    assert_eq!(part.headers(), &before);
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::SevenBit
    );
    let (raw, content) = part.into_parts();
    assert_eq!(raw, before);
    assert_eq!(content.encoding(), ContentEncoding::EightBit);
    assert_eq!(content.new_line_format(), Some(NewLineFormat::Mixed));
    assert_eq!(content.get_ref().position(), 2);
    assert_eq!(content.into_inner().into_inner(), b"new bytes");
}

#[test]
fn metadata_never_reads_or_seeks_and_borrowed_sources_remain_usable() {
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
    let mut part = MimePart::new(MimeContent::new(NoIo));
    part.set_content_type(&ContentType::new("text", "plain").unwrap())
        .unwrap();
    part.set_content_disposition(Some(&ContentDisposition::default()))
        .unwrap();
    part.set_content_transfer_encoding(ContentEncoding::Base64);
    assert!(part.content_type().unwrap().is_some());
    assert!(part.content_disposition().unwrap().is_some());
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Base64
    );
    part.set_content(MimeContent::new(NoIo));
    part.into_parts();

    let mut source = Cursor::new(b"borrowed bytes");
    source.set_position(3);
    {
        let mut part = MimePart::new(MimeContent::new(&mut source));
        part.set_content_transfer_encoding(ContentEncoding::Base64);
        assert_eq!(part.content().get_ref().position(), 3);
    }
    assert_eq!(source.position(), 3);
    let mut remaining = Vec::new();
    source.read_to_end(&mut remaining).unwrap();
    assert_eq!(remaining, b"rowed bytes");

    source.set_position(2);
    let oversized = ContentType::new(&"x".repeat(100), "plain").unwrap();
    assert!(MimePart::with_content_type(&oversized, MimeContent::new(&mut source)).is_err());
    assert_eq!(source.position(), 2);
}

#[test]
fn empty_body_and_detached_metadata_can_be_edited_and_extracted() {
    let mut source = Cursor::new(Vec::<u8>::new());
    let mut part = MimePart::new(MimeContent::new(&mut source));
    let mut value = part.content_type().unwrap().unwrap();
    value.set_name(Some("empty.bin"));
    assert_eq!(part.content_type().unwrap().unwrap().name(), None);
    part.set_content_type(&value).unwrap();
    assert_eq!(
        part.content_type().unwrap().unwrap().name(),
        Some("empty.bin")
    );
    assert_eq!(part.content_mut().decode_to(&mut Vec::new()).unwrap(), 0);
    let (raw, content) = part.into_parts();
    assert_eq!(
        raw.get("Content-Type").unwrap(),
        &value.to_header().unwrap()
    );
    let returned_source = content.into_inner();
    assert_eq!(returned_source.position(), 0);
    returned_source
        .get_mut()
        .extend_from_slice(b"still borrowed");
    assert_eq!(source.into_inner(), b"still borrowed");
}

// Adapted from MimeKit's Header.cs, MimeMessage.cs and MimeEntity.cs integration.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{Cursor, Read},
};

use mimecraft::{
    AddressComplianceMode, AddressHeaderFormatOptions, AddressList, AddressParseOptions,
    ContentDisposition, ContentType, Header, HeaderFormatOptions, HeaderId, HeaderList,
    HeaderParser, ParameterComplianceMode, ParameterParseOptions, charsets::Charset,
    encodings::NewLineFormat,
};

fn headers(input: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    assert_eq!(parser.parse(input, &mut records).unwrap(), input.len());
    parser.finish(&mut records).unwrap();
    records.into()
}

fn replace<E: Error + 'static>(
    headers: &mut HeaderList,
    encoded: Result<Header, E>,
) -> Result<Option<Header>, Box<dyn Error>> {
    Ok(headers.replace_all(encoded?)?)
}

#[test]
fn address_parsing_uses_raw_syntax_before_decoding_names() {
    let input = b"tO \t: =?utf-8?q?Doe=2C_Jane=3A_=22J=22?= <jane@example.com>,\r\n\t=?utf-8?q?A=0D=0AB?= <a@b>\n";
    let header = Header::parse(input).unwrap();
    let parsed = header.parse_addresses().unwrap();
    assert_eq!(parsed.len(), 2);
    let mut mailboxes = parsed.mailboxes();
    assert_eq!(mailboxes.next().unwrap().display_name(), "Doe, Jane: \"J\"");
    assert_eq!(mailboxes.next().unwrap().display_name(), "A\r\nB");
    assert_eq!(parsed, AddressList::parse(header.raw_value()).unwrap());
    assert_eq!(header.raw(), input);
    assert_eq!(header.id(), HeaderId::To);
}

#[test]
fn content_parsing_decodes_parameter_data_without_reinterpreting_delimiters() {
    let header =
        Header::parse(b"Content-Type: text/plain;\r\n\tname=\"=?utf-8?q?a=22=3Bb.txt?=\"\n")
            .unwrap();
    let value = header.parse_content_type().unwrap();
    assert_eq!(value.parameters().get("name"), Some("a\";b.txt"));
    assert_eq!(value, ContentType::parse(header.raw_value()).unwrap());

    let header =
        Header::parse(b"Content-Disposition: attachment;\r\n\tfilename*=utf-8''caf%C3%A9%3B.txt\n")
            .unwrap();
    let value = header.parse_content_disposition().unwrap();
    assert_eq!(value.file_name(), Some("caf\u{e9};.txt"));
    assert_eq!(
        value,
        ContentDisposition::parse(header.raw_value()).unwrap()
    );
}

#[test]
fn parse_errors_and_recovery_diagnostics_keep_raw_value_coordinates() {
    let header = Header::parse(b"Long-Extension-Field: \ttext plain\r\n").unwrap();
    let error = header.parse_content_type().unwrap_err();
    assert_eq!(error, ContentType::parse(header.raw_value()).unwrap_err());
    assert_eq!(error.offset(), 7);
    let header = Header::parse(b"Content-Disposition: \t; filename=x\r\n").unwrap();
    let error = header.parse_content_disposition().unwrap_err();
    assert_eq!(
        error,
        ContentDisposition::parse(header.raw_value()).unwrap_err()
    );
    assert_eq!(error.offset(), 2);
    let header = Header::parse(b"To: \ta@b, <broken@\r\n").unwrap();
    assert_eq!(
        header.parse_addresses().unwrap_err(),
        AddressList::parse(header.raw_value()).unwrap_err()
    );

    let header =
        Header::parse(b"Content-Type: \ttext/plain;\r\n\tname*=unknown''caf%E9.txt\r\n").unwrap();
    let value = header.parse_content_type().unwrap();
    assert!(!value.parameters().issues().is_empty());
    assert_eq!(
        value.parameters().issues(),
        ContentType::parse(header.raw_value())
            .unwrap()
            .parameters()
            .issues()
    );
    let header =
        Header::parse(b"Content-Disposition: \tattachment;\r\n\tfilename*=unknown''caf%E9.txt\r\n")
            .unwrap();
    let value = header.parse_content_disposition().unwrap();
    assert!(!value.parameters().issues().is_empty());
    assert_eq!(
        value.parameters().issues(),
        ContentDisposition::parse(header.raw_value())
            .unwrap()
            .parameters()
            .issues()
    );
    let header = Header::parse(b"To: Andr\xe9 <a@b>\r\n").unwrap();
    let value = header.parse_addresses().unwrap();
    assert!(
        !value
            .mailboxes()
            .next()
            .unwrap()
            .display_name_issues()
            .is_empty()
    );
    assert_eq!(
        value.mailboxes().next().unwrap().display_name_issues(),
        AddressList::parse(header.raw_value())
            .unwrap()
            .mailboxes()
            .next()
            .unwrap()
            .display_name_issues()
    );
}

#[test]
fn explicit_address_options_reach_nesting_syntax_and_name_decoding() {
    let header = Header::parse(b"To: Outer: Inner: a@b;;\r\n").unwrap();
    assert!(header.parse_addresses().is_err());
    let options = AddressParseOptions {
        max_group_depth: 2,
        ..Default::default()
    };
    assert_eq!(
        header.parse_addresses_with_options(&options).unwrap(),
        AddressList::parse_with_options(header.raw_value(), &options).unwrap()
    );
    let header = Header::parse(b"To: a@b c@d\r\n").unwrap();
    assert_eq!(header.parse_addresses().unwrap().len(), 2);
    let mut options = AddressParseOptions::default();
    options.mailbox.address.compliance_mode = AddressComplianceMode::Strict;
    assert!(header.parse_addresses_with_options(&options).is_err());
    let header = Header::parse(b"To: Price\x80 <a@b>\r\n").unwrap();
    options.mailbox.name_decoding.fallback_charset =
        Some(Charset::from_label("windows-1252").unwrap());
    let value = header.parse_addresses_with_options(&options).unwrap();
    assert_eq!(
        value.mailboxes().next().unwrap().display_name(),
        "Price\u{20ac}"
    );
}

#[test]
fn explicit_parameter_options_reach_both_content_parsers() {
    let content_type = Header::parse(b"Content-Type: text/plain; name=two words\r\n").unwrap();
    let disposition =
        Header::parse(b"Content-Disposition: attachment; filename=two words\r\n").unwrap();
    assert!(content_type.parse_content_type().is_ok());
    assert!(disposition.parse_content_disposition().is_ok());
    let options = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    assert!(
        content_type
            .parse_content_type_with_options(&options)
            .is_err()
    );
    assert!(
        disposition
            .parse_content_disposition_with_options(&options)
            .is_err()
    );
    let content_type = Header::parse(b"Content-Type: text/plain; name=\"\xe0\"\r\n").unwrap();
    let disposition =
        Header::parse(b"Content-Disposition: attachment; filename=\"\xe0\"\r\n").unwrap();
    let options = ParameterParseOptions {
        fallback_charset: Some(Charset::from_label("windows-1251").unwrap()),
        ..Default::default()
    };
    assert_eq!(
        content_type
            .parse_content_type_with_options(&options)
            .unwrap()
            .parameters()
            .get("name"),
        Some("\u{430}")
    );
    assert_eq!(
        disposition
            .parse_content_disposition_with_options(&options)
            .unwrap()
            .file_name(),
        Some("\u{430}")
    );
}

#[test]
fn typed_parsing_selects_value_grammar_without_dispatching_on_field_names() {
    assert_eq!(
        Header::parse(b"X-Recipients: a@b")
            .unwrap()
            .parse_addresses()
            .unwrap()
            .len(),
        1
    );
    assert!(
        Header::parse(b"X-Type: text/plain")
            .unwrap()
            .parse_content_type()
            .unwrap()
            .is_mime_type("text", "plain")
    );
    assert_eq!(
        Header::parse(b"X-Disposition: inline")
            .unwrap()
            .parse_content_disposition()
            .unwrap()
            .disposition(),
        "inline"
    );
    let list = headers(b"Subject: good\r\nMalformed record\r\n");
    let invalid = &list.as_slice()[1];
    assert!(invalid.is_invalid());
    for header in [invalid, &Header::parse(b"Bcc: \t\r\n").unwrap()] {
        assert!(header.parse_addresses().is_err());
        assert!(header.parse_content_type().is_err());
        assert!(header.parse_content_disposition().is_err());
    }
    let empty = AddressList::new().to_header("Bcc").unwrap();
    assert_eq!(empty.raw(), b"Bcc:\r\n");
    assert!(empty.parse_addresses().is_err());
}

#[test]
fn typed_models_create_identical_records_to_their_default_formatters() {
    let addresses: AddressList = "Alice <alice@example.com>".parse().unwrap();
    let content_type: ContentType = "text/plain; charset=utf-8".parse().unwrap();
    let disposition: ContentDisposition = "attachment; filename=report.txt".parse().unwrap();
    for (header, formatted, field, id) in [
        (
            addresses.to_header("tO").unwrap(),
            addresses.to_header_string("tO").unwrap(),
            "tO",
            HeaderId::To,
        ),
        (
            content_type.to_header().unwrap(),
            content_type.to_header_string().unwrap(),
            "Content-Type",
            HeaderId::ContentType,
        ),
        (
            disposition.to_header().unwrap(),
            disposition.to_header_string().unwrap(),
            "Content-Disposition",
            HeaderId::ContentDisposition,
        ),
    ] {
        assert_eq!(header.raw(), formatted.as_bytes());
        assert_eq!(header.field(), Some(field));
        assert_eq!(header.raw_field(), field.as_bytes());
        assert_eq!(header.id(), id);
        assert_eq!(header, Header::parse(formatted.as_bytes()).unwrap());
        assert!(!header.is_invalid());
    }
    assert_eq!(
        addresses
            .to_header("To")
            .unwrap()
            .parse_addresses()
            .unwrap(),
        addresses
    );
    assert_eq!(
        content_type
            .to_header()
            .unwrap()
            .parse_content_type()
            .unwrap(),
        content_type
    );
    assert_eq!(
        disposition
            .to_header()
            .unwrap()
            .parse_content_disposition()
            .unwrap(),
        disposition
    );
}

#[test]
fn custom_formatters_keep_folds_newlines_and_cached_identifiers() {
    let name = "Ren\u{e9}e ".repeat(30);
    let addresses: AddressList =
        vec![mimecraft::MailboxAddress::new(&name, "a@b".parse().unwrap()).into()].into();
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    let header = addresses
        .to_header_with_options("X-Recipients", &options)
        .unwrap();
    assert_eq!(
        header.raw(),
        addresses
            .to_header_string_with_options("X-Recipients", &options)
            .unwrap()
            .as_bytes()
    );
    assert_eq!(header.id(), HeaderId::Unknown);
    assert_eq!(header.field(), Some("X-Recipients"));
    assert!(header.raw().windows(2).any(|bytes| bytes == b"\n\t"));
    assert!(!header.raw().contains(&b'\r'));
    assert_eq!(header.parse_addresses().unwrap(), addresses);

    let options = HeaderFormatOptions {
        max_line_length: 60,
        new_line_format: NewLineFormat::Unix,
        always_quote_parameters: true,
    };
    let mut content_type = ContentType::new("text", "plain").unwrap();
    content_type.set_name(Some(&"long file name ".repeat(20)));
    let mut disposition = ContentDisposition::default();
    disposition.set_file_name(Some(&"long file name ".repeat(20)));
    let type_header = content_type.to_header_with_options(&options).unwrap();
    let disposition_header = disposition.to_header_with_options(&options).unwrap();
    assert_eq!(
        type_header.raw(),
        content_type
            .to_header_string_with_options(&options)
            .unwrap()
            .as_bytes()
    );
    assert_eq!(
        disposition_header.raw(),
        disposition
            .to_header_string_with_options(&options)
            .unwrap()
            .as_bytes()
    );
    for header in [&type_header, &disposition_header] {
        assert!(!header.raw().contains(&b'\r'));
        assert!(header.raw().ends_with(b"\n"));
        assert!(header.raw().windows(2).any(|bytes| bytes == b"\n\t"));
    }
    assert_eq!(type_header.parse_content_type().unwrap(), content_type);
    assert_eq!(
        disposition_header.parse_content_disposition().unwrap(),
        disposition
    );
}

#[test]
fn typed_values_are_detached_and_serialization_keeps_source_diagnostics() {
    let original =
        Header::parse(b"cOnTeNt-TyPe \t: text/plain;\r\n\tname*=unknown''caf%E9.txt\n").unwrap();
    let mut value = original.parse_content_type().unwrap();
    let issues = value.parameters().issues().to_vec();
    let before = original.clone();
    value.set_charset(Some("utf-8"));
    let replacement = value.to_header().unwrap();
    assert_eq!(value.parameters().issues(), issues);
    assert_eq!(original, before);
    assert_ne!(replacement.raw(), original.raw());
    assert_eq!(
        replacement.parse_content_type().unwrap().charset(),
        Some("utf-8")
    );
    assert!(
        replacement
            .parse_content_type()
            .unwrap()
            .parameters()
            .issues()
            .is_empty()
    );
}

#[test]
fn address_replacement_collapses_duplicates_without_touching_other_records() {
    let mut list = headers(b"Received: first\nTo \t: old@x\r\nX-Raw: \xff\r\nMalformed record\r\ntO: duplicate@x\nSubject: end");
    let before = list.clone();
    let addresses: AddressList = "New <new@x>".parse().unwrap();
    let replacement = addresses.to_header("TO").unwrap();
    let previous = list.replace_all(replacement.clone()).unwrap().unwrap();
    assert_eq!(previous, before.as_slice()[1]);
    assert_eq!(
        list.as_slice(),
        &[
            before.as_slice()[0].clone(),
            replacement,
            before.as_slice()[2].clone(),
            before.as_slice()[3].clone(),
            before.as_slice()[5].clone(),
        ]
    );
    assert_eq!(list.index_of("to"), Some(1));
    assert_eq!(list.last_index_of("To"), Some(1));
    assert_eq!(list.index_of("Subject"), Some(4));
    assert_eq!(list.get_all("to").count(), 1);
    assert_eq!(
        list.get("to").unwrap().parse_addresses().unwrap(),
        addresses
    );
}

#[test]
fn content_replacements_share_existing_field_order_and_append_behavior() {
    let mut list = headers(
        b"X-First: raw\ncontent-type: text/html\nSubject: unchanged\nCONTENT-TYPE: image/png\n",
    );
    let before = list.clone();
    let content_type = ContentType::new("text", "plain").unwrap();
    let previous = list
        .replace_all(content_type.to_header().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(previous, before.as_slice()[1]);
    assert_eq!(list.as_slice()[0], before.as_slice()[0]);
    assert_eq!(list.as_slice()[2], before.as_slice()[2]);
    assert_eq!(list.len(), 3);
    assert_eq!(
        list.get("content-type")
            .unwrap()
            .parse_content_type()
            .unwrap(),
        content_type
    );
    let disposition = ContentDisposition::default();
    assert_eq!(
        list.replace_all(disposition.to_header().unwrap()).unwrap(),
        None
    );
    assert_eq!(list.index_of("CONTENT-DISPOSITION"), Some(3));
    assert_eq!(
        list.get("Content-Disposition")
            .unwrap()
            .parse_content_disposition()
            .unwrap(),
        disposition
    );
}

#[test]
fn failed_typed_updates_leave_records_duplicates_and_lookup_unchanged() {
    let mut list = headers(b"To: old@x\nTO: duplicate@x\nContent-Type: text/plain\nContent-Disposition: inline\nX-End: raw\n");
    let before = list.clone();
    let addresses: AddressList = "caf\u{e9}@example.com".parse().unwrap();
    assert!(replace(&mut list, addresses.to_header("To")).is_err());
    assert_eq!(list, before);
    let addresses: AddressList = "a@b".parse().unwrap();
    assert!(replace(&mut list, addresses.to_header("To\r\nBcc")).is_err());
    assert_eq!(list, before);
    let content_type = ContentType::new(&"x".repeat(100), "plain").unwrap();
    assert!(replace(&mut list, content_type.to_header()).is_err());
    assert_eq!(list, before);
    let disposition = ContentDisposition::default();
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert!(replace(&mut list, disposition.to_header_with_options(&options)).is_err());
    assert_eq!(list, before);
    assert_eq!(list.index_of("to"), Some(0));
    assert_eq!(list.last_index_of("to"), Some(1));
    assert_eq!(list.index_of("X-End"), Some(4));
}

#[test]
fn load_edit_and_write_preserves_unedited_headers_and_body_position() {
    let input = b"Subject: caf\xe9\nContent-Type \t: Text/Plain; charset=iso-8859-1\nTo: old@example.com\n\r\nbody bytes";
    let mut reader = Cursor::new(input);
    let mut list = HeaderList::load(&mut reader).unwrap();
    let subject = list.get("Subject").unwrap().clone();
    let mut content_type = list
        .get("Content-Type")
        .unwrap()
        .parse_content_type()
        .unwrap();
    content_type.set_charset(Some("utf-8"));
    list.replace_all(content_type.to_header().unwrap()).unwrap();
    let addresses: AddressList = "New <new@example.com>".parse().unwrap();
    list.replace_all(addresses.to_header("To").unwrap())
        .unwrap();
    let mut body = Vec::new();
    reader.read_to_end(&mut body).unwrap();
    assert_eq!(body, b"body bytes");
    let mut output = Vec::new();
    list.write_raw_to(&mut output).unwrap();
    assert!(output.starts_with(subject.raw()));
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(&body);
    let mut reader = Cursor::new(output);
    let reread = HeaderList::load(&mut reader).unwrap();
    assert_eq!(reread, list);
    assert_eq!(
        reread.get("To").unwrap().parse_addresses().unwrap(),
        addresses
    );
    assert_eq!(
        reread
            .get("Content-Type")
            .unwrap()
            .parse_content_type()
            .unwrap(),
        content_type
    );
    let mut remainder = Vec::new();
    reader.read_to_end(&mut remainder).unwrap();
    assert_eq!(remainder, body);
}

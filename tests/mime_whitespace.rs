// Adapted from MimeKit's UnitTests/MimeWhitespaceTests.cs and MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::Cursor;

use mimecraft::{
    Address, AddressHeaderFormatOptions, AddressList, ContentDisposition, GroupAddress, Header,
    MailboxAddress, MessageId, MessageImportance, MessagePriority, MimeContent, MimeEntity,
    MimePart, Multipart, MultipartReport, TextFormat, TextPart, utils::unfold,
};

const NON_LINEAR: [char; 5] = ['\u{a0}', '\u{2008}', '\u{3000}', '\x0b', '\x0c'];

#[test]
fn headers_and_content_metadata_preserve_non_linear_whitespace() {
    for padding in NON_LINEAR {
        let value = format!("{padding}text{padding}");
        assert_eq!(unfold(&format!(" \t{value}\r\n")), value);
        let header = Header::from_unstructured("Subject", &value).unwrap();
        assert_eq!(header.decode_value().text, value);

        let encoded: String = value.bytes().map(|byte| format!("={byte:02X}")).collect();
        let header =
            Header::parse(format!("Subject: =?utf-8?q?{encoded}?=\r\n").as_bytes()).unwrap();
        assert_eq!(header.decode_value().text, value);

        let mut part = MimePart::new(MimeContent::new(Cursor::new(Vec::new())));
        part.set_content_description(Some(&value)).unwrap();
        part.set_content_md5(Some(&value)).unwrap();
        assert_eq!(part.content_description().unwrap().text, value);
        assert_eq!(part.content_md5().unwrap().text, value);
        for field in ["Content-Description", "Content-MD5"] {
            part.headers_mut()
                .replace_all(Header::parse(format!("{field}: {value}\r\n").as_bytes()).unwrap())
                .unwrap();
        }
        assert_eq!(part.content_description().unwrap().text, value);
        assert_eq!(part.content_md5().unwrap().text, value);
    }
}

#[test]
fn display_names_preserve_unicode_whitespace_in_assignment_and_parsing() {
    for padding in ['\u{a0}', '\u{2008}', '\u{3000}'] {
        let name = format!("{padding}Name{padding}");
        let mut mailbox = MailboxAddress::new(&name, "a@example.com".parse().unwrap());
        assert_eq!(mailbox.display_name(), name);
        mailbox.set_display_name(&format!(" \t{name} \t"));
        assert_eq!(mailbox.display_name(), name);
        for text in [
            format!("\"{name}\" <a@example.com>"),
            format!("a@example.com ({name})"),
        ] {
            assert_eq!(text.parse::<MailboxAddress>().unwrap().display_name(), name);
            assert_eq!(
                MailboxAddress::parse(text.as_bytes())
                    .unwrap()
                    .display_name(),
                name
            );
        }
    }
}

#[test]
fn international_display_name_folding_preserves_unicode_separators() {
    let name = format!(
        "{}\u{a0}{}\u{2008}{}",
        "A".repeat(30),
        "B".repeat(30),
        "C".repeat(30)
    );
    for address in [
        Address::from(MailboxAddress::new(&name, "a@example.com".parse().unwrap())),
        Address::from(GroupAddress::new(&name, AddressList::new())),
    ] {
        let list = AddressList::from(vec![address]);
        let raw = list
            .to_header_string_with_options(
                "To",
                &AddressHeaderFormatOptions {
                    international: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let header = Header::parse(raw.as_bytes()).unwrap();
        assert_eq!(AddressList::parse(header.raw_value()).unwrap(), list);
    }
}

#[test]
fn parameter_round_trip_preserves_boundary_and_filename_whitespace() {
    let mut multipart = Multipart::with_boundary("mixed", " boundary").unwrap();
    let mut part = MimePart::new(MimeContent::new(Cursor::new(Vec::new())));
    let filename = " \u{a0}report.txt\u{3000} ";
    part.set_file_name(Some(filename)).unwrap();
    assert_eq!(part.file_name().unwrap().as_deref(), Some(filename));
    part.set_content_disposition(None).unwrap();
    assert_eq!(part.file_name().unwrap().as_deref(), Some(filename));
    multipart.children_mut().push(part.into());
    let mut entity = MimeEntity::from(multipart);
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    let MimeEntity::Multipart(parsed) = MimeEntity::load(&mut Cursor::new(output)).unwrap() else {
        panic!("expected multipart");
    };
    assert_eq!(parsed.boundary().unwrap().as_deref(), Some(" boundary"));
    assert_eq!(parsed.children().len(), 1);
    let MimeEntity::Part(part) = &parsed.children()[0] else {
        panic!("expected leaf");
    };
    assert_eq!(part.file_name().unwrap().as_deref(), Some(filename));
}

#[test]
fn tokens_do_not_ignore_non_linear_whitespace() {
    for padding in NON_LINEAR {
        let mut part = TextPart::new("plain", "").unwrap();
        let mut content_type = part.part().content_type().unwrap().unwrap();
        content_type
            .parameters_mut()
            .set("format", format!("{padding}flowed{padding}"))
            .unwrap();
        part.part_mut().set_content_type(&content_type).unwrap();
        assert!(!part.is_flowed().unwrap());
        assert_eq!(part.format().unwrap(), TextFormat::Plain);

        // Rust retains its strict disposition and priority parse errors rather
        // than MimeKit's TryParse attachment or Normal fallbacks.
        assert!(
            ContentDisposition::parse(format!("\"{padding}inline{padding}\"").as_bytes(),).is_err()
        );
        assert!(MessageImportance::parse(&format!("{padding}high{padding}")).is_err());
        assert!(MessagePriority::parse(&format!("{padding}urgent{padding}")).is_err());
        let value = format!("{padding}delivery-status{padding}");
        let mut report: MultipartReport =
            MultipartReport::new("delivery-status", "boundary").unwrap();
        report.set_report_type(&value).unwrap();
        assert_eq!(
            report.report_type().unwrap().as_deref(),
            Some(value.as_str())
        );
    }
}

#[test]
fn message_id_unicode_whitespace_survives_getter_setter_round_trips() {
    for value in [
        "\u{a0}id@example.com\u{a0}",
        "\u{2008}id@example.com\u{3000}",
    ] {
        let parsed = MessageId::parse(format!("<{value}>").as_bytes()).unwrap();
        assert_eq!(parsed.as_str(), value);
        assert_eq!(MessageId::from_value(parsed.as_str()).unwrap(), parsed);
        assert_eq!(
            MessageId::from_value(&format!(" \t<{value}>\r\n")).unwrap(),
            parsed
        );
    }
}

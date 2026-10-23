// Adapted from MimeKit's InternetAddressListTests.cs and MailboxAddressTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::error::Error;

use mimecraft::{
    Address, AddressEncodeErrorKind, AddressHeaderFormatError, AddressHeaderFormatOptions,
    AddressList, AddressParseOptions, GroupAddress, Header, MailboxAddress, charsets::Charset,
    encodings::NewLineFormat, rfc2047::EncodeOptions,
};

fn mailbox(name: &str, address: &str) -> Address {
    MailboxAddress::new(name, address.parse().unwrap()).into()
}

fn assert_lines(header: &str, options: &AddressHeaderFormatOptions) {
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => panic!("invalid test options"),
    };
    assert!(header.is_ascii());
    assert!(header.ends_with(newline));
    assert!(!header.ends_with(&format!("{newline}{newline}")));
    if options.new_line_format == NewLineFormat::Unix {
        assert!(!header.contains('\r'));
    }
    for (index, line) in header.split_terminator(newline).enumerate() {
        assert!(!line.contains(['\r', '\n']));
        assert!(
            line.len() <= options.max_line_length,
            "{} > {}",
            line.len(),
            options.max_line_length
        );
        if index > 0 {
            assert!(line.starts_with('\t'));
        }
    }
}

fn assert_round_trip(header: &str, list: &AddressList) {
    let header = Header::parse(header.as_bytes()).unwrap();
    let parsed = AddressList::parse_with_options(
        header.raw_value(),
        &AddressParseOptions {
            max_group_depth: 255,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(&parsed, list);
    for (actual, expected) in parsed.mailboxes().zip(list.mailboxes()) {
        assert_eq!(actual.route(), expected.route());
    }
}

#[test]
fn complete_headers_preserve_field_spelling_and_empty_lists() {
    let list: AddressList = "Alice <a@b>, c@d".parse().unwrap();
    assert_eq!(
        list.to_header_string("tO").unwrap(),
        "tO: Alice <a@b>, c@d\r\n"
    );
    assert_eq!(
        AddressList::new().to_header_string("Bcc").unwrap(),
        "Bcc:\r\n"
    );
    assert_eq!(
        list.to_header_string("X-Recipients").unwrap(),
        "X-Recipients: Alice <a@b>, c@d\r\n"
    );
    let options = AddressHeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    assert_eq!(
        AddressList::new()
            .to_header_string_with_options("Bcc", &options)
            .unwrap(),
        "Bcc:\n"
    );
    let header = list.to_header_string_with_options("To", &options).unwrap();
    assert_lines(&header, &options);
    assert_round_trip(&header, &list);
}

#[test]
fn exact_physical_limits_include_field_names_indentation_and_commas() {
    for limit in [60, 78, 998] {
        let options = AddressHeaderFormatOptions {
            max_line_length: limit,
            ..Default::default()
        };
        let address = format!("{}@b", "a".repeat(limit - 6));
        let list: AddressList = vec![mailbox("", &address)].into();
        let header = list.to_header_string_with_options("To", &options).unwrap();
        assert_eq!(header, format!("To: {address}\r\n"));
        assert_lines(&header, &options);
        assert_round_trip(&header, &list);

        let address = format!("{}@b", "a".repeat(limit - 3));
        let list: AddressList = vec![mailbox("", &address)].into();
        let header = list.to_header_string_with_options("To", &options).unwrap();
        assert_eq!(header, format!("To:\r\n\t{address}\r\n"));
        assert_lines(&header, &options);
        assert_round_trip(&header, &list);

        let list: AddressList = vec![mailbox("", &address), mailbox("", "c@d")].into();
        assert_eq!(
            list.to_header_string_with_options("To", &options)
                .unwrap_err(),
            AddressHeaderFormatError::LineTooLong {
                required: limit + 1,
                limit
            }
        );
        let address = format!("{}@b", "a".repeat(limit - 7));
        let list: AddressList = vec![mailbox("", &address), mailbox("", "c@d")].into();
        let header = list.to_header_string_with_options("To", &options).unwrap();
        assert_eq!(header, format!("To: {address},\r\n\tc@d\r\n"));
        assert_lines(&header, &options);
        assert_round_trip(&header, &list);
    }
}

#[test]
fn field_prefixes_are_checked_even_with_no_addresses() {
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    let field = "X".repeat(59);
    assert_eq!(
        AddressList::new()
            .to_header_string_with_options(&field, &options)
            .unwrap(),
        format!("{field}:\r\n")
    );
    let list: AddressList = "a@b".parse().unwrap();
    assert_eq!(
        list.to_header_string_with_options(&field, &options)
            .unwrap(),
        format!("{field}:\r\n\ta@b\r\n")
    );
    let field = "X".repeat(60);
    assert_eq!(
        AddressList::new()
            .to_header_string_with_options(&field, &options)
            .unwrap_err(),
        AddressHeaderFormatError::LineTooLong {
            required: 61,
            limit: 60
        }
    );
}

#[test]
fn folding_does_not_change_plain_name_word_boundaries() {
    let name = "Alice Bob Carol Dave Eve Frank Grace Henry Iris Jane Kate";
    let list: AddressList = vec![mailbox(name, "a@b"), mailbox(name, "c@d")].into();
    for limit in 60..=100 {
        let options = AddressHeaderFormatOptions {
            max_line_length: limit,
            ..Default::default()
        };
        let header = list
            .to_header_string_with_options("Resent-To", &options)
            .unwrap();
        assert_lines(&header, &options);
        assert_round_trip(&header, &list);
    }
}

#[test]
fn quotes_escapes_angles_and_literal_delimiters_do_not_create_false_fold_points() {
    let mut routed = MailboxAddress::new(
        "A \"quoted\" \\ name",
        "\"a b\\\" c\"@example.com".parse().unwrap(),
    );
    *routed.route_mut() = ["[a\"<>b]", "relay.example"].into_iter().collect();
    let list: AddressList = vec![
        mailbox("A  B, C", "\"with spaces\tand tabs\"@example.com"),
        routed.into(),
        mailbox("", "user@[a\"<>b]"),
        mailbox("After Literal", "after@example.com"),
    ]
    .into();
    for limit in 78..=110 {
        let options = AddressHeaderFormatOptions {
            max_line_length: limit,
            ..Default::default()
        };
        let header = list.to_header_string_with_options("To", &options).unwrap();
        assert_lines(&header, &options);
        assert_round_trip(&header, &list);
        assert!(header.contains("\"with spaces\tand tabs\"@example.com"));
        assert!(header.contains("<@[a\"<>b],@relay.example:\"a b\\\" c\"@example.com>"));
    }
}

#[test]
fn unbreakable_quoted_local_parts_and_routes_fail_instead_of_being_rewritten() {
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        ..Default::default()
    };
    let list: AddressList = vec![mailbox("", &format!("\"{}\"@b", "a b ".repeat(20)))].into();
    assert!(matches!(
        list.to_header_string_with_options("To", &options),
        Err(AddressHeaderFormatError::LineTooLong { .. })
    ));
    let mut routed = MailboxAddress::new("", "a@b".parse().unwrap());
    *routed.route_mut() = [
        "relay-one.example",
        "relay-two.example",
        "relay-three.example",
    ]
    .into_iter()
    .collect();
    let list: AddressList = vec![routed.into()].into();
    assert!(matches!(
        list.to_header_string_with_options("To", &options),
        Err(AddressHeaderFormatError::LineTooLong { .. })
    ));
}

#[test]
fn long_names_fit_via_encoded_words_and_preserve_whitespace_and_punctuation() {
    for name in [
        "Ren\u{e9}e, \"a friend\" ".repeat(25),
        "A  B, C \\ D ".repeat(30),
        "x".repeat(500),
        "A\r\nB\0C ".repeat(30),
    ] {
        let group = GroupAddress::new(&name, vec![mailbox(&name, "a@b")].into());
        let list: AddressList = vec![group.into()].into();
        for newline in [NewLineFormat::Dos, NewLineFormat::Unix] {
            for limit in [60, 61, 75, 78, 998] {
                let options = AddressHeaderFormatOptions {
                    max_line_length: limit,
                    new_line_format: newline,
                    ..Default::default()
                };
                let header = list.to_header_string_with_options("To", &options).unwrap();
                assert_lines(&header, &options);
                assert_round_trip(&header, &list);
                assert!(!header.contains('\0'));
                for word in header
                    .split_ascii_whitespace()
                    .filter(|word| word.starts_with("=?"))
                {
                    let word = word.strip_suffix(':').unwrap_or(word);
                    assert!(word.len() <= 75.min(limit - 2));
                }
            }
        }
    }
}

#[test]
fn custom_name_charset_and_word_limit_propagate_without_mutation() {
    let name = "Andr\u{e9} ".repeat(20);
    let list: AddressList = vec![mailbox(&name, "a@b")].into();
    let before = list.to_string();
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        name_encoding: EncodeOptions {
            charset: Charset::LATIN1,
            max_word_length: 30,
        },
        ..Default::default()
    };
    let header = list.to_header_string_with_options("To", &options).unwrap();
    assert!(header.contains("=?iso-8859-1?"));
    assert_lines(&header, &options);
    assert_round_trip(&header, &list);
    for word in header
        .split_ascii_whitespace()
        .filter(|word| word.starts_with("=?"))
    {
        assert!(word.len() <= 30);
    }
    assert_eq!(list.to_string(), before);
}

#[test]
fn invalid_field_names_and_format_options_are_explicit_errors() {
    let empty = AddressList::new();
    for (field, offset) in [
        ("", 0),
        ("To:", 2),
        (" To", 0),
        ("To ", 2),
        ("To\r\nBcc", 2),
        ("X-\u{e9}", 2),
        ("X\0", 1),
    ] {
        assert_eq!(
            empty.to_header_string(field).unwrap_err(),
            AddressHeaderFormatError::InvalidFieldName { offset }
        );
    }
    for limit in [0, 59, 999, usize::MAX] {
        let options = AddressHeaderFormatOptions {
            max_line_length: limit,
            ..Default::default()
        };
        let error = empty
            .to_header_string_with_options("Bcc", &options)
            .unwrap_err();
        assert!(matches!(
            error,
            AddressHeaderFormatError::InvalidLineLength(_)
        ));
        assert!(error.source().is_some());
    }
    let options = AddressHeaderFormatOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert_eq!(
        empty
            .to_header_string_with_options("Bcc", &options)
            .unwrap_err(),
        AddressHeaderFormatError::InvalidNewLineFormat(NewLineFormat::Mixed)
    );
    for max_word_length in [0, 76, usize::MAX] {
        let options = AddressHeaderFormatOptions {
            max_line_length: 60,
            name_encoding: EncodeOptions {
                max_word_length,
                ..Default::default()
            },
            ..Default::default()
        };
        let error = empty
            .to_header_string_with_options("Bcc", &options)
            .unwrap_err();
        let AddressHeaderFormatError::Address(source) = &error else {
            panic!("{error}");
        };
        assert!(source.path().is_empty());
        assert!(matches!(source.kind(), AddressEncodeErrorKind::Name(_)));
        assert!(error.source().is_some());
    }
}

#[test]
fn nested_encoding_errors_keep_member_paths_and_diagnostics() {
    let value = MailboxAddress::parse(b"Andr\xe9 <caf\xe9@b>").unwrap();
    let issues = value.display_name_issues().to_vec();
    let list: AddressList = vec![
        mailbox("", "a@b"),
        GroupAddress::new("G", vec![value.into()].into()).into(),
    ]
    .into();
    let error = list.to_header_string("To").unwrap_err();
    let AddressHeaderFormatError::Address(source) = &error else {
        panic!("{error}");
    };
    assert_eq!(source.path(), &[1, 0]);
    assert_eq!(
        source.kind(),
        &AddressEncodeErrorKind::InternationalLocalPart
    );
    assert!(error.source().is_some());
    let value = list.mailboxes().nth(1).unwrap();
    assert_eq!(value.display_name_issues(), issues);
    assert!(value.address().had_latin1_fallback());
}

#[test]
fn group_boundaries_and_maximum_parsed_nesting_are_preserved() {
    let list: AddressList = vec![
        GroupAddress::new("", AddressList::new()).into(),
        GroupAddress::new(
            "Outer",
            vec![
                GroupAddress::new("", vec![mailbox("Alice", "a@b")].into()).into(),
                mailbox("", "a@b"),
            ]
            .into(),
        )
        .into(),
        mailbox("", "a@b"),
    ]
    .into();
    assert_round_trip(&list.to_header_string("To").unwrap(), &list);

    let options = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let input = format!("{}a@b{}", "g:".repeat(255), ";".repeat(255));
    let list = AddressList::parse_with_options(input.as_bytes(), &options).unwrap();
    let options = AddressHeaderFormatOptions {
        max_line_length: 998,
        ..Default::default()
    };
    let header = list.to_header_string_with_options("To", &options).unwrap();
    assert_lines(&header, &options);
    assert_round_trip(&header, &list);
    assert!(matches!(
        list.to_header_string("To"),
        Err(AddressHeaderFormatError::LineTooLong { .. })
    ));
}

// Adapted from MimeKit's InternetAddressListTests.cs and MailboxAddressTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::error::Error;

use mimecraft::{
    AddrSpec, AddrSpecParseOptions, Address, AddressComplianceMode, AddressEncodeErrorKind,
    AddressHeaderFormatError, AddressHeaderFormatOptions, AddressList, AddressParseOptions,
    GroupAddress, Header, MailboxAddress,
    charsets::Charset,
    encodings::NewLineFormat,
    rfc2047::{EncodeError, EncodeOptions},
};

fn options() -> AddressHeaderFormatOptions {
    AddressHeaderFormatOptions {
        international: true,
        ..Default::default()
    }
}

fn mailbox(name: &str, address: &str) -> Address {
    MailboxAddress::new(name, address.parse().unwrap()).into()
}

fn check(list: &AddressList, options: &AddressHeaderFormatOptions) -> String {
    let before = list.to_string();
    let output = list.to_header_string_with_options("To", options).unwrap();
    let newline = match options.new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => unreachable!(),
    };
    assert!(output.ends_with(newline));
    assert!(!output.ends_with(&format!("{newline}{newline}")));
    for (index, line) in output.split_terminator(newline).enumerate() {
        assert!(!line.contains(['\r', '\n']));
        assert!(line.len() <= options.max_line_length, "{output:?}");
        assert!(index == 0 || line.starts_with('\t'), "{output:?}");
    }
    let header = Header::parse(output.as_bytes()).unwrap();
    for compliance_mode in [AddressComplianceMode::Strict, AddressComplianceMode::Loose] {
        let mut parsing = AddressParseOptions {
            max_group_depth: 255,
            ..Default::default()
        };
        parsing.mailbox.address.compliance_mode = compliance_mode;
        let parsed = AddressList::parse_with_options(header.raw_value(), &parsing).unwrap();
        assert_eq!(&parsed, list, "{output:?}");
        for (actual, expected) in parsed.mailboxes().zip(list.mailboxes()) {
            assert_eq!(actual.address().as_str(), expected.address().as_str());
            assert_eq!(actual.route(), expected.route());
        }
        assert_eq!(
            parsed.to_header_string_with_options("To", options).unwrap(),
            output
        );
    }
    assert_eq!(list.to_string(), before);
    output
}

#[test]
fn mimekit_international_mailbox_and_route_fixtures_keep_utf8() {
    let chinese = "\u{4f0a}\u{662d}\u{5091}@\u{90f5}\u{4ef6}.\u{5546}\u{52d9}";
    let list: AddressList = chinese.parse().unwrap();
    assert_eq!(check(&list, &options()), format!("To: {chinese}\r\n"));

    let mut routed = MailboxAddress::new("User Name", "user@domain.com".parse().unwrap());
    *routed.route_mut() = [
        "route",
        "\u{4f0a}\u{662d}\u{5091}.\u{90f5}\u{4ef6}.\u{5546}\u{52d9}",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        routed.to_ascii().unwrap(),
        "User Name <@route,@xn--qoq00bf88b.xn--5nqv22n.xn--lhr59c:user@domain.com>"
    );
    let list: AddressList = vec![routed.into()].into();
    assert_eq!(
        check(&list, &options()),
        "To: User Name <@route,@\u{4f0a}\u{662d}\u{5091}.\u{90f5}\u{4ef6}.\u{5546}\u{52d9}:user@domain.com>\r\n"
    );
}

#[test]
fn raw_names_ignore_encoding_charset_and_word_limit_without_changing_ascii_apis() {
    assert!(!AddressHeaderFormatOptions::default().international);
    let encoding = EncodeOptions {
        charset: Charset::ASCII,
        max_word_length: 1,
    };
    let options = AddressHeaderFormatOptions {
        name_encoding: encoding,
        ..options()
    };
    let value = MailboxAddress::new(
        "Ren\u{e9}e",
        "caf\u{e9}@b\u{fc}cher.example".parse().unwrap(),
    );
    let group = GroupAddress::new("Amis \u{1f600}", vec![value.clone().into()].into());
    let list: AddressList = vec![group.clone().into()].into();
    assert_eq!(
        check(&list, &options),
        "To: Amis \u{1f600}: Ren\u{e9}e <caf\u{e9}@b\u{fc}cher.example>;\r\n"
    );
    for result in [value.to_ascii(), group.to_ascii(), list.to_ascii()] {
        assert_eq!(
            result.unwrap_err().kind(),
            &AddressEncodeErrorKind::InternationalLocalPart
        );
    }
    assert!(list.to_header_string("To").is_err());
    assert!(list.to_ascii_with_options(&encoding).is_err());
}

#[test]
fn stored_domain_spelling_and_local_only_and_quoted_addresses_are_preserved() {
    for address in [
        "caf\u{e9}@B\u{dc}CHER.Example",
        "caf\u{e9}@XN--BCHER-KVA.Example",
        "\"caf\u{e9} avec\\\" espaces\"@example.com",
        "\"caf\u{e9}\tavec\"@[IPv6:::1]",
        "caf\u{e9}",
    ] {
        let list: AddressList = vec![mailbox("", address)].into();
        let output = check(&list, &options());
        assert!(output.contains(address));
        if !address.contains('@') {
            assert_eq!(output, format!("To: <{address}>\r\n"));
        }
    }
    let mut routed = MailboxAddress::new("", "caf\u{e9}@Example.com".parse().unwrap());
    *routed.route_mut() = ["B\u{dc}CHER.Example", "XN--BCHER-KVA.Example"]
        .into_iter()
        .collect();
    let list: AddressList = vec![routed.into()].into();
    assert!(check(&list, &options()).contains("@B\u{dc}CHER.Example,@XN--BCHER-KVA.Example:"));
}

#[test]
fn edited_routes_skip_empty_entries_and_normalize_literals_without_mutating_them() {
    let mut value = MailboxAddress::new("", "caf\u{e9}@example.com".parse().unwrap());
    *value.route_mut() = [" \r\n\t ", "[IPv6: ::1]", "", "B\u{dc}CHER.Example"]
        .into_iter()
        .collect();
    let list: AddressList = vec![value.into()].into();
    let before = list.to_string();
    let header = list.to_header_with_options("To", &options()).unwrap();
    assert_eq!(
        header.raw(),
        "To: <@[IPv6:::1],@B\u{dc}CHER.Example:caf\u{e9}@example.com>\r\n".as_bytes()
    );
    let parsed = header.parse_addresses().unwrap();
    assert_eq!(
        parsed.mailboxes().next().unwrap().route().as_slice(),
        ["[IPv6:::1]", "B\u{dc}CHER.Example"]
    );
    assert_eq!(list.to_string(), before);
}

#[test]
fn raw_phrase_quoting_preserves_specials_repeated_spaces_and_unicode_scalars() {
    for name in [
        "Ren\u{e9}e Smith",
        "Ren\u{e9}e  Smith",
        "Ren\u{e9}e, \"a friend\"",
        "A\u{161} (B) <C> @D; :E\\F. [G]",
        "\u{1f600} e\u{301} \u{1041}",
    ] {
        let list: AddressList = vec![
            GroupAddress::new(name, vec![mailbox(name, "caf\u{e9}@b")].into()).into(),
            GroupAddress::new(
                "",
                vec![GroupAddress::new("", AddressList::new()).into()].into(),
            )
            .into(),
        ]
        .into();
        for limit in 60..=90 {
            let options = AddressHeaderFormatOptions {
                max_line_length: limit,
                ..options()
            };
            let output = check(&list, &options);
            assert!(!output.contains("=?"), "{output:?}");
        }
    }
}

#[test]
fn many_raw_words_fold_only_at_existing_single_spaces() {
    let name = "Ren\u{e9}e \u{1f600} e\u{301} Smith ".repeat(40);
    let list: AddressList = vec![mailbox(name.trim(), "caf\u{e9}@b")].into();
    for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for max_line_length in [60, 61, 75, 78, 998] {
            let options = AddressHeaderFormatOptions {
                max_line_length,
                new_line_format,
                ..options()
            };
            assert!(!check(&list, &options).contains("=?"));
        }
    }
}

#[test]
fn oversized_or_unsafe_names_use_lossless_bounded_encoding() {
    for name in [
        "x".repeat(1200),
        "\u{1f600}e\u{301}".repeat(240),
        "Ren\u{e9}e, \"a friend\" ".repeat(100),
        "A  B \\ C ".repeat(150),
        "Ren\u{e9}e\r\nX-Injected: yes\0".to_owned(),
        "Literal =?utf-8?q?Ren=C3=A9e?= name".to_owned(),
    ] {
        let list: AddressList =
            vec![GroupAddress::new(&name, vec![mailbox(&name, "caf\u{e9}@b")].into()).into()]
                .into();
        for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            for max_line_length in [60, 61, 78, 998] {
                let options = AddressHeaderFormatOptions {
                    max_line_length,
                    new_line_format,
                    ..options()
                };
                let output = check(&list, &options);
                assert!(output.contains("=?utf-8?"));
                assert!(!output.contains("X-Injected:"));
                assert!(!output.contains('\0'));
            }
        }
    }
    for control in (0..=31).chain(std::iter::once(127)) {
        let name = format!("Ren\u{e9}e{}Smith", char::from(control));
        let list: AddressList = vec![mailbox(&name, "caf\u{e9}@b")].into();
        assert!(check(&list, &options()).contains("=?utf-8?"));
    }
}

#[test]
fn raw_name_boundaries_count_bytes_indentation_and_group_colons() {
    for max_line_length in [60, 61, 78, 998] {
        let options = AddressHeaderFormatOptions {
            max_line_length,
            ..options()
        };
        for extra in [0, 1] {
            let length = max_line_length - 2 + extra;
            let plain = format!("{}{}", "\u{e9}".repeat(length / 2), "x".repeat(length % 2));
            let quoted = format!("\u{e9}\"{}", "x".repeat(length - 6));
            let raw_quoted = format!("\"\u{e9}\\\"{}\"", "x".repeat(length - 6));
            for (name, raw) in [(&plain, &plain), (&quoted, &raw_quoted)] {
                assert_eq!(raw.len(), length);
                let list: AddressList =
                    vec![GroupAddress::new(name, AddressList::new()).into()].into();
                let output = check(&list, &options);
                if extra == 0 {
                    assert_eq!(output, format!("To:\r\n\t{raw}:\r\n\t;\r\n"));
                } else {
                    assert!(output.contains("=?utf-8?"));
                }
            }
        }
    }
}

#[test]
fn unicode_addr_specs_routes_and_attached_commas_are_indivisible_byte_counted_pieces() {
    for max_line_length in [60, 61, 78, 998] {
        let options = AddressHeaderFormatOptions {
            max_line_length,
            ..options()
        };
        let address = format!("\u{e9}{}@b", "a".repeat(max_line_length - 5));
        let list: AddressList = vec![mailbox("", &address)].into();
        assert_eq!(check(&list, &options), format!("To:\r\n\t{address}\r\n"));
        let list: AddressList = vec![mailbox("", &address), mailbox("", "a@b")].into();
        assert_eq!(
            list.to_header_string_with_options("To", &options)
                .unwrap_err(),
            AddressHeaderFormatError::LineTooLong {
                required: max_line_length + 1,
                limit: max_line_length,
            }
        );
    }
    let options = AddressHeaderFormatOptions {
        max_line_length: 60,
        ..options()
    };
    let address = format!("\"{}\"@b", "\u{e9} ".repeat(20));
    let mut routed = MailboxAddress::new("", "caf\u{e9}@b".parse().unwrap());
    *routed.route_mut() = ["B\u{dc}CHER.Example"; 4].into_iter().collect();
    for value in [mailbox("", &address), routed.into()] {
        let list: AddressList = vec![value].into();
        assert!(matches!(
            list.to_header_string_with_options("To", &options),
            Err(AddressHeaderFormatError::LineTooLong { .. })
        ));
    }
}

#[test]
fn international_output_still_validates_domain_mappings_routes_and_strict_syntax() {
    for domain in [
        "-bad.example",
        "bad\u{ff20}example.com",
        "bad\u{2009}example.com",
    ] {
        let list: AddressList = vec![mailbox("", &format!("caf\u{e9}@{domain}"))].into();
        let AddressHeaderFormatError::Address(error) = list
            .to_header_string_with_options("To", &options())
            .unwrap_err()
        else {
            panic!("expected a domain error");
        };
        assert!(matches!(error.kind(), AddressEncodeErrorKind::Domain(_)));
        assert_eq!(error.path(), &[0]);
        assert!(error.source().is_some());
    }
    for domain in [
        "bad domain",
        "evil\r\nBcc:other@example.com",
        "bad\0domain",
        "[broken",
        "[ok]junk",
        "bad\u{ff20}example.com",
        "bad\u{2009}example.com",
    ] {
        let mut value = MailboxAddress::new("Ren\u{e9}e", "caf\u{e9}@b".parse().unwrap());
        *value.route_mut() = ["", "relay.example", domain].into_iter().collect();
        let list: AddressList = vec![
            mailbox("", "a@b"),
            GroupAddress::new("Amis", vec![value.into()].into()).into(),
        ]
        .into();
        let before = list.to_string();
        let AddressHeaderFormatError::Address(error) = list
            .to_header_string_with_options("To", &options())
            .unwrap_err()
        else {
            panic!("expected a route error");
        };
        assert_eq!(error.path(), &[1, 0]);
        let AddressEncodeErrorKind::Route(source) = error.kind() else {
            panic!("{error}");
        };
        assert_eq!(source.domain_index(), 2);
        assert!(error.source().is_some());
        assert_eq!(list.to_string(), before);
    }
    for address in ["caf\u{e9}..local@b", "caf\u{e9}.@b"] {
        let address = AddrSpec::parse_with_options(
            address.as_bytes(),
            &AddrSpecParseOptions {
                compliance_mode: AddressComplianceMode::Looser,
                ..Default::default()
            },
        )
        .unwrap();
        let list: AddressList = vec![MailboxAddress::new("", address).into()].into();
        let AddressHeaderFormatError::Address(error) = list
            .to_header_string_with_options("To", &options())
            .unwrap_err()
        else {
            panic!("expected a syntax error");
        };
        assert!(matches!(error.kind(), AddressEncodeErrorKind::AddrSpec(_)));
    }
}

#[test]
fn fallback_honors_explicit_charset_and_word_limits_and_keeps_error_paths() {
    let name = "\u{1f600}".repeat(20);
    let list: AddressList = vec![
        mailbox("Good", "a@b"),
        GroupAddress::new("G", vec![mailbox(&name, "caf\u{e9}@b")].into()).into(),
    ]
    .into();
    for encoding in [
        EncodeOptions {
            charset: Charset::ASCII,
            ..Default::default()
        },
        EncodeOptions {
            max_word_length: 1,
            ..Default::default()
        },
    ] {
        let options = AddressHeaderFormatOptions {
            name_encoding: encoding,
            ..options()
        };
        let AddressHeaderFormatError::Address(error) = list
            .to_header_string_with_options("To", &options)
            .unwrap_err()
        else {
            panic!("expected a name error");
        };
        assert_eq!(error.path(), &[1, 0]);
        if encoding.charset == Charset::ASCII {
            assert!(matches!(
                error.kind(),
                AddressEncodeErrorKind::Name(EncodeError::Charset(_))
            ));
        } else {
            assert!(matches!(
                error.kind(),
                AddressEncodeErrorKind::Name(EncodeError::WordTooLong { .. })
            ));
        }
        assert!(error.source().is_some());
    }
    for max_word_length in [0, 76, usize::MAX] {
        let options = AddressHeaderFormatOptions {
            name_encoding: EncodeOptions {
                max_word_length,
                ..Default::default()
            },
            ..options()
        };
        for list in [
            AddressList::new(),
            vec![mailbox("Ren\u{e9}e", "a@b")].into(),
        ] {
            let AddressHeaderFormatError::Address(error) = list
                .to_header_string_with_options("To", &options)
                .unwrap_err()
            else {
                panic!("expected an option error");
            };
            assert!(error.path().is_empty());
            assert_eq!(
                error.kind(),
                &AddressEncodeErrorKind::Name(EncodeError::InvalidWordLength {
                    requested: max_word_length
                })
            );
        }
    }
}

#[test]
fn maximum_nesting_and_recovered_input_remain_unchanged() {
    let input = format!("{}caf\u{e9}@b{}", "\u{e9}:".repeat(255), ";".repeat(255));
    let parsing = AddressParseOptions {
        max_group_depth: 255,
        ..Default::default()
    };
    let list = AddressList::parse_with_options(input.as_bytes(), &parsing).unwrap();
    let options = AddressHeaderFormatOptions {
        max_line_length: 998,
        ..options()
    };
    check(&list, &options);

    let value = MailboxAddress::parse(b"Andr\xe9 <caf\xe9@b>").unwrap();
    let issues = value.display_name_issues().to_vec();
    let list: AddressList = vec![value.into()].into();
    assert_eq!(check(&list, &options), "To: Andr\u{e9} <caf\u{e9}@b>\r\n");
    let value = list.mailboxes().next().unwrap();
    assert_eq!(value.display_name_issues(), issues);
    assert!(value.address().had_latin1_fallback());
}

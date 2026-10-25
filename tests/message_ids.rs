// Includes cases adapted from MimeKit's UnitTests/Utils/MimeUtilsTests.cs,
// UnitTests/Utils/ParseUtilsTests.cs, and UnitTests/MimeMessageTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    HeaderId, HeaderParser, MessageId, MessageIdGenerationError, MessageIdParseErrorKind as Kind,
    MimeContent, MimeMessage, MimePart,
};

#[test]
fn generated_identifiers_normalize_domains_and_round_trip_through_headers() {
    let longest = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert_eq!(longest.len(), 253);
    for (domain, expected) in [
        ("Mj\u{f6}lnir", "xn--mjlnir-xxa"),
        ("EXAMPLE.COM", "example.com"),
        ("example.com.", "example.com"),
        ("localhost", "localhost"),
        ("b\u{fc}cher.example", "xn--bcher-kva.example"),
        ("example.com\u{ff0e}", "example.com"),
        (longest.as_str(), longest.as_str()),
    ] {
        let id = MessageId::generate(domain).unwrap();
        assert_eq!(id.as_str().split_once('@').unwrap().1, expected);
        assert_eq!(MessageId::parse(id.as_str().as_bytes()).unwrap(), id);
        assert_eq!(MessageId::from_value(id.as_str()).unwrap(), id);

        let mut message: MimeMessage = MimeMessage::new();
        message.set_message_id(id.as_str()).unwrap();
        let mut part = MimePart::new(MimeContent::new(std::io::Cursor::new(b"body".to_vec())));
        part.set_content_id(Some(id.as_str())).unwrap();
        message.set_body(Some(part.into()));
        let mut wire = Vec::new();
        message.write_to(&mut wire).unwrap();
        let loaded = MimeMessage::load(&mut std::io::Cursor::new(wire)).unwrap();
        assert_eq!(loaded.message_id().unwrap(), Some(id.clone()));
        assert_eq!(loaded.body().unwrap().content_id().unwrap(), Some(id));
    }
}

#[test]
fn generated_identifiers_reject_invalid_or_unsafe_domains() {
    for domain in [
        "",
        ".",
        "a..b",
        "-a.example",
        "a-.example",
        "a@b",
        "a b",
        "a\tb",
        "a\r\nBcc: victim@example.com",
        "a\0b",
        "[127.0.0.1]",
        "example\u{ff20}evil.com",
        "a\u{2009}b",
    ] {
        assert!(
            matches!(
                MessageId::generate(domain),
                Err(MessageIdGenerationError::InvalidDomain(_))
            ),
            "{domain:?}"
        );
    }
    assert!(MessageId::generate(&"x".repeat(64)).is_err());
    assert!(MessageId::generate(&format!("{}.com", "x.".repeat(126))).is_err());
}

#[test]
fn generated_identifiers_have_expected_shape_across_concurrent_calls() {
    let ids = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    (0..128)
                        .map(|_| MessageId::generate("example.com").unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        let (local, domain) = id.as_str().split_once('@').unwrap();
        assert_eq!(domain, "example.com");
        let components: Vec<_> = local.split('.').collect();
        assert_eq!(components.len(), 2);
        for component in components {
            assert!((1..=13).contains(&component.len()));
            assert!(
                component
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte.is_ascii_uppercase())
            );
        }
        assert!(seen.insert(id));
    }
    assert_eq!(seen.len(), 512);
}

#[test]
fn source_message_identifier_vectors() {
    let cases = [
        ("<local-part@domain1@domain2>", "local-part@domain1@domain2"),
        ("<local-part@>", "local-part@"),
        ("<local-part>", "local-part"),
        (
            "<:invalid-local-part;@domain.com>",
            ":invalid-local-part;@domain.com",
        ),
        (
            " <Messe_Bauma_rz(1)_ae284449-6bdc-488f-8ec3-5be5e5b09efb.jpg>",
            "Messe_Bauma_rz(1)_ae284449-6bdc-488f-8ec3-5be5e5b09efb.jpg",
        ),
        (
            " Messe_Bauma_rz(1)_ae284449-6bdc-488f-8ec3-5be5e5b09efb.jpg",
            "Messe_Bauma_rz(1)_ae284449-6bdc-488f-8ec3-5be5e5b09efb.jpg",
        ),
        (
            " <15627601.388658.1676916781911.JavaMail.\"xxxxxx@united.com\"@xxxxxxx.ual.com>",
            "15627601.388658.1676916781911.JavaMail.\"xxxxxx@united.com\"@xxxxxxx.ual.com",
        ),
        (" @0@", "@0@"),
        ("<(paren@example.com>", "(paren@example.com"),
        ("<a.@example.com.>", "a.@example.com"),
        ("<a b@example.com>", "ab@example.com"),
    ];
    for (input, expected) in cases {
        let id = MessageId::parse(input.as_bytes()).unwrap();
        assert_eq!(id.as_str(), expected, "{input:?}");
        assert_eq!(input.parse::<MessageId>(), Ok(id.clone()));
        assert_eq!(id.to_string(), expected);
        assert_eq!(id.into_string(), expected);
        let (id, consumed) = MessageId::parse_prefix(input.as_bytes()).unwrap();
        assert_eq!(id.as_str(), expected);
        assert_eq!(consumed, input.len());
    }
}

#[test]
fn source_broken_identifiers_are_errors() {
    for input in [
        " (this is an unterminated comment...",
        "(this is just a comment)",
        "<",
        "<local-part",
        "<local-part;",
        "<local-part@ (unterminated comment...",
        "<local-part@",
        "<local-part @ bad-domain (comment) . (comment com",
        "<local-part@[127.0",
        " <local-part.",
        " <local-part@domain",
        " <\"quoted-string@domain>",
        " <id@domain@>",
        "<>",
        " \t\r\n",
    ] {
        assert!(MessageId::parse(input.as_bytes()).is_err(), "{input:?}");
        assert!(
            MessageId::parse_prefix(input.as_bytes()).is_err(),
            "{input:?}"
        );
    }
}

#[test]
fn repeated_at_signs_and_multiple_domains() {
    for (input, expected) in [
        ("<a@@example.com>", "a@example.com"),
        ("<a@@@@example.com>", "a@example.com"),
        ("<a@@>", "a@"),
        ("<@>", "@"),
        ("<a@b@c>", "a@b@c"),
        ("<a@b.@c.>", "a@b@c"),
        ("<a@[127.0.0.1]@domain>", "a@[127.0.0.1]@domain"),
        ("a@b@", "a@b@"),
        ("local@@", "local@"),
    ] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap().as_str(),
            expected
        );
    }
    for input in ["<a@b@>", "<a@b@@c>", "<a@b@ c>", "<a@b (comment) @c>"] {
        assert!(MessageId::parse(input.as_bytes()).is_err(), "{input:?}");
    }
}

#[test]
fn exchange_square_brackets_are_preserved() {
    let token = "[d7e8bc604f797c18ba8120250cbd8c04-JFBVALKQOJXWILKCJQZFA7CDNRQXE2LUPF6EIYLUMFGG643TPRCXQ32TNV2HA===@microsoft.com]";
    for input in [token.to_owned(), format!("<{token}>")] {
        assert_eq!(MessageId::parse(input.as_bytes()).unwrap().as_str(), token);
    }
    for (input, expected) in [
        ("<[a@@b]>", "[a@b]"),
        ("<[a@b (comment)]>", "[a@b]"),
        ("<[a@[127.0.0.1]]>", "[a@[127.0.0.1]]"),
    ] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap().as_str(),
            expected
        );
    }
    // Only the Exchange workaround consumes the extra closing square bracket.
    assert!(MessageId::parse(b"<a@b]>").is_err());
    assert!(MessageId::parse(b"<[a@b] >").is_err());
}

#[test]
fn quoted_local_parts_keep_quotes_and_escapes_but_unfold() {
    for (input, expected) in [
        ("<\"x\r\n y\"@b>", "\"x y\"@b"),
        ("<\"x\n\ty\rz\"@b>", "\"x\tyz\"@b"),
        ("<\"a\\\"b\\\\c\"@d>", "\"a\\\"b\\\\c\"@d"),
        (
            "<\"a@b.c > (d)\" . \"e f\" @g>",
            "\"a@b.c > (d)\".\"e f\"@g",
        ),
        ("<\"\"@b>", "\"\"@b"),
    ] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap().as_str(),
            expected
        );
    }
}

#[test]
fn quoted_pair_backslash_parity_and_trailing_escape() {
    for count in 0..10 {
        let local = format!("\"a{}\"", "\\".repeat(count));
        let input = format!("<{local}@b>");
        if count % 2 == 0 {
            assert_eq!(
                MessageId::parse(input.as_bytes()).unwrap().as_str(),
                format!("{local}@b")
            );
        } else {
            let error = MessageId::parse(input.as_bytes()).unwrap_err();
            assert_eq!(error.kind(), Kind::IncompleteQuotedString);
            assert_eq!(error.offset(), 1);
        }
    }
    assert_eq!(
        MessageId::parse(b"<\"a\\").unwrap_err().kind(),
        Kind::IncompleteQuotedString
    );
}

#[test]
fn comments_are_removed_only_where_the_grammar_allows_them() {
    let input = b"(outer (inner\\) still inner)) <a@(domain) amazon (x).(y) com (z)> (tail)";
    assert_eq!(MessageId::parse(input).unwrap().as_str(), "a@amazon.com");
    assert_eq!(
        MessageId::parse(b"<a(comment)@b>").unwrap().as_str(),
        "a(comment)@b"
    );
    assert_eq!(
        MessageId::parse(b"(\xff) <a@b> (\xfe)").unwrap().as_str(),
        "a@b"
    );
    // Local-part comments are not CFWS, including an opening parenthesis
    // immediately after '<'. Before the token, it instead starts a comment.
    assert_eq!(
        MessageId::parse(b"<(paren@example.com>").unwrap().as_str(),
        "(paren@example.com"
    );
    assert_eq!(
        MessageId::parse(b"(paren@example.com").unwrap_err().kind(),
        Kind::IncompleteComment
    );
}

#[test]
fn domain_literals_strip_whitespace_without_ip_validation() {
    for (input, expected) in [
        ("<a@[127.0.0.1]>", "a@[127.0.0.1]"),
        ("<a@[ \tIPv6:2001:db8::1 \r\n]>", "a@[IPv6:2001:db8::1]"),
        ("<a@[1 2\t3\r\n4]>", "a@[1234]"),
        ("<a@[]>", "a@[]"),
        ("<a@[not-an-ip]>", "a@[not-an-ip]"),
        ("<a@[(literal)]>", "a@[(literal)]"),
    ] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap().as_str(),
            expected
        );
    }
    for input in [
        "<a@[127\\.0.0.1]>",
        "<a@[[127.0.0.1]]>",
        "<a@[127.0.0.1",
        "<a@[127.0.0.1] .com>",
    ] {
        assert!(MessageId::parse(input.as_bytes()).is_err(), "{input:?}");
    }
}

#[test]
fn domain_atom_and_literal_byte_classes_match_mimekit() {
    let atom_ascii =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!#$%&'*+-/=?^_`{|}~";
    for byte in 0..=u8::MAX {
        let input = [b'<', b'a', b'@', byte, b'b', b'>'];
        // A single non-ASCII byte cannot form a valid UTF-8 atom; whitespace
        // and repeated initial '@' are handled before the domain parser.
        let valid = atom_ascii.contains(&byte) || b" \t\r\n@".contains(&byte);
        assert_eq!(MessageId::parse(&input).is_ok(), valid, "atom {byte:#x}");

        let input = [b'<', b'a', b'@', b'[', b'x', byte, b'y', b']', b'>'];
        let valid =
            ((33..=126).contains(&byte) && !b"[\\]".contains(&byte)) || b" \t\r\n".contains(&byte);
        assert_eq!(MessageId::parse(&input).is_ok(), valid, "literal {byte:#x}");
    }
}

#[test]
fn utf8_is_preserved_without_case_folding_or_idn_conversion() {
    for token in [
        "\u{e6}\u{f8}\u{e5}@example.com",
        "a@\u{6d4b}\u{8bd5}\u{6587}\u{672c}.cn",
        "id@xn--v8jxj3d1dzdz08w.com",
        "LOCAL@EXAMPLE.COM",
        "\"\u{e9} \u{1f600}\"@example.com",
        "a\u{a0}b@example.com",
    ] {
        let input = format!("<{token}>");
        assert_eq!(MessageId::parse(input.as_bytes()).unwrap().as_str(), token);
    }
    assert_ne!(
        MessageId::parse(b"<LOCAL@EXAMPLE.COM>").unwrap(),
        MessageId::parse(b"<local@example.com>").unwrap()
    );
}

#[test]
fn invalid_utf8_is_never_silently_replaced() {
    let cases: &[(&[u8], usize)] = &[
        (b" <x\xff@b>", 3),
        (b"<a@\xffb>", 3),
        (b"<\"x\xff\"@b>", 3),
        (b"<a@ok.\xff>", 6),
        (b"<\xc3 @b>", 1),
        (b"<a@\xc3.b>", 3),
        (b"<a@\xc0\xaf>", 3),
    ];
    for &(input, offset) in cases {
        let error = MessageId::parse(input).unwrap_err();
        assert_eq!(error.kind(), Kind::InvalidUtf8, "{input:?}");
        assert_eq!(error.offset(), offset);
    }
    assert_eq!(
        MessageId::parse("<a@[\u{e9}]>".as_bytes())
            .unwrap_err()
            .kind(),
        Kind::InvalidDomain
    );
}

#[test]
fn prefix_consumption_and_complete_parsing_are_distinct() {
    for (input, expected, remainder) in [
        (" (lead) <a@b> <c@d>", "a@b", " <c@d>"),
        ("<a@b> (tail)", "a@b", " (tail)"),
        ("a@b,tail", "a@b", ",tail"),
        ("a@b (comment) next", "a@b", "next"),
        ("a@b>tail", "a@b", "tail"),
        ("<local> (unterminated", "local", " (unterminated"),
    ] {
        let (id, consumed) = MessageId::parse_prefix(input.as_bytes()).unwrap();
        assert_eq!(id.as_str(), expected);
        assert_eq!(&input[consumed..], remainder);
        assert_eq!(MessageId::parse(&input.as_bytes()[..consumed]), Ok(id));
    }
    for input in ["<a@b> <c@d>", "<a@b>>", "a@b garbage", "a@b,tail"] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap_err().kind(),
            Kind::TrailingInput,
            "{input:?}"
        );
    }
    // The prefix API stops before an invalid suffix; complete parsing sees it.
    assert_eq!(MessageId::parse_prefix(b"<a@b>\xff").unwrap().1, 5);
    assert_eq!(
        MessageId::parse(b"<a@b>\xff").unwrap_err().kind(),
        Kind::TrailingInput
    );
}

#[test]
fn complete_parsing_is_not_setter_validation() {
    for (input, expected) in [
        ("a b@c", "ab@c"),
        ("a@b>", "a@b"),
        ("a@b (comment)", "a@b"),
        ("<a\0b@c>", "a\0b@c"),
        ("a\r\nb@c", "ab@c"),
        ("<a\u{b}b@c>", "a\u{b}b@c"),
    ] {
        assert_eq!(
            MessageId::parse(input.as_bytes()).unwrap().as_str(),
            expected
        );
    }
    assert_eq!(
        MessageId::parse(b"garbage <a@example.com>")
            .unwrap()
            .as_str(),
        "garbage<a@example.com"
    );
    assert_eq!(
        MessageId::parse(b"local.").unwrap_err().kind(),
        Kind::IncompleteId
    );
    assert_eq!(MessageId::parse(b"<local.>").unwrap().as_str(), "local.");
}

#[test]
fn errors_expose_kinds_and_byte_offsets() {
    let cases = [
        ("", Kind::MissingId, 0),
        (" (ok) ", Kind::MissingId, 6),
        ("<abc", Kind::IncompleteId, 4),
        (" < >", Kind::EmptyId, 1),
        ("(abc", Kind::IncompleteComment, 0),
        ("<\"abc", Kind::IncompleteQuotedString, 1),
        ("<a@[127", Kind::IncompleteDomainLiteral, 3),
        ("<a@.b>", Kind::InvalidDomain, 3),
        ("<a@b,>", Kind::IncompleteId, 4),
        ("<a@b> junk", Kind::TrailingInput, 6),
        ("<a@b> (bad", Kind::IncompleteComment, 6),
        ("(\u{e9}) <a@.b>", Kind::InvalidDomain, 8),
    ];
    for (input, kind, offset) in cases {
        let error = MessageId::parse(input.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        assert!(error.to_string().ends_with(&format!("at byte {offset}")));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn deeply_nested_comments_are_not_recursive() {
    let input = format!("{}x{} <a@b>", "(".repeat(20_000), ")".repeat(20_000));
    assert_eq!(MessageId::parse(input.as_bytes()).unwrap().as_str(), "a@b");
}

#[test]
fn raw_header_values_parse_without_modifying_the_records() {
    let input =
        b"Message-Id: <\"x\r\n y\"@@example.com>\r\nContent-Id: <[image@example.com]>\n\nbody";
    for chunk_size in 1..=input.len() {
        let mut parser = HeaderParser::new();
        let mut headers = Vec::new();
        let mut consumed = 0;
        for chunk in input.chunks(chunk_size) {
            consumed += parser.parse(chunk, &mut headers).unwrap();
            if parser.is_finished() {
                break;
            }
        }
        assert_eq!(&input[consumed..], b"body");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].id(), HeaderId::MessageId);
        assert_eq!(headers[1].id(), HeaderId::ContentId);
        for (header, expected) in headers
            .iter()
            .zip(["\"x y\"@example.com", "[image@example.com]"])
        {
            let original = header.raw().to_vec();
            assert_eq!(
                MessageId::parse(header.raw_value()).unwrap().as_str(),
                expected
            );
            assert_eq!(header.raw(), original);
        }
    }
}

fn check_prefix_invariants(input: &[u8]) {
    match MessageId::parse_prefix(input) {
        Ok((id, consumed)) => {
            assert!((1..=input.len()).contains(&consumed), "{input:?}");
            assert!(!id.as_str().is_empty());
            assert!(id.as_str().len() <= consumed);
            assert_eq!(MessageId::parse(&input[..consumed]), Ok(id), "{input:?}");
        }
        Err(error) => assert!(error.offset() <= input.len(), "{input:?}"),
    }
    if let Err(error) = MessageId::parse(input) {
        assert!(error.offset() <= input.len(), "{input:?}");
    }
}

#[test]
fn arbitrary_bytes_and_truncated_tokens_preserve_bounds_and_progress() {
    for input in [
        &b"(lead) <\"a\\\"b\".c@(d)example.(e)com> (tail)"[..],
        b"<[a@[IPv6:2001:db8::1]]>",
        "<\u{e9}@\u{6d4b}\u{8bd5}.cn>".as_bytes(),
    ] {
        for end in 0..=input.len() {
            check_prefix_invariants(&input[..end]);
        }
    }
    let alphabet = b"ab01.@<>[]()\"\\ \t\r\n";
    let mut state = 0x1234_5678u32;
    for sample in 0..20_000 {
        let mut input = Vec::new();
        for _ in 0..sample % 64 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let byte = (state >> 24) as u8;
            input.push(if sample % 2 == 0 {
                alphabet[usize::from(byte) % alphabet.len()]
            } else {
                byte
            });
        }
        check_prefix_invariants(&input);
    }
}

#[test]
fn generated_valid_identifiers_normalize_and_round_trip() {
    for length in 1..=128 {
        let local = "x".repeat(length);
        let expected = format!("{local}@example.test");
        let input = format!(" (lead) < {local} @@ (domain) example . test . > (tail)");
        let parsed = MessageId::parse(input.as_bytes()).unwrap();
        assert_eq!(parsed.as_str(), expected);
        assert_eq!(expected.parse::<MessageId>(), Ok(parsed));
    }
}

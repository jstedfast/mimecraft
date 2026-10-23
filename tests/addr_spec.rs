// Adapted from MimeKit's UnitTests/MailboxAddressTests.cs,
// InternetAddressTests.cs, and InternetAddressListTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use mimecraft::{
    AddrSpec, AddrSpecParseErrorKind as Kind, AddrSpecParseOptions, AddressComplianceMode as Mode,
};

fn options(mode: Mode) -> AddrSpecParseOptions {
    AddrSpecParseOptions {
        compliance_mode: mode,
        ..Default::default()
    }
}

#[test]
fn escaped_quote_termination_depends_on_backslash_parity() {
    for count in 0..10 {
        let input = format!("\"a{}\"@x", "\\".repeat(count));
        let result = input.parse::<AddrSpec>();
        if count % 2 == 0 {
            assert_eq!(result.unwrap().as_str(), input);
        } else {
            assert_eq!(result.unwrap_err().kind(), Kind::IncompleteQuotedString);
        }
    }
}

#[test]
fn strict_mode_is_not_a_full_smtp_authoring_validator() {
    let local = "a".repeat(100);
    let domain = "b".repeat(100);
    let input = format!("{local}@{domain}");
    let address = AddrSpec::parse_with_options(input.as_bytes(), &options(Mode::Strict)).unwrap();
    assert_eq!(address.as_str(), input);
    assert!(address.with_ascii_domain().is_err());
    let obsolete = "a.\"b c\".d@example";
    assert_eq!(
        AddrSpec::parse_with_options(obsolete.as_bytes(), &options(Mode::Strict))
            .unwrap()
            .as_str(),
        obsolete
    );
}

#[test]
fn every_prefix_of_multibyte_and_quoted_addresses_is_safe_to_parse() {
    for text in [
        "\"\u{e9}\\\"x\"@b\u{fc}cher.example",
        "a (nested (comment)).b@[IPv6:::1]",
    ] {
        for end in 0..=text.len() {
            for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
                let result =
                    AddrSpec::parse_prefix_with_options(&text.as_bytes()[..end], &options(mode));
                match result {
                    Ok((address, consumed)) => {
                        assert!(consumed > 0 && consumed <= end);
                        assert!(!address.as_str().is_empty());
                    }
                    Err(error) => assert!(error.offset() <= end),
                }
            }
        }
    }
}

#[test]
fn basic_addresses_preserve_spelling_and_components() {
    for value in [
        "jeff@xamarin.com",
        "User.Name@EXAMPLE.COM",
        "jeff",
        "\"\"@example.com",
        "test.t\u{e4}st@test.net",
        "\u{200b}test@test.co.uk",
        "\"x@y\"@example.com",
        "user@xn--bcher-kva.example",
        "user@b\u{fc}cher.example",
    ] {
        let parsed: AddrSpec = value.parse().unwrap();
        assert_eq!(parsed.as_str(), value);
        assert_eq!(parsed.to_string(), value);
        assert_eq!(parsed.clone().as_str(), value);
        assert_eq!(parsed.is_international(), !value.is_ascii());
        assert!(!parsed.had_latin1_fallback());
        let rebuilt = match parsed.domain() {
            Some(domain) => format!("{}@{domain}", parsed.local_part()),
            None => parsed.local_part().to_owned(),
        };
        assert_eq!(rebuilt, value);
    }
    let quoted: AddrSpec = "\"x@y\"@example.com".parse().unwrap();
    assert_eq!(quoted.local_part(), "\"x@y\"");
    assert_eq!(quoted.domain(), Some("example.com"));
}

#[test]
fn comments_and_folding_are_removed_outside_quotes() {
    let parsed = AddrSpec::parse(
        b" (lead (nested)) user (local) . \r\n name (note) @ (host) \
          Example (label) . COM (tail)\r\n",
    )
    .unwrap();
    assert_eq!(parsed.as_str(), "user.name@Example.COM");
    assert_eq!(parsed.local_part(), "user.name");
    assert_eq!(parsed.domain(), Some("Example.COM"));
    let quoted: AddrSpec = "\" a \t b \" (x) . (y) \"c\\\"d\" @ example"
        .parse()
        .unwrap();
    assert_eq!(quoted.as_str(), "\" a \t b \".\"c\\\"d\"@example");
}

#[test]
fn canonical_quoted_local_parts_preserve_all_escaping() {
    let text = "\"\t !\\\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\\\]^_`abcdefghijklmnopqrstuvwxyz{|}~\"@domain.com";
    assert_eq!(text.parse::<AddrSpec>().unwrap().as_str(), text);
    let text = "\"\u{540d}\u{304c}\u{30c9}\u{30e1}\u{30a4}\u{30f3}\"@domain.com";
    assert_eq!(text.parse::<AddrSpec>().unwrap().as_str(), text);
    let text = "\"escaped\\@local\"@example.com";
    let parsed = AddrSpec::parse_with_options(text.as_bytes(), &options(Mode::Looser)).unwrap();
    assert_eq!(parsed.as_str(), text);
}

#[test]
fn every_ascii_quoted_character_and_pair_is_checked() {
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        for byte in 0..128u8 {
            let plain = [b'"', byte, b'"', b'@', b'x'];
            let valid_plain =
                (byte == b'\t' || (32..127).contains(&byte)) && !matches!(byte, b'"' | b'\\');
            let parsed = AddrSpec::parse_with_options(&plain, &options(mode));
            assert_eq!(parsed.is_ok(), valid_plain, "{mode:?}: quoted {byte}");
            if let Ok(parsed) = parsed {
                assert_eq!(parsed.as_str().as_bytes(), plain);
            }
            let escaped = [b'"', b'\\', byte, b'"', b'@', b'x'];
            let valid_pair = byte == b'\t' || (32..127).contains(&byte);
            let parsed = AddrSpec::parse_with_options(&escaped, &options(mode));
            assert_eq!(parsed.is_ok(), valid_pair, "{mode:?}: escaped {byte}");
            if let Ok(parsed) = parsed {
                assert_eq!(parsed.as_str().as_bytes(), escaped);
            }
        }
    }
}

#[test]
fn every_ascii_atom_character_obeys_the_local_part_grammar() {
    for byte in 0..128u8 {
        let input = [b'a', byte, b'b', b'@', b'x'];
        let expected = byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~.".contains(&byte);
        assert_eq!(AddrSpec::parse(&input).is_ok(), expected, "byte {byte}");
    }
}

#[test]
fn loose_modes_report_whole_word_latin1_recovery() {
    for mode in [Mode::Loose, Mode::Looser] {
        for (input, expected) in [
            (
                b"\xe6\xf8\xe5@example.com".as_slice(),
                "\u{e6}\u{f8}\u{e5}@example.com",
            ),
            (
                b"caf\xc3\xa9\xff@example.com",
                "caf\u{c3}\u{a9}\u{ff}@example.com",
            ),
            (
                b"\"\xe9\".utf\xc3\xa9@example.com",
                "\"\u{e9}\".utf\u{e9}@example.com",
            ),
        ] {
            let parsed = AddrSpec::parse_with_options(input, &options(mode)).unwrap();
            assert_eq!(parsed.as_str(), expected);
            assert_eq!(parsed.domain(), Some("example.com"));
            assert!(parsed.had_latin1_fallback());
            assert!(parsed.is_international());
            assert_eq!(parsed.with_ascii_domain().unwrap(), expected);
            assert!(parsed.had_latin1_fallback());
        }
    }
}

#[test]
fn strict_utf8_errors_and_domain_utf8_errors_report_original_offsets() {
    let input = b" (c) caf\xc3\xa9\xff@example.com";
    let error = AddrSpec::parse_with_options(input, &options(Mode::Strict)).unwrap_err();
    assert_eq!(error.kind(), Kind::InvalidUtf8);
    assert_eq!(error.offset(), 10);
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let error = AddrSpec::parse_with_options(b"user@caf\xe9.com", &options(mode)).unwrap_err();
        assert_eq!(error.kind(), Kind::InvalidUtf8);
        assert_eq!(error.offset(), 8);
    }
}

#[test]
fn valid_utf8_and_literal_replacement_characters_do_not_report_recovery() {
    for text in [
        "\u{e9}@example",
        "\"\u{fffd}\"@example",
        "\u{10428}@example",
    ] {
        for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
            let parsed = AddrSpec::parse_with_options(text.as_bytes(), &options(mode)).unwrap();
            assert_eq!(parsed.as_str(), text);
            assert!(!parsed.had_latin1_fallback());
            assert_eq!(parsed.domain(), Some("example"));
        }
    }
}

#[test]
fn looser_mode_preserves_repeated_and_trailing_local_dots() {
    for input in [
        "test.@gmail.com",
        "test..@gmail.com",
        "test..test@gmail.com",
    ] {
        assert_eq!(
            AddrSpec::parse_with_options(input.as_bytes(), &options(Mode::Looser))
                .unwrap()
                .as_str(),
            input
        );
        for mode in [Mode::Strict, Mode::Loose] {
            let error = AddrSpec::parse_with_options(input.as_bytes(), &options(mode)).unwrap_err();
            assert_eq!(error.kind(), Kind::InvalidLocalPart);
            assert_eq!(error.offset(), 5);
        }
    }
    let parsed = AddrSpec::parse_with_options(b"a . (x) . @ x", &options(Mode::Looser)).unwrap();
    assert_eq!(parsed.as_str(), "a..@x");
    for input in ["a.", "a..", ".a@x", ".@x"] {
        assert!(AddrSpec::parse_with_options(input.as_bytes(), &options(Mode::Looser)).is_err());
    }
}

#[test]
fn escaped_at_atoms_follow_the_canonical_looser_repair() {
    for (input, expected, local) in [
        (
            "webmaster\\@custom-domain.com@mail-host.com",
            "webmaster%40custom-domain.com@mail-host.com",
            "webmaster%40custom-domain.com",
        ),
        (
            "escaped\\@local@example.com",
            "escaped%40local@example.com",
            "escaped%40local",
        ),
        ("a\\@b\\@c@x", "a%40b%40c@x", "a%40b%40c"),
        ("a\\@@x", "a%40@x", "a%40"),
        ("a\\@", "a%40", "a%40"),
    ] {
        let parsed =
            AddrSpec::parse_with_options(input.as_bytes(), &options(Mode::Looser)).unwrap();
        assert_eq!(parsed.as_str(), expected);
        assert_eq!(parsed.local_part(), local);
        for mode in [Mode::Loose, Mode::Strict] {
            assert!(AddrSpec::parse_with_options(input.as_bytes(), &options(mode)).is_err());
        }
    }
    assert!(AddrSpec::parse_with_options(b"a\\@\\@b@x", &options(Mode::Looser)).is_err());
}

#[test]
fn local_only_acceptance_is_independent_of_compliance() {
    assert_eq!(AddrSpecParseOptions::default().compliance_mode, Mode::Loose);
    assert!(AddrSpecParseOptions::default().allow_local_only);
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        for input in ["jeff", "\"\"", "jeff (Display Name)"] {
            let parsed = AddrSpec::parse_with_options(input.as_bytes(), &options(mode)).unwrap();
            assert_eq!(parsed.domain(), None);
            let strict_domain = AddrSpecParseOptions {
                allow_local_only: false,
                ..options(mode)
            };
            let error = AddrSpec::parse_with_options(input.as_bytes(), &strict_domain).unwrap_err();
            assert_eq!(error.kind(), Kind::MissingDomain);
            assert_eq!(error.offset(), input.len());
            assert!(AddrSpec::parse_with_options(b"jeff@example", &strict_domain).is_ok());
        }
    }
}

#[test]
fn prefixes_stop_at_enclosing_delimiters_and_complete_parses_reject_them() {
    for address in ["user@example", "\"x@y\"@[::1]", "local", "a..@x"] {
        for suffix in [",next@x", ">tail", ";rest"] {
            let prefix = format!(" (c) {address} (tail)\r\n");
            let input = format!("{prefix}{suffix}");
            let (parsed, consumed) =
                AddrSpec::parse_prefix_with_options(input.as_bytes(), &options(Mode::Looser))
                    .unwrap();
            assert_eq!(parsed.as_str(), address);
            assert_eq!(consumed, prefix.len());
            assert_eq!(&input[consumed..], suffix);
            let error =
                AddrSpec::parse_with_options(input.as_bytes(), &options(Mode::Looser)).unwrap_err();
            assert_eq!(error.kind(), Kind::TrailingInput);
            assert_eq!(error.offset(), consumed);
        }
    }
    let (parsed, consumed) = AddrSpec::parse_prefix(b"a@b:unparsed").unwrap();
    assert_eq!(parsed.as_str(), "a@b");
    assert_eq!(consumed, 3);
    assert!(AddrSpec::parse_prefix(b"a:unparsed").is_err());
}

#[test]
fn fast_and_full_paths_agree_without_consuming_hidden_domain_labels() {
    for local in ["a", "A.B", "user+tag", "a!b_/?=c"] {
        for domain in ["x", "EXAMPLE.COM", "a_b.c"] {
            for suffix in ["", ",next@x", ">", ";"] {
                let plain = format!("{local}@{domain}{suffix}");
                let commented = format!(
                    "(lead){}(local)@(host){}(end){suffix}",
                    local.replace('.', "(dot).(word)"),
                    domain.replace('.', "(dot).(label)")
                );
                let (fast, consumed) = AddrSpec::parse_prefix(plain.as_bytes()).unwrap();
                let (slow, commented_consumed) =
                    AddrSpec::parse_prefix(commented.as_bytes()).unwrap();
                assert_eq!(fast.as_str(), slow.as_str());
                assert_eq!(fast.local_part(), slow.local_part());
                assert_eq!(fast.domain(), slow.domain());
                assert_eq!(&plain[consumed..], suffix);
                assert_eq!(&commented[commented_consumed..], suffix);
            }
        }
    }
    assert_eq!(
        AddrSpec::parse(b"user@example (hidden) . com")
            .unwrap()
            .as_str(),
        "user@example.com"
    );
}

#[test]
fn domains_reuse_literal_and_trailing_dot_compatibility() {
    for (input, expected) in [
        ("user@domain.", "user@domain"),
        ("user@domain. (tail)", "user@domain"),
        ("user@[ 127.0.0.1 ]", "user@[127.0.0.1]"),
        ("user@[IPv6:\r\n ::1]", "user@[IPv6:::1]"),
        ("user@[]", "user@[]"),
        ("user@[not (an ip)]", "user@[not(anip)]"),
    ] {
        assert_eq!(input.parse::<AddrSpec>().unwrap().as_str(), expected);
    }
    let (parsed, consumed) = AddrSpec::parse_prefix(b"user@domain.>tail").unwrap();
    assert_eq!(parsed.as_str(), "user@domain");
    assert_eq!(&b"user@domain.>tail"[consumed..], b">tail");
}

#[test]
fn syntax_failures_report_precise_original_byte_offsets() {
    let cases: &[(&[u8], Kind, usize)] = &[
        (b"", Kind::MissingLocalPart, 0),
        (b" (c) ", Kind::MissingLocalPart, 5),
        (b".x@y", Kind::InvalidLocalPart, 0),
        (b"a.", Kind::IncompleteLocalPart, 2),
        (b"a@ ", Kind::IncompleteAddress, 3),
        (b"a@ (note)", Kind::IncompleteAddress, 9),
        (b"a x", Kind::InvalidAddress, 2),
        (b"\"bad\nx\"@y", Kind::InvalidQuotedCharacter, 4),
        (b"\"bad\\\nx\"@y", Kind::InvalidQuotedCharacter, 5),
        (b"\"unterminated", Kind::IncompleteQuotedString, 0),
        (b" (c) \"ends\\", Kind::IncompleteQuotedString, 5),
        (b"x@[", Kind::IncompleteDomainLiteral, 2),
        (b"x@(oops", Kind::IncompleteComment, 2),
        (b"x@a (oops", Kind::IncompleteComment, 4),
        (b"x@a..b", Kind::InvalidDomain, 4),
        (b"x@a\xff", Kind::InvalidUtf8, 3),
        (b"a@x@y", Kind::TrailingInput, 3),
        (b"a,@b", Kind::TrailingInput, 1),
        (b" a@b (unterminated", Kind::IncompleteComment, 5),
    ];
    for &(input, kind, offset) in cases {
        let error = AddrSpec::parse(input).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), offset, "{input:?}");
        assert!(error.to_string().ends_with(&format!("at byte {offset}")));
        let source: &dyn std::error::Error = &error;
        assert!(source.source().is_none());
    }
}

#[test]
fn display_names_angles_routes_and_groups_are_not_address_specifications() {
    for input in [
        "Name <user@example>",
        "<user@example>",
        "@relay:user@example",
        "Friends:user@example;",
        "user@example>",
        "user@example,user@other",
    ] {
        assert!(input.parse::<AddrSpec>().is_err(), "{input:?}");
    }
}

#[test]
fn domain_conversion_preserves_local_parts_and_original_domain_spelling() {
    let address: AddrSpec = "\"\u{e9}@LOCAL\"@B\u{dc}CHER.example".parse().unwrap();
    let original = address.as_str().to_owned();
    assert_eq!(
        address.ascii_domain().unwrap().as_deref(),
        Some("xn--bcher-kva.example")
    );
    assert_eq!(
        address.unicode_domain().unwrap().as_deref(),
        Some("b\u{fc}cher.example")
    );
    assert_eq!(
        address.with_ascii_domain().unwrap(),
        "\"\u{e9}@LOCAL\"@xn--bcher-kva.example"
    );
    assert_eq!(
        address.with_unicode_domain().unwrap(),
        "\"\u{e9}@LOCAL\"@b\u{fc}cher.example"
    );
    assert_eq!(address.as_str(), original);
    let encoded: AddrSpec = "user@XN--BCHER-KVA.example".parse().unwrap();
    assert_eq!(encoded.domain(), Some("XN--BCHER-KVA.example"));
    assert_eq!(
        encoded.with_unicode_domain().unwrap(),
        "user@b\u{fc}cher.example"
    );
    assert!(!encoded.is_international());
}

#[test]
fn literal_ace_text_and_local_only_addresses_bypass_idna() {
    for input in [
        "user@[xn--bcher-kva]",
        "user@[IPv6:::1]",
        "user@[]",
        "\u{e9}",
        "\"x@y\"",
    ] {
        let address: AddrSpec = input.parse().unwrap();
        assert!(matches!(
            address.with_ascii_domain().unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            address.with_unicode_domain().unwrap(),
            Cow::Borrowed(_)
        ));
        assert_eq!(address.with_ascii_domain().unwrap(), input);
        assert_eq!(address.with_unicode_domain().unwrap(), input);
        assert_eq!(address.ascii_domain().unwrap().as_deref(), address.domain());
        assert_eq!(
            address.unicode_domain().unwrap().as_deref(),
            address.domain()
        );
    }
    let plain: AddrSpec = "User@example.com".parse().unwrap();
    assert!(matches!(
        plain.with_ascii_domain().unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn idna_failure_is_explicit_and_does_not_change_the_address() {
    for domain in ["xn--.example", "example\u{ff20}evil.com", "-bad.example"] {
        let input = format!("user@{domain}");
        let address: AddrSpec = input.parse().unwrap();
        assert!(address.ascii_domain().is_err());
        assert!(address.unicode_domain().is_err());
        assert!(address.with_ascii_domain().is_err());
        assert!(address.with_unicode_domain().is_err());
        assert_eq!(address.as_str(), input);
    }
}

#[test]
fn arbitrary_input_has_bounded_progress_and_stable_normalization() {
    let mut state = 0x793c_a170u32;
    for mode in [Mode::Strict, Mode::Loose, Mode::Looser] {
        let mut successes = 0;
        for length in 0..80 {
            for iteration in 0..24 {
                let mut input = Vec::new();
                for _ in 0..length {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    input.push(if iteration % 2 == 0 {
                        b'a' + (state % 26) as u8
                    } else {
                        state as u8
                    });
                }
                match AddrSpec::parse_prefix_with_options(&input, &options(mode)) {
                    Ok((address, consumed)) => {
                        successes += 1;
                        assert!((1..=input.len()).contains(&consumed));
                        assert!(
                            !address
                                .as_str()
                                .bytes()
                                .any(|byte| matches!(byte, 0 | b'\r' | b'\n'))
                        );
                        let reparsed = AddrSpec::parse_with_options(
                            address.as_str().as_bytes(),
                            &options(mode),
                        )
                        .unwrap();
                        assert_eq!(reparsed.as_str(), address.as_str());
                        assert_eq!(reparsed.local_part(), address.local_part());
                        assert_eq!(reparsed.domain(), address.domain());
                        assert!(!reparsed.had_latin1_fallback());
                        let complete = AddrSpec::parse_with_options(&input, &options(mode));
                        if consumed == input.len() {
                            assert_eq!(complete.unwrap().as_str(), address.as_str());
                        } else {
                            let error = complete.unwrap_err();
                            assert_eq!(error.kind(), Kind::TrailingInput);
                            assert_eq!(error.offset(), consumed);
                        }
                    }
                    Err(error) => assert!(error.offset() <= input.len()),
                }
            }
        }
        assert!(successes > 500);
    }
}

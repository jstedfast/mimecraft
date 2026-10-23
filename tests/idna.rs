// Adapted from MimeKit's UnitTests/Encodings/PunycodeTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use mimecraft::{
    DomainList, DomainListEncodeError,
    idna::{IdnaError, to_ascii, to_unicode},
};

const VECTORS: &[(&str, &str)] = &[
    ("abc.org", "abc.org"),
    ("my_company.com", "my_company.com"),
    ("b\u{fc}cher.com", "xn--bcher-kva.com"),
    (
        "\u{43c}\u{43e}\u{439}\u{434}\u{43e}\u{43c}\u{435}\u{43d}.\u{440}\u{444}",
        "xn--d1acklchcc.xn--p1ai",
    ),
    (
        "\u{3c0}\u{3b1}\u{3c1}\u{3ac}\u{3b4}\u{3b5}\u{3b9}\u{3b3}\u{3bc}\u{3b1}.\u{3b4}\u{3bf}\u{3ba}\u{3b9}\u{3bc}\u{3ae}",
        "xn--hxajbheg2az3al.xn--jxalpdlp",
    ),
];

#[test]
fn fullwidth_mappings_cannot_bypass_ascii_syntax_checks() {
    for byte in b'!'..=b'~' {
        let fullwidth = char::from_u32(0xfee0 + u32::from(byte)).unwrap();
        let input = format!("a{fullwidth}b.example");
        let mapped = format!("a{}b.example", char::from(byte));
        let expected = to_ascii(&mapped).map(Cow::into_owned);
        let encoded = to_ascii(&input).map(Cow::into_owned);
        assert_eq!(encoded.is_ok(), expected.is_ok(), "{input:?}");
        if let Ok(encoded) = encoded {
            assert_eq!(encoded, expected.unwrap());
            let route = format!("@{encoded}").parse::<DomainList>().unwrap();
            assert_eq!(route.len(), 1);
            assert_eq!(route.to_ascii().unwrap(), format!("@{encoded}"));
        }
    }
}

#[test]
fn supplementary_letters_round_trip_as_scalars() {
    let lowercase = "\u{10428}\u{10429}.example";
    let uppercase = "\u{10400}\u{10401}.example";
    let ascii = to_ascii(lowercase).unwrap();
    assert!(ascii.starts_with("xn--"));
    assert!(ascii.is_ascii());
    assert_eq!(to_ascii(uppercase).unwrap(), ascii);
    assert_eq!(to_unicode(&ascii).unwrap(), lowercase);
    assert_eq!(to_unicode(uppercase).unwrap(), lowercase);
}

#[test]
fn canonical_encoding_and_decoding_vectors() {
    for &(unicode, ascii) in VECTORS {
        assert_eq!(to_ascii(unicode).unwrap(), ascii);
        assert_eq!(to_unicode(ascii).unwrap(), unicode);
        assert_eq!(to_ascii(ascii).unwrap(), ascii);
        assert_eq!(to_unicode(unicode).unwrap(), unicode);
    }
    assert_eq!(to_ascii("mycharity\u{3002}org").unwrap(), "mycharity.org");
}

#[test]
fn slices_replace_index_and_count_overloads() {
    for &(unicode, ascii) in VECTORS {
        let mailbox = format!("user@{unicode}");
        assert_eq!(to_ascii(&mailbox[5..]).unwrap(), ascii);
        let wrapped = format!("(user@{unicode})");
        assert_eq!(to_ascii(&wrapped[6..wrapped.len() - 1]).unwrap(), ascii);
        let mailbox = format!("user@{ascii}");
        assert_eq!(to_unicode(&mailbox[5..]).unwrap(), unicode);
        let wrapped = format!("(user@{ascii})");
        assert_eq!(to_unicode(&wrapped[6..wrapped.len() - 1]).unwrap(), unicode);
    }
}

#[test]
fn canonical_invalid_domains_fail_instead_of_passing_through() {
    let overlong = format!("{}.example", "a".repeat(64));
    for domain in [
        overlong.as_str(),
        "xn--.example",
        "uccvm\u{2009}cop.edu",
        "example\u{ff20}evil.com",
        "ex\u{ff08}ample\u{ff09}.com",
    ] {
        assert!(to_ascii(domain).is_err(), "{domain:?}");
        assert!(to_unicode(domain).is_err(), "{domain:?}");
        let wrapped = format!("(user@{domain})");
        assert!(to_ascii(&wrapped[6..wrapped.len() - 1]).is_err());
        assert!(to_unicode(&wrapped[6..wrapped.len() - 1]).is_err());
    }
}

#[test]
fn normalization_case_and_nontransitional_processing() {
    for (input, ascii, unicode) in [
        ("EXAMPLE.COM", "example.com", "example.com"),
        ("B\u{dc}CHER.COM", "xn--bcher-kva.com", "b\u{fc}cher.com"),
        ("XN--BCHER-KVA.COM", "xn--bcher-kva.com", "b\u{fc}cher.com"),
        ("caf\u{e9}.com", "xn--caf-dma.com", "caf\u{e9}.com"),
        ("cafe\u{301}.com", "xn--caf-dma.com", "caf\u{e9}.com"),
        ("fa\u{df}.de", "xn--fa-hia.de", "fa\u{df}.de"),
        ("\u{ff21}\u{ff22}\u{ff23}.com", "abc.com", "abc.com"),
        ("exam\u{ad}ple.com", "example.com", "example.com"),
    ] {
        assert_eq!(to_ascii(input).unwrap(), ascii, "{input:?}");
        assert_eq!(to_unicode(input).unwrap(), unicode, "{input:?}");
    }
    for dot in ['\u{3002}', '\u{ff0e}', '\u{ff61}'] {
        assert_eq!(
            to_ascii(&format!("example{dot}com")).unwrap(),
            "example.com"
        );
    }
}

#[test]
fn root_dot_and_dns_length_boundaries() {
    let label = "a".repeat(63);
    assert_eq!(to_ascii(&label).unwrap(), label);
    assert!(to_ascii(&"a".repeat(64)).is_err());
    assert!(to_unicode(&"a".repeat(64)).is_err());
    let max = format!("{label}.{label}.{label}.{}", "a".repeat(61));
    assert_eq!(max.len(), 253);
    assert_eq!(to_ascii(&max).unwrap(), max);
    assert_eq!(to_unicode(&max).unwrap(), max);
    let rooted = format!("{max}.");
    assert_eq!(rooted.len(), 254);
    assert_eq!(to_ascii(&rooted).unwrap(), rooted);
    assert_eq!(to_unicode(&rooted).unwrap(), rooted);
    let overlong = format!("{max}a");
    assert_eq!(overlong.len(), 254);
    for invalid in [overlong.clone(), format!("{overlong}.")] {
        assert!(to_ascii(&invalid).is_err());
        assert!(to_unicode(&invalid).is_err());
    }
    assert_eq!(to_ascii("b\u{fc}cher.com.").unwrap(), "xn--bcher-kva.com.");
    assert_eq!(
        to_unicode("xn--bcher-kva.com.").unwrap(),
        "b\u{fc}cher.com."
    );
}

#[test]
fn dns_lengths_are_measured_after_ace_encoding() {
    let unicode = "\u{fc}".repeat(57);
    let ascii = to_ascii(&unicode).unwrap();
    assert_eq!(ascii.len(), 63);
    assert_eq!(to_unicode(&ascii).unwrap(), unicode);
    assert!(to_ascii(&"\u{fc}".repeat(58)).is_err());
    assert!(to_unicode(&"\u{fc}".repeat(58)).is_err());
}

#[test]
fn empty_labels_bad_hyphens_and_malformed_ace_are_rejected() {
    for domain in [
        "",
        ".",
        ".com",
        "a..com",
        "a.com..",
        "-a.com",
        "a-.com",
        "ab--cd.com",
        "xn--",
        "xn--.com",
        "xn--a.com",
        "xn--abc-.com",
        "\u{ad}.com",
    ] {
        assert_eq!(
            to_ascii(domain).unwrap_err(),
            IdnaError::InvalidDomain,
            "{domain:?}"
        );
        assert!(to_unicode(domain).is_err(), "{domain:?}");
    }
}

#[test]
fn bidi_joiners_combining_marks_and_disallowed_characters() {
    for domain in [
        "abc\u{5d0}.com",
        "a\u{200c}b.com",
        "a\u{200d}b.com",
        "\u{301}a.com",
        "a\u{fffd}b.com",
        "a\u{ffff}b.com",
    ] {
        assert!(to_ascii(domain).is_err(), "{domain:?}");
        assert!(to_unicode(domain).is_err(), "{domain:?}");
    }
    let hebrew = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}.com";
    let ascii = to_ascii(hebrew).unwrap();
    assert!(ascii.starts_with("xn--"));
    assert_eq!(to_unicode(&ascii).unwrap(), hebrew);
}

#[test]
fn every_ascii_character_obeys_mime_domain_syntax() {
    for byte in 0..128 {
        let domain = format!("a{}b.example", char::from(byte));
        let allowed = byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~.".contains(&byte);
        let encoded = to_ascii(&domain);
        assert_eq!(encoded.is_ok(), allowed, "ASCII byte {byte}");
        let decoded = to_unicode(&domain);
        assert_eq!(decoded.is_ok(), allowed, "ASCII byte {byte}");
        if allowed {
            assert_eq!(encoded.unwrap(), domain.to_ascii_lowercase());
            assert_eq!(decoded.unwrap(), domain.to_ascii_lowercase());
        }
    }
}

#[test]
fn domain_helpers_do_not_parse_mailboxes_urls_or_literals() {
    for input in [
        "user@example.com",
        "https://example.com",
        "[127.0.0.1]",
        "[IPv6:::1]",
        "example (comment).com",
        "example.com:25",
        "example.com\r\nBcc:someone",
    ] {
        assert!(to_ascii(input).is_err(), "{input:?}");
        assert!(to_unicode(input).is_err(), "{input:?}");
    }
}

#[test]
fn unchanged_ascii_is_borrowed_and_failures_have_diagnostics() {
    assert!(matches!(
        to_ascii("my_company.com").unwrap(),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        to_unicode("my_company.com").unwrap(),
        Cow::Borrowed(_)
    ));
    let unsafe_ascii = to_ascii("a@b.com").unwrap_err();
    assert_eq!(unsafe_ascii, IdnaError::UnsafeAscii);
    assert!(unsafe_ascii.to_string().contains("unsafe ASCII"));
    assert!(IdnaError::InvalidDomain.to_string().contains("invalid"));
    let error: &dyn std::error::Error = &unsafe_ascii;
    assert!(error.source().is_none());
}

#[test]
fn route_encoding_skips_blank_domains_and_encodes_idns() {
    let route: DomainList = ["  \t ", "b\u{fc}cher.example", "example.net", "\u{2008}"]
        .into_iter()
        .collect();
    let original = route.clone();
    assert_eq!(
        route.to_ascii().unwrap(),
        "@xn--bcher-kva.example,@example.net"
    );
    assert_eq!(route, original);
    assert_eq!(route.to_string(), "@b\u{fc}cher.example,@example.net");
    assert_eq!(DomainList::new().to_ascii().unwrap(), "");
    let blank: DomainList = ["", " \t\r\n", "\u{3000}"].into_iter().collect();
    assert_eq!(blank.to_ascii().unwrap(), "");
}

#[test]
fn route_encoding_preserves_order_duplicates_and_literal_spelling() {
    let route: DomainList = [
        "RELAY.Example",
        "[ IPv6: ::ABCD ]",
        "b\u{fc}cher.example",
        "RELAY.Example",
        "example.",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        route.to_ascii().unwrap(),
        "@relay.example,@[IPv6:::ABCD],@xn--bcher-kva.example,@relay.example,@example."
    );
    let literals: DomainList = ["[]", "[not an (ip)]"].into_iter().collect();
    assert_eq!(literals.to_ascii().unwrap(), "@[],@[notan(ip)]");
}

#[test]
fn route_errors_identify_original_indices_without_partial_output_or_mutation() {
    for invalid in [
        "xn--.com",
        "example\u{ff20}evil.com",
        "example.com,@evil.com",
        "a b",
    ] {
        let route: DomainList = ["", "good.example", "\u{2008}", invalid, "later.example"]
            .into_iter()
            .collect();
        let original = route.clone();
        let error = route.to_ascii().unwrap_err();
        assert!(matches!(
            error,
            DomainListEncodeError::Idna { index: 3, .. }
        ));
        assert_eq!(error.domain_index(), 3);
        assert!(error.to_string().contains("index 3"));
        let source: &dyn std::error::Error = &error;
        assert!(source.source().is_some());
        assert_eq!(route, original);
    }
}

#[test]
fn route_literals_require_a_complete_ascii_literal() {
    for invalid in [
        "[unclosed",
        "[a\\b]",
        "[a[b]",
        "[a]\r\nBcc:x",
        "[a] ",
        "[a],@b",
        "[\u{e9}]",
        "[\0]",
    ] {
        let route: DomainList = ["good", invalid].into_iter().collect();
        let error = route.to_ascii().unwrap_err();
        assert_eq!(error, DomainListEncodeError::InvalidLiteral { index: 1 });
        assert_eq!(error.domain_index(), 1);
        assert!(error.to_string().contains("index 1"));
        let source: &dyn std::error::Error = &error;
        assert!(source.source().is_none());
    }
}

#[test]
fn route_encoding_round_trips_through_the_existing_parser() {
    let source: DomainList = "@B\u{dc}CHER.example,,@my_company.com,@[IPv6: ::1]"
        .parse()
        .unwrap();
    let ascii = source.to_ascii().unwrap();
    assert!(ascii.is_ascii());
    let reparsed: DomainList = ascii.parse().unwrap();
    assert_eq!(reparsed.to_ascii().unwrap(), ascii);
    assert_eq!(reparsed.to_string(), ascii);
    assert_eq!(
        to_unicode(&reparsed.as_slice()[0]).unwrap(),
        "b\u{fc}cher.example"
    );
}

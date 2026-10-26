// Adapted from MimeKit's UnitTests/Utils/SmtpUtf8AddressValidatorTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use SmtpUtf8AddressError::*;
use mimecraft::{
    MailboxAddress, SmtpUtf8AddressError, SmtpUtf8AddressValidationOptions,
    utils::{validate_smtp_utf8_address, validate_smtp_utf8_address_with_options},
};

#[path = "data/smtp_utf8_cases.rs"]
mod fixtures;

const LENGTHS: SmtpUtf8AddressValidationOptions = SmtpUtf8AddressValidationOptions {
    decode_idn: false,
    ascii_is_latin: false,
    enforce_length_limits: true,
};
const DECODE: SmtpUtf8AddressValidationOptions = SmtpUtf8AddressValidationOptions {
    decode_idn: true,
    ascii_is_latin: false,
    enforce_length_limits: false,
};

#[test]
fn upstream_vectors_and_option_monotonicity() {
    assert_eq!(fixtures::CASES.len(), 277);
    for &(group, address, decode_idn, ascii_is_latin, expected) in fixtures::CASES {
        let options = SmtpUtf8AddressValidationOptions {
            decode_idn,
            ascii_is_latin,
            ..Default::default()
        };
        let result = validate_smtp_utf8_address_with_options(address, &options);
        let error = result.err().map(|error| format!("{error:?}"));
        assert_eq!(error.as_deref(), expected, "{group}: {address:?}");
        if options == SmtpUtf8AddressValidationOptions::default() {
            assert_eq!(
                validate_smtp_utf8_address(address),
                result,
                "{group}: {address:?}"
            );
        }
        // Counting ASCII as Latin never permits an address rejected otherwise.
        let strict = SmtpUtf8AddressValidationOptions {
            ascii_is_latin: true,
            ..options
        };
        if result.is_err() {
            assert_eq!(
                validate_smtp_utf8_address_with_options(address, &strict),
                result,
                "{group}: {address:?}"
            );
        }
    }
}

#[test]
fn error_precedence() {
    for (address, expected) in [
        ("xn--abc@example..com", Syntax),
        ("xn--abc\u{200e}@example.com", ALabel),
        (
            "I\u{412}M\u{200d}\u{3164}@d\u{f8}mi.fo",
            DisallowedCodePoint,
        ),
        ("cafe\u{301}\u{3164}@example.com", DisallowedCodePoint),
        ("cafe\u{301}\u{200d}@example.com", NotNormalized),
        ("I\u{412}Me\u{301}@d\u{f8}mi.fo", NotNormalized),
        ("I\u{412}M\u{200d}@d\u{f8}mi.fo", ContextRule),
    ] {
        assert_eq!(
            validate_smtp_utf8_address(address),
            Err(expected),
            "{address:?}"
        );
    }
    // All encoded labels must be valid before checking any decoded code point.
    assert_eq!(
        validate_smtp_utf8_address_with_options("user@xn--a.xn--99999999999", &DECODE),
        Err(ALabel)
    );
    assert_eq!(
        validate_smtp_utf8_address_with_options("user@xn--a", &DECODE),
        Err(DisallowedCodePoint)
    );
    assert_eq!(
        validate_smtp_utf8_address_with_options(
            &format!("user@xn--{}.example.com", "a".repeat(60)),
            &DECODE
        ),
        Err(ALabel)
    );
    // A quoted xn-- string is not a local-part atom or an encoded local part.
    assert_eq!(
        validate_smtp_utf8_address_with_options("\"xn--gr-zia\"@example.com", &DECODE),
        Ok(())
    );
}

#[test]
fn large_inputs_and_long_combining_runs() {
    // These inputs become quadratic if global context is rescanned per character.
    assert_eq!(
        validate_smtp_utf8_address(&format!("{}@example.com", "\u{30fb}".repeat(50_000))),
        Err(ContextRule)
    );
    let joining = format!(
        "\u{628}{}\u{200c}{}",
        "\u{64e}".repeat(10),
        "\u{64e}".repeat(10)
    );
    assert_eq!(
        validate_smtp_utf8_address(&format!("{}\u{628}@example.com", joining.repeat(10_000))),
        Ok(())
    );
    assert_eq!(
        validate_smtp_utf8_address(&format!(
            "\u{628}{}{}{}\u{628}@example.com",
            "\u{64e}".repeat(50_000),
            "\u{200c}".repeat(50_000),
            "\u{64e}".repeat(50_000)
        )),
        Err(ContextRule)
    );
    assert_eq!(
        validate_smtp_utf8_address(&format!(
            "{}c@{}example",
            "ab.".repeat(50_000),
            "\u{f8}.".repeat(50_000)
        )),
        Ok(())
    );
    // NFC checking must remain linear even with long runs of combining marks.
    assert_eq!(
        validate_smtp_utf8_address(&format!("a{}@example.com", "\u{301}".repeat(50_000))),
        Err(NotNormalized)
    );
    assert_eq!(
        validate_smtp_utf8_address(&format!("\u{e1}{}@example.com", "\u{301}".repeat(50_000))),
        Ok(())
    );
}

// Use <=62-byte labels to permit adding a byte without triggering a label error.
fn ascii_domain(mut length: usize) -> String {
    let mut out = String::new();
    while length > 0 {
        let n = if length <= 62 {
            length
        } else {
            62.min(length - 2)
        };
        out.push_str(&"a".repeat(n));
        length -= n;
        if length > 0 {
            out.push('.');
            length -= 1;
        }
    }
    out
}

#[test]
fn exact_local_part_utf8_limits_and_default_opt_out() {
    for (local, expected) in [
        ("a".repeat(64), Ok(())),
        ("a".repeat(65), Err(LocalPartTooLong)),
        ("\u{f8}".repeat(32), Ok(())),
        (format!("{}a", "\u{f8}".repeat(32)), Err(LocalPartTooLong)),
        (format!("{}a", "\u{436}".repeat(21)), Ok(())),
        (format!("{}a", "\u{4f8b}".repeat(21)), Ok(())),
        (
            format!("{}ab", "\u{4f8b}".repeat(21)),
            Err(LocalPartTooLong),
        ),
        ("\u{20000}".repeat(16), Ok(())),
        (
            format!("{}a", "\u{20000}".repeat(16)),
            Err(LocalPartTooLong),
        ),
        (format!("\"{}\"", "a".repeat(62)), Ok(())),
        (format!("\"{}\"", "a".repeat(63)), Err(LocalPartTooLong)),
        (format!("\"{}\"", "\\a".repeat(31)), Ok(())),
        (format!("\"{}a\"", "\\a".repeat(31)), Err(LocalPartTooLong)),
    ] {
        let address = format!("{local}@example.com");
        assert_eq!(validate_smtp_utf8_address(&address), Ok(()));
        assert_eq!(
            validate_smtp_utf8_address_with_options(&address, &LENGTHS),
            expected,
            "{address:?}"
        );
    }
}

#[test]
fn label_lengths_precede_code_point_and_ace_policy() {
    for (domain, expected) in [
        (format!("{}.com", "a".repeat(63)), Ok(())),
        (format!("{}.com", "a".repeat(64)), Err(LabelTooLong)),
        (format!("example.{}", "a".repeat(64)), Err(LabelTooLong)),
        (format!("xn--{}.com", "a".repeat(59)), Err(ALabel)),
        (format!("xn--{}.com", "a".repeat(60)), Err(LabelTooLong)),
        (format!("{}.com", "\u{436}".repeat(64)), Err(LabelTooLong)),
        (
            format!("{}.com", "\u{436}".repeat(100_000)),
            Err(LabelTooLong),
        ),
        (
            "[IPv6:0000:0000:0000:0000:0000:ffff:192.168.000.001]".into(),
            Ok(()),
        ),
    ] {
        assert_eq!(
            validate_smtp_utf8_address_with_options(&format!("user@{domain}"), &LENGTHS),
            expected
        );
    }
    assert_eq!(
        validate_smtp_utf8_address(&format!("user@{}.com", "a".repeat(64))),
        Ok(())
    );
}

#[test]
fn u_label_encoded_length_boundaries() {
    for (first, second) in [
        ("\u{436}", ""),
        ("\u{436}", "a"),
        ("\u{f8}", "a"),
        ("\u{4f8b}", ""),
        ("\u{4f8b}", "\u{3048}"),
        ("\u{20000}", ""),
        ("\u{e4}", "zz9"),
    ] {
        let mut label = String::new();
        let mut previous = String::new();
        let mut found = false;
        for i in 0..64 {
            label.push_str(if i % 2 == 0 || second.is_empty() {
                first
            } else {
                second
            });
            // Use the independent UTS #46 provider only on these unchanged,
            // well-formed labels to establish the exact 63-byte boundary.
            if let Ok(ascii) = mimecraft::idna::to_ascii(&label) {
                assert!(ascii.len() <= 63);
                previous.clone_from(&label);
            } else {
                assert!(!previous.is_empty());
                assert_ne!(
                    validate_smtp_utf8_address_with_options(
                        &format!("user@{previous}.com"),
                        &LENGTHS
                    ),
                    Err(LabelTooLong)
                );
                assert_eq!(
                    validate_smtp_utf8_address_with_options(&format!("user@{label}.com"), &LENGTHS),
                    Err(LabelTooLong)
                );
                found = true;
                break;
            }
        }
        assert!(found, "{first} / {second}");
    }
}

#[test]
fn randomized_domain_lengths_match_idna_without_mapping() {
    let alphabet = [
        'a',
        'z',
        '0',
        '-',
        '\u{f8}',
        '\u{e9}',
        '\u{436}',
        '\u{3bb}',
        '\u{4f8b}',
        '\u{3048}',
        '\u{d55c}',
        '\u{627}',
        '\u{915}',
        '\u{20000}',
        '\u{1f600}',
    ];
    let mut seed = 6531u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed as usize
    };
    let mut tested = 0;
    for _ in 0..10_000 {
        let label: String = (0..1 + next() % 29)
            .map(|_| alphabet[next() % alphabet.len()])
            .collect();
        let Ok(ascii) = mimecraft::idna::to_ascii(&label) else {
            continue;
        };
        // Exclude mappings (e.g. NFC) so the length really measures the input.
        if !ascii.starts_with("xn--") || mimecraft::idna::to_unicode(&ascii).unwrap() != label {
            continue;
        }
        let filler = ascii_domain(253 - ascii.len() - 1);
        let address = format!("u@{label}.{filler}");
        let result = validate_smtp_utf8_address_with_options(&address, &LENGTHS);
        assert!(
            !matches!(result, Err(DomainTooLong | LabelTooLong)),
            "{label:?}"
        );
        assert_eq!(
            validate_smtp_utf8_address_with_options(&format!("{address}a"), &LENGTHS),
            Err(DomainTooLong),
            "{label:?}"
        );
        tested += 1;
        if tested == 500 {
            break;
        }
    }
    assert_eq!(tested, 500);
}

#[test]
fn domain_total_lengths_and_precedence() {
    let domain = format!("{}com", "\u{e9}.".repeat(32));
    assert!(format!("u@{domain}").len() <= 254);
    assert_eq!(
        validate_smtp_utf8_address_with_options(&format!("u@{domain}"), &LENGTHS),
        Err(DomainTooLong)
    );
    for (address, expected) in [
        (format!("u@{}", ascii_domain(254)), Err(DomainTooLong)),
        (format!("u@{}", ascii_domain(252)), Ok(())),
        (format!("u@{}", ascii_domain(253)), Err(AddressTooLong)),
        (format!("{}@{}", "a".repeat(64), ascii_domain(189)), Ok(())),
        (
            format!("{}@{}", "a".repeat(64), ascii_domain(190)),
            Err(AddressTooLong),
        ),
        (
            format!("{}@{}", "\u{f8}".repeat(32), ascii_domain(189)),
            Ok(()),
        ),
        (
            format!("{}@{}", "\u{f8}".repeat(32), ascii_domain(190)),
            Err(AddressTooLong),
        ),
        (
            format!(
                "{}@[IPv6:0000:0000:0000:0000:0000:ffff:192.168.000.001]",
                "a".repeat(64)
            ),
            Ok(()),
        ),
        (format!("a@[x:{}]", "a".repeat(248)), Ok(())),
        (format!("a@[x:{}]", "a".repeat(249)), Err(AddressTooLong)),
    ] {
        assert_eq!(validate_smtp_utf8_address(&address), Ok(()));
        assert_eq!(
            validate_smtp_utf8_address_with_options(&address, &LENGTHS),
            expected,
            "{address:?}"
        );
    }
    for (address, expected) in [
        (format!("{}@exa..mple.com", "a".repeat(65)), Syntax),
        (
            format!("xn--{}@example.com", "a".repeat(61)),
            LocalPartTooLong,
        ),
        (
            format!("{}@example.com", "\u{200e}".repeat(22)),
            LocalPartTooLong,
        ),
        (
            format!("{}@{}.com", "a".repeat(65), "a".repeat(64)),
            LocalPartTooLong,
        ),
        (
            format!("u@{}.{}", ascii_domain(200), "a".repeat(64)),
            LabelTooLong,
        ),
    ] {
        assert_eq!(
            validate_smtp_utf8_address_with_options(&address, &LENGTHS),
            Err(expected)
        );
    }
}

#[test]
fn decoded_domains_keep_original_length_spelling() {
    let options = SmtpUtf8AddressValidationOptions {
        decode_idn: true,
        ..LENGTHS
    };
    // Decoding this label expands its UTF-8 spelling, but lengths use the input.
    let label = "\u{4f8b}".repeat(40);
    let encoded = mimecraft::idna::to_ascii(&label).unwrap();
    let filler = ascii_domain(252 - encoded.len() - 1);
    let address = format!("u@{encoded}.{filler}");
    assert_eq!(address.len(), 254);
    assert_eq!(
        validate_smtp_utf8_address_with_options(&address, &options),
        Ok(())
    );
    assert_eq!(
        validate_smtp_utf8_address_with_options(&format!("u@{label}.{filler}"), &options),
        Err(AddressTooLong)
    );
    assert_eq!(
        validate_smtp_utf8_address_with_options(&format!("{address}a"), &options),
        Err(AddressTooLong)
    );
    assert_eq!(
        validate_smtp_utf8_address_with_options(&format!("user@xn--{}", "a".repeat(60)), &options),
        Err(LabelTooLong)
    );
}

#[test]
fn mailbox_checks_do_not_mutate_or_validate_names_routes() {
    for (address, expected, decoded) in [
        ("g\u{f8}ril@example.com", Ok(()), Ok(())),
        (
            "I\u{412}M@d\u{f8}mi.fo",
            Err(MixedScripts),
            Err(MixedScripts),
        ),
        ("local", Err(Syntax), Err(Syntax)),
        ("xn--gr-zia@example.com", Err(ALabel), Err(ALabel)),
        ("user@xn--gr-zia.example.com", Err(ALabel), Ok(())),
        ("user@xn--wxa9v.example.com", Err(ALabel), Err(MixedScripts)),
        (
            "user@xn--a.example.com",
            Err(ALabel),
            Err(DisallowedCodePoint),
        ),
        ("user@xn--99999999999.example.com", Err(ALabel), Err(ALabel)),
    ] {
        let mut mailbox = MailboxAddress::new("\u{202e}\u{1f42a}", address.parse().unwrap());
        mailbox.route_mut().as_mut_vec().push("bad\r\nroute".into());
        let original = mailbox.clone();
        assert_eq!(mailbox.validate_smtp_utf8(), expected);
        assert_eq!(mailbox.validate_smtp_utf8_with_options(&DECODE), decoded);
        assert_eq!(mailbox, original);
        assert_eq!(mailbox.route(), original.route());
        assert_eq!(mailbox.address().as_str(), address);
    }
    let options = SmtpUtf8AddressValidationOptions {
        ascii_is_latin: true,
        ..DECODE
    };
    let mailbox: MailboxAddress = "PayPal <service@xn--aypal-uye.com>".parse().unwrap();
    assert_eq!(
        mailbox.validate_smtp_utf8_with_options(&options),
        Err(MixedScripts)
    );
    let mailbox: MailboxAddress = "<\u{438}\u{432}\u{430}\u{43d}@xn--e1afmkfd.com>"
        .parse()
        .unwrap();
    assert_eq!(mailbox.validate_smtp_utf8_with_options(&options), Ok(()));
}

#[test]
fn defaults_error_traits_and_iso2022_separation() {
    assert_eq!(
        SmtpUtf8AddressValidationOptions::default(),
        SmtpUtf8AddressValidationOptions {
            decode_idn: false,
            ascii_is_latin: false,
            enforce_length_limits: false,
        }
    );
    for error in [
        Syntax,
        LocalPartTooLong,
        LabelTooLong,
        DomainTooLong,
        AddressTooLong,
        ALabel,
        DisallowedCodePoint,
        NotNormalized,
        ContextRule,
        MixedScripts,
    ] {
        let error: &dyn std::error::Error = &error;
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
    // Legacy ESC compatibility is only an option on the non-SMTPUTF8 validator.
    let address = "\"\u{1b}$BF|\u{1b}(B\"@example.jp";
    assert_eq!(validate_smtp_utf8_address(address), Err(Syntax));
    assert_eq!(
        validate_smtp_utf8_address_with_options(
            address,
            &SmtpUtf8AddressValidationOptions {
                decode_idn: true,
                ascii_is_latin: true,
                enforce_length_limits: true,
            }
        ),
        Err(Syntax)
    );
}

// Adapted from MimeKit's UnitTests/Utils/SmtpAddressValidatorTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use SmtpAddressError::*;
use mimecraft::{
    MailboxAddress, SmtpAddressError, SmtpAddressValidationOptions,
    utils::{validate_smtp_address, validate_smtp_address_with_options},
};

const ISO: SmtpAddressValidationOptions = SmtpAddressValidationOptions {
    allow_iso2022_jp_local_part: true,
    enforce_length_limits: false,
};
const LENGTHS: SmtpAddressValidationOptions = SmtpAddressValidationOptions {
    allow_iso2022_jp_local_part: false,
    enforce_length_limits: true,
};
const BOTH: SmtpAddressValidationOptions = SmtpAddressValidationOptions {
    allow_iso2022_jp_local_part: true,
    enforce_length_limits: true,
};

// Japanese text encoded as ISO-2022-JP within a quoted string.
// The backslash of the JIS pair "K\" is escaped on the wire.
const ISO_ADDRESS: &str = "\"test\u{1b}$BF|K\\\\8l%a!<%k%F%9%H\u{1b}(B123\"@iso2022.jp";

fn check(address: &str, expected: Result<(), SmtpAddressError>) {
    assert_eq!(validate_smtp_address(address), expected, "{address:?}");
    for options in [SmtpAddressValidationOptions::default(), ISO, LENGTHS, BOTH] {
        assert_eq!(
            validate_smtp_address_with_options(address, &options),
            expected,
            "{address:?}, {options:?}"
        );
    }
}

#[test]
fn valid_ascii_mailboxes() {
    for address in [
        "example@example.com",
        "first.last@example.com",
        "!#$%&'*+-/=?^_`{|}~@example.com",
        "\"john doe\"@example.com",
        "\"a\\\"b\\\\c\"@example.com",
        "\"(comment)\"@example.com",
        "\"\"@example.com",
        "\"@\"@example.com",
        "user@example",
        "user@a-b.example.com",
        "user@xn--gr-zia.example.com",
        "xn--gr-zia@example.com",
        "user@XN--not-real-punycode",
        "user@[192.168.0.1]",
        "user@[IPv6:2001:db8::1]",
        "user@[IPv6:::ffff:1.2.3.4]",
        "user@[x-tag:content]",
    ] {
        check(address, Ok(()));
    }
}

#[test]
fn invalid_syntax_is_not_header_parsing() {
    for address in [
        "",
        "user",
        "@example.com",
        "user@",
        "us..er@example.com",
        ".user@example.com",
        "user.@example.com",
        "user@example.com.",
        "user@-example.com",
        "user@example-.com",
        "us er@example.com",
        "user@exa mple.com",
        "user@@example.com",
        "<user@example.com>",
        "user(comment)@example.com",
        "\"user@example.com",
        "user@[192.168.0.1",
        "user@[1.2.3]",
        "user@[IPv6:1::2::3]",
        "us\u{1}er@example.com",
        " user@example.com",
        "user@example.com ",
        "User <user@example.com>",
        "user@exa_mple.com",
        "user@.example",
        "user@exa..mple.com",
        "\"user\".name@example.com",
        "user@[]",
        "<>",
        "\"a\\",
        "\"a\\\"@example.com",
        "\"a\"b@example.com",
        "@route:user@example.com",
        "user@example.com\r\n",
        "\"user\r\n name\"@example.com",
    ] {
        check(address, Err(Syntax));
    }
    assert_eq!(
        validate_smtp_address("\"\u{1b}(B\"@example.jp"),
        Err(Syntax)
    );
}

#[test]
fn unicode_error_classification() {
    for address in [
        "d\u{f8}mi@example.com",
        "user@d\u{f8}mi.fo",
        "d\u{f8}mi@d\u{f8}mi.fo",
        "\"gr \u{e5}\"@example.com",
        "\u{4f8b}@\u{4f8b}\u{3048}.jp",
        "\u{1f600}@example.com",
        "user@\u{1f600}.example",
        "\"a\u{85}b\"@example.com",
    ] {
        check(address, Err(NonAscii));
    }
    for address in [
        "gr \u{e5}@example.com",
        "gr\u{e5}..x@example.com",
        "\"\\\u{e9}\"@example.com",
        "user@-\u{e9}.example",
        "user@\u{e9}-.example",
        "user@[tag:\u{e9}]",
        "user@[IPv6:\u{e9}]",
        "user@[\u{e9}:content]",
    ] {
        check(address, Err(Syntax));
    }
}

#[test]
fn iso2022_jp_compatibility() {
    for address in [
        ISO_ADDRESS,
        "\"\u{1b}$BF|K\\\\8l\u{1b}(B\"@example.jp",
        "\"\u{1b}$@F|K\\\\8l\u{1b}(B\"@example.jp",
        "\"\u{1b}(Jtaro\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|\u{1b}(B.\u{1b}$BK\\\\\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|\u{1b}(J\u{1b}$BK\\\\\u{1b}(B\"@example.jp",
        "\"\u{1b}(B\"@example.jp",
        "\"\u{1b}$B$\\\"\u{1b}(B\"@example.jp",
        "\"\u{1b}$B%a!&%k\u{1b}(B\"@example.jp",
        "\"\u{1b}$B/!\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|K\\\\8l\u{1b}(B\"@xn--wgv71a.jp",
        "\"\u{1b}$B\u{1b}(B\"@example.jp",
    ] {
        assert_eq!(
            validate_smtp_address_with_options(address, &ISO),
            Ok(()),
            "{address:?}"
        );
        assert_eq!(
            validate_smtp_address_with_options(address, &BOTH),
            Ok(()),
            "{address:?}"
        );
        assert_eq!(validate_smtp_address(address), Err(Syntax), "{address:?}");
        assert_eq!(
            validate_smtp_address_with_options(address, &LENGTHS),
            Err(Syntax),
            "{address:?}"
        );
    }
    check("\"plain\"@example.jp", Ok(()));
    check("plain@example.jp", Ok(()));
    for address in [
        "\"\u{65e5}\u{672c}\u{1b}$B8l\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|K\\\\8l\u{1b}(B\"@\u{65e5}\u{672c}.jp",
    ] {
        assert_eq!(
            validate_smtp_address_with_options(address, &ISO),
            Err(NonAscii)
        );
        assert_eq!(
            validate_smtp_address_with_options(address, &BOTH),
            Err(NonAscii)
        );
    }
}

#[test]
fn malformed_iso2022_jp() {
    for address in [
        "\u{1b}$BF|\u{1b}(B@example.jp",
        "\"\u{1b}$BF|\"@example.jp",
        "\"\u{1b}(Jtaro\"@example.jp",
        "\"\u{1b}$BF|K\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|K\u{1b}$B\\\\8l\u{1b}(B\"@example.jp",
        "\"\u{1b}$B$\"\u{1b}(B\"@example.jp",
        "\"\u{1b}$AF|\u{1b}(B\"@example.jp",
        "\"\u{1b}(I1\u{1b}(B\"@example.jp",
        "\"\u{1b}$(DF|\u{1b}(B\"@example.jp",
        "\"\u{1b}(\"@example.jp",
        "\"\u{1b}\"@example.jp",
        "\"\u{1b}$BF \u{1b}(B\"@example.jp",
        "\"\u{1b}$BF\u{1}\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF\u{e9}\u{1b}(B\"@example.jp",
        "\"\u{1b}(Jtar\u{e9}\u{1b}(B\"@example.jp",
        "\"\u{1b}$BF|\u{1b}(B\"@exa\u{1b}mple.jp",
        "\"\u{1b}$BF|\u{1b}(B\"@[\u{1b}(B1.2.3.4]",
        "\"\u{1b}$B!\\ \u{1b}(B\"@example.jp",
        "\"\u{1b}$B!\\\u{1b}(B\"@example.jp",
        "\"\u{1b}$B!\\\u{e9}\u{1b}(B\"@example.jp",
        "\"\u{1b}$B!\\\u{7f}\u{1b}(B\"@example.jp",
        "\"\u{1b}",
        "\"\u{1b}(",
        "\"\u{1b}$",
    ] {
        check(address, Err(Syntax));
    }
}

// Build an exact-length domain with labels <=62 bytes so appending a byte
// in a boundary test cannot accidentally exceed the label length limit.
fn ascii_domain(mut length: usize) -> String {
    let mut domain = String::new();
    while length > 0 {
        let count = if length <= 62 {
            length
        } else {
            62.min(length - 2)
        };
        domain.push_str(&"a".repeat(count));
        length -= count;
        if length > 0 {
            domain.push('.');
            length -= 1;
        }
    }
    domain
}

#[test]
fn exact_length_boundaries_and_default_opt_out() {
    for (address, expected) in [
        (format!("{}@example.com", "a".repeat(64)), Ok(())),
        (
            format!("{}@example.com", "a".repeat(65)),
            Err(LocalPartTooLong),
        ),
        // Quotes and wire escapes count toward the local-part limit.
        (format!("\"{}\"@example.com", "a".repeat(62)), Ok(())),
        (
            format!("\"{}\"@example.com", "a".repeat(63)),
            Err(LocalPartTooLong),
        ),
        (format!("\"{}\"@example.com", "\\a".repeat(31)), Ok(())),
        (
            format!("\"{}a\"@example.com", "\\a".repeat(31)),
            Err(LocalPartTooLong),
        ),
        (format!("u@{}.com", "a".repeat(63)), Ok(())),
        (format!("u@{}.com", "a".repeat(64)), Err(LabelTooLong)),
        (format!("u@xn--{}.com", "a".repeat(59)), Ok(())),
        (format!("u@xn--{}.com", "a".repeat(60)), Err(LabelTooLong)),
        (format!("u@{}", ascii_domain(252)), Ok(())),
        (format!("u@{}", ascii_domain(253)), Err(AddressTooLong)),
        (format!("u@{}", ascii_domain(254)), Err(DomainTooLong)),
        (format!("{}@{}", "a".repeat(64), ascii_domain(189)), Ok(())),
        (
            format!("{}@{}", "a".repeat(64), ascii_domain(190)),
            Err(AddressTooLong),
        ),
        (
            format!(
                "{}@[IPv6:0000:0000:0000:0000:0000:ffff:192.168.000.001]",
                "a".repeat(64)
            ),
            Ok(()),
        ),
    ] {
        assert_eq!(validate_smtp_address(&address), Ok(()), "{address:?}");
        assert_eq!(validate_smtp_address_with_options(&address, &ISO), Ok(()));
        for options in [LENGTHS, BOTH] {
            assert_eq!(
                validate_smtp_address_with_options(&address, &options),
                expected,
                "{address:?}"
            );
        }
    }
}

#[test]
fn iso2022_jp_length_boundaries() {
    for (pairs, odd, expected) in [
        (28, "", Ok(())),
        (29, "", Err(LocalPartTooLong)),
        (29, "F", Err(Syntax)),
    ] {
        // The quotes and escape sequences are part of the SMTP address.
        let address = format!("\"\u{1b}$B{}{odd}\u{1b}(B\"@example.jp", "F|".repeat(pairs));
        assert_eq!(address.find('@').unwrap(), 8 + 2 * pairs + odd.len());
        assert_eq!(
            validate_smtp_address_with_options(&address, &BOTH),
            expected
        );
        assert_eq!(
            validate_smtp_address_with_options(&address, &ISO),
            if odd.is_empty() { Ok(()) } else { Err(Syntax) }
        );
    }
}

#[test]
fn length_error_precedence() {
    for (address, expected) in [
        (format!("{}@exa..mple.com", "a".repeat(65)), Syntax),
        (format!("{}@example.com", "\u{f8}".repeat(65)), NonAscii),
        (
            format!("{}@{}.com", "a".repeat(65), "a".repeat(64)),
            LocalPartTooLong,
        ),
        (
            format!("u@{}.{}", ascii_domain(200), "a".repeat(64)),
            LabelTooLong,
        ),
        (format!("u@{}", ascii_domain(254)), DomainTooLong),
        (format!("u@{}", ascii_domain(253)), AddressTooLong),
    ] {
        assert_eq!(
            validate_smtp_address_with_options(&address, &LENGTHS),
            Err(expected)
        );
        assert_eq!(
            validate_smtp_address_with_options(&address, &BOTH),
            Err(expected)
        );
    }
}

#[test]
fn literals_only_obey_local_and_total_length_limits() {
    for (length, expected) in [(254, Ok(())), (255, Err(AddressTooLong))] {
        // An unregistered general tag/content is syntax-checked, not resolved.
        // Neither its tag nor content is subject to domain/label limits.
        let address = format!("a@[{}:{}]", "x".repeat(80), "a".repeat(length - 85));
        assert_eq!(address.len(), length);
        assert_eq!(validate_smtp_address(&address), Ok(()));
        assert_eq!(
            validate_smtp_address_with_options(&address, &LENGTHS),
            expected
        );
    }
    let address = format!("{}@[x:{}]", "a".repeat(65), "a".repeat(300));
    assert_eq!(
        validate_smtp_address_with_options(&address, &LENGTHS),
        Err(LocalPartTooLong)
    );
}

#[test]
fn ipv4_literal_grammar_and_full_consumption() {
    for (literal, valid) in [
        ("1.2.3.4", true),
        ("0.0.0.0", true),
        ("255.255.255.255", true),
        ("001.002.003.004", true),
        ("256.1.2.3", false),
        ("1.2.3", false),
        ("1.2.3.4.5", false),
        ("1..2.3", false),
        ("1.2.3.", false),
        ("1.2.3.4444", false),
        ("1.2.3.0000", false),
        ("0x1.2.3.4", false),
        ("1.2.3.-4", false),
        ("", false),
        (" 1.2.3.4", false),
        ("1.2.3.4 ", false),
    ] {
        let address = format!("a@[{literal}]");
        check(&address, if valid { Ok(()) } else { Err(Syntax) });
        check(&format!("{address}junk"), Err(Syntax));
        check(&format!("{address}]"), Err(Syntax));
    }
}

#[test]
fn ipv6_smtp_grammar_not_uri_grammar() {
    for (literal, valid) in [
        ("::", true),
        ("::1", true),
        ("1::", true),
        ("2001:db8::1", true),
        ("1:2:3:4:5:6:7:8", true),
        ("1:2:3:4:5:6::", true),
        ("FE80:0000:0000:0000:0202:B3FF:FE1E:8329", true),
        // SMTP compression must represent at least two groups, unlike URI hosts.
        ("1:2:3:4:5:6::8", false),
        ("::2:3:4:5:6:7:8", false),
        ("1:2:3:4:5:6:7::", false),
        ("::192.0.2.1", true),
        ("::ffff:192.0.2.1", true),
        ("1:2:3:4:5:6:192.0.2.1", true),
        ("1:2:3:4::192.0.2.1", true),
        ("1:2:3:4:5::192.0.2.1", false),
        ("::ffff:192.000.002.1", true),
        ("", false),
        (":", false),
        ("1:2:3:4:5:6:7:", false),
        (":1:2:3:4:5:6:7", false),
        ("1:2:3:4:5:6:7", false),
        ("1:2:3:4:5:6:7:8:9", false),
        ("1:2:3:4:5:6:7:8::", false),
        ("1::2::3", false),
        (":::1", false),
        ("1:::", false),
        ("fffff::1", false),
        ("::g", false),
        ("192.0.2.1", false),
        ("::ffff:192.0.2", false),
        ("::ffff:192.0.2.", false),
        ("::ffff:192.0.2.256", false),
        ("::ffff:192.0.2.1:2", false),
        ("::ffff:192.0.2.1.2", false),
        ("::ffff:192.0.2.1234", false),
        ("::ffff:0xc0.0.2.1", false),
        ("1:2:3:4:5:6:7:192.0.2.1", false),
        ("fe80::1%25eth0", false),
        ("192.0.2.1::", false),
    ] {
        for tag in ["IPv6", "iPv6", "IPV6"] {
            let address = format!("a@[{tag}:{literal}]");
            check(&address, if valid { Ok(()) } else { Err(Syntax) });
            check(&format!("{address}junk"), Err(Syntax));
        }
    }
}

#[test]
fn general_address_literal_grammar() {
    for (literal, valid) in [
        ("x-tag:content", true),
        ("-tag:content", true),
        ("0:!", true),
        ("tag:!\"#$%&'()*+,-./0123456789:;<=>?@AZ^_`az{|}~", true),
        (":content", false),
        ("tag:", false),
        ("tag-:content", false),
        ("-:content", false),
        ("tag_:content", false),
        ("tag:a b", false),
        ("tag:a[b", false),
        ("tag:a\\b", false),
        ("tag:a]b", false),
        ("tag:\u{7f}", false),
        ("tag:\u{80}", false),
        ("IPv6:arbitrary", false),
    ] {
        check(
            &format!("a@[{literal}]"),
            if valid { Ok(()) } else { Err(Syntax) },
        );
    }
}

#[test]
fn ascii_controls_and_quoted_pairs() {
    for byte in (0..32).chain(std::iter::once(127)) {
        let ch = char::from(byte);
        for address in [
            format!("a{ch}b@example.com"),
            format!("\"a{ch}b\"@example.com"),
            format!("\"a\\{ch}b\"@example.com"),
            format!("a@exa{ch}mple.com"),
            format!("a@[tag:a{ch}b]"),
        ] {
            check(&address, Err(Syntax));
        }
    }
    for byte in 32..=126 {
        // Any printable ASCII character may be quoted, even when not needed.
        check(&format!("\"a\\{}b\"@example.com", char::from(byte)), Ok(()));
    }
}

#[test]
fn truncated_literals_and_iso_sequences() {
    for address in [
        "a@[1.2.3.4]",
        "a@[IPv6:::]",
        "a@[IPv6:::ffff:1.2.3.4]",
        "a@[x:content]",
        "a@[\u{1f600}:content]",
    ] {
        for (end, _) in address.char_indices() {
            check(&address[..end], Err(Syntax));
        }
    }
    for fragment in ["\u{1b}", "\u{1b}(", "\u{1b}$", "\u{1b}$B!", "\u{1b}(J"] {
        check(&format!("\"{fragment}"), Err(Syntax));
        check(&format!("\"{fragment}\"@example.com"), Err(Syntax));
    }
}

#[test]
fn mailbox_validates_only_stored_addr_spec_without_mutation() {
    for (address, expected) in [
        ("goril@example.com", Ok(())),
        ("g\u{f8}ril@example.com", Err(NonAscii)),
        ("local", Err(Syntax)),
        ("user@xn--gr-zia.example.com", Ok(())),
        ("user@gr\u{e5}.example.com", Err(NonAscii)),
    ] {
        let mut mailbox = MailboxAddress::new("G\u{f8}ril\r\n", address.parse().unwrap());
        mailbox
            .route_mut()
            .as_mut_vec()
            .push("bad route\r\n".into());
        let before = mailbox.clone();
        assert_eq!(mailbox.validate_smtp(), expected);
        assert_eq!(mailbox.validate_smtp_with_options(&LENGTHS), expected);
        assert_eq!(mailbox, before);
        assert_eq!(mailbox.route(), before.route());
        assert_eq!(mailbox.address().as_str(), address);
    }
    let address = format!("{}@example.com", "a".repeat(65));
    let mailbox = MailboxAddress::new("Long", address.parse().unwrap());
    assert_eq!(mailbox.validate_smtp(), Ok(()));
    assert_eq!(
        mailbox.validate_smtp_with_options(&LENGTHS),
        Err(LocalPartTooLong)
    );

    // Parsing preserves domain spelling; IDNA conversion is explicitly requested.
    let mailbox: MailboxAddress = "<user@gr\u{e5}.example.com>".parse().unwrap();
    assert_eq!(mailbox.validate_smtp(), Err(NonAscii));
    assert_eq!(
        validate_smtp_address(&mailbox.address().with_ascii_domain().unwrap()),
        Ok(())
    );
    assert_eq!(mailbox.address().as_str(), "user@gr\u{e5}.example.com");

    // Header parsing is tolerant and normalizes comments/whitespace first.
    let input = "user (comment) @ example.com";
    assert_eq!(validate_smtp_address(input), Err(Syntax));
    let mailbox: MailboxAddress = input.parse().unwrap();
    assert_eq!(mailbox.validate_smtp(), Ok(()));
}

#[test]
fn options_defaults_and_error_traits() {
    let options = SmtpAddressValidationOptions::default();
    assert!(!options.allow_iso2022_jp_local_part);
    assert!(!options.enforce_length_limits);
    for error in [
        Syntax,
        NonAscii,
        LocalPartTooLong,
        LabelTooLong,
        DomainTooLong,
        AddressTooLong,
    ] {
        let error: &dyn std::error::Error = &error;
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
}

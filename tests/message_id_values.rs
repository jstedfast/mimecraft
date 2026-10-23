// Includes cases adapted from MimeKit's UnitTests/MimeMessageTests.cs.
// Assignment semantics follow MimeKit's Utils/ParseUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{MessageId, MessageIdParseErrorKind as Kind, References};

#[test]
fn source_setter_normalization_vectors() {
    for (input, expected) in [
        ("a@b", "a@b"),
        ("<a@b>", "a@b"),
        ("  <a@b>  ", "a@b"),
        ("<a@b> (comment)", "a@b"),
        ("<a@b>\r\n (comment)", "a@b"),
        ("\"x\r\n y\"@b", "\"x y\"@b"),
        ("\"x y\"@b", "\"x y\"@b"),
        ("(paren@example.com", "(paren@example.com"),
        ("[a@b]", "[a@b]"),
        ("a@b@c", "a@b@c"),
        ("local@", "local@"),
    ] {
        let id = MessageId::from_value(input).unwrap();
        assert_eq!(id.as_str(), expected, "{input:?}");
        assert_eq!(id.to_string(), expected);
        assert_eq!(id.clone().into_string(), expected);
        assert_eq!(MessageId::from_value(id.as_str()), Ok(id));
    }
}

#[test]
fn source_invalid_setter_vectors() {
    for input in [
        "a@b garbage",
        "a b@c",
        "a@b (comment)",
        "<a@b> <c@d>",
        "<a@b>>",
        "a@b>",
        "<a@b",
        "<>",
        "",
        "   ",
        "a@b\r\nBcc: evil@example.com",
        "a@b\r\n\r\nbody",
        "a@b\nc",
        "a@b\rc",
        "a\0b@c",
    ] {
        assert!(MessageId::from_value(input).is_err(), "{input:?}");
    }
}

#[test]
fn outer_unicode_whitespace_is_trimmed_before_validation() {
    for whitespace in [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{85}', '\u{a0}', '\u{1680}', '\u{2000}',
        '\u{2007}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ] {
        for token in ["a@b", "<a@b>"] {
            let input = format!("{whitespace}{token}{whitespace}");
            assert_eq!(MessageId::from_value(&input).unwrap().as_str(), "a@b");
        }
        let empty = whitespace.to_string().repeat(3);
        let error = MessageId::from_value(&empty).unwrap_err();
        assert_eq!(error.kind(), Kind::MissingId);
        assert_eq!(error.offset(), empty.len());
    }
    // Like MimeKit's Trim(), outer line endings are removed, not rejected.
    assert_eq!(
        MessageId::from_value("\r\n a@b\r\n").unwrap().as_str(),
        "a@b"
    );
}

#[test]
fn crlf_cr_and_lf_folds_require_ascii_space_or_tab() {
    for line_break in ["\r\n", "\r", "\n"] {
        for blank in [" ", "\t", " \t"] {
            let input = format!("\"x{line_break}{blank}y\"@b");
            assert_eq!(
                MessageId::from_value(&input).unwrap().as_str(),
                format!("\"x{blank}y\"@b")
            );
            let input = format!("<a@b>{line_break}{blank}(tail)");
            assert_eq!(MessageId::from_value(&input).unwrap().as_str(), "a@b");
        }
        for nonblank in ["x", "\u{a0}", "\u{b}", "\u{c}", "\u{2028}", "\r", "\n"] {
            let input = format!("<a{line_break}{nonblank}b@c>");
            let error = MessageId::from_value(&input).unwrap_err();
            assert_eq!(error.kind(), Kind::InvalidLineBreak, "{input:?}");
            assert_eq!(error.offset(), 2, "{input:?}");
        }
    }
}

#[test]
fn nul_and_unfolded_line_breaks_are_rejected_even_inside_comments_or_quotes() {
    for forbidden in ["\0", "\r", "\n", "\r\n"] {
        for input in [
            format!("<a{forbidden}b@c>"),
            format!("<\"a{forbidden}b\"@c>"),
            format!("<a@b> (a{forbidden}b)"),
            format!("<a@b> (escaped\\{forbidden}b)"),
        ] {
            let error = MessageId::from_value(&input).unwrap_err();
            let kind = if forbidden == "\0" {
                Kind::NulCharacter
            } else {
                Kind::InvalidLineBreak
            };
            assert_eq!(error.kind(), kind, "{input:?}");
            assert_eq!(error.offset(), input.find(forbidden).unwrap());
        }
    }
}

#[test]
fn unquoted_whitespace_rules_depend_on_explicit_brackets() {
    for (input, expected) in [
        ("<a b@c>", "ab@c"),
        ("<a\r\n b@c>", "ab@c"),
        ("<a\tb@c>", "ab@c"),
        ("<a@ (domain) b>", "a@b"),
        ("a@b(comment)", "a@b"),
        ("<a@b> (comment)", "a@b"),
        ("\"a b\"@c", "\"a b\"@c"),
    ] {
        assert_eq!(MessageId::from_value(input).unwrap().as_str(), expected);
    }
    for input in ["a b@c", "a\tb@c", "a\r\n b@c", "a@b (comment)", "a@(a b)c"] {
        assert_eq!(
            MessageId::from_value(input).unwrap_err().kind(),
            Kind::UnquotedWhitespace,
            "{input:?}"
        );
    }
}

#[test]
fn quote_and_backslash_parity_follow_setter_scanning() {
    for count in 0..10 {
        let input = format!("\"a{}\" @b", "\\".repeat(count));
        let error = MessageId::from_value(&input).unwrap_err();
        let expected = if count % 2 == 0 {
            Kind::UnquotedWhitespace
        } else {
            Kind::IncompleteQuotedString
        };
        assert_eq!(error.kind(), expected, "{input:?}");
    }
    for input in ["\"a\\\" b\"@c", "\"a\\\\ b\"@c", "\"a\\\r\n b\"@c"] {
        let expected = input.replace(['\r', '\n'], "");
        assert_eq!(MessageId::from_value(input).unwrap().as_str(), expected);
    }
}

#[test]
fn compatible_ids_are_not_restricted_to_rfc_syntax() {
    for (input, expected) in [
        ("local", "local"),
        ("local.", "local."),
        ("local@", "local@"),
        ("@", "@"),
        ("[image@@example.com]", "[image@example.com]"),
        ("id@domain1@domain2", "id@domain1@domain2"),
        ("id@[not-an-ip]", "id@[not-an-ip]"),
        (
            ":invalid-local-part;@domain.com",
            ":invalid-local-part;@domain.com",
        ),
        ("a\u{b}b@c", "a\u{b}b@c"),
        ("a\u{7f}b@c", "a\u{7f}b@c"),
        ("a\u{a0}b@c", "a\u{a0}b@c"),
        ("\u{e9}@\u{6d4b}\u{8bd5}.cn", "\u{e9}@\u{6d4b}\u{8bd5}.cn"),
        ("LOCAL@EXAMPLE.COM", "LOCAL@EXAMPLE.COM"),
    ] {
        assert_eq!(MessageId::from_value(input).unwrap().as_str(), expected);
    }
}

#[test]
fn source_parsed_id_vectors_round_trip_through_assignment() {
    for raw in [
        "<a@example.com>",
        "<(paren@example.com>",
        "<\"x\r\n y\"@example.com>",
        "garbage <a@example.com>",
        "<a@b@example.com>",
        "<local@>",
        "<[a@example.com]>",
        "<a.@example.com.>",
        "<a@@example.com>",
        "<a b@example.com>",
        "<d7e8bc604f797c18ba8120250cbd8c04-JFBVALKQOJXWILKCJQZFA7CDNRQXE2LUPF6EIYLUMFGG643TPRCXQ32TNV2HA===@microsoft.com>",
        "<1234@local.machine.example> (comment)",
        "a@example.com",
    ] {
        let parsed = MessageId::parse(raw.as_bytes()).unwrap();
        assert_eq!(
            MessageId::from_value(parsed.as_str()),
            Ok(parsed),
            "{raw:?}"
        );
        for reference in References::new(raw.as_bytes()) {
            let reference = reference.unwrap();
            assert_eq!(MessageId::from_value(reference.as_str()), Ok(reference));
        }
    }
}

#[test]
fn reading_apis_remain_tolerant() {
    for input in ["a b@c", "a@b>", "a\0b@c", "a\r\nb@c"] {
        assert!(MessageId::parse(input.as_bytes()).is_ok(), "{input:?}");
        assert!(input.parse::<MessageId>().is_ok(), "{input:?}");
        assert!(MessageId::from_value(input).is_err(), "{input:?}");
    }
    assert!(MessageId::parse(b"(paren@example.com").is_err());
    assert!(MessageId::from_value("(paren@example.com").is_ok());
}

#[test]
fn offsets_refer_to_original_bytes_after_trim_unfold_and_wrapping() {
    for (input, kind, marker) in [
        ("\u{a0} <a@\r\n .bad> \t", Kind::InvalidDomain, ".bad"),
        (" \u{e9}@.bad \t", Kind::InvalidDomain, ".bad"),
        ("<\"x\r\n y\n\tz\"@.bad>", Kind::InvalidDomain, ".bad"),
        ("<a@b>\r\n (unfinished", Kind::IncompleteComment, "("),
        ("  \"x\r\n y@b", Kind::IncompleteQuotedString, "\""),
        ("\u{a0}<\u{e9}\0@b>", Kind::NulCharacter, "\0"),
        ("\u{a0}<\u{e9}\rX@b>", Kind::InvalidLineBreak, "\r"),
        (" \u{e9}\r\n @b ", Kind::UnquotedWhitespace, " @"),
        ("<a@b>\r\n <c@d>", Kind::TrailingInput, "<c"),
        ("<a@b>\r\n (tail) \n\t<c@d>", Kind::TrailingInput, "<c"),
        (
            "\u{a0}<a@b>\r\n (tail\r\n (inner))\n\t<c@d>",
            Kind::TrailingInput,
            "<c",
        ),
    ] {
        let error = MessageId::from_value(input).unwrap_err();
        assert_eq!(error.kind(), kind, "{input:?}");
        assert_eq!(error.offset(), input.find(marker).unwrap(), "{input:?}");
        assert!(
            error
                .to_string()
                .ends_with(&format!("at byte {}", error.offset()))
        );
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn missing_input_and_synthetic_bracket_errors_point_to_original_boundaries() {
    let input = " \u{e9}@b> \t";
    let error = MessageId::from_value(input).unwrap_err();
    assert_eq!(error.kind(), Kind::TrailingInput);
    assert_eq!(error.offset(), input.trim_end().len());
    let input = "\u{a0}<abc \t";
    let error = MessageId::from_value(input).unwrap_err();
    assert_eq!(error.kind(), Kind::IncompleteId);
    assert_eq!(error.offset(), input.trim_end().len());
    let error = MessageId::from_value(" \u{a0}< >").unwrap_err();
    assert_eq!(error.kind(), Kind::EmptyId);
    assert_eq!(error.offset(), 3);
    let error = MessageId::from_value("").unwrap_err();
    assert_eq!(error.kind(), Kind::MissingId);
    assert_eq!(error.offset(), 0);
}

#[test]
fn multiple_folds_preserve_original_error_offsets_without_a_source_map() {
    let mut input = String::from("\u{a0}<\"x");
    for _ in 0..10_000 {
        input.push_str("\r\n \u{e9}");
    }
    input.push_str("\"@.bad> \t");
    let error = MessageId::from_value(&input).unwrap_err();
    assert_eq!(error.kind(), Kind::InvalidDomain);
    assert_eq!(error.offset(), input.find(".bad").unwrap());
}

#[test]
fn generated_values_have_bounded_errors_and_no_nul_or_line_breaks_on_success() {
    let alphabet = [
        'a',
        'b',
        '0',
        '.',
        '@',
        '<',
        '>',
        '[',
        ']',
        '(',
        ')',
        '"',
        '\\',
        ' ',
        '\t',
        '\r',
        '\n',
        '\0',
        '\u{e9}',
        '\u{1f600}',
        '\u{a0}',
    ];
    let mut state = 0x1234_5678u32;
    for sample in 0..20_000 {
        let mut input = String::new();
        for _ in 0..sample % 64 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            input.push(alphabet[((state >> 24) as usize) % alphabet.len()]);
        }
        match MessageId::from_value(&input) {
            Ok(id) => {
                assert!(!id.as_str().is_empty());
                assert!(!id.as_str().contains(['\0', '\r', '\n']));
            }
            Err(error) => {
                assert!(error.offset() <= input.len(), "{input:?}");
                assert!(input.is_char_boundary(error.offset()), "{input:?}");
                assert_ne!(error.kind(), Kind::InvalidUtf8, "{input:?}");
            }
        }
    }
}

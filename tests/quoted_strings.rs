// Includes cases adapted from MimeKit's UnitTests/Utils/MimeUtilsTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use mimecraft::utils::{
    append_quoted, append_unquoted, append_unquoted_bytes, quote, unquote, unquote_bytes,
};

#[test]
fn source_append_quoted_vector() {
    let input = "This is a multi-line quoted string with \\backslashes\\ and an escaped dquote (\") inside of it.";
    let expected = "\"This is a multi-line quoted string with \\\\backslashes\\\\ and an escaped dquote (\\\") inside of it.\"";
    assert_eq!(quote(input), expected);
    let mut output = String::new();
    append_quoted(input, &mut output);
    assert_eq!(output, expected);
}

#[test]
fn source_unquote_vector() {
    let input = "\"This is a multi-line quoted string with\r\n\t\\\\backslashes\\\\ and an escaped dquote (\\\") inside of it.\"";
    let expected = "This is a multi-line quoted string with \\backslashes\\ and an escaped dquote (\") inside of it.";
    assert_eq!(unquote(input, true), expected);
    assert_eq!(
        unquote_bytes(input.as_bytes(), true).as_ref(),
        expected.as_bytes()
    );
}

#[test]
fn source_utf8_append_and_subslice_vector() {
    let quoted = "\"Ren\u{e9}e\\\" \r\n\tM\u{fc}ller\\\\\"";
    let wrapped = format!("###{quoted}###");
    let bytes = wrapped.as_bytes();
    for convert_tabs in [false, true] {
        let expected = if convert_tabs {
            "Ren\u{e9}e\"  M\u{fc}ller\\"
        } else {
            "Ren\u{e9}e\" \tM\u{fc}ller\\"
        };
        let mut output = vec![b'>'];
        append_unquoted_bytes(&bytes[3..bytes.len() - 3], convert_tabs, &mut output);
        assert_eq!(output, format!(">{expected}").as_bytes());
        let mut text = String::from(">");
        append_unquoted(quoted, convert_tabs, &mut text);
        assert_eq!(text, format!(">{expected}"));
        assert_eq!(unquote(quoted, convert_tabs), expected);
    }
}

#[test]
fn quoting_escapes_only_backslashes_and_double_quotes() {
    for (input, expected) in [
        ("", "\"\""),
        ("a", "\"a\""),
        ("\\", "\"\\\\\""),
        ("\"", "\"\\\"\""),
        ("a\r\nb\t\0c", "\"a\r\nb\t\0c\""),
        ("(comment) <x> a;b", "\"(comment) <x> a;b\""),
        ("\u{e9}\u{1f600}", "\"\u{e9}\u{1f600}\""),
        ("\"already quoted\"", "\"\\\"already quoted\\\"\""),
    ] {
        assert_eq!(quote(input), expected);
    }
}

#[test]
fn malformed_quotes_and_unknown_escapes_follow_tolerant_source_rules() {
    for (input, expected) in [
        ("", ""),
        ("\"", ""),
        ("\"\"", ""),
        ("\"unterminated", "unterminated"),
        ("a\"b\"c", "abc"),
        ("a\"\"b", "ab"),
        ("\\", ""),
        ("\\\\", "\\"),
        ("\\\\\\", "\\"),
        ("\\a\\q\\z", "aqz"),
        ("\\\"", "\""),
        ("a\\\rb", "ab"),
        ("a\\\nb", "ab"),
        ("a\\\r\nb", "ab"),
        ("a\\\r\"b", "ab"),
        ("a\\\n\\\"b", "a\"b"),
        ("(comment) \"x\"", "(comment) x"),
        ("\\\u{e9}\\\u{1f600}", "\u{e9}\u{1f600}"),
    ] {
        for convert_tabs in [false, true] {
            assert_eq!(unquote(input, convert_tabs), expected, "{input:?}");
            assert_eq!(
                unquote_bytes(input.as_bytes(), convert_tabs).as_ref(),
                expected.as_bytes(),
                "{input:?}"
            );
        }
    }
}

#[test]
fn tabs_convert_individually_and_reset_escape_state() {
    let input = " \t\"a\\\t\"b\r\n\t\t ";
    assert_eq!(unquote(input, false), " \ta\tb\t\t ");
    assert_eq!(unquote(input, true), "  a b   ");
    for convert_tabs in [false, true] {
        assert_eq!(
            unquote_bytes(input.as_bytes(), convert_tabs).as_ref(),
            unquote(input, convert_tabs).as_bytes()
        );
    }
}

#[test]
fn backslash_run_parity_controls_following_quotes() {
    for length in 0..64 {
        let slashes = "\\".repeat(length);
        let expected = "\\".repeat(length / 2);
        assert_eq!(unquote(&slashes, false), expected);
        let quoted = format!("{slashes}\"");
        let expected_quote = if length % 2 == 0 {
            expected.clone()
        } else {
            format!("{expected}\"")
        };
        assert_eq!(unquote(&quoted, false), expected_quote);
        let quoted = format!("{slashes}\r\n\"");
        assert_eq!(unquote(&quoted, false), expected);
    }
}

#[test]
fn all_raw_bytes_are_preserved_except_documented_ascii_transformations() {
    for byte in 0..=u8::MAX {
        for convert_tabs in [false, true] {
            let expected = match byte {
                b'\r' | b'\n' | b'\\' | b'"' => Vec::new(),
                b'\t' if convert_tabs => vec![b' '],
                _ => vec![byte],
            };
            assert_eq!(unquote_bytes(&[byte], convert_tabs).as_ref(), expected);

            let expected_escaped = match byte {
                b'\r' | b'\n' => Vec::new(),
                b'\t' if convert_tabs => vec![b' '],
                _ => vec![byte],
            };
            assert_eq!(
                unquote_bytes(&[b'\\', byte], convert_tabs).as_ref(),
                expected_escaped,
                "{byte:#x}"
            );
        }
    }
    let raw = b"\"\xff\\\x80\xc0\xaf\0\r\n\t\"";
    assert_eq!(unquote_bytes(raw, false).as_ref(), b"\xff\x80\xc0\xaf\0\t");
    assert_eq!(unquote_bytes(raw, true).as_ref(), b"\xff\x80\xc0\xaf\0 ");
}

#[test]
fn unchanged_inputs_are_borrowed_including_preserved_tabs_and_raw_bytes() {
    for input in ["", "plain", "a\0b", "\u{e9}\u{1f600}", "(comment)"] {
        for convert_tabs in [false, true] {
            let result = unquote(input, convert_tabs);
            assert!(matches!(result, Cow::Borrowed(_)));
            assert_eq!(result.as_ptr(), input.as_ptr());
        }
    }
    assert!(matches!(unquote("a\tb", false), Cow::Borrowed(_)));
    assert!(matches!(unquote("a\tb", true), Cow::Owned(_)));
    for input in [b"".as_slice(), b"plain", b"\xff\x80\0"] {
        for convert_tabs in [false, true] {
            let result = unquote_bytes(input, convert_tabs);
            assert!(matches!(result, Cow::Borrowed(_)));
            assert_eq!(result.as_ptr(), input.as_ptr());
        }
    }
    assert!(matches!(unquote_bytes(b"a\tb", false), Cow::Borrowed(_)));
    assert!(matches!(unquote_bytes(b"a\tb", true), Cow::Owned(_)));
    for input in ["\\", "\"", "\r", "\n"] {
        assert!(matches!(unquote(input, false), Cow::Owned(_)));
        assert!(matches!(
            unquote_bytes(input.as_bytes(), false),
            Cow::Owned(_)
        ));
    }
}

#[test]
fn append_operations_preserve_existing_output_and_treat_each_call_independently() {
    let prefix = "\"prefix\\\r\n";
    let mut output = prefix.to_owned();
    append_quoted("a", &mut output);
    append_quoted("\\\"", &mut output);
    assert_eq!(output, format!("{prefix}{}{}", quote("a"), quote("\\\"")));

    let mut output = prefix.to_owned();
    append_unquoted("\"a\\\"b\"", false, &mut output);
    append_unquoted("\\", false, &mut output);
    append_unquoted("\"", false, &mut output);
    assert_eq!(output, format!("{prefix}a\"b"));
    assert_eq!(unquote("\\\"", false), "\"");

    let prefix = b"\xff\\\r\n";
    let mut output = prefix.to_vec();
    append_unquoted_bytes(b"\"a\\\"b\"", false, &mut output);
    append_unquoted_bytes(b"\\", false, &mut output);
    append_unquoted_bytes(b"\"", false, &mut output);
    assert_eq!(&output[..prefix.len()], prefix);
    assert_eq!(&output[prefix.len()..], b"a\"b");
}

// A run-based reference model, independent of the implementation's escape flag.
fn reference_unquote(input: &[u8], convert_tabs: bool) -> Vec<u8> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'\\' {
            let start = index;
            while input.get(index) == Some(&b'\\') {
                index += 1;
            }
            let count = index - start;
            output.extend(std::iter::repeat_n(b'\\', count / 2));
            if count % 2 == 0 || index == input.len() {
                continue;
            }
            match input[index] {
                b'\r' | b'\n' => {}
                b'\t' if convert_tabs => output.push(b' '),
                byte => output.push(byte),
            }
            index += 1;
            continue;
        }
        match input[index] {
            b'\r' | b'\n' | b'"' => {}
            b'\t' if convert_tabs => output.push(b' '),
            byte => output.push(byte),
        }
        index += 1;
    }
    output
}

#[test]
fn exhaustive_short_inputs_match_an_independent_model() {
    let alphabet = b"a\\\"\r\n\t";
    for length in 0..=5u32 {
        for mut value in 0..alphabet.len().pow(length) {
            let mut input = vec![0; length as usize];
            for byte in &mut input {
                *byte = alphabet[value % alphabet.len()];
                value /= alphabet.len();
            }
            for convert_tabs in [false, true] {
                let expected = reference_unquote(&input, convert_tabs);
                assert_eq!(unquote_bytes(&input, convert_tabs).as_ref(), expected);
                let text = std::str::from_utf8(&input).unwrap();
                assert_eq!(unquote(text, convert_tabs).as_bytes(), expected);
            }
        }
    }
}

#[test]
fn generated_raw_bytes_match_an_independent_model() {
    let mut state = 0x1234_5678u32;
    for sample in 0..10_000 {
        let mut input = Vec::new();
        for _ in 0..sample % 128 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            input.push((state >> 24) as u8);
        }
        for convert_tabs in [false, true] {
            let expected = reference_unquote(&input, convert_tabs);
            assert_eq!(unquote_bytes(&input, convert_tabs).as_ref(), expected);
            let mut output = b"prefix".to_vec();
            append_unquoted_bytes(&input, convert_tabs, &mut output);
            assert_eq!(&output[..6], b"prefix");
            assert_eq!(&output[6..], expected);
        }
    }
}

#[test]
fn generated_unicode_round_trips_except_documented_line_break_and_tab_changes() {
    let alphabet = [
        'a',
        '\\',
        '"',
        '\r',
        '\n',
        '\t',
        '\0',
        '\u{e9}',
        '\u{1f600}',
        '\u{301}',
    ];
    let mut state = 0x4321_8765u32;
    for sample in 0..10_000 {
        let mut input = String::new();
        for _ in 0..sample % 64 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            input.push(alphabet[((state >> 24) as usize) % alphabet.len()]);
        }
        let quoted = quote(&input);
        let escapes = input
            .bytes()
            .filter(|&byte| matches!(byte, b'\\' | b'"'))
            .count();
        assert_eq!(quoted.len(), input.len() + escapes + 2);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        for convert_tabs in [false, true] {
            let expected: String = input
                .chars()
                .filter(|ch| !matches!(ch, '\r' | '\n'))
                .map(|ch| if convert_tabs && ch == '\t' { ' ' } else { ch })
                .collect();
            assert_eq!(unquote(&quoted, convert_tabs), expected);
            assert_eq!(
                unquote_bytes(quoted.as_bytes(), convert_tabs).as_ref(),
                expected.as_bytes()
            );
        }
    }
}

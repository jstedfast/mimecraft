// Adapted from MimeKit's UnitTests/MimeAnonymizerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{DateTimeOffset, HeaderList, anonymize::received_header_value};

fn assert_case(input: &[u8], expected: &[u8]) {
    let original = input.to_vec();
    assert_eq!(expected.len(), input.len());
    assert_eq!(received_header_value(input), expected, "input {input:?}");
    // Unlike the other masking helpers, keyword and date lookahead can change
    // a prefix's output. Truncation must still preserve byte counts.
    for end in 0..=input.len() {
        assert_eq!(received_header_value(&input[..end]).len(), end);
    }
    assert_eq!(input, original);
}

#[test]
fn canonical_received_values() {
    for (input, expected) in [
        (
            " (qmail 21619 invoked from network); 15 Nov 2017 14:16:18 -0000\r\n",
            " (xxxxx xxxxx xxxxxxx xxxx xxxxxxx); 15 Nov 2017 14:16:18 -0000\r\n",
        ),
        (
            " from unknown (HELO EUR01-HE1-obe.outbound.protection.outlook.com) (80.68.177.35)\r\n  by  with SMTP; 15 Nov 2017 14:16:18 -0000\r\n",
            " from xxxxxxx (xxxx xxxxxxxxxxxxx.xxxxxxxx.xxxxxxxxxx.xxxxxxx.xxx) (xx.xx.xxx.xx)\r\n  by  with xxxx; 15 Nov 2017 14:16:18 -0000\r\n",
        ),
        (
            " from mail-he1eur01on0133.outbound.protection.outlook.com\r\n\t([104.47.0.133] helo=EUR01-HE1-obe.outbound.protection.outlook.com) by\r\n\tmyassp01.mynet.it with SMTP (2.5.5); 15 Nov 2017 15:16:20 +0100\r\n",
            " from xxxxxxxxxxxxxxxxxxx.xxxxxxxx.xxxxxxxxxx.xxxxxxx.xxx\r\n\t([xxx.xx.x.xxx] xxxxxxxxxxxxxxxxxx.xxxxxxxx.xxxxxxxxxx.xxxxxxx.xxx) by\r\n\txxxxxxxx.xxxxx.xx with xxxx (x.x.x); 15 Nov 2017 15:16:20 +0100\r\n",
        ),
        (
            " from AM4PR01MB1444.eurprd01.prod.exchangelabs.com (10.164.76.26) by\r\n AM4PR01MB1442.eurprd01.prod.exchangelabs.com (10.164.76.24) with Microsoft\r\n SMTP Server (version=TLS1_2,\r\n cipher=TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384_P256) id 15.20.218.12; Wed, 15\r\n Nov 2017 14:16:14 +0000\r\n",
            " from xxxxxxxxxxxxx.xxxxxxxx.xxxx.xxxxxxxxxxxx.xxx (xx.xxx.xx.xx) by\r\n xxxxxxxxxxxxx.xxxxxxxx.xxxx.xxxxxxxxxxxx.xxx (xx.xxx.xx.xx) with xxxxxxxxx\r\n xxxx xxxxxx (xxxxxxxxxxxxxx,\r\n xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx) id xx.xx.xxx.xx; Wed, 15\r\n Nov 2017 14:16:14 +0000\r\n",
        ),
        (
            " from AM4PR01MB1444.eurprd01.prod.exchangelabs.com\r\n ([fe80::7830:c66f:eaa8:e3dd]) by AM4PR01MB1444.eurprd01.prod.exchangelabs.com\r\n ([fe80::7830:c66f:eaa8:e3dd%14]) with mapi id 15.20.0218.015; Wed, 15 Nov\r\n 2017 14:16:14 +0000\r\n",
            " from xxxxxxxxxxxxx.xxxxxxxx.xxxx.xxxxxxxxxxxx.xxx\r\n ([xxxx::xxxx:xxxx:xxxx:xxxx]) by xxxxxxxxxxxxx.xxxxxxxx.xxxx.xxxxxxxxxxxx.xxx\r\n ([xxxx::xxxx:xxxx:xxxx:xxxxxxx]) with xxxx id xx.xx.xxxx.xxx; Wed, 15 Nov\r\n 2017 14:16:14 +0000\r\n",
        ),
        (
            " from unknown (this is a (nested comment) with an \\\"escaped quoted\")\r\n by AM4PR01MB1442.eurprd01.prod.exchangelabs.com (10.164.76.24) \r\n",
            " from xxxxxxx (xxxx xx x (xxxxxx xxxxxxx) xxxx xx \\xxxxxxxx xxxxxxx)\r\n by xxxxxxxxxxxxx.xxxxxxxx.xxxx.xxxxxxxxxxxx.xxx (xx.xxx.xx.xx) \r\n",
        ),
        (
            " by mail.example.com with ESMTP id abc123\r\n",
            " by xxxx.xxxxxxx.xxx with xxxxx id xxxxxx\r\n",
        ),
    ] {
        assert_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn keywords_match_all_ascii_casings_only_before_mime_whitespace() {
    for keyword in ["from", "by", "via", "with", "id", "for"] {
        for casing in 0..1 << keyword.len() {
            let word: Vec<_> = keyword
                .bytes()
                .enumerate()
                .map(|(index, byte)| {
                    if casing & (1 << index) != 0 {
                        byte.to_ascii_uppercase()
                    } else {
                        byte
                    }
                })
                .collect();
            for whitespace in *b" \t\r\n" {
                let mut input = word.clone();
                input.push(whitespace);
                assert_case(&input, &input);
            }
        }
        for byte in 0..=255u8 {
            let mut input = keyword.as_bytes().to_vec();
            input.push(byte);
            let mut expected = if b" \t\r\n".contains(&byte) {
                keyword.as_bytes().to_vec()
            } else {
                vec![b'x'; keyword.len()]
            };
            expected.push(if b" \t\r\n<>[]:@,.;(".contains(&byte) {
                byte
            } else {
                b'x'
            });
            assert_case(&input, &expected);
        }
        assert_case(keyword.as_bytes(), &vec![b'x'; keyword.len()]);
    }
}

#[test]
fn keyword_boundaries_do_not_follow_arbitrary_punctuation_or_decode_words() {
    for (input, expected) in [
        ("FROM by via with ID for ", "FROM by via with ID for "),
        (
            "xfrom from.example FROM@host ",
            "xxxxx xxxx.xxxxxxx xxxx@xxxx ",
        ),
        ("from(foo)by mx", "xxxx(xxx)by xx"),
        ("from; by mx", "xxxx; by xx"),
        ("<from mx>", "<xxxx xx>"),
        ("x)from ", "xxxxxx "),
        ("=?utf-8?q?A?=", "xxxxxxxxxxxxx"),
        ("from\u{a0}host ", "xxxxxxxxxx "),
        ("f\u{212a}om host", "xxxxxx xxxx"),
    ] {
        assert_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn nested_and_malformed_comments_preserve_source_escape_rules() {
    for (input, expected) in [
        ("(from by (via with) id for)", "(xxxx xx (xxx xxxx) xx xxx)"),
        ("(a\\) from host)", "(x\\x xxxx xxxx)"),
        ("(a\\\\)from host", "(x\\\\)from xxxx"),
        ("(a\\(b)from host", "(x\\xx)from xxxx"),
        ("\\) outside", "xx xxxxxxx"),
        ("(a\\ b)", "(x\\xx)"),
        ("(a\\\r\nb)", "(x\\x\nx)"),
        ("(a\\\r\\\nb)", "(x\\x\\xx)"),
        ("(a;) ; junk", "(xx) ; xxxx"),
        (")from host", "xxxxx xxxx"),
        ("(from host; ", "(xxxx xxxxx "),
        ("(a\"b)", "(xxx)"),
        ("(a\\\"b)", "(x\\xx)"),
    ] {
        assert_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn every_byte_in_plain_comment_and_escaped_comment_contexts() {
    for byte in 0..=255u8 {
        let plain = if b" \t\r\n<>[]:@,.;(".contains(&byte) {
            byte
        } else {
            b'x'
        };
        assert_case(&[byte], &[plain]);
        let comment = if b" \t\r\n<>[]:@,.()\\".contains(&byte) {
            byte
        } else {
            b'x'
        };
        assert_case(&[b'(', byte], &[b'(', comment]);
        let escaped = if byte == b'\\' { b'\\' } else { b'x' };
        assert_case(&[b'(', b'\\', byte], &[b'(', b'\\', escaped]);
    }
}

#[test]
fn accepted_date_suffixes_are_preserved_including_ignored_private_bytes() {
    for suffix in [
        b"15 Nov 2017 14:16:18 -0000\r\n".as_slice(),
        b"Wed, 15\r\n Nov 2017 14:16:18 +0100",
        b"15 Nov 2017",
        b"2017/11/15",
        b"15 Nov 2017 14:16:18 +0000 secret@example.com (private)\r\n\xff\0",
        b"15 Nov 2017 14:16:18 +0000 (unclosed private",
        b"private 15 Nov 2017 14:16:18 +0000",
        b"bogus; 15 Nov 2017",
    ] {
        assert!(DateTimeOffset::parse(suffix).is_ok(), "suffix {suffix:?}");
        let mut input = b"from host; \t\r\n ".to_vec();
        input.extend_from_slice(suffix);
        let mut expected = b"from xxxx; \t\r\n ".to_vec();
        expected.extend_from_slice(suffix);
        assert_case(&input, &expected);
    }
}

#[test]
fn rejected_dates_resume_masking_and_comments_do_not_trigger_date_recognition() {
    for (suffix, expected) in [
        ("not a date", "xxx x xxxx"),
        ("31 Feb 2017 14:16:18 +0000", "xx xxx xxxx xx:xx:xx xxxxx"),
        ("15 Nope 2017", "xx xxxx xxxx"),
        (";\t; nonsense; from host", ";\t; xxxxxxxx; from xxxx"),
        (
            "(15 Nov 2017 14:16:18 +0000) secret",
            "(xx xxx xxxx xx:xx:xx xxxxx) xxxxxx",
        ),
    ] {
        assert!(DateTimeOffset::parse(suffix.as_bytes()).is_err());
        let input = format!("; {suffix}");
        let expected = format!("; {expected}");
        assert_case(input.as_bytes(), expected.as_bytes());
    }
    assert_case(b"15 Nov 2017 14:16:18 -0000", b"xx xxx xxxx xx:xx:xx xxxxx");
    assert_case(b"(a; 15 Nov 2017) from host", b"(xx xx xxx xxxx) from xxxx");
    assert_case(b"; \t\r\n", b"; \t\r\n");
}

#[test]
fn lookahead_and_fresh_call_state_are_explicit() {
    assert_case(b"from", b"xxxx");
    assert_case(b"from ", b"from ");
    assert_case(b"; 15 Nov ", b"; xx xxx ");
    assert_case(b"; 15 Nov 2017", b"; 15 Nov 2017");
    assert_case(b"(", b"(");
    assert_case(b"from host", b"from xxxx");
    assert_case(b"", b"");
}

#[test]
fn deep_comments_are_iterative() {
    let depth = 16_384;
    let mut input = vec![b'('; depth];
    input.extend_from_slice(b"secret");
    input.extend(std::iter::repeat_n(b')', depth));
    input.extend_from_slice(b"from host ");
    let mut expected = input.clone();
    expected[depth..depth + 6].fill(b'x');
    let host = expected.len() - 5;
    expected[host..host + 4].fill(b'x');
    assert_eq!(received_header_value(&input), expected);

    input.truncate(depth + 6);
    expected.truncate(depth + 6);
    assert_eq!(received_header_value(&input), expected);
}

#[test]
fn received_raw_header_records_remain_unchanged() {
    let raw =
        b"Received: from host; 15 Nov 2017 14:16:18 -0000\nReceived: by second\r\nReceived: from";
    let headers = HeaderList::load(&mut raw.as_slice()).unwrap();
    let before = headers.clone();
    assert_eq!(
        received_header_value(headers.get("Received").unwrap().raw_value()),
        b" from xxxx; 15 Nov 2017 14:16:18 -0000\n",
    );
    assert_eq!(
        received_header_value(headers.iter().nth(1).unwrap().raw_value()),
        b" by xxxxxx\r\n",
    );
    assert_eq!(
        received_header_value(headers.iter().last().unwrap().raw_value()),
        b" xxxx",
    );
    assert_eq!(headers, before);
}

// Adapted from MimeKit's UnitTests/MimeAnonymizerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    HeaderList,
    anonymize::{
        address_header_value, content_disposition_value, content_type_value,
        unstructured_header_value,
    },
};

fn assert_case(mask: fn(&[u8]) -> Vec<u8>, input: &[u8], expected: &[u8]) {
    let original = input.to_vec();
    assert_eq!(expected.len(), input.len());
    assert_eq!(mask(input), expected, "input {input:?}");
    // Every truncation must retain the source's incomplete-token behavior,
    // including truncation inside UTF-8, an escape, or an encoded-word label.
    for end in 0..=input.len() {
        assert_eq!(
            mask(&input[..end]),
            &expected[..end],
            "end {end}, input {input:?}"
        );
    }
    assert_eq!(input, original);
}

fn assert_content_case(input: &[u8], expected: &[u8]) {
    // MimeKit uses identical parameter rules for both content-header kinds,
    // without validating the leading media type or disposition.
    assert_case(content_type_value, input, expected);
    assert_case(content_disposition_value, input, expected);
}

#[test]
fn canonical_content_disposition_values() {
    for (input, expected) in [
        (" attachment\r\n", " attachment\r\n"),
        (" attachment; \r\n", " attachment; \r\n"),
        (
            " attachment; filename=winmail.dat\r\n",
            " attachment; filename=xxxxxxxxxxx\r\n",
        ),
        (
            " attachment; filename=\"escaped \\\"quotes\\\".doc\";;\r\n",
            " attachment; filename=\"xxxxxxxx\\\"xxxxxx\\\"xxxx\";;\r\n",
        ),
        (
            " attachment;\r\n filename*0*=UTF-8''UnicodeFile;\n filename*1*=name.doc\r\n",
            " attachment;\r\n filename*0*=xxxxxxxxxxxxxxxxxx;\n filename*1*=xxxxxxxx\r\n",
        ),
        (
            " attachment;\r\n filename*0*=UTF-8''UnicodeFile;\n filename*1=\"name.doc\";\r\n",
            " attachment;\r\n filename*0*=xxxxxxxxxxxxxxxxxx;\n filename*1=\"xxxxxxxx\";\r\n",
        ),
        (
            " inline; filename; size=32767;\r\n",
            " inline; filename; size=xxxxx;\r\n",
        ),
        (
            " inline; filename*; filename*0; filename*1*; filename*2*=;\r\n",
            " inline; filename*; filename*0; filename*1*; filename*2*=;\r\n",
        ),
    ] {
        assert_content_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn canonical_content_type_values() {
    for (input, expected) in [
        (
            " application/octet-stream\r\n",
            " application/octet-stream\r\n",
        ),
        (
            " application/octet-stream; \r\n",
            " application/octet-stream; \r\n",
        ),
        (
            " text/plain; charset=us-ascii\r\n",
            " text/plain; charset=us-ascii\r\n",
        ),
        (
            " text/plain; charset=\"us-ascii\"\r\n",
            " text/plain; charset=\"us-ascii\"\r\n",
        ),
        (
            " text/plain; charset=us-ascii; format=flowed\r\n deslsp=yes; name=anonymize.txt\r\n",
            " text/plain; charset=us-ascii; format=flowed\r\n deslsp=yes; name=xxxxxxxxxxxxx\r\n",
        ),
        (
            " multipart/mixed;\r\n\tboundary=\"----=_NextPart_000_0031_01D36222.8A648550\"\r\n",
            " multipart/mixed;\r\n\tboundary=\"----=_NextPart_000_0031_01D36222.8A648550\"\r\n",
        ),
        (
            " multipart/mixed;\r\n\tboundary*=\"----=_NextPart_000_0031_01D36222.8A648550\"\r\n",
            " multipart/mixed;\r\n\tboundary*=\"----=_NextPart_000_0031_01D36222.8A648550\"\r\n",
        ),
        (
            " multipart/mixed;\r\n\tboundary*0*=US-ASCII''----=3D_NextPart_000_;\r\n\tboundary*1*=0031_01D36222.8A648550;\r\n",
            " multipart/mixed;\r\n\tboundary*0*=US-ASCII''----=3D_NextPart_000_;\r\n\tboundary*1*=0031_01D36222.8A648550;\r\n",
        ),
        (
            " application/octet-stream;\r\n name*0*=UTF-8''anonymize;\n name*1*=this.doc\r\n",
            " application/octet-stream;\r\n name*0*=xxxxxxxxxxxxxxxx;\n name*1*=xxxxxxxx\r\n",
        ),
        (
            " application/octet-stream;\r\n name*0*=UTF-8''UnicodeFile;\n name*1=\"name.doc\";\r\n",
            " application/octet-stream;\r\n name*0*=xxxxxxxxxxxxxxxxxx;\n name*1=\"xxxxxxxx\";\r\n",
        ),
        (
            " application/octet-stream;\r\n name=\"unterminated qstring value;\r\n",
            " application/octet-stream;\r\n name=\"xxxxxxxxxxxxxxxxxxxxxxxxxxx\r\n",
        ),
    ] {
        assert_content_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn preserved_parameter_names_are_ascii_case_insensitive_before_any_star() {
    for name in ["boundary", "charset", "delsp", "format"] {
        for spelling in [name.to_owned(), name.to_ascii_uppercase()] {
            for suffix in ["", "*", "*0", "*12*", "*invalid suffix", "**"] {
                let input = format!("x; {spelling}{suffix}=\"A B\\C\\\"D\r\n\tE\"");
                assert_content_case(input.as_bytes(), input.as_bytes());
            }
        }
    }
    let input = b"x;\tBoUnDaRy=ABC; ChArSeT=UTF-8; DeLsP=yes; FoRmAt=flowed";
    assert_content_case(input, input);
    for name in [
        "charset ",
        "char\tset",
        "charsetx",
        "xcharset",
        "format\r\n",
        "name",
        "filename",
        "size",
        "creation-date",
        "modification-date",
        "read-date",
        "x-private",
        "char\u{17f}et",
    ] {
        let input = format!("x; {name}=SECRET");
        let expected = format!("x; {name}=xxxxxx");
        assert_content_case(input.as_bytes(), expected.as_bytes());
    }
    assert_content_case(b"x; char\xffset=SECRET", b"x; char\xffset=xxxxxx");
}

#[test]
fn malformed_parameters_keep_source_name_accumulation_and_recovery() {
    for (input, expected) in [
        // A semicolon in NameStar does not clear the base name: char + set
        // becomes charset. A plain incomplete name does clear it.
        (
            "x; char*; set=private; name=secret",
            "x; char*; set=private; name=xxxxxx",
        ),
        ("x; char*;; \tset=private", "x; char*;; \tset=private"),
        ("x; char; set=private", "x; char; set=xxxxxxx"),
        (
            "x; filename*; charset=utf-8; charset=UTF-8",
            "x; filename*; charset=xxxxx; charset=UTF-8",
        ),
        ("x; charset *0*=utf-8", "x; charset *0*=xxxxx"),
        ("x; charset* ignored =utf-8", "x; charset* ignored =utf-8"),
        ("x; ;=secret; name=abc", "x; ;=xxxxxx; name=xxx"),
        (
            "x; charset=utf-8 filename=SECRET",
            "x; charset=utf-8 filename=SECRET",
        ),
        ("x; name=abc secret=XYZ", "x; name=xxx xxxxxxxxxx"),
        (
            "x; name=; charset=; name=END",
            "x; name=; charset=; name=xxx",
        ),
        (
            "x; filename=\"A; charset=UTF-8",
            "x; filename=\"xxxxxxxxxxxxxxxx",
        ),
        (
            "not a media type; UNKNOWN=secret",
            "not a media type; UNKNOWN=xxxxxx",
        ),
    ] {
        assert_content_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn content_parameter_quotes_and_escapes_do_not_decode_or_expose_masked_bytes() {
    for (input, expected) in [
        (
            "x; name=\"A\\B\\\"C\"; charset=UTF-8",
            "x; name=\"x\\x\\\"x\"; charset=UTF-8",
        ),
        (
            "x; name=\"A\\\\B\"; charset=UTF-8",
            "x; name=\"x\\\\x\"; charset=UTF-8",
        ),
        (
            "x; name=\"A\\\r\n\"; charset=UTF-8",
            "x; name=\"x\\\r\n\"; charset=UTF-8",
        ),
        (
            "x; name=\"A\r\n B\tC\"; size=123",
            "x; name=\"x\r\nxxxx\"; size=xxx",
        ),
        ("x; name=\\A; charset=UTF-8", "x; name=xx; charset=UTF-8"),
        (
            "x; name=abc\\; charset=UTF-8",
            "x; name=xxxx; charset=UTF-8",
        ),
        ("x; name=A\"B C\"D", "x; name=x\"xxx\"x"),
        ("x; name=\"=?u?q?a?=\"", "x; name=\"xxxxxxxxx\""),
        ("x; name==?u?q?a?=", "x; name=xxxxxxxxx"),
        ("x; name=\u{4f0a}\u{a0}", "x; name=xxxxx"),
    ] {
        assert_content_case(input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn every_byte_in_content_header_names_and_value_contexts() {
    for byte in 0..=255u8 {
        for prefix in [
            b"".as_slice(),
            b"x; n",
            b"x; name*",
            b"x; charset=",
            b"x; charset=\"",
        ] {
            let mut input = prefix.to_vec();
            input.push(byte);
            assert_content_case(&input, &input);
        }
        for (prefix, preserved) in [
            (b"x; name=".as_slice(), b" \t\r\n;\"".as_slice()),
            (b"x; name=\"".as_slice(), b"\r\n\"\\".as_slice()),
            (b"x; name=\"\\".as_slice(), b"\r\n\"\\".as_slice()),
        ] {
            let mut input = prefix.to_vec();
            input.push(byte);
            let mut expected = prefix.to_vec();
            expected.push(if preserved.contains(&byte) {
                byte
            } else {
                b'x'
            });
            assert_content_case(&input, &expected);
        }
    }
}

#[test]
fn content_header_calls_are_independent_and_leave_raw_records_unchanged() {
    assert_content_case(b"x; charset=", b"x; charset=");
    assert_content_case(b"x; name=SECRET", b"x; name=xxxxxx");
    assert_content_case(b"x; name=\"", b"x; name=\"");
    assert_content_case(b"x; charset=UTF-8", b"x; charset=UTF-8");
    assert_content_case(b"", b"");
    assert_content_case(b"\xff\0no parameters", b"\xff\0no parameters");

    let raw = b"Content-Type: text/plain; charset=UTF-8; name=abc\nContent-Disposition: attachment; filename=abc\r\nContent-Type: invalid; name=xyz";
    let headers = HeaderList::load(&mut raw.as_slice()).unwrap();
    let before = headers.clone();
    assert_eq!(
        content_type_value(headers.get("Content-Type").unwrap().raw_value()),
        b" text/plain; charset=UTF-8; name=xxx\n",
    );
    assert_eq!(
        content_disposition_value(headers.get("Content-Disposition").unwrap().raw_value()),
        b" attachment; filename=xxx\r\n",
    );
    assert_eq!(
        content_type_value(headers.iter().last().unwrap().raw_value()),
        b" invalid; name=xxx",
    );
    assert_eq!(headers, before);
}

#[test]
fn canonical_unstructured_values_and_malformed_encoded_words() {
    for (input, expected) in [
        (
            " This is a simple subject...",
            " xxxx xx x xxxxxx xxxxxxxxxx",
        ),
        (
            " blurdy bloop =??q?no_charset?= beep boop\r\n",
            " xxxxxx xxxxx =??x?xxxxxxxxxx?= xxxx xxxx\r\n",
        ),
        (
            " blurdy bloop =?iso-8859-1?q?this_is_english?= beep boop\r\n",
            " xxxxxx xxxxx =?iso-8859-1?q?xxxxxxxxxxxxxxx?= xxxx xxxx\r\n",
        ),
        (
            " I'm so happy! =?utf-8?b?8J+YgA==?= I love MIME so much =?utf-8?b?4p2k77iP4oCN8J+UpSE=?= Isn't it great?\r\n",
            " xxx xx xxxxxx =?utf-8?b?xxxxxxxx?= x xxxx xxxx xx xxxx =?utf-8?b?xxxxxxxxxxxxxxxxxxxx?= xxxxx xx xxxxx?\r\n",
        ),
        (
            " blurdy bloop =?=?q?this_is_english?= beep boop\r\n",
            " xxxxxx xxxxx =?=?x?xxxxxxxxxxxxxxx?= xxxx xxxx\r\n",
        ),
        (
            " blurdy bloop =?iso-8859-1??this_is_english?= beep boop\r\n",
            " xxxxxx xxxxx =?iso-8859-1??xxxxxxxxxxxxxxx?= xxxx xxxx\r\n",
        ),
        (
            " blurdy bloop =?iso-8859-1?=?this_is_english?= beep boop\r\n",
            " xxxxxx xxxxx =?iso-8859-1?=?xxxxxxxxxxxxxxx?= xxxx xxxx\r\n",
        ),
    ] {
        assert_case(
            unstructured_header_value,
            input.as_bytes(),
            expected.as_bytes(),
        );
    }
}

#[test]
fn canonical_address_values_preserve_structure_without_parsing_it() {
    for (input, expected) in [
        (
            " \":sysmail\"@  Some-Group. Some-Org,\r\n Muhammed.(I am  the greatest) Ali @(the)Vegas.WBA\r\n",
            " \"xxxxxxxx\"@  xxxxxxxxxx. xxxxxxxx,\r\n xxxxxxxx.(x xx  xxx xxxxxxxx) xxx @(xxx)xxxxx.xxx\r\n",
        ),
        (
            " Pete(A nice \\) chap) <pete(his account)@silly.test(his host)>\r\n",
            " xxxx(x xxxx \\) xxxx) <xxxx(xxx xxxxxxx)@xxxxx.xxxx(xxx xxxx)>\r\n",
        ),
        (
            " GNOME Hackers: Miguel de Icaza <miguel@gnome.org>, Havoc Pennington\r\n\t<hp@redhat.com>;, fejj@helixcode.com\r\n",
            " xxxxx xxxxxxx: xxxxxx xx xxxxx <xxxxxx@xxxxx.xxx>, xxxxx xxxxxxxxxx\r\n\t<xx@xxxxxx.xxx>;, xxxx@xxxxxxxxx.xxx\r\n",
        ),
        (
            " A Group(Some people):Chris Jones <c@(Chris's host.)public.example>, joe@example.org,\r\n John <jdoe@one.test> (my dear friend); (the end of the group)\r\n",
            " x xxxxx(xxxx xxxxxx):xxxxx xxxxx <x@(xxxxxxx xxxx.)xxxxxx.xxxxxxx>, xxx@xxxxxxx.xxx,\r\n xxxx <xxxx@xxx.xxxx> (xx xxxx xxxxxx); (xxx xxx xx xxx xxxxx)\r\n",
        ),
        (
            " \"Nathaniel S. Borenstein\" <nsb@thumper.bellcore.com>\r\n",
            " \"xxxxxxxxxxxxxxxxxxxxxxx\" <xxx@xxxxxxx.xxxxxxxx.xxx>\r\n",
        ),
        (
            " \"Nathaniel\r\nS. Borenstein\" <nsb@thumper.bellcore.com>\r\n",
            " \"xxxxxxxxx\r\nxxxxxxxxxxxxx\" <xxx@xxxxxxx.xxxxxxxx.xxx>\r\n",
        ),
        (
            " =?utf-8?b?2YfZhCDYqtiq2YPZhNmFINin2YTZhNi62Kkg2KfZhNil2YbYrNmE2YrYstmK2Kk=?=\r\n =?utf-8?b?IC/Yp9mE2LnYsdio2YrYqdif?= <do.you.speak@arabic.com>\r\n",
            " =?utf-8?b?xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx?=\r\n =?utf-8?b?xxxxxxxxxxxxxxxxxxxxxxxx?= <xx.xxx.xxxxx@xxxxxx.xxx>\r\n",
        ),
        (
            " =?utf-8?b?54uC44Gj44Gf44GT44Gu5LiW44Gn54u\r\nC44GG44Gq44KJ5rCX44Gv56K644GL44Gg?=\r\n =?utf-8?b?44CC?= <famous@quotes.ja>\r\n",
            " =?utf-8?b?xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\r\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx?=\r\n =?utf-8?b?xxxx?= <xxxxxx@xxxxxx.xx>\r\n",
        ),
        (
            " <<<user2@example.org>>>, <another@example.net, second@example.org>\r\n",
            " <<<xxxxx@xxxxxxx.xxx>>>, <xxxxxxx@xxxxxxx.xxx, xxxxxx@xxxxxxx.xxx>\r\n",
        ),
        (
            " <user@[domain.com\r\n <img src=x onerror=alert()>]>\r\n",
            " <xxxx@[xxxxxx.xxx\r\n <xxx xxx=x xxxxxxx=xxxxx()>]>\r\n",
        ),
        (
            " <user@[domain.com]\0\r\n]>\r\n",
            " <xxxx@[xxxxxx.xxx]x\r\n]>\r\n",
        ),
        (
            " \"User Name\" <user@example.com>, \"Unterminated qstring token\r\n",
            " \"xxxxxxxxx\" <xxxx@xxxxxxx.xxx>, \"xxxxxxxxxxxxxxxxxxxxxxxxxx\r\n",
        ),
        (
            " \"User Name\" <user@example.com>, (Unterminated comment\r\n",
            " \"xxxxxxxxx\" <xxxx@xxxxxxx.xxx>, (xxxxxxxxxxxx xxxxxxx\r\n",
        ),
        (
            " Display Name <escaped\\\"quoted@example.com>\r\n",
            " xxxxxxx xxxx <xxxxxxx\\\"xxxxxx@xxxxxxx.xxx>\r\n",
        ),
    ] {
        assert_case(address_header_value, input.as_bytes(), expected.as_bytes());
    }
}

#[test]
fn arbitrary_bytes_in_each_encoded_word_state_follow_the_source_transitions() {
    // Prefixes reach each of the nine states without quoting or escapes.
    let prefixes: [(&[u8], &[u8]); 9] = [
        (b"", b""),
        (b"=", b"="),
        (b"=?", b"=?"),
        (b"=?utf-8", b"=?utf-8"),
        (b"=?utf-8?", b"=?utf-8?"),
        (b"=?utf-8?q", b"=?utf-8?q"),
        (b"=?utf-8?q?", b"=?utf-8?q?"),
        (b"=?utf-8?q?data", b"=?utf-8?q?xxxx"),
        (b"=?utf-8?q?data?", b"=?utf-8?q?xxxx?"),
    ];
    for (state, (prefix, masked_prefix)) in prefixes.into_iter().enumerate() {
        for byte in 0..=255u8 {
            let mut input = prefix.to_vec();
            input.push(byte);
            let mut expected = masked_prefix.to_vec();
            let masked = if byte == b'=' {
                if state == 7 { b'x' } else { b'=' }
            } else if byte == b'?' || (2..=5).contains(&state) || b" \t\r\n".contains(&byte) {
                byte
            } else {
                b'x'
            };
            expected.push(masked);
            assert_eq!(
                unstructured_header_value(&input),
                expected,
                "state {state}, byte {byte}"
            );
        }
    }
}

#[test]
fn malformed_word_transitions_resume_ordinary_masking_without_dropping_bytes() {
    for (input, expected) in [
        ("=A", "=x"),
        ("==A", "==x"),
        ("?A", "?x"),
        ("=?utf-8?q?A=B?=", "=?utf-8?q?xxx?="),
        ("=?utf-8?q?A?B", "=?utf-8?q?x?x"),
        ("=?utf-8?q?A??B", "=?utf-8?q?x??x"),
        ("=?utf-8?q?A?==?u?Q?B?=", "=?utf-8?q?x?==?u?Q?x?="),
        ("=?utf-8?q?A\r\n\tB C?=", "=?utf-8?q?x\r\n\tx x?="),
        (
            "=?utf-8?long-encoding?DATA?=",
            "=?utf-8?long-encoding?xxxx?=",
        ),
    ] {
        assert_case(
            unstructured_header_value,
            input.as_bytes(),
            expected.as_bytes(),
        );
    }
    assert_case(address_header_value, b"=?u?q?A?@foo", b"=?u?q?x?@xxx");
    assert_case(unstructured_header_value, b"=?u?q?A?@foo", b"=?u?q?x?xxxx");
}

#[test]
fn address_quotes_escapes_and_paused_word_state_match_source_behavior() {
    for (input, expected) in [
        (b"\"A \tB\r\n C\"".as_slice(), b"\"xxxx\r\nxx\"".as_slice()),
        (b"\\A".as_slice(), b"\\A".as_slice()),
        (b"\\\\A".as_slice(), b"\\\\x".as_slice()),
        (b"\\\\\\A".as_slice(), b"\\\\\\A".as_slice()),
        (b"\"\\A \tB\"".as_slice(), b"\"\\Axxx\"".as_slice()),
        (
            b"=?utf\"SECRET\"-8?q?data?=".as_slice(),
            b"=?utf\"xxxxxx\"-8?q?xxxx?=".as_slice(),
        ),
        (
            b"=?u\\Ztf-8?q?data?=".as_slice(),
            b"=?u\\Ztf-8?q?xxxx?=".as_slice(),
        ),
        (
            b"\"=?utf-8?q?word?=\"".as_slice(),
            b"\"xxxxxxxxxxxxxxxx\"".as_slice(),
        ),
    ] {
        assert_case(address_header_value, input, expected);
    }
    assert_case(
        unstructured_header_value,
        b"\"A \tB\r\n C\"",
        b"xx \tx\r\n xx",
    );
    assert_case(unstructured_header_value, b"\\A", b"xx");
}

#[test]
fn every_address_byte_in_plain_quoted_and_escaped_contexts() {
    for byte in 0..=255u8 {
        let plain = if b" \t\r\n()<>[]:;@,.=?\"\\".contains(&byte) {
            byte
        } else {
            b'x'
        };
        assert_eq!(address_header_value(&[byte]), [plain], "byte {byte}");
        let quoted = if b"\r\n\"\\".contains(&byte) {
            byte
        } else {
            b'x'
        };
        assert_eq!(
            address_header_value(&[b'"', byte, b'"']),
            [b'"', quoted, b'"']
        );
        assert_eq!(
            address_header_value(&[b'\\', byte, b'A']),
            [b'\\', byte, b'x']
        );
    }
}

#[test]
fn unicode_and_invalid_utf8_are_masked_by_byte_except_in_preserved_contexts() {
    assert_case(
        address_header_value,
        "\u{4f0a}\u{662d}\u{5091}@\u{90f5}\u{4ef6}.\u{5546}\u{52d9}".as_bytes(),
        b"xxxxxxxxx@xxxxxx.xxxxxx",
    );
    assert_case(
        unstructured_header_value,
        "\u{a0}\u{2003}\u{1f408}".as_bytes(),
        b"xxxxxxxxx",
    );
    assert_case(address_header_value, b"\xff\xc0\x80\0\x0b\x0c", b"xxxxxx");
    // Potential labels and escaped bytes are copied even if they are invalid
    // UTF-8 or controls. This is syntax inspection, not a privacy guarantee.
    assert_case(
        unstructured_header_value,
        b"=?\xff\0?Q?payload?=",
        b"=?\xff\0?Q?xxxxxxx?=",
    );
    assert_case(address_header_value, b"\\\xff\\\0", b"\\\xff\\\0");
}

#[test]
fn each_call_starts_fresh_state_and_raw_header_records_remain_unchanged() {
    assert_eq!(unstructured_header_value(b"=?utf-8"), b"=?utf-8");
    assert_eq!(unstructured_header_value(b"SECRET"), b"xxxxxx");
    assert_eq!(address_header_value(b"\""), b"\"");
    assert_eq!(address_header_value(b" A"), b" x");
    assert!(address_header_value(b"").is_empty());
    assert!(unstructured_header_value(b"").is_empty());

    let raw = b"To: Alice <alice@example.com>\nSubject: =?utf-8?q?Hello?=\r\nSubject: second";
    let headers = HeaderList::load(&mut raw.as_slice()).unwrap();
    let before = headers.clone();
    assert_eq!(
        address_header_value(headers.get("To").unwrap().raw_value()),
        b" xxxxx <xxxxx@xxxxxxx.xxx>\n"
    );
    assert_eq!(
        unstructured_header_value(headers.get("Subject").unwrap().raw_value()),
        b" =?utf-8?q?xxxxx?=\r\n"
    );
    assert_eq!(
        unstructured_header_value(headers.iter().last().unwrap().raw_value()),
        b" xxxxxx"
    );
    assert_eq!(headers, before);
}

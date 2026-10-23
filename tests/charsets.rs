// Compatibility cases adapted from MimeKit's UnitTests/Utils/CharsetUtilsTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::borrow::Cow;

use mimecraft::charsets::{Charset, CharsetError, decode_best};

#[test]
fn mimekit_aliases_and_codepage_spellings() {
    let groups: &[(&str, &[&str])] = &[
        ("utf-8", &["utf-8", "utf8", "unicode", "cp65001"]),
        (
            "us-ascii",
            &[
                "ascii",
                "us-ascii",
                "ansi_x3.4-1968",
                "ansi_x3.4-1986",
                "iso-ir-6",
                "iso646-us",
                "iso_646.irv:1991",
                "csascii",
                "us",
                "cp20127",
            ],
        ),
        (
            "iso-8859-1",
            &[
                "iso8859-1",
                "iso8859_1",
                "iso-8859-1",
                "iso_8859_1",
                "iso88591",
                "latin1",
                "latin-1",
                "l1",
                "iso-ir-100",
                "csisolatin1",
                "iso_8859-1:1987",
                "ansi_x3.110-1983",
                "cp28591",
                "cp819",
                "ibm819",
            ],
        ),
        (
            "windows-1252",
            &[
                "windows-cp1252",
                "windows-1252",
                "cp-1252",
                "cp1252",
                "windowscp1252",
                "windows_cP1252",
                "windows1252",
                "ibm-1252",
                "x-cp1252",
            ],
        ),
        (
            "utf-16be",
            &[
                "iso10646",
                "iso-10646",
                "iso10646-1",
                "iso-10646-1",
                "utf-16be",
                "unicodefffe",
                "cp1201",
            ],
        ),
        ("utf-16", &["utf-16", "utf-16le", "cp1200"]),
        (
            "iso-2022-jp",
            &["iso2022-jp", "iso-2022-jp", "iso_2022_jp", "cp50220"],
        ),
        (
            "euc-kr",
            &[
                "ks_c_5601-1987",
                "ksc-5601-1987",
                "ksc-5601_1987",
                "ksc-5601",
                "5601",
                "ks_c_5861-1992",
                "ksc-5861-1992",
                "ksc-5861_1992",
                "euckr-0",
                "euc-kr",
                "cp949",
                "cp51949",
            ],
        ),
        (
            "big5",
            &[
                "big5",
                "big5-0",
                "big5-hkscs",
                "big5.eten-0",
                "big5hkscs-0",
                "cp950",
            ],
        ),
        (
            "gbk",
            &[
                "gb2312",
                "gb-2312",
                "gb2312-0",
                "gb2312-80",
                "gb2312.1980-0",
                "euc-cn",
                "gbk-0",
                "x-gbk",
                "gbk",
                "cp936",
                "cp51936",
            ],
        ),
        ("gb18030", &["gb18030-0", "gb18030", "cp54936"]),
        (
            "euc-jp",
            &["eucjp-0", "euc-jp", "ujis-0", "ujis", "cp51932"],
        ),
        (
            "shift_jis",
            &[
                "shift_jis",
                "jisx0208.1983-0",
                "jisx0212.1990-0",
                "pck",
                "cp932",
            ],
        ),
        ("macintosh", &["macintosh", "cp10000"]),
        ("ibm866", &["ibm866", "ibm-866", "cp866"]),
    ];
    for &(canonical, aliases) in groups {
        let expected = Charset::from_label(canonical).unwrap();
        for alias in aliases {
            for spelling in [alias.to_string(), alias.to_ascii_uppercase()] {
                let charset: Charset = spelling.parse().expect(&spelling);
                assert_eq!(charset, expected, "{spelling}");
                assert_eq!(charset.name(), canonical, "{spelling}");
                assert_eq!(charset.to_string(), canonical);
            }
        }
    }
}

#[test]
fn unsupported_labels_do_not_become_browser_replacement_encodings() {
    for label in [
        "",
        " ",
        " utf-8",
        "utf-8 ",
        "utf-8\n",
        "utf-8\0",
        "\u{feff}utf-8",
        "x-undefined",
        "cp999999",
        "cp",
        "iso",
        "ibm",
        "windows",
        "windows-",
        "iso-8859",
        "iso-BB59",
        "iso-8859-",
        "iso-8859-A",
        "iso-2022-US",
        "iso-4999-1",
        "iso-abcd-1",
        "cp-",
        "cp125x",
        "ibm-",
        "ibm437x",
        "windows_",
        "windows-125x",
        "cp2147483648",
        "cp99999999999999999999999",
        "cp-+1252",
        "cp- 1252",
        "utf-7",
        "utf-32",
        "utf-32le",
        "utf-32be",
        "cp65000",
        "cp12000",
        "cp12001",
        "replacement",
        "x-user-defined",
        "iso-2022-kr",
        "iso2022-kr",
        "iso_2022_kr",
        "hz-gb-2312",
        "hz-gb2312",
        "iso-2022-cn",
        "cp50225",
        "cp50221",
        "cp50222",
        "cp437",
        "ibm437",
        "x-mac-icelandic",
        "iso-8859-9",
        "iso88599",
        "latin5",
        "l5",
        "iso-8859-11",
        "tis-620",
        "iso885911",
        "cp28599",
        "cp28601",
    ] {
        assert_eq!(
            Charset::from_label(label),
            Err(CharsetError::UnsupportedLabel(label.to_owned())),
            "{label:?}",
        );
    }
}

#[test]
fn canonical_names_are_lowercase_and_resolve_back_to_the_same_codec() {
    for label in [
        "us-ascii",
        "iso-8859-1",
        "utf-8",
        "utf-16",
        "utf-16be",
        "big5",
        "euc-jp",
        "euc-kr",
        "gbk",
        "gb18030",
        "ibm866",
        "iso-2022-jp",
        "iso-8859-2",
        "iso-8859-3",
        "iso-8859-4",
        "iso-8859-5",
        "iso-8859-6",
        "iso-8859-7",
        "iso-8859-8",
        "iso-8859-8-i",
        "iso-8859-10",
        "iso-8859-13",
        "iso-8859-14",
        "iso-8859-15",
        "iso-8859-16",
        "koi8-r",
        "koi8-u",
        "macintosh",
        "x-mac-cyrillic",
        "windows-874",
        "windows-1250",
        "windows-1251",
        "windows-1252",
        "windows-1253",
        "windows-1254",
        "windows-1255",
        "windows-1256",
        "windows-1257",
        "windows-1258",
    ] {
        let charset = Charset::from_label(label).expect(label);
        assert_eq!(charset.name(), charset.name().to_ascii_lowercase());
        assert_eq!(
            Charset::from_label(charset.name()).unwrap(),
            charset,
            "{label}"
        );
        let bytes = charset.encode("hello").unwrap();
        assert_eq!(charset.decode(&bytes).unwrap(), "hello", "{label}");
    }
}

#[test]
fn ascii_latin1_and_windows_1252_are_distinct() {
    let windows = Charset::from_label("windows-1252").unwrap();
    assert_ne!(Charset::ASCII, Charset::LATIN1);
    assert_ne!(Charset::LATIN1, windows);
    for byte in 0..=255u8 {
        let input = [byte];
        assert_eq!(
            Charset::LATIN1.decode(&input).unwrap(),
            char::from(byte).to_string()
        );
        assert_eq!(
            Charset::LATIN1
                .encode(&char::from(byte).to_string())
                .unwrap()
                .as_ref(),
            input
        );
        if byte < 128 {
            assert_eq!(
                Charset::ASCII.decode(&input).unwrap(),
                char::from(byte).to_string()
            );
        } else {
            assert_eq!(
                Charset::ASCII.decode(&input),
                Err(CharsetError::InvalidBytes(Charset::ASCII))
            );
            assert_eq!(
                Charset::ASCII.decode_lossy(&input),
                (Cow::Borrowed("\u{fffd}"), true)
            );
        }
    }
    assert_eq!(
        Charset::LATIN1.decode(b"\x80\x91\x9f").unwrap(),
        "\u{80}\u{91}\u{9f}"
    );
    assert_eq!(
        windows.decode(b"\x80\x91\x9f").unwrap(),
        "\u{20ac}\u{2018}\u{178}"
    );
    assert_eq!(windows.encode("\u{20ac}").unwrap().as_ref(), b"\x80");
    assert!(Charset::LATIN1.encode("\u{20ac}").is_err());
    assert!(Charset::ASCII.encode("\u{e9}").is_err());
}

#[test]
fn known_legacy_byte_vectors() {
    let cases: &[(&str, &[u8], &str)] = &[
        (
            "windows-1251",
            b"\xcf\xf0\xe8\xe2\xe5\xf2",
            "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}",
        ),
        ("shift_jis", b"\x93\xfa\x96\x7b", "\u{65e5}\u{672c}"),
        ("euc-jp", b"\xc6\xfc\xcb\xdc", "\u{65e5}\u{672c}"),
        ("iso-2022-jp", b"\x1b$BF|K\\\x1b(B", "\u{65e5}\u{672c}"),
        ("gb2312", b"\xd6\xd0\xce\xc4", "\u{4e2d}\u{6587}"),
        ("big5", b"\xa4\xa4\xa4\xe5", "\u{4e2d}\u{6587}"),
        ("euc-kr", b"\xb0\xa1", "\u{ac00}"),
        ("iso-8859-15", b"\xa4", "\u{20ac}"),
        ("iso-8859-2", b"\xa1", "\u{104}"),
    ];
    for &(label, bytes, text) in cases {
        let charset = Charset::from_label(label).unwrap();
        assert_eq!(charset.decode(bytes).unwrap(), text, "{label}");
        assert_eq!(charset.encode(text).unwrap().as_ref(), bytes, "{label}");
        assert_eq!(charset.decode_lossy(bytes), (Cow::Borrowed(text), false));
    }
}

#[test]
fn strict_errors_and_explicit_lossy_replacement() {
    let utf16_le = Charset::from_label("utf-16le").unwrap();
    let utf16_be = Charset::from_label("utf-16be").unwrap();
    for (charset, bytes, expected) in [
        (Charset::UTF8, b"A\xffB".as_slice(), "A\u{fffd}B"),
        (Charset::ASCII, b"A\xffB".as_slice(), "A\u{fffd}B"),
        (utf16_le, b"A\0\x00\xd8".as_slice(), "A\u{fffd}"),
        (utf16_be, b"\0A\xd8\0".as_slice(), "A\u{fffd}"),
        (utf16_le, b"A".as_slice(), "\u{fffd}"),
        (
            Charset::from_label("shift_jis").unwrap(),
            b"\x82".as_slice(),
            "\u{fffd}",
        ),
        (
            Charset::from_label("iso-2022-jp").unwrap(),
            b"\x1b$".as_slice(),
            "\u{fffd}$",
        ),
    ] {
        assert_eq!(
            charset.decode(bytes),
            Err(CharsetError::InvalidBytes(charset))
        );
        assert_eq!(charset.decode_lossy(bytes), (Cow::Borrowed(expected), true));
    }
    let literal = "\u{fffd}";
    assert_eq!(
        Charset::UTF8.decode_lossy(literal.as_bytes()),
        (Cow::Borrowed(literal), false)
    );
    for label in [
        "us-ascii",
        "iso-8859-1",
        "windows-1252",
        "iso-8859-2",
        "iso-2022-jp",
    ] {
        let charset = Charset::from_label(label).unwrap();
        assert_eq!(
            charset.encode("hello \u{1f600}"),
            Err(CharsetError::UnmappableText(charset))
        );
    }
}

#[test]
fn explicit_decoding_preserves_boms_and_does_not_switch_charset() {
    let utf16_le = Charset::from_label("utf-16le").unwrap();
    let utf16_be = Charset::from_label("utf-16be").unwrap();
    assert_eq!(Charset::UTF8.decode(b"\xef\xbb\xbfA").unwrap(), "\u{feff}A");
    assert_eq!(utf16_le.decode(b"\xff\xfeA\0").unwrap(), "\u{feff}A");
    assert_eq!(utf16_be.decode(b"\xfe\xff\0A").unwrap(), "\u{feff}A");
    assert_eq!(utf16_le.decode(b"\xfe\xff\0A").unwrap(), "\u{fffe}\u{4100}");
    assert_eq!(
        Charset::LATIN1.decode(b"\xef\xbb\xbfA").unwrap(),
        "\u{ef}\u{bb}\u{bf}A"
    );
    assert!(Charset::UTF8.decode(b"\xff\xfeA\0").is_err());
    for charset in [Charset::UTF8, utf16_le, utf16_be] {
        let bytes = charset.encode("A\u{1f600}").unwrap();
        assert_eq!(charset.decode(&bytes).unwrap(), "A\u{1f600}");
        assert!(!bytes.starts_with(b"\xef\xbb\xbf"));
        assert!(!bytes.starts_with(b"\xff\xfe"));
        assert!(!bytes.starts_with(b"\xfe\xff"));
    }
    assert_eq!(
        utf16_le.encode("A\u{1f600}").unwrap().as_ref(),
        b"A\0\x3d\xd8\0\xde"
    );
    assert_eq!(
        utf16_be.encode("A\u{1f600}").unwrap().as_ref(),
        b"\0A\xd8\x3d\xde\0"
    );
}

#[test]
fn mimekit_guessing_prefers_utf8_over_configured_legacy_charsets() {
    for text in [
        "",
        "plain ASCII",
        "Ren\u{e9}e M\u{fc}ller",
        "\u{674e}\u{5c0f}\u{9f8d}",
        "\0\u{7f}\u{80}\u{7ff}\u{800}\u{d7ff}\u{e000}\u{ffff}\u{10000}\u{10ffff}",
    ] {
        let input = [b"\xff".as_slice(), text.as_bytes(), b"\xff"].concat();
        for preferred in [
            None,
            Some(Charset::UTF8),
            Some(Charset::LATIN1),
            Some(Charset::from_label("windows-1252").unwrap()),
            Some(Charset::from_label("gbk").unwrap()),
            Some(Charset::from_label("shift_jis").unwrap()),
        ] {
            let (decoded, charset) = decode_best(&input[1..input.len() - 1], preferred);
            assert_eq!(decoded, text);
            assert_eq!(charset, Charset::UTF8);
            assert!(matches!(decoded, Cow::Borrowed(_)));
        }
    }
}

#[test]
fn mimekit_invalid_utf8_fallback_vectors() {
    let cases: &[(&[u8], &str)] = &[
        (b"\xc3", "\u{c3}"),
        (b"\xc0\xaf", "\u{c0}\u{af}"),
        (b"\xed\xa0\x80", "\u{ed}\u{a0}\u{80}"),
        (b"\xf4\x90\x80\x80", "\u{f4}\u{90}\u{80}\u{80}"),
        (b"\xc3\xa9\xff", "\u{c3}\u{a9}\u{ff}"),
        (b"He\xc3(o", "He\u{c3}(o"),
    ];
    for &(bytes, expected) in cases {
        for preferred in [
            None,
            Some(Charset::UTF8),
            Some(Charset::LATIN1),
            Some(Charset::ASCII),
        ] {
            assert_eq!(
                decode_best(bytes, preferred),
                (Cow::Borrowed(expected), Charset::LATIN1)
            );
        }
    }
    let windows = Charset::from_label("windows-1252").unwrap();
    assert_eq!(
        decode_best(b"\x80", Some(windows)),
        (Cow::Borrowed("\u{20ac}"), windows)
    );
    let gbk = Charset::from_label("gb2312").unwrap();
    assert_eq!(
        decode_best(b"\xd6\xd0\xce\xc4", Some(gbk)),
        (Cow::Borrowed("\u{4e2d}\u{6587}"), gbk)
    );
    let sjis = Charset::from_label("shift_jis").unwrap();
    assert_eq!(
        decode_best(b"\x82", Some(sjis)),
        (Cow::Borrowed("\u{82}"), Charset::LATIN1)
    );
}

#[test]
fn borrowed_results_and_complete_buffer_state() {
    for charset in [Charset::ASCII, Charset::LATIN1, Charset::UTF8] {
        let input = b"plain text";
        let decoded = charset.decode(input).unwrap();
        assert!(matches!(decoded, Cow::Borrowed(_)));
        assert_eq!(decoded.as_ptr(), input.as_ptr());
        let encoded = charset.encode("plain text").unwrap();
        assert!(matches!(encoded, Cow::Borrowed(_)));
    }
    let iso2022 = Charset::from_label("iso-2022-jp").unwrap();
    assert_eq!(iso2022.decode(b"\x1b$BF|").unwrap(), "\u{65e5}");
    assert_eq!(iso2022.decode(b"K\\").unwrap(), "K\\");
    assert_eq!(iso2022.decode(b"\x1b$BF|K\\").unwrap(), "\u{65e5}\u{672c}");
}

#[test]
fn generated_unicode_round_trips_in_utf_encodings() {
    let utf16_le = Charset::from_label("utf-16le").unwrap();
    let utf16_be = Charset::from_label("utf-16be").unwrap();
    let mut state = 0x7a9d04c1u32;
    for _ in 0..1000 {
        let text: String = (0..32)
            .filter_map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                char::from_u32(state % 0x110000)
            })
            .collect();
        for charset in [Charset::UTF8, utf16_le, utf16_be] {
            let bytes = charset.encode(&text).unwrap();
            assert_eq!(charset.decode(&bytes).unwrap(), text);
            assert_eq!(
                charset.decode_lossy(&bytes),
                (Cow::Borrowed(text.as_str()), false)
            );
        }
    }
}

#[test]
fn generated_bytes_preserve_explicit_fallback_invariants() {
    let mut state = 0x17391a2du32;
    let preferred = Charset::from_label("shift_jis").unwrap();
    for length in (0..128).cycle().take(10_000) {
        let input: Vec<_> = (0..length)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        let (text, selected) = decode_best(&input, Some(preferred));
        assert_eq!(selected.decode(&input).unwrap(), text);
        let (lossy, errors) = selected.decode_lossy(&input);
        assert!(!errors);
        assert_eq!(lossy, text);
        let latin1 = Charset::LATIN1.decode(&input).unwrap();
        assert_eq!(Charset::LATIN1.encode(&latin1).unwrap().as_ref(), input);
    }
}

#[test]
fn error_values_implement_standard_error() {
    for error in [
        CharsetError::UnsupportedLabel("x-undefined".into()),
        CharsetError::InvalidBytes(Charset::UTF8),
        CharsetError::UnmappableText(Charset::ASCII),
    ] {
        let standard: &dyn std::error::Error = &error;
        assert!(!standard.to_string().is_empty());
        assert!(standard.source().is_none());
        assert_eq!(error.clone(), error);
    }
}

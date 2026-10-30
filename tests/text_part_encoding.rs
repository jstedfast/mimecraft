// Adapted from MimeKit's UnitTests/TextPartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MimeContent, MimeContentStorage, MimeEntity, MimeMessage,
    MimeParser, MimePart, TextFormat, TextPart, TextPartError,
    charsets::{Charset, CharsetError},
    encodings::Base64Encoder,
    text::TextEncodingConfidence::{Certain, Tentative},
};

fn part<R: Read + Seek>(content_type: &str, source: R, encoding: ContentEncoding) -> TextPart<R> {
    let mut headers = HeaderList::new();
    headers.add(Header::parse(format!("Content-Type: {content_type}\r\n").as_bytes()).unwrap());
    TextPart::from_part(MimePart::from_parts(
        headers,
        MimeContent::with_encoding(source, encoding),
    ))
    .unwrap()
}

fn text_part(content_type: &str, bytes: &[u8]) -> TextPart {
    part(
        content_type,
        Cursor::new(bytes.to_vec()),
        ContentEncoding::Unspecified,
    )
}

#[test]
fn empty_streams_are_not_mimekits_null_content() {
    for content_type in ["text/plain", "text/html", "text/html; charset=flubber"] {
        let mut body = text_part(content_type, b"");
        assert_eq!(body.detect_encoding().unwrap(), None);
        assert_eq!(
            body.text_and_charset().unwrap(),
            (String::new(), Charset::UTF8)
        );
        assert_eq!(body.text_with_charset(Charset::ASCII).unwrap(), "");
    }
    let mut body = text_part("text/html; charset=us-ascii", b"");
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::ASCII, Certain))
    );
    assert_eq!(
        body.text_and_charset().unwrap(),
        (String::new(), Charset::ASCII)
    );
}

#[test]
fn upstream_latin1_recovery_with_missing_or_invalid_charset() {
    for content_type in ["text/plain", "text/plain; charset=flubber"] {
        let mut body = text_part(content_type, b"This is some L\xe5t\xedn1 text.");
        assert_eq!(body.detect_encoding().unwrap(), None);
        assert_eq!(
            body.text_and_charset().unwrap(),
            (
                "This is some L\u{e5}t\u{ed}n1 text.".into(),
                Charset::LATIN1
            )
        );
    }
}

#[test]
fn upstream_boms_precede_html_metadata_and_are_preserved_in_text() {
    let html = "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-1\" /></head><body><p>Hello, world!</p></body></html>";
    for (label, bom) in [
        ("utf-8", &b"\xef\xbb\xbf"[..]),
        ("utf-16", &b"\xff\xfe"[..]),
        ("utf-16be", &b"\xfe\xff"[..]),
    ] {
        let charset = Charset::from_label(label).unwrap();
        for (content_type, text) in [
            ("text/html", html),
            ("text/plain", "This is some text.\r\nThis is line #2.\n"),
            ("text/html; charset=x-unknown", html),
        ] {
            let mut bytes = bom.to_vec();
            bytes.extend_from_slice(&charset.encode(text).unwrap());
            let mut body = text_part(content_type, &bytes);
            assert_eq!(body.detect_encoding().unwrap(), Some((charset, Certain)));
            assert_eq!(
                body.text_and_charset().unwrap(),
                (format!("\u{feff}{text}"), charset),
            );
            assert_eq!(body.part().content().get_ref().position(), 0);
        }
    }
}

#[test]
fn upstream_html_meta_detection_cases() {
    for (attributes, expected) in [
        (
            "http-equiv=\"Content-Type\" content=\"text/html; charset=euc-kr\"",
            Some("euc-kr"),
        ),
        (
            "http-equiv=\"Content-Type\" content=\"text/html; charset=x-unknown\"",
            None,
        ),
        (
            "http-equiv=\"Content-Type\" content=\"text/html; charset=x-user-defined\"",
            Some("windows-1252"),
        ),
        (
            "http-equiv=\"Content-Type\" content=\"text/html; charset=utf-16\"",
            Some("utf-8"),
        ),
        ("charset=\"x-user-defined\"", Some("windows-1252")),
        (
            "http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-1\" charset=\"windows-1252\"",
            Some("windows-1252"),
        ),
        (
            "charset=\"windows-1252\" http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-1\"",
            Some("windows-1252"),
        ),
        ("http-equiv=\"Content-Type\" content=\"text/html\"", None),
        (
            "http-equiv=\"Content-Type\" content=\"this is invalid\"",
            None,
        ),
        ("http-equiv=\"Content-Type\" content", None),
        ("http-equiv=\"Content-Type\"", None),
        ("http-equiv=\"Content-Transfer-Encoding\"", None),
        ("http-equiv", None),
        ("data=\"metadata\"", None),
    ] {
        let html = format!(
            "<html><head><meta {attributes} /></head><body><p>Hello, world!</p></body></html>"
        );
        let mut body = text_part("text/html", html.as_bytes());
        assert_eq!(
            body.detect_encoding().unwrap(),
            expected.map(|label| (Charset::from_label(label).unwrap(), Tentative)),
            "{attributes}"
        );
        assert_eq!(body.text().unwrap(), html);
    }
    for html in [
        "<html><head></head><body><p>Hello, world!</p></body></html>",
        "<html /><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=euc-kr\" /></head><body><p>Hello, world!</p></body></html>",
        "<html><head /><meta http-equiv=\"Content-Type\" content=\"text/html; charset=euc-kr\" /></head><body><p>Hello, world!</p></body></html>",
    ] {
        assert_eq!(
            text_part("text/html", html.as_bytes())
                .detect_encoding()
                .unwrap(),
            None
        );
    }
}

#[test]
fn meta_attribute_order_duplicates_and_pragma_rules() {
    for (attributes, expected) in [
        ("CHARSET=' Windows-1252 '", Some("windows-1252")),
        ("charset='windows&#45;1252'", Some("windows-1252")),
        ("charset=utf-16be", Some("utf-8")),
        ("charset=utf-16le", Some("utf-8")),
        ("charset='unicodeFFFE'", Some("utf-8")),
        ("charset=x-user-defined", Some("windows-1252")),
        ("charset=''", None),
        ("charset='   '", None),
        ("content='text/html; charset=windows-1252'", None),
        (
            "content='text/html; charset=windows-1252' http-equiv=content-type",
            Some("windows-1252"),
        ),
        (
            "http-equiv=' Content-Type' content='text/html; charset=windows-1252'",
            None,
        ),
        (
            "http-equiv='Content-Type ' content='text/html; charset=windows-1252'",
            None,
        ),
        (
            "http-equiv=other content='text/html; charset=windows-1252'",
            None,
        ),
        (
            "http-equiv=Content-Type content='text/html; charset=\"\"'",
            None,
        ),
        ("charset=x-unknown charset=windows-1252", None),
        (
            "charset=windows-1252 charset=x-unknown",
            Some("windows-1252"),
        ),
        ("charset charset=windows-1252", Some("windows-1252")),
        ("charset='' charset=windows-1252", None),
        (
            "http-equiv http-equiv=Content-Type content='text/html; charset=windows-1252'",
            Some("windows-1252"),
        ),
        (
            "http-equiv=other http-equiv=Content-Type content='text/html; charset=windows-1252'",
            None,
        ),
        (
            "content content='text/html; charset=windows-1252' http-equiv=Content-Type",
            Some("windows-1252"),
        ),
        (
            "content='' content='text/html; charset=windows-1252' http-equiv=Content-Type",
            None,
        ),
        (
            "charset=windows-1252 content='text/html; charset=iso-8859-1'",
            Some("windows-1252"),
        ),
        (
            "content='text/html; charset=iso-8859-1' charset=windows-1252",
            Some("windows-1252"),
        ),
        (
            "content='text/html; charset=iso-8859-1' charset=x-unknown http-equiv=Content-Type",
            None,
        ),
        (
            "charset='' content='text/html; charset=iso-8859-1' http-equiv=Content-Type",
            Some("iso-8859-1"),
        ),
    ] {
        let html = format!("<html><head><meta {attributes}></head><body>\u{80}</body></html>");
        let bytes = Charset::LATIN1.encode(&html).unwrap();
        for content_type in ["text/html", "text/html; charset=flubber"] {
            let mut body = text_part(content_type, &bytes);
            assert_eq!(
                body.detect_encoding().unwrap(),
                expected.map(|label| (Charset::from_label(label).unwrap(), Tentative)),
                "{content_type}: {attributes}"
            );
            let decoded = body.text().unwrap();
            let expected = if expected == Some("windows-1252") {
                html.replace('\u{80}', "\u{20ac}")
            } else {
                html.clone()
            };
            assert_eq!(decoded, expected, "{attributes}");
        }
    }
}

#[test]
fn scanner_requires_explicit_structure_and_respects_tokenizer_context() {
    for (html, detected) in [
        ("<meta charset=windows-1252>", false),
        ("<head><meta charset=windows-1252>", false),
        ("<html><meta charset=windows-1252>", false),
        ("</html><html><head><meta charset=windows-1252>", false),
        ("<html></head><head><meta charset=windows-1252>", false),
        ("<body><html><head><meta charset=windows-1252>", false),
        ("<html><head><body><meta charset=windows-1252>", false),
        ("<html><head></body><meta charset=windows-1252>", false),
        ("<html><head></meta charset=windows-1252>", false),
        ("<html><head><!-- <meta charset=windows-1252> -->", false),
        (
            "<html><head><script><meta charset=windows-1252></script>",
            false,
        ),
        (
            "<html><head><style><meta charset=windows-1252></style>",
            false,
        ),
        (
            "<html><head><title><meta charset=windows-1252></title>",
            false,
        ),
        (
            "<html><head><noscript><meta charset=windows-1252></noscript>",
            false,
        ),
        ("<html><head><meta charset=windows-1252", false),
        ("<html><head><meta charset='windows-1252", false),
        ("<html><head><meta charset='windows-1252'", false),
        ("<html><head><meta charset=utf-", false),
        (
            "<html><head><meta charset=x-unknown><meta charset=windows-1252>",
            true,
        ),
        (
            "<html><head><meta charset=windows-1252><meta charset=utf-8>",
            true,
        ),
        (
            "<html><head><script>\"<body>\"</script><meta charset=windows-1252>",
            true,
        ),
        (
            "<html><head><!-- <body> --><meta charset=windows-1252>",
            true,
        ),
        // MimeKit's prescan does not leave Head on an end-head tag.
        ("<html><head></head><meta charset=windows-1252>", true),
    ] {
        let expected = detected.then(|| (Charset::from_label("windows-1252").unwrap(), Tentative));
        assert_eq!(
            text_part("text/html", html.as_bytes())
                .detect_encoding()
                .unwrap(),
            expected,
            "{html}"
        );
    }
}

#[test]
fn mime_header_precedes_bom_and_meta_and_only_html_sniffs_meta() {
    let bytes = b"\xef\xbb\xbf<html><head><meta charset=windows-1252></head><body>\x80</body>";
    let mut body = text_part("text/html; charset=iso-8859-1", bytes);
    let headers = body.part().headers().clone();
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::LATIN1, Certain))
    );
    assert_eq!(body.text().unwrap(), Charset::LATIN1.decode(bytes).unwrap());
    assert_eq!(body.part().headers(), &headers);
    assert_eq!(body.part().content().get_ref().get_ref(), bytes);
    for content_type in ["text/plain", "text/enriched", "text/rtf", "text/x-custom"] {
        let mut body = text_part(content_type, &bytes[3..]);
        assert_eq!(body.detect_encoding().unwrap(), None);
        assert!(body.text().unwrap().ends_with("\u{80}</body>"));
    }
    let mut body = text_part("text/html; charset=windows-1252", b"\xff\xfeH\0i\0");
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::from_label("windows-1252").unwrap(), Certain))
    );
    assert_eq!(body.text().unwrap(), "\u{ff}\u{fe}H\0i\0");
}

#[test]
fn recovery_policy_depends_on_encoding_source_and_override_stays_strict() {
    for (content_type, bytes, expected, used) in [
        ("text/plain", &b"caf\xe9"[..], "caf\u{e9}", Charset::LATIN1),
        (
            "text/plain; charset=flubber",
            b"caf\xe9",
            "caf\u{e9}",
            Charset::LATIN1,
        ),
        (
            "text/plain; charset=utf-8",
            b"caf\xe9",
            "caf\u{fffd}",
            Charset::UTF8,
        ),
        (
            "text/plain; charset=us-ascii",
            b"caf\xe9",
            "caf\u{fffd}",
            Charset::ASCII,
        ),
        (
            "text/plain",
            b"\xef\xbb\xbfcaf\xe9",
            "\u{ef}\u{bb}\u{bf}caf\u{e9}",
            Charset::LATIN1,
        ),
        (
            "text/plain",
            b"\xff\xfea\0\xff",
            "\u{feff}a\u{fffd}",
            Charset::from_label("utf-16").unwrap(),
        ),
        (
            "text/plain",
            b"\xfe\xff\0a\xff",
            "\u{feff}a\u{fffd}",
            Charset::from_label("utf-16be").unwrap(),
        ),
    ] {
        let mut body = text_part(content_type, bytes);
        assert_eq!(body.text_and_charset().unwrap(), (expected.into(), used));
        assert!(matches!(
            body.text_with_charset(Charset::UTF8),
            Err(TextPartError::Charset(CharsetError::InvalidBytes(_)))
        ));
    }
    for label in ["utf-8", "utf-16", "utf-16be"] {
        let mut bytes = format!("<html><head><meta charset={label}></head><body>").into_bytes();
        bytes.push(0xe9);
        let mut body = text_part("text/html", &bytes);
        assert_eq!(
            body.detect_encoding().unwrap(),
            Some((Charset::UTF8, Tentative))
        );
        let (text, used) = body.text_and_charset().unwrap();
        if label == "utf-8" {
            assert_eq!(used, Charset::UTF8);
            assert!(text.ends_with('\u{fffd}'));
        } else {
            assert_eq!(used, Charset::LATIN1);
            assert!(text.ends_with('\u{e9}'));
        }
        assert!(body.text_with_charset(Charset::UTF8).is_err());
    }
}

#[test]
fn limit_counts_decoded_bytes_not_encoded_bytes_or_unicode_characters() {
    let meta = b"<meta charset=windows-1252>";
    for (end, detected) in [
        (1023, true),
        (1024, true),
        (1025, false),
        (1026, false),
        (1024 + meta.len(), false),
    ] {
        let mut bytes = b"<html><head>".to_vec();
        // Non-ASCII Latin-1 bytes expand to two UTF-8 bytes during tokenization.
        bytes.resize(end - meta.len(), 0xe9);
        bytes.extend_from_slice(meta);
        bytes.extend_from_slice(b"</head><body>\x80</body></html>");
        let mut base64 = Vec::new();
        let mut encoder = Base64Encoder::new();
        encoder.encode(&bytes, &mut base64);
        encoder.finish(&mut base64);
        let quoted: Vec<u8> = bytes
            .iter()
            .flat_map(|byte| format!("={byte:02X}").into_bytes())
            .collect();
        for (source, encoding) in [
            (&bytes, ContentEncoding::Unspecified),
            (&base64, ContentEncoding::Base64),
            (&quoted, ContentEncoding::QuotedPrintable),
        ] {
            let mut body = part("text/html", Cursor::new(source), encoding);
            body.part_mut()
                .set_content_transfer_encoding(ContentEncoding::SevenBit);
            assert_eq!(
                body.detect_encoding().unwrap(),
                detected.then(|| (Charset::from_label("windows-1252").unwrap(), Tentative)),
                "end={end}, encoding={encoding:?}"
            );
            let decoded = body.text().unwrap();
            assert!(decoded.ends_with(if detected {
                "\u{20ac}</body></html>"
            } else {
                "\u{80}</body></html>"
            }));
        }
    }
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    fail_at: Option<u64>,
    fail_seek: bool,
    interrupt: bool,
    reads: usize,
    seeks: usize,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: Cursor::new(bytes.to_vec()),
            fail_at: None,
            fail_seek: false,
            interrupt: true,
            reads: 0,
            seeks: 0,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self
            .fail_at
            .is_some_and(|position| self.bytes.position() >= position)
        {
            return Err(io::Error::other("read failed"));
        }
        let length = buffer.len().min(1);
        self.bytes.read(&mut buffer[..length])
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek {
            return Err(io::Error::other("seek failed"));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn short_interrupted_reads_and_prefix_limits() {
    for (bytes, limit, expected) in [
        (&b"\xef\xbb\xbf"[..], 3, Some((Charset::UTF8, Certain))),
        (
            &b"\xff\xfeA"[..],
            3,
            Some((Charset::from_label("utf-16").unwrap(), Certain)),
        ),
        (
            &b"\xfe\xff\0"[..],
            3,
            Some((Charset::from_label("utf-16be").unwrap(), Certain)),
        ),
        (
            &b"\xff\xfe"[..],
            3,
            Some((Charset::from_label("utf-16").unwrap(), Certain)),
        ),
        (&b"\xef\xbb"[..], 3, None),
        (&b"x"[..], 3, None),
    ] {
        let mut source = Source::new(bytes);
        source.fail_at = Some(limit);
        let mut body = part("text/html", source, ContentEncoding::Unspecified);
        assert_eq!(body.detect_encoding().unwrap(), expected);
    }
    let mut bytes = b"<html><head><meta charset=windows-1252>".to_vec();
    bytes.resize(2048, b' ');
    let mut source = Source::new(&bytes);
    source.fail_at = Some(1024);
    source.bytes.set_position(100);
    let mut body = part("text/html", source, ContentEncoding::Unspecified);
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::from_label("windows-1252").unwrap(), Tentative))
    );
    assert_eq!(body.part().content().get_ref().bytes.position(), 1024);
    assert!(matches!(body.text(), Err(TextPartError::Io(_))));

    let mut source = Source::new(b"abc suffix");
    source.fail_at = Some(3);
    let mut body = part("text/plain", source, ContentEncoding::Unspecified);
    assert_eq!(body.detect_encoding().unwrap(), None);
    assert_eq!(body.part().content().get_ref().bytes.position(), 3);

    let mut body = part(
        "text/html",
        Source::new(b"<html><head><meta charset=windows-1252><body>\x80"),
        ContentEncoding::Unspecified,
    );
    assert!(body.text().unwrap().ends_with('\u{20ac}'));
    assert_eq!(body.part().content().get_ref().bytes.position(), 0);
}

#[test]
fn metadata_precedes_io_and_source_failures_are_not_no_detection() {
    let mut source = Source::new(b"ignored");
    source.fail_seek = true;
    let mut body = part(
        "text/html; charset=utf-8",
        source,
        ContentEncoding::Unspecified,
    );
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::UTF8, Certain))
    );
    assert_eq!(body.part().content().get_ref().seeks, 0);
    assert_eq!(body.part().content().get_ref().reads, 0);
    assert!(matches!(body.text(), Err(TextPartError::Io(_))));

    for (fail_seek, fail_at) in [(true, None), (false, Some(0)), (false, Some(8))] {
        let mut source = Source::new(b"<html><head>");
        source.fail_seek = fail_seek;
        source.fail_at = fail_at;
        let mut body = part(
            "text/html; charset=flubber",
            source,
            ContentEncoding::Unspecified,
        );
        assert!(matches!(body.detect_encoding(), Err(TextPartError::Io(_))));
    }
    for content_type in [None, Some("invalid"), Some("image/png")] {
        let mut body = part(
            "text/html",
            Source::new(b"ignored"),
            ContentEncoding::Unspecified,
        );
        body.part_mut().headers_mut().remove_all("Content-Type");
        if let Some(value) = content_type {
            body.part_mut()
                .headers_mut()
                .add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
        }
        assert!(body.detect_encoding().is_err());
        assert!(body.text().is_err());
        assert_eq!(body.part().content().get_ref().seeks, 0);
        assert_eq!(body.part().content().get_ref().reads, 0);
    }
}

#[test]
fn automatic_message_and_entity_access_share_html_sniffing_and_preserve_framing() {
    let html = b"<html><head><meta charset=windows-1252></head><body>\x80</body>";
    let mut wire = b"Subject: retained\r\nContent-Type: text/html\r\n\r\n".to_vec();
    wire.extend_from_slice(html);
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let mut parser = MimeParser::with_storage(Cursor::new(&wire), storage).unwrap();
        let mut message = parser.parse_message().unwrap();
        drop(parser);
        let expected = Charset::from_label("windows-1252")
            .unwrap()
            .decode(html)
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                message.text_body(TextFormat::Html).unwrap().as_deref(),
                Some(expected.as_ref())
            );
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_eq!(output, wire);
        }
    }
    let mut body: MimeEntity = text_part("text/html", html).into();
    assert!(
        body.text_body(TextFormat::Html)
            .unwrap()
            .unwrap()
            .contains('\u{20ac}')
    );
    let mut message = MimeMessage::with_body(body);
    assert!(
        message
            .text_body(TextFormat::Html)
            .unwrap()
            .unwrap()
            .contains('\u{20ac}')
    );
}

#[test]
fn direct_text_part_access_preserves_parsed_framing_and_rechecks_raw_headers() {
    for wire in [
        &b"Content-Type: text/html\r\nX-Retained: value\r\n"[..],
        &b"Content-Type: text/html\r\n\r\n<html><head><meta charset=windows-1252><body>\x80"[..],
    ] {
        let MimeEntity::Part(parsed) = MimeEntity::load(&mut &wire[..]).unwrap() else {
            panic!("expected a text leaf");
        };
        let mut body = TextPart::from_part(parsed).unwrap();
        body.detect_encoding().unwrap();
        body.text_and_charset().unwrap();
        let mut output = Vec::new();
        body.part_mut().write_to(&mut output).unwrap();
        assert_eq!(output, wire);
    }

    let bytes = b"<html><head><meta charset=windows-1252><body>\x80";
    let mut body = text_part("text/html; charset=flubber", bytes);
    body.part_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Type: text/html; charset=iso-8859-1").unwrap());
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::from_label("windows-1252").unwrap(), Tentative))
    );
    assert!(body.text().unwrap().ends_with('\u{20ac}'));
    body.part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: text/html; charset=iso-8859-1").unwrap())
        .unwrap();
    assert_eq!(
        body.detect_encoding().unwrap(),
        Some((Charset::LATIN1, Certain))
    );
    assert!(body.text().unwrap().ends_with('\u{80}'));
}

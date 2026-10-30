// Adapted from MimeKit's UnitTests/Text/TextPreviewerTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentEncoding, ContentType, Header, HeaderList, MimeContent, MimeEntity, MimePart,
    TextFormat, TextPart, TextPartError, charsets::Charset, encodings::Base64Encoder,
    text::TextPreviewer,
};

const PREFIX_LIMIT: usize = 16 * 1024;

fn raw_part(content_type: &str, bytes: &[u8], encoding: ContentEncoding) -> TextPart {
    let mut headers = HeaderList::new();
    headers.add(Header::parse(format!("Content-Type: {content_type}\r\n").as_bytes()).unwrap());
    // Stored transfer encoding, not this deliberately conflicting header, wins.
    headers.add(Header::parse(b"Content-Transfer-Encoding: unknown\r\n").unwrap());
    TextPart::from_part(MimePart::from_parts(
        headers,
        MimeContent::with_encoding(Cursor::new(bytes.to_vec()), encoding),
    ))
    .unwrap()
}

fn preview(content_type: &str, bytes: &[u8]) -> String {
    TextPreviewer::new()
        .preview(&mut raw_part(
            content_type,
            bytes,
            ContentEncoding::Unspecified,
        ))
        .unwrap()
}

#[test]
fn defaults_and_configurable_limits_apply_to_both_formats_and_recovery() {
    assert_eq!(TextPreviewer::new(), TextPreviewer::default());
    assert_eq!(TextPreviewer::new().maximum_preview_length(), 230);
    for limit in [0, 1025, usize::MAX] {
        assert_eq!(
            TextPreviewer::with_maximum_preview_length(limit)
                .unwrap_err()
                .requested(),
            limit
        );
    }
    for limit in [1, 3, 230, 1024] {
        let previewer = TextPreviewer::with_maximum_preview_length(limit).unwrap();
        assert_eq!(previewer.maximum_preview_length(), limit);
        let expected = format!("{}\u{2026}", "\u{e9}".repeat(limit - 1));
        for html in [false, true] {
            let content_type = if html { "text/html" } else { "text/plain" };
            for label in ["; charset=utf-8", "; charset=x-unknown"] {
                let mut bytes = if html { b"<body>".to_vec() } else { Vec::new() };
                if label == "; charset=utf-8" {
                    bytes.extend_from_slice("\u{e9}".repeat(limit + 1).as_bytes());
                } else {
                    bytes.extend(std::iter::repeat_n(0xe9, limit + 1));
                }
                let mut part = raw_part(
                    &format!("{content_type}{label}"),
                    &bytes,
                    ContentEncoding::Unspecified,
                );
                assert_eq!(previewer.preview(&mut part).unwrap(), expected);
            }
        }
    }
}

#[test]
fn upstream_document_previews() {
    for (bytes, format, expected) in [
        (
            &include_bytes!("data/text/homedepot-check-inside-now.html")[..],
            TextFormat::Html,
            "FREE DELIVERY Appliance Purchases $396 or More",
        ),
        (
            &include_bytes!("data/text/mimekit.net.html")[..],
            TextFormat::Html,
            "Toggle navigation MimeKit Home About Help Documentation Donate \u{d7} Install with NuGet (recommended) NuGet PM> Install-Package MimeKit PM> Install-Package MailKit or Install via VS Package Management window. Direct Download ZIP fil\u{2026}",
        ),
        (
            &include_bytes!("data/text/planet-fitness.html")[..],
            TextFormat::Html,
            "Don\u{2019}t miss our celebrity guest Monday evening",
        ),
        (
            &include_bytes!("data/text/planet-fitness.txt")[..],
            TextFormat::Plain,
            "Planet Fitness https://view.email.planetfitness.com/?qs=9a098a031cabde68c0a4260051cd6fe473a2e997a53678ff26b4b199a711a9d2ad0536530d6f837c246b09f644d42016ecfb298f930b7af058e9e454b34f3d818ceb3052ae317b1ac4594aab28a2d788 View web ver\u{2026}",
        ),
    ] {
        // Upstream loads the full document as text before constructing the part.
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let input = String::from_utf8_lossy(bytes);
        let mut part = TextPart::with_format(format, &input).unwrap();
        assert_eq!(TextPreviewer::new().preview(&mut part).unwrap(), expected);
    }
}

#[test]
fn upstream_unknown_charset_recovery() {
    for (bytes, expected) in [
        (
            &b"\xa4\xe9\xa4\xeb\xacP\xa8\xb0"[..],
            "\u{a4}\u{e9}\u{a4}\u{eb}\u{ac}P\u{a8}\u{b0}",
        ),
        (
            "\u{65e5}\u{6708}\u{661f}\u{8fb0}".as_bytes(),
            "\u{65e5}\u{6708}\u{661f}\u{8fb0}",
        ),
        (b"L'encyclop\xe9die libre", "L'encyclop\u{e9}die libre"),
    ] {
        for content_type in ["text/plain; charset=x-unknown", "text/plain"] {
            assert_eq!(preview(content_type, bytes), expected);
        }
    }
}

#[test]
fn only_html_dispatches_to_the_html_previewer() {
    for subtype in ["html", "HTML"] {
        assert_eq!(preview(&format!("text/{subtype}"), b"<body>a<br>b"), "a b");
        assert_eq!(preview(&format!("text/{subtype}"), b"<p>fragment</p>"), "");
    }
    for format in [
        TextFormat::Plain,
        TextFormat::Flowed,
        TextFormat::Enriched,
        TextFormat::RichText,
    ] {
        let input = "> flowed \r\nline <b>markup</b> {\\rtf1}";
        let mut part = TextPart::with_format(format, input).unwrap();
        assert_eq!(
            TextPreviewer::new().preview(&mut part).unwrap(),
            "> flowed line <b>markup</b> {\\rtf1}"
        );
    }
    assert_eq!(
        preview("text/x-custom", b"<body>raw</body>"),
        "<body>raw</body>"
    );
    for content_type in ["text/plain", "text/html", "text/plain; charset=x-unknown"] {
        assert_eq!(preview(content_type, b""), "");
        assert_eq!(preview(content_type, b" \t\r\n"), "");
    }
}

#[test]
fn declared_charsets_win_and_previewing_does_not_change_text_decoding() {
    let input = "hello \u{65e5}\u{672c}";
    for label in ["utf-8", "utf-16", "utf-16be", "shift_jis", "gb18030"] {
        let charset = Charset::from_label(label).unwrap();
        let bytes = charset.encode(input).unwrap();
        assert_eq!(
            preview(&format!("text/plain; charset={label}"), &bytes),
            input
        );
        let html = format!("<body>{input}</body>");
        let bytes = charset.encode(&html).unwrap();
        assert_eq!(
            preview(&format!("text/html; charset={label}"), &bytes),
            input
        );
    }
    assert_eq!(
        preview("text/plain; charset=iso-8859-1", b"\xc3\xa9"),
        "\u{c3}\u{a9}"
    );
    for (content_type, bytes, expected) in [
        ("text/plain; charset=utf-8", &b"caf\xe9"[..], "caf\u{fffd}"),
        ("text/plain; charset=us-ascii", b"caf\xe9", "caf\u{fffd}"),
        ("text/plain; charset=utf-16", b"a\0\xff", "a\u{fffd}"),
        ("text/plain; charset=x-unknown", b"caf\xe9", "caf\u{e9}"),
        ("text/plain", b"caf\xe9", "caf\u{e9}"),
    ] {
        let mut part = raw_part(content_type, bytes, ContentEncoding::Unspecified);
        let headers = part.part().headers().clone();
        let strict_charset = part
            .part()
            .content_type()
            .unwrap()
            .unwrap()
            .charset_encoding()
            .ok()
            .flatten()
            .unwrap_or(Charset::UTF8);
        assert!(matches!(
            part.text_with_charset(strict_charset),
            Err(TextPartError::Charset(_))
        ));
        assert_eq!(part.text().unwrap(), expected);
        assert_eq!(TextPreviewer::new().preview(&mut part).unwrap(), expected);
        assert_eq!(part.part().headers(), &headers);
        assert_eq!(part.part().content().get_ref().get_ref(), bytes);
        assert!(matches!(
            part.text_with_charset(strict_charset),
            Err(TextPartError::Charset(_))
        ));
        assert_eq!(part.text().unwrap(), expected);
    }
    let mut unknown = raw_part(
        "text/plain; charset=x-unknown",
        b"valid",
        ContentEncoding::Unspecified,
    );
    assert_eq!(TextPreviewer::new().preview(&mut unknown).unwrap(), "valid");
    assert_eq!(unknown.text().unwrap(), "valid");
}

#[test]
fn bom_selection_and_matching_preamble_removal() {
    for (label, bom) in [
        ("utf-8", &b"\xef\xbb\xbf"[..]),
        ("utf-16", &b"\xff\xfe"[..]),
        ("utf-16be", &b"\xfe\xff"[..]),
    ] {
        let charset = Charset::from_label(label).unwrap();
        let mut bytes = bom.to_vec();
        bytes.extend_from_slice(&charset.encode("caf\u{e9}").unwrap());
        for suffix in [
            "".to_owned(),
            "; charset=x-unknown".into(),
            format!("; charset={label}"),
        ] {
            assert_eq!(preview(&format!("text/plain{suffix}"), &bytes), "caf\u{e9}");
        }
        let mut repeated = bom.to_vec();
        repeated.extend_from_slice(&bytes);
        assert_eq!(preview("text/plain", &repeated), "\u{feff}caf\u{e9}");
    }
    assert_eq!(
        preview("text/plain; charset=iso-8859-1", b"\xef\xbb\xbfhi"),
        "\u{ef}\u{bb}\u{bf}hi"
    );
    assert_eq!(
        preview("text/plain; charset=utf-8", b"\xff\xfehi"),
        "\u{fffd}\u{fffd}hi"
    );
    // A strict UTF-8 failure replays the original BOM bytes as Latin-1.
    assert_eq!(
        preview("text/plain", b"\xef\xbb\xbfhi\xff"),
        "\u{ef}\u{bb}\u{bf}hi\u{ff}"
    );
    assert_eq!(preview("text/plain", b"\xff\xfea\0\xff"), "a\u{fffd}");
    assert_eq!(preview("text/plain", b"\xef\xbb"), "\u{ef}\u{bb}");
    assert_eq!(
        preview("text/html", b"<body><meta charset=windows-1252>\x80"),
        "\u{80}"
    );
}

#[test]
fn unicode_limits_and_whitespace_follow_the_existing_previewers() {
    let input = format!("{}extra", "\u{1f408}".repeat(230));
    let expected = format!("{}\u{2026}", "\u{1f408}".repeat(229));
    for subtype in ["plain", "html"] {
        let text = if subtype == "html" {
            format!("<body>{input}</body>")
        } else {
            input.clone()
        };
        let mut part = TextPart::new(subtype, &text).unwrap();
        assert_eq!(TextPreviewer::new().preview(&mut part).unwrap(), expected);
    }
    assert_eq!(
        preview("text/plain", " a\u{200b}\u{200c}b ".as_bytes()),
        "a b"
    );
    let exact = "a".repeat(230);
    assert_eq!(preview("text/plain", exact.as_bytes()), exact);
    let with_space = format!("{}  extra", "a".repeat(229));
    assert_eq!(
        preview("text/plain", with_space.as_bytes()),
        format!("{}\u{2026}", "a".repeat(228))
    );
}

#[test]
fn cap_applies_to_decoded_bytes_not_encoded_bytes_or_characters() {
    for html in [false, true] {
        let start = if html { "<body>" } else { "" };
        let input = format!("{start}{}xy", " ".repeat(PREFIX_LIMIT - start.len() - 1));
        let content_type = if html { "text/html" } else { "text/plain" };
        let mut encoded = Vec::new();
        let mut encoder = Base64Encoder::new();
        encoder.encode(input.as_bytes(), &mut encoded);
        encoder.finish(&mut encoded);
        assert!(encoded.len() > PREFIX_LIMIT);
        for (bytes, encoding) in [
            (input.as_bytes(), ContentEncoding::Unspecified),
            (encoded.as_slice(), ContentEncoding::Base64),
        ] {
            let mut part = raw_part(content_type, bytes, encoding);
            let previewer = TextPreviewer::new();
            assert_eq!(previewer.preview(&mut part).unwrap(), "x");
            assert_eq!(previewer.preview(&mut part).unwrap(), "x");
        }
    }
    let utf16 = Charset::from_label("utf-16").unwrap();
    let input = format!("{}xy", " ".repeat(PREFIX_LIMIT / 2 - 1));
    assert_eq!(
        preview("text/plain; charset=utf-16", &utf16.encode(&input).unwrap()),
        "x"
    );
    let input = format!("{}\u{e9}", " ".repeat(PREFIX_LIMIT - 1));
    assert_eq!(preview("text/plain", input.as_bytes()), "\u{c3}");
    assert_eq!(
        preview("text/plain; charset=utf-8", input.as_bytes()),
        "\u{fffd}"
    );
    assert_eq!(
        preview(
            "text/html",
            format!("{}<body>hidden", " ".repeat(PREFIX_LIMIT)).as_bytes()
        ),
        ""
    );
}

#[test]
fn transfer_decoding_precedes_charset_decoding() {
    for charset in [
        Charset::UTF8,
        Charset::LATIN1,
        Charset::from_label("utf-16").unwrap(),
    ] {
        for encoding in [
            ContentEncoding::Base64,
            ContentEncoding::QuotedPrintable,
            ContentEncoding::UuEncode,
        ] {
            let mut original =
                TextPart::with_charset("html", "<body>caf\u{e9}</body>", charset).unwrap();
            original.part_mut().set_content_transfer_encoding(encoding);
            let mut serialized = Vec::new();
            original.part_mut().write_to(&mut serialized).unwrap();
            let MimeEntity::Part(parsed) = MimeEntity::load(&mut serialized.as_slice()).unwrap()
            else {
                panic!("leaf")
            };
            let mut parsed = TextPart::from_part(parsed).unwrap();
            assert_eq!(
                TextPreviewer::new().preview(&mut parsed).unwrap(),
                "caf\u{e9}"
            );
        }
    }
}

#[test]
fn unused_invalid_utf8_suffix_does_not_force_latin1() {
    let mut input = "\u{e9}".repeat(231).into_bytes();
    input.extend_from_slice(&[0xff]);
    assert_eq!(
        preview("text/plain", &input),
        format!("{}\u{2026}", "\u{e9}".repeat(229))
    );
    let mut input = format!("<body>{}<p>", "\u{e9}".repeat(231)).into_bytes();
    input.extend_from_slice(&[0xff]);
    assert_eq!(
        preview("text/html", &input),
        format!("{}\u{2026}", "\u{e9}".repeat(229))
    );
    // An error in the same data token can still be encountered before truncation.
    assert_eq!(preview("text/html", b"<body>caf\xe9"), "caf\u{e9}");
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    chunk: usize,
    read_error_at: usize,
    fail_seek: bool,
    interrupted: bool,
    seeks: usize,
    reads: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Cursor::new(bytes),
            chunk: 1,
            read_error_at: usize::MAX,
            fail_seek: false,
            interrupted: true,
            seeks: 0,
            reads: 0,
        }
    }
}

impl Read for Source {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if std::mem::take(&mut self.interrupted) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let remaining = self
            .read_error_at
            .saturating_sub(self.bytes.position() as usize);
        if remaining == 0 {
            return Err(io::Error::other("read failed"));
        }
        let count = output.len().min(self.chunk).min(remaining);
        self.bytes.read(&mut output[..count])
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

fn borrowed_part(source: &mut Source) -> TextPart<&mut Source> {
    TextPart::from_part(
        MimePart::with_content_type(
            &ContentType::new("text", "plain").unwrap(),
            MimeContent::new(source),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn bounded_short_interrupted_reads_and_reuse() {
    let mut source = Source::new(format!("{}tail", " ".repeat(PREFIX_LIMIT)).into_bytes());
    source.read_error_at = PREFIX_LIMIT;
    source.bytes.set_position(100);
    {
        let mut part = borrowed_part(&mut source);
        let previewer = TextPreviewer::new();
        assert_eq!(previewer.preview(&mut part).unwrap(), "");
        assert_eq!(previewer.preview(&mut part).unwrap(), "");
    }
    assert_eq!(source.bytes.position(), PREFIX_LIMIT as u64);
    assert_eq!(source.seeks, 2);
    source.read_error_at = usize::MAX;
    let mut suffix = String::new();
    source.read_to_string(&mut suffix).unwrap();
    assert_eq!(suffix, "tail");
}

#[test]
fn read_errors_are_not_hidden_by_early_preview_completion_or_fallback() {
    let bytes = format!("{}tail", "a".repeat(5000)).into_bytes();
    for position in [0, 1, 230, 4096, bytes.len()] {
        let mut source = Source::new(bytes.clone());
        source.read_error_at = position;
        source.chunk = 19;
        let error = TextPreviewer::new()
            .preview(&mut borrowed_part(&mut source))
            .unwrap_err();
        assert!(matches!(&error, TextPartError::Io(error) if error.to_string() == "read failed"));
        assert!(std::error::Error::source(&error).unwrap().is::<io::Error>());
    }
    let mut source = Source::new(b"caf\xe9".to_vec());
    source.read_error_at = 4;
    assert!(matches!(
        TextPreviewer::new().preview(&mut borrowed_part(&mut source)),
        Err(TextPartError::Io(_))
    ));
    let mut source = Source::new(b"text".to_vec());
    source.fail_seek = true;
    let error = TextPreviewer::new()
        .preview(&mut borrowed_part(&mut source))
        .unwrap_err();
    assert!(matches!(error, TextPartError::Io(error) if error.to_string() == "seek failed"));
    assert_eq!(source.reads, 0);
}

#[test]
fn metadata_errors_precede_io() {
    for content_type in [None, Some("invalid"), Some("image/png")] {
        let mut source = Source::new(b"original".to_vec());
        let mut part = borrowed_part(&mut source);
        part.part_mut().headers_mut().remove_all("Content-Type");
        if let Some(value) = content_type {
            part.part_mut()
                .headers_mut()
                .add(Header::parse(format!("Content-Type: {value}\r\n").as_bytes()).unwrap());
            part.part_mut()
                .headers_mut()
                .add(Header::parse(b"Content-Type: text/plain\r\n").unwrap());
        }
        let headers = part.part().headers().clone();
        let error = TextPreviewer::new().preview(&mut part).unwrap_err();
        match content_type {
            None => assert!(matches!(error, TextPartError::MissingContentType)),
            Some("invalid") => assert!(matches!(error, TextPartError::ContentType(_))),
            Some(_) => assert!(matches!(error, TextPartError::NotText(_))),
        }
        assert_eq!(part.part().headers(), &headers);
        drop(part);
        assert_eq!((source.reads, source.seeks), (0, 0));
    }
}

#[test]
fn preview_preserves_parsed_framing() {
    for bytes in [
        &b"Content-Type: text/plain\r\n"[..],
        b"Content-Type: text/plain\n\n",
        b"Content-Type: text/html\r\n\r\n<body>hello</body>",
    ] {
        let mut input = bytes;
        let MimeEntity::Part(part) = MimeEntity::load(&mut input).unwrap() else {
            panic!("leaf")
        };
        let mut part = TextPart::from_part(part).unwrap();
        let mut before = Vec::new();
        part.part_mut().write_to(&mut before).unwrap();
        let expected = if bytes.ends_with(b"</body>") {
            "hello"
        } else {
            ""
        };
        assert_eq!(TextPreviewer::new().preview(&mut part).unwrap(), expected);
        let mut after = Vec::new();
        part.part_mut().write_to(&mut after).unwrap();
        assert_eq!(after, before);
    }
}

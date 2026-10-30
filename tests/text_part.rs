// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, ContentType, Header, HeaderList, MimeContent, MimePart, TextPart,
    TextPartError,
    charsets::{Charset, CharsetError},
    encodings::NewLineFormat,
};

fn raw_part(
    content_type: &str,
    bytes: &[u8],
    encoding: ContentEncoding,
) -> MimePart<Cursor<Vec<u8>>> {
    let mut headers = HeaderList::new();
    headers.add(Header::parse(format!("Content-Type: {content_type}").as_bytes()).unwrap());
    MimePart::from_parts(
        headers,
        MimeContent::with_encoding(Cursor::new(bytes.to_vec()), encoding),
    )
}

#[test]
fn utf8_construction_preserves_text_and_sets_only_content_type() {
    for (subtype, input) in [
        ("plain", ""),
        ("PLAIN", "hello\r\nworld\n"),
        ("HTML", "<b>caf\u{e9}</b>"),
        ("x-custom", "\u{feff}hello"),
    ] {
        let mut text = TextPart::new(subtype, input).unwrap();
        assert_eq!(text.part().headers().len(), 1);
        let content_type = text.part().content_type().unwrap().unwrap();
        assert_eq!(content_type.media_subtype(), subtype);
        assert_eq!(content_type.charset(), Some("utf-8"));
        assert_eq!(
            text.part().content().encoding(),
            ContentEncoding::Unspecified
        );
        assert_eq!(
            text.part().content_transfer_encoding().unwrap(),
            ContentEncoding::Unspecified
        );
        assert_eq!(text.part().content().get_ref().get_ref(), input.as_bytes());
        assert_eq!(text.text().unwrap(), input);
        assert_eq!(
            text.is_plain().unwrap(),
            subtype.eq_ignore_ascii_case("plain")
        );
        assert_eq!(
            text.is_html().unwrap(),
            subtype.eq_ignore_ascii_case("html")
        );
    }
}

#[test]
fn charset_construction_and_updates_use_actual_codec_labels_and_bytes() {
    for (label, input) in [
        ("us-ascii", "hello"),
        ("iso-8859-1", "Ren\u{e9}e"),
        ("utf-16", "\u{1f408}\r\n\u{feff}"),
        ("utf-16be", "Ren\u{e9}e"),
        ("shift_jis", "\u{65e5}\u{672c}\u{8a9e}"),
        ("iso-2022-jp", "\u{65e5}\u{672c}\u{8a9e}"),
        ("gb2312", "\u{4e2d}\u{6587}"),
    ] {
        let charset = Charset::from_label(label).unwrap();
        let mut text = TextPart::with_charset("plain", input, charset).unwrap();
        assert_eq!(
            text.part().content().get_ref().get_ref(),
            charset.encode(input).unwrap().as_ref()
        );
        assert_eq!(
            text.part().content_type().unwrap().unwrap().charset(),
            Some(charset.name())
        );
        assert_eq!(text.text().unwrap(), input);
        text.set_text(input).unwrap();
        assert_eq!(
            text.part().content_type().unwrap().unwrap().charset(),
            Some("utf-8")
        );
        assert_eq!(text.part().content().get_ref().get_ref(), input.as_bytes());
    }
}

#[test]
fn unlabelled_text_detects_boms_and_recovers_latin1_without_changing_explicit_decoding() {
    let mut text = TextPart::from_part(raw_part(
        "text/plain",
        b"\xef\xbb\xbfhello\n",
        ContentEncoding::Unspecified,
    ))
    .unwrap();
    assert_eq!(text.text().unwrap(), "\u{feff}hello\n");
    let mut text = TextPart::from_part(raw_part(
        "text/plain",
        b"Ren\xe9e",
        ContentEncoding::Unspecified,
    ))
    .unwrap();
    assert_eq!(
        text.text_and_charset().unwrap(),
        ("Ren\u{e9}e".into(), Charset::LATIN1)
    );
    assert!(matches!(
        text.text_with_charset(Charset::UTF8),
        Err(TextPartError::Charset(CharsetError::InvalidBytes(charset)))
            if charset == Charset::UTF8
    ));
    assert_eq!(
        text.text_with_charset(Charset::LATIN1).unwrap(),
        "Ren\u{e9}e"
    );
    assert_eq!(text.part().content_type().unwrap().unwrap().charset(), None);

    let mut text = TextPart::from_part(raw_part(
        "text/html",
        b"\xff\xfeH\0i\0",
        ContentEncoding::Unspecified,
    ))
    .unwrap();
    assert_eq!(text.text().unwrap(), "\u{feff}Hi");
    assert_eq!(
        text.text_with_charset(Charset::from_label("utf-16").unwrap())
            .unwrap(),
        "\u{feff}Hi"
    );
    let mut text = TextPart::from_part(raw_part(
        "text/html",
        b"<meta charset=iso-8859-1>Ren\xe9e",
        ContentEncoding::Unspecified,
    ))
    .unwrap();
    assert_eq!(text.text().unwrap(), "<meta charset=iso-8859-1>Ren\u{e9}e");
}

#[test]
fn declared_charset_wins_and_explicit_override_does_not_edit_headers() {
    let mut text = TextPart::from_part(raw_part(
        "text/plain; charset=iso-8859-1",
        b"\xc3\xa9",
        ContentEncoding::Unspecified,
    ))
    .unwrap();
    assert_eq!(text.text().unwrap(), "\u{c3}\u{a9}");
    let before = text.part().headers().clone();
    assert_eq!(text.text_with_charset(Charset::UTF8).unwrap(), "\u{e9}");
    assert_eq!(text.part().headers(), &before);

    text.part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: text/plain; charset=x-unknown").unwrap())
        .unwrap();
    text.part_mut().content_mut().get_mut().set_position(1);
    assert_eq!(text.text().unwrap(), "\u{e9}");
    assert_eq!(text.part().content().get_ref().position(), 0);
    assert_eq!(text.text_with_charset(Charset::UTF8).unwrap(), "\u{e9}");
    assert_eq!(
        text.part().content_type().unwrap().unwrap().charset(),
        Some("x-unknown")
    );
}

#[test]
fn text_decoding_uses_stored_transfer_encoding_and_rewinds_on_charset_failure() {
    let mut part = raw_part(
        "text/plain; charset=iso-8859-1",
        b"UmVu6WU=\r\n",
        ContentEncoding::Base64,
    );
    part.set_content_transfer_encoding(ContentEncoding::SevenBit);
    part.content_mut().get_mut().set_position(4);
    let mut text = TextPart::from_part(part).unwrap();
    assert_eq!(text.part().content().get_ref().position(), 4);
    assert_eq!(text.text().unwrap(), "Ren\u{e9}e");
    assert_eq!(text.part().content().get_ref().position(), 0);
    assert_eq!(text.part().content().encoding(), ContentEncoding::Base64);
    assert_eq!(
        text.part().content_transfer_encoding().unwrap(),
        ContentEncoding::SevenBit
    );
    text.part_mut().content_mut().get_mut().set_position(3);
    assert!(matches!(
        text.text_with_charset(Charset::UTF8),
        Err(TextPartError::Charset(CharsetError::InvalidBytes(_)))
    ));
    assert_eq!(text.part().content().get_ref().position(), 0);
}

#[test]
fn raw_edits_are_rechecked_without_falling_back_to_valid_duplicates() {
    let mut text = TextPart::new("plain", "hello").unwrap();
    text.part_mut()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: TeXt/HtMl; charset=utf-8").unwrap())
        .unwrap();
    assert!(text.is_html().unwrap());
    assert!(!text.is_plain().unwrap());
    for (raw, kind) in [
        ("Content-Type: image/png", "nontext"),
        ("Content-Type: invalid", "invalid"),
    ] {
        text.part_mut()
            .headers_mut()
            .replace_all(Header::parse(raw.as_bytes()).unwrap())
            .unwrap();
        text.part_mut()
            .headers_mut()
            .add(Header::parse(b"Content-Type: text/plain").unwrap());
        let error = text.text().unwrap_err();
        match kind {
            "nontext" => {
                assert!(matches!(error, TextPartError::NotText(ref value) if value == "image/png"))
            }
            _ => assert!(matches!(error, TextPartError::ContentType(_))),
        }
        assert!(text.is_html().is_err());
        assert!(text.text_with_charset(Charset::UTF8).is_err());
    }
    text.part_mut().headers_mut().remove_all("content-type");
    assert!(matches!(
        text.text(),
        Err(TextPartError::MissingContentType)
    ));
    assert!(text.is_plain().is_err());
}

#[test]
fn text_update_preserves_other_parameters_and_headers_but_replaces_stored_encoding() {
    let mut part = raw_part(
        "text/plain; charset=iso-8859-1; format=flowed; name=old.txt",
        b"T2xk\r\n",
        ContentEncoding::Base64,
    );
    part.headers_mut()
        .insert(0, Header::parse(b"X-Keep: \xff\n").unwrap())
        .unwrap();
    part.headers_mut()
        .add(Header::parse(b"CONTENT-TYPE: text/html").unwrap());
    part.set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    part.content_mut()
        .set_new_line_format(Some(NewLineFormat::Mixed));
    part.content_mut().get_mut().set_position(3);
    let unrelated = part.headers().as_slice()[0].clone();
    let transfer = part
        .headers()
        .get("Content-Transfer-Encoding")
        .unwrap()
        .clone();
    let mut text = TextPart::from_part(part).unwrap();
    text.set_text_with_charset("caf\u{e9}\n", Charset::LATIN1)
        .unwrap();
    assert_eq!(text.part().headers().get_all("Content-Type").count(), 1);
    assert_eq!(text.part().headers().index_of("Content-Type"), Some(1));
    let content_type = text.part().content_type().unwrap().unwrap();
    assert_eq!(content_type.charset(), Some("iso-8859-1"));
    assert_eq!(content_type.format(), Some("flowed"));
    assert_eq!(content_type.name(), Some("old.txt"));
    assert_eq!(text.part().headers().as_slice()[0], unrelated);
    assert_eq!(
        text.part().headers().get("Content-Transfer-Encoding"),
        Some(&transfer)
    );
    assert_eq!(
        text.part().content().encoding(),
        ContentEncoding::Unspecified
    );
    assert_eq!(text.part().content().new_line_format(), None);
    assert_eq!(text.part().content().get_ref().position(), 0);
    assert_eq!(text.part().content().get_ref().get_ref(), b"caf\xe9\n");
}

#[test]
fn failed_updates_leave_headers_bytes_position_and_hints_unchanged() {
    let mut text = TextPart::new("plain", "original").unwrap();
    text.part_mut().content_mut().get_mut().set_position(3);
    text.part_mut()
        .content_mut()
        .set_new_line_format(Some(NewLineFormat::Unix));
    let before = text.part().headers().clone();
    assert!(matches!(
        text.set_text_with_charset("\u{1f408}", Charset::LATIN1),
        Err(TextPartError::Charset(CharsetError::UnmappableText(_)))
    ));
    assert_eq!(text.part().headers(), &before);
    for value in [
        "text/".to_owned() + &"x".repeat(100),
        "image/png".into(),
        "invalid".into(),
    ] {
        text.part_mut()
            .headers_mut()
            .replace_all(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap())
            .unwrap();
        text.part_mut()
            .headers_mut()
            .add(Header::parse(b"Content-Type: text/plain").unwrap());
        let before = text.part().headers().clone();
        assert!(text.set_text("replacement").is_err());
        assert_eq!(text.part().headers(), &before);
        assert_eq!(text.part().content().get_ref().get_ref(), b"original");
        assert_eq!(text.part().content().get_ref().position(), 3);
        assert_eq!(
            text.part().content().new_line_format(),
            Some(NewLineFormat::Unix)
        );
    }
    text.part_mut().headers_mut().remove_all("Content-Type");
    assert!(matches!(
        text.set_text("replacement"),
        Err(TextPartError::MissingContentType)
    ));
    assert_eq!(text.part().content().get_ref().get_ref(), b"original");
}

#[test]
fn invalid_constructors_and_wrapping_report_specific_errors_without_io() {
    assert!(matches!(
        TextPart::new("", ""),
        Err(TextPartError::ContentType(_))
    ));
    assert!(matches!(
        TextPart::new("bad type", ""),
        Err(TextPartError::ContentType(_))
    ));
    assert!(matches!(
        TextPart::new(&"x".repeat(100), ""),
        Err(TextPartError::HeaderFormat(_))
    ));
    assert!(matches!(
        TextPart::with_charset("plain", "\u{e9}", Charset::ASCII),
        Err(TextPartError::Charset(_))
    ));

    for value in [
        None,
        Some("invalid"),
        Some("application/octet-stream"),
        Some("text/plain; charset=x-unknown"),
    ] {
        let mut source = Source::new(b"hello");
        source.inner.set_position(2);
        let mut part = MimePart::from_parts(HeaderList::new(), MimeContent::new(&mut source));
        if let Some(value) = value {
            part.headers_mut()
                .add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
        }
        let result = TextPart::from_part(part);
        if value == Some("text/plain; charset=x-unknown") {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
        drop(result);
        assert_eq!(source.reads + source.seeks, 0);
        assert_eq!(source.inner.position(), 2);
    }
}

#[test]
fn borrowed_chunked_content_is_not_closed_or_copied_on_wrap_and_extraction() {
    let mut source = Source::new(b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e");
    let part = MimePart::with_content_type(
        &ContentType::new("text", "plain").unwrap(),
        MimeContent::new(&mut source),
    )
    .unwrap();
    let mut text = TextPart::from_part(part).unwrap();
    assert_eq!(text.text().unwrap(), "\u{65e5}\u{672c}\u{8a9e}");
    let part = text.into_part();
    assert_eq!(part.content().get_ref().inner.position(), 0);
    let (_, content) = part.into_parts();
    let returned = content.into_inner();
    returned.inner.set_position(3);
    assert_eq!(source.inner.position(), 3);
    assert!(source.reads > 3);
}

#[test]
fn source_read_and_seek_errors_retain_their_error_chain() {
    for (fail_read, fail_seek) in [(true, None), (false, Some(1)), (false, Some(2))] {
        let mut source = Source::new(b"hello");
        source.fail_read = fail_read;
        source.fail_seek = fail_seek;
        let part = MimePart::with_content_type(
            &ContentType::new("text", "plain").unwrap(),
            MimeContent::new(source),
        )
        .unwrap();
        let mut text = TextPart::from_part(part).unwrap();
        let error = text.text().unwrap_err();
        assert!(matches!(&error, TextPartError::Io(error) if error.kind() == io::ErrorKind::Other));
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .is_some()
        );
    }
}

#[test]
fn authored_text_serializes_and_transfer_decodes_with_its_declared_charset() {
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
            let mut text = TextPart::with_charset("html", "<p>Ren\u{e9}e</p>", charset).unwrap();
            text.part_mut().set_content_transfer_encoding(encoding);
            let mut output = Vec::new();
            text.part_mut().write_to(&mut output).unwrap();
            let mut reader = Cursor::new(output);
            let headers = HeaderList::load(&mut reader).unwrap();
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            let content = MimeContent::with_encoding(Cursor::new(bytes), encoding);
            let mut restored = TextPart::from_part(MimePart::from_parts(headers, content)).unwrap();
            assert_eq!(restored.text().unwrap(), "<p>Ren\u{e9}e</p>");
            assert!(restored.is_html().unwrap());
        }
    }
}

struct Source {
    inner: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_read: bool,
    fail_seek: Option<usize>,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            inner: Cursor::new(bytes.to_vec()),
            reads: 0,
            seeks: 0,
            fail_read: false,
            fail_seek: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        let count = bytes.len().min(1);
        self.inner.read(&mut bytes[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.inner.seek(position)
    }
}

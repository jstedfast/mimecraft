// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MessagePart, MimeContent, MimeEntity, MimeMessage,
    MimePart, Multipart, TextBodyError, TextFormat, TextPart, TextPartError,
    charsets::CharsetError,
};

fn raw_part<R: Read + Seek>(
    content_type: Option<&str>,
    source: R,
    encoding: ContentEncoding,
) -> MimePart<R> {
    let mut headers = HeaderList::new();
    if let Some(value) = content_type {
        headers.add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
    }
    MimePart::from_parts(headers, MimeContent::with_encoding(source, encoding))
}

fn entity(content_type: Option<&str>, bytes: &[u8], encoding: ContentEncoding) -> MimeEntity {
    raw_part(content_type, Cursor::new(bytes.to_vec()), encoding).into()
}

#[test]
fn absent_and_empty_bodies_remain_distinct_and_attached_messages_are_not_read() {
    let mut message: MimeMessage = MimeMessage::new();
    assert_eq!(message.text_body(TextFormat::Plain).unwrap(), None);
    message.set_body(Some(entity(None, b"", ContentEncoding::Default)));
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("")
    );
    assert_eq!(message.text_body(TextFormat::Html).unwrap(), None);
    let mut wrapper: MimeEntity = MessagePart::new(message).into();
    assert_eq!(wrapper.text_body(TextFormat::Plain).unwrap(), None);
}

#[test]
fn decoded_plain_and_html_preserve_newlines_boms_and_markup() {
    for (media_type, format, text) in [
        ("text/plain", TextFormat::Plain, "\u{feff}hello\nworld\r"),
        (
            "text/html",
            TextFormat::Html,
            "<script>unchanged()</script>\r\n",
        ),
        (
            "text/enriched",
            TextFormat::Enriched,
            "<bold>hello</bold>\n",
        ),
        ("text/richtext", TextFormat::Enriched, "<bold>old</bold>"),
        ("text/rtf", TextFormat::RichText, "{\\rtf1 hello}\n"),
        (
            "application/rtf",
            TextFormat::RichText,
            "{\\rtf1 hello}\r\n",
        ),
    ] {
        let mut body = entity(Some(media_type), text.as_bytes(), ContentEncoding::Default);
        let before = body.headers().clone();
        assert_eq!(body.text_body(format).unwrap().as_deref(), Some(text));
        assert_eq!(body.headers(), &before);
    }
}

#[test]
fn stored_transfer_encoding_and_declared_charset_are_used_without_header_edits() {
    let mut part = raw_part(
        Some("text/plain; charset=iso-8859-1"),
        Cursor::new(b"UmVu6WU=\r\n".to_vec()),
        ContentEncoding::Base64,
    );
    part.set_content_transfer_encoding(ContentEncoding::SevenBit);
    part.content_mut().get_mut().set_position(4);
    let headers = part.headers().clone();
    let mut body: MimeEntity = part.into();
    assert_eq!(
        body.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("Ren\u{e9}e")
    );
    let part = body.text_body_part(TextFormat::Plain).unwrap().unwrap();
    assert_eq!(part.headers(), &headers);
    assert_eq!(part.content().get_ref().position(), 0);
    assert_eq!(part.content().get_ref().get_ref(), b"UmVu6WU=\r\n");
    assert_eq!(part.content().encoding(), ContentEncoding::Base64);
}

#[test]
fn flowed_decoding_follows_transfer_and_charset_decoding_and_honors_delsp() {
    for (delsp, expected) in [
        ("", "caf\u{e9} suite\r\n"),
        ("; delsp=no", "caf\u{e9} suite\r\n"),
        ("; delsp=YeS", "caf\u{e9}suite\r\n"),
        ("; delsp=\" yes \"", "caf\u{e9} suite\r\n"),
    ] {
        let value = format!("text/plain; charset=iso-8859-1; format=\" Flowed \"{delsp}");
        let mut body = entity(
            Some(&value),
            b"caf=E9=20\r\nsuite",
            ContentEncoding::QuotedPrintable,
        );
        for format in [TextFormat::Plain, TextFormat::Flowed] {
            assert_eq!(body.text_body(format).unwrap().as_deref(), Some(expected));
        }
        let part = body.text_body_part_mut(TextFormat::Plain).unwrap().unwrap();
        part.headers_mut()
            .replace_all(Header::parse(b"Content-Type: text/plain; charset=iso-8859-1").unwrap())
            .unwrap();
        assert_eq!(
            body.text_body(TextFormat::Plain).unwrap().as_deref(),
            Some("caf\u{e9} \r\nsuite")
        );
    }
}

#[test]
fn text_part_text_keeps_flowed_formatting_and_other_formats_do_not_reflow() {
    let mut text = TextPart::from_part(raw_part(
        Some("text/plain; format=flowed; delsp=yes"),
        Cursor::new(b"soft \nbreak"),
        ContentEncoding::Default,
    ))
    .unwrap();
    assert_eq!(text.text().unwrap(), "soft \nbreak");
    let mut body: MimeEntity<_> = text.into();
    assert_eq!(
        body.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("softbreak\r\n")
    );
    let mut html = entity(
        Some("text/html; format=flowed; delsp=yes"),
        b"hello \nworld",
        ContentEncoding::Default,
    );
    assert_eq!(
        html.text_body(TextFormat::Html).unwrap().as_deref(),
        Some("hello \nworld")
    );
}

#[test]
fn unknown_charsets_fail_before_io_and_invalid_bytes_fail_after_rewind() {
    let mut body = entity(
        Some("text/plain; charset=x-unknown"),
        b"data",
        ContentEncoding::Default,
    );
    let part = body.text_body_part_mut(TextFormat::Plain).unwrap().unwrap();
    part.content_mut().get_mut().set_position(2);
    let error = body.text_body(TextFormat::Plain).unwrap_err();
    assert!(
        matches!(&error, TextBodyError::Text(TextPartError::Charset(CharsetError::UnsupportedLabel(label))) if label == "x-unknown")
    );
    assert!(error.source().unwrap().source().is_some());
    assert!(error.to_string().contains("x-unknown"));
    let part = body.text_body_part(TextFormat::Plain).unwrap().unwrap();
    assert_eq!(part.content().get_ref().position(), 2);
    for content_type in [None, Some("text/plain"), Some("text/plain; charset=utf-8")] {
        let mut body = entity(content_type, b"Ren\xe9e", ContentEncoding::Default);
        body.text_body_part_mut(TextFormat::Plain)
            .unwrap()
            .unwrap()
            .content_mut()
            .get_mut()
            .set_position(2);
        assert!(matches!(
            body.text_body(TextFormat::Plain),
            Err(TextBodyError::Text(TextPartError::Charset(
                CharsetError::InvalidBytes(_)
            )))
        ));
        assert_eq!(
            body.text_body_part(TextFormat::Plain)
                .unwrap()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            0
        );
        assert_eq!(
            body.headers().contains("Content-Type"),
            content_type.is_some()
        );
    }
}

#[test]
fn decoding_failure_does_not_fall_back_to_an_earlier_alternative() {
    let mut alternative = Multipart::new("alternative", "alt").unwrap();
    alternative.children_mut().extend([
        entity(Some("text/plain"), b"valid", ContentEncoding::Default),
        entity(Some("text/plain"), b"\xff", ContentEncoding::Default),
    ]);
    let mut message = MimeMessage::with_body(alternative.into());
    assert!(matches!(
        message.text_body(TextFormat::Plain),
        Err(TextBodyError::Text(TextPartError::Charset(_)))
    ));
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    fail_read: bool,
    fail_seek: Option<usize>,
    seeks: usize,
    reads: usize,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(b"body".to_vec()),
            fail_read: false,
            fail_seek: None,
            seeks: 0,
            reads: 0,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        self.bytes.read(buffer)
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn initial_seek_read_and_final_rewind_failures_are_not_hidden() {
    for (fail_read, fail_seek, expected_reads, expected_seeks) in [
        (false, Some(1), 0, 1),
        (true, None, 1, 1),
        (false, Some(2), 2, 2),
    ] {
        let mut source = Source {
            fail_read,
            fail_seek,
            ..Source::new()
        };
        {
            let mut message = MimeMessage::with_body(
                raw_part(Some("text/plain"), &mut source, ContentEncoding::Default).into(),
            );
            let error = message.text_body(TextFormat::Plain).unwrap_err();
            assert!(
                matches!(&error, TextBodyError::Text(TextPartError::Io(inner)) if inner.kind() == io::ErrorKind::Other)
            );
            assert!(error.source().unwrap().source().is_some());
        }
        assert_eq!(source.reads, expected_reads);
        assert_eq!(source.seeks, expected_seeks);
    }
}

#[test]
fn metadata_failures_and_nonmatching_bodies_never_touch_the_source() {
    for (content_type, fails) in [
        ("text/plain; charset=x-unknown", true),
        ("invalid", true),
        ("text/html", false),
    ] {
        let mut source = Source::new();
        {
            let mut body: MimeEntity<_> =
                raw_part(Some(content_type), &mut source, ContentEncoding::Default).into();
            let result = body.text_body(TextFormat::Plain);
            if fails {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), None);
            }
        }
        assert_eq!(source.seeks, 0);
        assert_eq!(source.reads, 0);
    }
}

#[test]
fn loaded_related_alternative_message_reads_only_selected_body() {
    let raw = b"Content-Type: multipart/related; boundary=r; start=\"<root>\"\r\n\r\n\
--r\r\nContent-Type: text/plain; charset=x-unknown\r\n\r\nresource\r\n\
--r\r\nContent-Type: multipart/alternative; boundary=a\r\nContent-Id: <root>\r\n\r\n\
--a\r\nContent-Type: text/plain; format=flowed\r\n\r\nhello \r\nworld\r\n\
--a\r\nContent-Type: text/html\r\n\r\n<b>hello</b>\r\n--a--\r\n--r--\r\n";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("hello world\r\n")
    );
    assert_eq!(
        message.text_body(TextFormat::Html).unwrap().as_deref(),
        Some("<b>hello</b>")
    );
    let MimeEntity::Multipart(related) = message.body().unwrap() else {
        panic!()
    };
    let MimeEntity::Part(resource) = &related.children()[0] else {
        panic!()
    };
    assert_eq!(resource.content().get_ref().position(), 0);
}

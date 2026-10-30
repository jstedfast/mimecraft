// Adapted from MimeKit's UnitTests/TextPartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MimeContent, MimeEntity, MimePart, TextFormat, TextPart,
    TextPartError,
    charsets::{Charset, CharsetError},
    encodings::NewLineFormat,
};

const FORMATS: [TextFormat; 5] = [
    TextFormat::Plain,
    TextFormat::Flowed,
    TextFormat::Html,
    TextFormat::Enriched,
    TextFormat::RichText,
];

fn flags<R: Read + Seek>(text: &TextPart<R>) -> [bool; 5] {
    [
        text.is_plain().unwrap(),
        text.is_flowed().unwrap(),
        text.is_html().unwrap(),
        text.is_enriched().unwrap(),
        text.is_rich_text().unwrap(),
    ]
}

fn expected_flags(format: TextFormat) -> [bool; 5] {
    [
        matches!(format, TextFormat::Plain | TextFormat::Flowed),
        format == TextFormat::Flowed,
        format == TextFormat::Html,
        format == TextFormat::Enriched,
        format == TextFormat::RichText,
    ]
}

fn replace_type<R: Read + Seek>(text: &mut TextPart<R>, content_type: &str) {
    text.part_mut()
        .headers_mut()
        .replace_all(Header::parse(format!("Content-Type: {content_type}\r\n").as_bytes()).unwrap())
        .unwrap();
}

#[test]
fn format_constructors_match_upstream_predicates_and_metadata() {
    for (format, subtype) in FORMATS
        .into_iter()
        .zip(["plain", "plain", "html", "enriched", "rtf"])
    {
        for input in ["", "literal <b>text</b>\r\n From sender \n-- \r\n\u{feff}"] {
            let mut text = TextPart::with_format(format, input).unwrap();
            assert_eq!(text.format().unwrap(), format);
            assert_eq!(flags(&text), expected_flags(format));
            let content_type = text.part().content_type().unwrap().unwrap();
            assert_eq!(content_type.media_type(), "text");
            assert_eq!(content_type.media_subtype(), subtype);
            assert_eq!(content_type.charset(), Some("utf-8"));
            assert_eq!(
                content_type.format(),
                (format == TextFormat::Flowed).then_some("flowed")
            );
            assert_eq!(content_type.parameters().get("delsp"), None);
            assert_eq!(text.part().headers().len(), 1);
            assert_eq!(
                text.part().content_transfer_encoding().unwrap(),
                ContentEncoding::Unspecified
            );
            assert_eq!(
                text.part().content().encoding(),
                ContentEncoding::Unspecified
            );
            assert_eq!(text.part().content().new_line_format(), None);
            assert_eq!(text.part().content().get_ref().position(), 0);
            assert_eq!(text.part().content().get_ref().get_ref(), input.as_bytes());
            assert_eq!(text.text().unwrap(), input);
        }
    }
}

#[test]
fn explicit_charset_format_construction_uses_the_existing_strict_codec() {
    let input = "caf\u{e9}\r\nliteral text";
    for format in FORMATS {
        for charset in [
            Charset::UTF8,
            Charset::LATIN1,
            Charset::from_label("utf-16be").unwrap(),
        ] {
            let mut text = TextPart::with_format_and_charset(format, input, charset).unwrap();
            let content_type = text.part().content_type().unwrap().unwrap();
            assert_eq!(text.format().unwrap(), format);
            assert_eq!(content_type.charset(), Some(charset.name()));
            assert_eq!(
                text.part().content().get_ref().get_ref(),
                charset.encode(input).unwrap().as_ref()
            );
            assert_eq!(text.text().unwrap(), input);
        }
        assert!(matches!(
            TextPart::with_format_and_charset(format, input, Charset::ASCII),
            Err(TextPartError::Charset(CharsetError::UnmappableText(
                Charset::ASCII
            )))
        ));
    }
}

#[test]
fn classification_recognizes_aliases_case_and_trimmed_flowed_parameters() {
    for (content_type, format) in [
        ("text/plain", TextFormat::Plain),
        ("text/PLAIN", TextFormat::Plain),
        ("text/PLAIN; format=fixed", TextFormat::Plain),
        ("TeXt/PLAIN; FORMAT=\" Flowed \"", TextFormat::Flowed),
        (
            "text/plain; format*=utf-8''%20FLOWED%20",
            TextFormat::Flowed,
        ),
        (
            "text/plain; format=\"=?utf-8?q?flowed?=\"",
            TextFormat::Flowed,
        ),
        (
            "text/plain; format=\"\u{a0}flowed\u{2003}\"",
            TextFormat::Plain,
        ),
        ("text/plain; format=fixed", TextFormat::Plain),
        ("text/plain; format=\"\"", TextFormat::Plain),
        ("text/plain; format=\"flow ed\"", TextFormat::Plain),
        ("text/plain; delsp=yes", TextFormat::Plain),
        (
            "text/plain; format=flowed; delsp=invalid",
            TextFormat::Flowed,
        ),
        ("text/HTML; format=flowed", TextFormat::Html),
        ("text/EnRicHeD", TextFormat::Enriched),
        ("text/RICHTEXT", TextFormat::Enriched),
        ("text/RTF", TextFormat::RichText),
    ] {
        let mut text = TextPart::new("plain", "no markup validation").unwrap();
        replace_type(&mut text, content_type);
        let before = text.part().headers().clone();
        assert_eq!(text.format().unwrap(), format, "{content_type}");
        assert_eq!(flags(&text), expected_flags(format), "{content_type}");
        assert_eq!(text.part().headers(), &before);
    }
}

#[test]
fn classification_uses_first_content_type_and_first_format_parameter() {
    let mut text = TextPart::new("plain", "body").unwrap();
    for (first, expected) in [
        ("text/plain; format=fixed; format=flowed", TextFormat::Plain),
        (
            "text/plain; format=flowed; format=fixed",
            TextFormat::Flowed,
        ),
    ] {
        replace_type(&mut text, first);
        text.part_mut()
            .headers_mut()
            .add(Header::parse(b"Content-Type: text/html\r\n").unwrap());
        assert_eq!(text.format().unwrap(), expected);
        assert_eq!(flags(&text), expected_flags(expected));
        text.part_mut().headers_mut().remove("Content-Type");
        assert_eq!(text.format().unwrap(), TextFormat::Html);
    }
}

#[test]
fn unknown_subtypes_have_plain_classification_without_matching_plain_bodies() {
    for subtype in ["x-custom", "markdown", "rtf-compressed", "plain-extra"] {
        let text = TextPart::new(subtype, "body").unwrap();
        assert_eq!(text.format().unwrap(), TextFormat::Plain);
        assert_eq!(flags(&text), [false; 5]);
        let entity: MimeEntity = text.into();
        for format in FORMATS {
            assert!(entity.text_body_part(format).unwrap().is_none());
        }
    }
}

#[test]
fn predicates_and_body_selection_share_format_matching() {
    for format in FORMATS {
        let text = TextPart::with_format(format, "body").unwrap();
        let flags = flags(&text);
        let entity: MimeEntity = text.into();
        for (requested, expected) in FORMATS.into_iter().zip(flags) {
            assert_eq!(
                entity.text_body_part(requested).unwrap().is_some(),
                expected
            );
        }
    }
}

struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("classification must not read content");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("classification must not seek content");
    }
}

fn no_io_text() -> TextPart<NoIo> {
    let headers = HeaderList::load(&mut b"Content-Type: text/plain\r\n".as_slice()).unwrap();
    TextPart::from_part(MimePart::from_parts(headers, MimeContent::new(NoIo))).unwrap()
}

#[test]
fn classification_never_reads_sources_or_validates_charsets_and_transfer_encoding() {
    let mut text = no_io_text();
    for (content_type, expected) in [
        (
            "text/plain; format=flowed; charset=unknown",
            TextFormat::Flowed,
        ),
        ("text/richtext; charset=unknown", TextFormat::Enriched),
        ("text/rtf; charset=unknown", TextFormat::RichText),
    ] {
        replace_type(&mut text, content_type);
        text.part_mut()
            .headers_mut()
            .add(Header::parse(b"Content-Transfer-Encoding: not-an-encoding\r\n").unwrap());
        let before = text.part().headers().clone();
        assert_eq!(text.format().unwrap(), expected);
        assert_eq!(flags(&text), expected_flags(expected));
        assert_eq!(text.part().headers(), &before);
    }
}

#[test]
fn invalid_metadata_errors_do_not_fall_back_to_later_headers() {
    let mut text = no_io_text();
    for content_type in [
        None,
        Some("invalid"),
        Some("image/png"),
        Some("application/rtf"),
    ] {
        text.part_mut().headers_mut().remove_all("Content-Type");
        if let Some(content_type) = content_type {
            replace_type(&mut text, content_type);
            text.part_mut()
                .headers_mut()
                .add(Header::parse(b"Content-Type: text/plain\r\n").unwrap());
        }
        let before = text.part().headers().clone();
        let error = text.format().unwrap_err();
        match content_type {
            None => assert!(matches!(error, TextPartError::MissingContentType)),
            Some("invalid") => assert!(matches!(error, TextPartError::ContentType(_))),
            Some(value) => {
                assert!(matches!(error, TextPartError::NotText(ref actual) if actual == value))
            }
        }
        assert!(text.is_plain().is_err());
        assert!(text.is_flowed().is_err());
        assert!(text.is_html().is_err());
        assert!(text.is_enriched().is_err());
        assert!(text.is_rich_text().is_err());
        assert_eq!(text.part().headers(), &before);
    }
    replace_type(&mut text, "text/html");
    assert_eq!(text.format().unwrap(), TextFormat::Html);
}

#[test]
fn application_rtf_remains_an_ordinary_rich_text_body_not_a_text_wrapper() {
    let mut entity: MimeEntity =
        MimePart::new(MimeContent::new(Cursor::new(b"rtf".to_vec()))).into();
    entity
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: application/rtf\r\n").unwrap())
        .unwrap();
    assert!(
        entity
            .text_body_part(TextFormat::RichText)
            .unwrap()
            .is_some()
    );
    let MimeEntity::Part(part) = entity else {
        unreachable!()
    };
    assert!(matches!(
        TextPart::from_part(part),
        Err(TextPartError::NotText(_))
    ));
}

#[test]
fn text_replacement_preserves_format_and_serialization_round_trips_metadata() {
    for format in FORMATS {
        let mut text = TextPart::with_format_and_charset(format, "old", Charset::LATIN1).unwrap();
        text.part_mut()
            .set_content_transfer_encoding(ContentEncoding::Base64);
        text.part_mut()
            .content_mut()
            .set_new_line_format(Some(NewLineFormat::Mixed));
        text.set_text("caf\u{e9}\nunchanged ").unwrap();
        assert_eq!(text.format().unwrap(), format);
        let mut output = Vec::new();
        text.part_mut().write_to(&mut output).unwrap();
        let MimeEntity::Part(part) = MimeEntity::load(&mut output.as_slice()).unwrap() else {
            unreachable!()
        };
        let mut parsed = TextPart::from_part(part).unwrap();
        assert_eq!(parsed.format().unwrap(), format);
        assert_eq!(flags(&parsed), expected_flags(format));
        assert_eq!(parsed.text().unwrap(), "caf\u{e9}\nunchanged ");
    }
}

// Adapted from MimeKit's UnitTests/MultipartAlternativeTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MessagePart, MimeContent, MimeContentStorage, MimeEntity,
    MimeMessage, MimeParser, MimePart, Multipart, MultipartAlternative, MultipartAlternativeError,
    MultipartError, TextBodyError, TextFormat, TextPart, TextPartError,
};

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn raw_container(raw: &[u8]) -> Multipart<()> {
    Multipart::from_parts(headers(raw), Vec::new(), Vec::new(), Vec::new())
}

#[test]
fn constructors_generate_or_preserve_boundaries_without_source_traits() {
    let first: MultipartAlternative<()> = MultipartAlternative::new().unwrap();
    let second: MultipartAlternative<()> = MultipartAlternative::new().unwrap();
    assert_ne!(
        first.multipart().boundary().unwrap(),
        second.multipart().boundary().unwrap()
    );
    assert_eq!(first.multipart().headers().len(), 1);
    assert!(first.multipart().children().is_empty());
    assert!(first.text_body_part(TextFormat::Plain).unwrap().is_none());
    let explicit: MultipartAlternative<()> = MultipartAlternative::with_boundary("chosen").unwrap();
    assert_eq!(
        explicit.multipart().boundary().unwrap().as_deref(),
        Some("chosen")
    );
    let entity: MimeEntity<()> = explicit.into();
    let MimeEntity::Multipart(part) = entity else {
        panic!()
    };
    let wrapped = MultipartAlternative::from_multipart(part).unwrap();
    assert_eq!(
        wrapped.multipart().boundary().unwrap().as_deref(),
        Some("chosen")
    );
    for boundary in ["", "bad\nboundary", "trailing "] {
        let error = MultipartAlternative::<()>::with_boundary(boundary).unwrap_err();
        assert!(matches!(
            error,
            MultipartAlternativeError::Multipart(MultipartError::InvalidBoundary { .. })
        ));
        assert!(error.source().is_some());
    }
}

#[test]
fn generic_constructor_headers_and_children_survive_typed_wrapping() {
    let mut multipart = Multipart::with_boundary("alternative", "b").unwrap();
    multipart.headers_mut().add(
        Header::parse(b"Content-Description: This is a description of the multipart.").unwrap(),
    );
    let mut image = MimePart::new(MimeContent::new(Cursor::new(Vec::new())));
    image
        .set_content_type(&mimecraft::ContentType::new("image", "gif").unwrap())
        .unwrap();
    image.set_file_name(Some("attachment.gif")).unwrap();
    multipart.children_mut().extend([
        TextPart::new("plain", "This is the message body.")
            .unwrap()
            .into(),
        image.into(),
    ]);
    let before = multipart.headers().clone();
    let children = multipart.children().as_ptr();
    let mut alternative = MultipartAlternative::from_multipart(multipart).unwrap();
    assert_eq!(alternative.multipart().headers(), &before);
    assert_eq!(alternative.multipart().children().as_ptr(), children);
    assert_eq!(alternative.multipart().children().len(), 2);
    for (child, expected) in alternative
        .multipart()
        .children()
        .iter()
        .zip(["text/plain", "image/gif"])
    {
        assert_eq!(
            child
                .headers()
                .get("Content-Type")
                .unwrap()
                .parse_content_type()
                .unwrap()
                .mime_type(),
            expected
        );
    }
    assert_eq!(
        alternative.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("This is the message body.")
    );
}

#[test]
fn wrapping_and_body_access_observe_first_container_headers() {
    let raw = b"Content-Type: MULTIPART/ALTERNATIVE\nContent-Type: invalid\nX-Keep: value\n";
    let mut alternative = MultipartAlternative::from_multipart(raw_container(raw)).unwrap();
    assert_eq!(alternative.multipart().headers(), &headers(raw));
    assert!(alternative.multipart().boundary().unwrap().is_none());
    assert!(
        alternative
            .text_body_part(TextFormat::Plain)
            .unwrap()
            .is_none()
    );
    for (value, expected) in [
        (None, "missing"),
        (Some("invalid"), "invalid"),
        (Some("multipart/mixed"), "other"),
        (Some("text/alternative"), "other"),
    ] {
        let mut fields = HeaderList::new();
        if let Some(value) = value {
            fields.add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
            fields.add(Header::parse(b"Content-Type: multipart/alternative; boundary=b").unwrap());
        }
        *alternative.multipart_mut().headers_mut() = fields.clone();
        let wrapped = MultipartAlternative::<()>::from_multipart(Multipart::from_parts(
            fields.clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap_err();
        for error in [
            wrapped,
            alternative.text_body_part(TextFormat::Plain).unwrap_err(),
        ] {
            match expected {
                "missing" => assert!(matches!(
                    error,
                    MultipartAlternativeError::MissingContentType
                )),
                "invalid" => {
                    assert!(matches!(error, MultipartAlternativeError::ContentType(_)));
                    assert!(error.source().is_some());
                }
                _ => assert!(matches!(
                    error,
                    MultipartAlternativeError::NotAlternative(_)
                )),
            }
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(alternative.multipart().headers(), &fields);
    }
}

fn inner_mut(alternative: &mut MultipartAlternative, depth: usize) -> &mut Multipart {
    let mut part = alternative.multipart_mut();
    for _ in 0..depth {
        let MimeEntity::Multipart(child) = &mut part.children_mut()[0] else {
            panic!()
        };
        part = child;
    }
    part
}

#[test]
fn upstream_text_body_cases_cover_direct_nested_related_and_mixed_alternatives() {
    for mode in ["direct", "nested", "related", "mixed"] {
        let mut inner = Multipart::with_boundary(
            if mode == "mixed" {
                "mixed"
            } else {
                "alternative"
            },
            "inner",
        )
        .unwrap();
        inner.children_mut().extend([
            TextPart::new("plain", "plain\n").unwrap().into(),
            TextPart::new("rtf", "rtf\n").unwrap().into(),
            TextPart::new("html", "html\n").unwrap().into(),
        ]);
        let depth = match mode {
            "direct" => 0,
            "related" => 2,
            _ => 1,
        };
        let mut alternative = if mode == "direct" {
            MultipartAlternative::from_multipart(inner).unwrap()
        } else {
            let child = if mode == "related" {
                let mut related = Multipart::with_boundary("related", "related").unwrap();
                related.children_mut().push(inner.into());
                related
            } else {
                inner
            };
            let mut outer = MultipartAlternative::with_boundary("outer").unwrap();
            outer.multipart_mut().children_mut().push(child.into());
            outer
        };
        assert_eq!(
            alternative.text_body(TextFormat::Plain).unwrap().as_deref(),
            Some("plain\n")
        );
        assert_eq!(
            alternative.text_body(TextFormat::Html).unwrap().as_deref(),
            if mode == "mixed" {
                None
            } else {
                Some("html\n")
            }
        );
        inner_mut(&mut alternative, depth).children_mut().insert(
            1,
            TextPart::with_format(TextFormat::Flowed, "flowed\n")
                .unwrap()
                .into(),
        );
        for (format, expected) in [
            (
                TextFormat::Plain,
                if mode == "mixed" {
                    Some("plain\n")
                } else {
                    Some("flowed\n")
                },
            ),
            (TextFormat::Flowed, Some("flowed\n")),
            (TextFormat::RichText, Some("rtf\n")),
            (TextFormat::Html, Some("html\n")),
            (TextFormat::Enriched, None),
        ] {
            // Flowed also matches Plain and wins when it is a later alternative.
            // Mixed containers, however, only consider their first text body.
            let expected = if mode == "mixed" && format != TextFormat::Plain {
                None
            } else {
                expected
            };
            let body = alternative
                .text_body(format)
                .unwrap()
                .map(|text| text.replace("\r\n", "\n"));
            assert_eq!(body.as_deref(), expected, "{mode}: {format:?}");
            assert_eq!(
                alternative.text_body_part(format).unwrap().is_some(),
                expected.is_some()
            );
        }
    }
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("unselected source read")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("unselected source seek")
    }
}

fn no_io_leaf(content_type: Option<&str>) -> MimeEntity<NoIo> {
    let mut fields = HeaderList::new();
    if let Some(value) = content_type {
        fields.add(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap());
    }
    MimePart::from_parts(fields, MimeContent::new(NoIo)).into()
}

#[test]
fn selection_keeps_attached_alternatives_and_ignores_unvisited_metadata() {
    let mut alternative = MultipartAlternative::with_boundary("b").unwrap();
    let mut attached = no_io_leaf(Some("text/plain; charset=x-unknown"));
    attached
        .headers_mut()
        .add(Header::parse(b"Content-Disposition: attachment").unwrap());
    alternative.multipart_mut().children_mut().extend([
        no_io_leaf(Some("invalid")),
        no_io_leaf(None),
        attached,
        MessagePart::new(MimeMessage::with_body(no_io_leaf(Some("text/html")))).into(),
    ]);
    let selected = alternative
        .text_body_part(TextFormat::Plain)
        .unwrap()
        .unwrap();
    let MimeEntity::Part(expected) = &alternative.multipart().children()[2] else {
        panic!()
    };
    assert!(std::ptr::eq(selected, expected));
    assert_eq!(
        selected.content_type().unwrap().unwrap().charset(),
        Some("x-unknown")
    );
    // A malformed visited earlier alternative is an error, not a reason to skip it.
    assert!(matches!(
        alternative.text_body_part(TextFormat::Html),
        Err(MultipartAlternativeError::Body(TextBodyError::ContentType(
            _
        )))
    ));
    alternative.multipart_mut().children_mut().remove(0);
    assert!(
        alternative
            .text_body_part(TextFormat::Html)
            .unwrap()
            .is_none()
    );
    alternative.multipart_mut().children_mut().remove(1);
    assert!(
        alternative
            .text_body_part(TextFormat::Plain)
            .unwrap()
            .is_some()
    );
}

#[test]
fn empty_bodies_are_not_missing_and_flowed_decoding_honors_delsp() {
    let mut alternative: MultipartAlternative = MultipartAlternative::with_boundary("b").unwrap();
    assert_eq!(alternative.text_body(TextFormat::Plain).unwrap(), None);
    alternative
        .multipart_mut()
        .children_mut()
        .push(TextPart::new("plain", "").unwrap().into());
    assert_eq!(
        alternative.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("")
    );
    for (delsp, expected) in [("yes", "caf\u{e9}suite\r\n"), ("no", "caf\u{e9} suite\r\n")] {
        let fields = headers(format!(
            "Content-Type: text/plain; charset=iso-8859-1; format=flowed; delsp={delsp}\r\nContent-Transfer-Encoding: 7bit\r\n",
        ).as_bytes());
        let part = MimePart::from_parts(
            fields,
            MimeContent::with_encoding(
                Cursor::new(b"caf=E9=20\r\nsuite".to_vec()),
                ContentEncoding::QuotedPrintable,
            ),
        );
        alternative.multipart_mut().children_mut().push(part.into());
        for format in [TextFormat::Plain, TextFormat::Flowed] {
            assert_eq!(
                alternative.text_body(format).unwrap().as_deref(),
                Some(expected)
            );
        }
    }
}

#[test]
fn decoding_preserves_nested_and_truncated_framing_in_both_storage_modes() {
    let mut wires = Vec::new();
    for end in ["", "--i-- \t\r\n--o-- \t\r\n"] {
        wires.push(format!(
            "Content-Type: multipart/alternative; boundary=o\r\n\r\n\
--o \t\r\nContent-Type: multipart/related; boundary=i\r\n\r\n\
--i \t\r\nContent-Type: text/plain; format=flowed\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\nsoft=20\r\nbreak\r\n{end}"
        ));
    }
    wires.push("Content-Type: multipart/alternative; boundary=o\r\n\r\n--o \t\r\nContent-Type: text/plain\r\n".into());
    for (index, wire) in wires.iter().enumerate() {
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser =
                MimeParser::with_storage(Cursor::new(wire.as_bytes()), storage).unwrap();
            let MimeEntity::Multipart(part) = parser.parse_entity().unwrap() else {
                panic!()
            };
            drop(parser);
            let mut alternative = MultipartAlternative::from_multipart(part).unwrap();
            let expected = if index == 2 { "" } else { "soft break\r\n" };
            for _ in 0..2 {
                assert_eq!(
                    alternative.text_body(TextFormat::Plain).unwrap().as_deref(),
                    Some(expected)
                );
            }
            let mut entity: MimeEntity<_> = alternative.into();
            assert_eq!(
                entity.text_body(TextFormat::Plain).unwrap().as_deref(),
                Some(expected)
            );
            let mut output = Vec::new();
            entity.write_to(&mut output).unwrap();
            assert_eq!(output, wire.as_bytes());
        }
    }
}

#[derive(Debug)]
struct Source {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_read: bool,
    fail_seek: Option<usize>,
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read {
            return Err(io::Error::other("read failed"));
        }
        self.bytes.read(bytes)
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
fn source_errors_and_charset_recovery_never_select_earlier_bodies() {
    for fault in ["none", "initial-seek", "read", "final-seek", "charset"] {
        let mut alternative = MultipartAlternative::with_boundary("b").unwrap();
        for selected in [false, true] {
            let mut bytes = Cursor::new(b"Qm9keQ==".to_vec());
            bytes.set_position(2);
            let source = Source {
                bytes,
                reads: 0,
                seeks: 0,
                fail_read: selected && fault == "read",
                fail_seek: if selected {
                    match fault {
                        "initial-seek" => Some(1),
                        "final-seek" => Some(2),
                        _ => None,
                    }
                } else {
                    None
                },
            };
            let charset = if selected && fault == "charset" {
                "x-unknown"
            } else {
                "utf-8"
            };
            let fields =
                headers(format!("Content-Type: text/plain; charset={charset}\r\n").as_bytes());
            alternative.multipart_mut().children_mut().push(
                MimePart::from_parts(
                    fields,
                    MimeContent::with_encoding(source, ContentEncoding::Base64),
                )
                .into(),
            );
        }
        let result = alternative.text_body(TextFormat::Plain);
        match fault {
            "none" | "charset" => assert_eq!(result.unwrap().as_deref(), Some("Body")),
            _ => assert!(matches!(
                result,
                Err(MultipartAlternativeError::Body(TextBodyError::Text(
                    TextPartError::Io(_)
                )))
            )),
        }
        let MimeEntity::Part(first) = &alternative.multipart().children()[0] else {
            panic!()
        };
        assert_eq!(first.content().get_ref().reads, 0);
        assert_eq!(first.content().get_ref().seeks, 0);
        assert_eq!(first.content().get_ref().bytes.position(), 2);
        let selected = alternative
            .text_body_part(TextFormat::Plain)
            .unwrap()
            .unwrap()
            .content()
            .get_ref();
        if matches!(fault, "none" | "charset") {
            assert_eq!(selected.seeks, 2);
            assert_eq!(selected.bytes.position(), 0);
        }
    }
}

// Includes tests adapted from MimeKit's UnitTests/MultipartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
    iter::FusedIterator,
};

use mimecraft::{
    ContentType, Header, HeaderFormatOptions, HeaderList, HeaderParser, MimeContent,
    MimeContentStorage, MimeEntity, MimeParser, MimePart, MimePartWriteOptions, Multipart,
    MultipartError, MultipartRelated, MultipartReport, ParameterComplianceMode,
    ParameterParseOptions, TextPart,
    charsets::{Charset, CharsetError},
    encodings::{EncoderOptionsError, NewLineFormat},
};

fn multipart() -> Multipart {
    Multipart::with_boundary("mixed", "outer-boundary").unwrap()
}

fn leaf(bytes: &[u8]) -> MimeEntity {
    let mut stream = Cursor::new(bytes.to_vec());
    stream.set_position(1);
    MimePart::new(MimeContent::new(stream)).into()
}

fn headers(bytes: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    assert_eq!(parser.parse(bytes, &mut records).unwrap(), bytes.len());
    parser.finish(&mut records).unwrap();
    records.into()
}

#[test]
fn construction_creates_only_checked_content_type_and_empty_components() {
    let part: Multipart = Multipart::with_boundary("Alternative", "one:two").unwrap();
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("multipart", "alternative")
    );
    assert_eq!(part.boundary().unwrap().as_deref(), Some("one:two"));
    assert_eq!(part.headers().len(), 1);
    assert_eq!(
        part.headers().as_slice()[0].raw(),
        b"Content-Type: multipart/Alternative; boundary=\"one:two\"\r\n"
    );
    assert!(part.children().is_empty());
    assert!(part.preamble().is_empty());
    assert!(part.epilogue().is_empty());
    assert_eq!(part.leaf_parts().count(), 0);
    assert!(matches!(
        Multipart::<Cursor<Vec<u8>>>::with_boundary("", "b"),
        Err(MultipartError::ContentType(_))
    ));
}

#[test]
fn default_construction_generates_a_boundary_and_needs_no_source_traits() {
    for subtype in [
        "mixed",
        "Alternative",
        "digest",
        "related",
        "report",
        "x-custom",
        "multipart/mixed",
    ] {
        let part: Multipart<()> = Multipart::new(subtype).unwrap();
        let boundary = part.boundary().unwrap().unwrap();
        assert_eq!(boundary.len(), 26);
        assert!(boundary.starts_with("=-"));
        assert!(boundary.ends_with("=="));
        assert_eq!(part.headers().len(), 1);
        assert_eq!(
            part.headers(),
            Multipart::<()>::with_boundary(subtype, &boundary)
                .unwrap()
                .headers()
        );
        assert!(part.children().is_empty());
        assert!(part.preamble().is_empty());
        assert!(part.epilogue().is_empty());
    }
    for subtype in ["", "bad subtype", "bad\r\n", "\u{e9}"] {
        assert!(
            matches!(
                Multipart::<()>::new(subtype),
                Err(MultipartError::ContentType(_))
            ),
            "{subtype:?}"
        );
    }
    assert!(matches!(
        Multipart::<()>::new(&"x".repeat(100)),
        Err(MultipartError::HeaderFormat(_))
    ));

    let boundary = Multipart::generate_boundary().unwrap();
    let related: MultipartRelated<()> = MultipartRelated::with_boundary(&boundary).unwrap();
    assert_eq!(
        related.multipart().boundary().unwrap(),
        Some(boundary.clone())
    );
    let report: MultipartReport<()> = MultipartReport::new("delivery-status", &boundary).unwrap();
    assert_eq!(report.multipart().boundary().unwrap(), Some(boundary));
}

#[test]
fn generated_boundaries_have_the_expected_shape_across_threads() {
    let boundaries = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    (0..64)
                        .map(|_| Multipart::generate_boundary().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut seen = std::collections::HashSet::new();
    for boundary in boundaries {
        assert_eq!(boundary.len(), 26);
        assert!(boundary.starts_with("=-"));
        assert!(boundary.ends_with("=="));
        assert!(
            boundary.as_bytes()[2..24]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
        );
        assert!(seen.insert(boundary));
    }
    assert_eq!(seen.len(), 256);
}

#[test]
fn generated_nested_boundaries_round_trip_with_both_newlines_and_storage_modes() {
    let mut alternative: Multipart = Multipart::new("alternative").unwrap();
    let inner_boundary = alternative.boundary().unwrap().unwrap();
    alternative
        .children_mut()
        .push(TextPart::new("plain", "hello").unwrap().into());
    alternative
        .children_mut()
        .push(TextPart::new("html", "<b>hello</b>").unwrap().into());
    let mut mixed: Multipart = Multipart::new("mixed").unwrap();
    let outer_boundary = mixed.boundary().unwrap().unwrap();
    assert_ne!(inner_boundary, outer_boundary);
    mixed.set_preamble_text("preamble");
    mixed.set_epilogue_text("epilogue");
    mixed.children_mut().push(alternative.into());

    for newline in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let options = MimePartWriteOptions {
            new_line_format: newline,
            ..Default::default()
        };
        let mut wire = Vec::new();
        mixed.write_to_with_options(&mut wire, &options).unwrap();
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser =
                MimeParser::with_storage(Cursor::new(wire.as_slice()), storage).unwrap();
            let mut entity = parser.parse_entity().unwrap();
            drop(parser);
            let MimeEntity::Multipart(outer) = &entity else {
                panic!()
            };
            assert_eq!(
                outer.boundary().unwrap().as_deref(),
                Some(outer_boundary.as_str())
            );
            let MimeEntity::Multipart(inner) = &outer.children()[0] else {
                panic!()
            };
            assert_eq!(
                inner.boundary().unwrap().as_deref(),
                Some(inner_boundary.as_str())
            );
            assert_eq!(entity.leaf_parts().count(), 2);
            for _ in 0..2 {
                let mut output = Vec::new();
                entity.write_to_with_options(&mut output, &options).unwrap();
                assert_eq!(output, wire);
            }
        }
    }
}

#[test]
fn generated_boundary_assignment_replaces_parsed_markers_including_truncation() {
    for closed in [false, true] {
        let tail = if closed {
            "\r\n--old-- \t\r\nepilogue\r\n"
        } else {
            ""
        };
        let wire = format!(
            "Content-Type: multipart/mixed; boundary=old\r\nX-Keep: yes\r\n\r\n\
preamble\r\n--old \t\r\nContent-Type: text/plain\r\n\r\nbody{tail}"
        );
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser =
                MimeParser::with_storage(Cursor::new(wire.as_bytes()), storage).unwrap();
            let mut entity = parser.parse_entity().unwrap();
            drop(parser);
            assert_eq!(text_edit_output(&mut entity), wire.as_bytes());
            let MimeEntity::Multipart(part) = &mut entity else {
                panic!()
            };
            let boundary = Multipart::generate_boundary().unwrap();
            part.set_boundary(&boundary).unwrap();
            // Once the boundary changes, MimeKit no longer preserves the raw
            // parsed markers; writing generates new markers, including the end.
            let epilogue = if closed { "epilogue\r\n" } else { "" };
            let expected = format!(
                "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\nX-Keep: yes\r\n\r\n\
preamble\r\n--{boundary}\r\nContent-Type: text/plain\r\n\r\nbody\r\n--{boundary}--\r\n{epilogue}"
            );
            for _ in 0..2 {
                assert_eq!(text_edit_output(&mut entity), expected.as_bytes());
            }
            let MimeEntity::Multipart(reparsed) =
                MimeEntity::load(&mut Cursor::new(expected.as_bytes())).unwrap()
            else {
                panic!()
            };
            assert_eq!(reparsed.boundary().unwrap(), Some(boundary));
            assert_eq!(reparsed.preamble(), b"preamble\r\n");
            assert_eq!(reparsed.epilogue(), epilogue.as_bytes());
            assert_eq!(reparsed.children().len(), 1);
            assert_eq!(
                reparsed
                    .leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .get_ref(),
                b"body"
            );
        }
    }
}

#[test]
fn boundary_validation_checks_the_exact_alphabet_and_length() {
    for length in 1..=70 {
        let boundary = "a".repeat(length);
        let part: Multipart = Multipart::with_boundary("mixed", &boundary).unwrap();
        assert_eq!(part.boundary().unwrap().as_deref(), Some(boundary.as_str()));
    }
    for byte in 0u8..=127 {
        let boundary = format!("a{}z", char::from(byte));
        let result = Multipart::<Cursor<Vec<u8>>>::with_boundary("mixed", &boundary);
        let valid = byte.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&byte);
        assert_eq!(result.is_ok(), valid, "byte {byte}");
        if !valid {
            assert!(matches!(
                result,
                Err(MultipartError::InvalidBoundary { offset: 1 })
            ));
        }
    }
    for (boundary, offset) in [
        ("", 0),
        ("ends ", 4),
        (" ", 0),
        ("a\u{e9}b", 1),
        ("a\r\nb", 1),
    ] {
        assert!(
            matches!(Multipart::<Cursor<Vec<u8>>>::with_boundary("mixed", boundary),
            Err(MultipartError::InvalidBoundary { offset: actual }) if actual == offset)
        );
    }
    assert!(matches!(
        Multipart::<Cursor<Vec<u8>>>::with_boundary("mixed", &"x".repeat(71)),
        Err(MultipartError::InvalidBoundary { offset: 70 })
    ));
    let leading: Multipart = Multipart::with_boundary("mixed", " leading").unwrap();
    assert_eq!(leading.boundary().unwrap().as_deref(), Some(" leading"));
}

#[test]
fn raw_components_are_adopted_and_extracted_without_repair_or_copying() {
    let raw = headers(b"Broken record\nX-Raw: \xff\r\nContent-Type: multipart/mixed; boundary=b\nCONTENT-TYPE: text/plain\n");
    let children = vec![leaf(b"first"), leaf(b"second")];
    let preamble = b"\xffpreamble\r\n".to_vec();
    let epilogue = b"\0epilogue\n".to_vec();
    let children_ptr = children.as_ptr();
    let preamble_ptr = preamble.as_ptr();
    let mut part = Multipart::from_parts(raw.clone(), children, preamble.clone(), epilogue.clone());
    assert_eq!(part.headers(), &raw);
    assert_eq!(part.children().as_ptr(), children_ptr);
    assert_eq!(part.preamble(), preamble);
    assert_eq!(part.epilogue(), epilogue);
    assert_eq!(part.boundary().unwrap().as_deref(), Some("b"));
    let old = part.set_preamble(preamble);
    assert_eq!(old, b"\xffpreamble\r\n");
    assert_eq!(part.preamble().as_ptr(), preamble_ptr);
    assert_eq!(part.set_epilogue(b"replacement".to_vec()), epilogue);
    let (actual_headers, actual_children, actual_preamble, actual_epilogue) = part.into_parts();
    assert_eq!(actual_headers, raw);
    assert_eq!(actual_children.as_ptr(), children_ptr);
    assert_eq!(actual_preamble.as_ptr(), preamble_ptr);
    assert_eq!(actual_epilogue, b"replacement");
    assert_eq!(
        actual_children[0]
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        1
    );
}

fn assert_folded_text(input: &str, expected: &str) {
    for newline in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = if newline == NewLineFormat::Dos {
            expected.replace('\n', "\r\n")
        } else {
            expected.to_owned()
        };
        // Text operations do not require readable or seekable child sources.
        let mut part: Multipart<()> = Multipart::with_boundary("mixed", "b").unwrap();
        let original_headers = part.headers().clone();
        part.set_preamble_text_with_new_line_format(input, newline)
            .unwrap();
        part.set_epilogue_text_with_new_line_format(input, newline)
            .unwrap();
        assert_eq!(part.preamble(), expected.as_bytes(), "preamble: {input:?}");
        assert_eq!(part.epilogue(), expected.as_bytes(), "epilogue: {input:?}");
        assert_eq!(part.preamble_text().unwrap(), expected);
        assert_eq!(part.epilogue_text().unwrap(), expected);
        assert_eq!(part.headers(), &original_headers);
        assert!(part.children().is_empty());
    }
    let mut part: Multipart<()> = Multipart::with_boundary("mixed", "b").unwrap();
    part.set_preamble_text(input);
    part.set_epilogue_text(input);
    assert_eq!(part.preamble(), expected.replace('\n', "\r\n").as_bytes());
    assert_eq!(part.epilogue(), part.preamble());
    part.set_preamble_text("");
    part.set_epilogue_text("");
    assert!(part.preamble().is_empty());
    assert!(part.epilogue().is_empty());
    assert_eq!(part.preamble_text().unwrap(), "");
    assert_eq!(part.epilogue_text().unwrap(), "");
}

#[test]
fn multiline_and_long_preamble_and_epilogue() {
    let cases = [
        (
            concat!(
                "This is a part in a (multipart) message generated with the MimeKit library.\n\n",
                "All of the parts of this message are identical, however they've been encoded ",
                "for transport using different methods.\n",
            ),
            concat!(
                "This is a part in a (multipart) message generated with the MimeKit library.\n\n",
                "All of the parts of this message are identical, however they've been encoded\n",
                "for transport using different methods.\n",
            ),
        ),
        (
            concat!(
                "This is a part in a (multipart) message generated with the MimeKit library. ",
                "All of the parts of this message are identical, however they've been encoded ",
                "for transport using different methods.",
            ),
            concat!(
                "This is a part in a (multipart) message generated with the MimeKit library.\n",
                "All of the parts of this message are identical, however they've been encoded\n",
                "for transport using different methods.\n",
            ),
        ),
        (
            "This is a multipart MIME message. If you are reading this text, then it means that your mail client does not support MIME.\n",
            "This is a multipart MIME message. If you are reading this text, then it means\nthat your mail client does not support MIME.\n",
        ),
        (
            "This is a multipart epilogue.",
            "This is a multipart epilogue.\n",
        ),
    ];
    for (input, expected) in cases {
        assert_folded_text(input, expected);
        assert_folded_text(&input.replace('\n', "\r\n"), expected);
    }
}

#[test]
fn text_folding_preserves_words_and_observes_exact_soft_limits() {
    for (input, expected) in [
        ("", ""),
        ("\n", "\n"),
        ("\r\n", "\n"),
        (" \t", " \t\n"),
        ("one \t\r\n \t\n  two \t\n", "one\n\n  two\n"),
        ("one\r two", "one\r two\n"),
        ("one\r", "one\r\n"),
        ("a\u{a0}b", "a\u{a0}b\n"),
        ("a\u{85}b", "a\u{85}b\n"),
    ] {
        assert_folded_text(input, expected);
    }
    for (input, expected) in [
        (
            format!("{} y", "x".repeat(75)),
            format!("{} y\n", "x".repeat(75)),
        ),
        (
            format!("{} y", "x".repeat(76)),
            format!("{}\ny\n", "x".repeat(76)),
        ),
        (
            format!("{}\t y", "x".repeat(76)),
            format!("{}\ny\n", "x".repeat(76)),
        ),
        (
            format!("{}\u{a0}y", "x".repeat(76)),
            format!("{}\ny\n", "x".repeat(76)),
        ),
        (
            format!("{} ", "x".repeat(77)),
            format!("{}\n", "x".repeat(77)),
        ),
        ("x".repeat(78), format!("{}\n", "x".repeat(78))),
        ("x".repeat(100_000), format!("{}\n", "x".repeat(100_000))),
        (
            format!("{} hello", "\u{e9}".repeat(36)),
            format!("{}\nhello\n", "\u{e9}".repeat(36)),
        ),
        (
            format!("{} end", "\u{1f600}".repeat(20)),
            format!("{}\nend\n", "\u{1f600}".repeat(20)),
        ),
    ] {
        assert_folded_text(&input, &expected);
    }
}

#[test]
fn text_getters_decode_strictly_without_losing_raw_bytes_or_initial_newlines() {
    let mut part: Multipart<()> = Multipart::with_boundary("mixed", "b").unwrap();
    assert_eq!(part.preamble_text().unwrap(), "");
    assert_eq!(part.epilogue_text().unwrap(), "");
    let preamble = b"caf\xe9\r\n";
    let epilogue = b"\r\ncaf\xe9\n";
    part.set_preamble(preamble.to_vec());
    part.set_epilogue(epilogue.to_vec());
    assert_eq!(
        part.preamble_text(),
        Err(CharsetError::InvalidBytes(Charset::UTF8))
    );
    assert_eq!(
        part.epilogue_text(),
        Err(CharsetError::InvalidBytes(Charset::UTF8))
    );
    assert_eq!(
        part.preamble_text_with_charset(Charset::LATIN1).unwrap(),
        "caf\u{e9}\r\n"
    );
    assert_eq!(
        part.epilogue_text_with_charset(Charset::LATIN1).unwrap(),
        "\r\ncaf\u{e9}\n"
    );
    assert_eq!(part.preamble(), preamble);
    assert_eq!(part.epilogue(), epilogue);
    part.set_preamble_text("\u{feff}caf\u{e9}");
    part.set_epilogue_text("\n\u{feff}caf\u{e9}");
    assert!(matches!(
        part.preamble_text().unwrap(),
        Cow::Borrowed("\u{feff}caf\u{e9}\r\n")
    ));
    assert!(matches!(
        part.epilogue_text().unwrap(),
        Cow::Borrowed("\r\n\u{feff}caf\u{e9}\r\n")
    ));
    assert_eq!(
        part.preamble_text_with_charset(Charset::ASCII),
        Err(CharsetError::InvalidBytes(Charset::ASCII))
    );
}

fn text_edit_output<R: Read + Seek>(entity: &mut MimeEntity<R>) -> Vec<u8> {
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    output
}

#[test]
fn text_no_ops_and_failed_edits_preserve_parsed_framing() {
    let wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\nold preamble\r\n\
--b \t\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b-- \t\r\n\r\nold epilogue\r\n";
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let mut entity = MimeParser::with_storage(Cursor::new(wire.as_slice()), storage)
            .unwrap()
            .parse_entity()
            .unwrap();
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        let original_headers = part.headers().clone();
        let preamble_ptr = part.preamble().as_ptr();
        let epilogue_ptr = part.epilogue().as_ptr();
        part.set_preamble_text("old preamble");
        part.set_epilogue_text("\nold epilogue");
        assert_eq!(part.preamble().as_ptr(), preamble_ptr);
        assert_eq!(part.epilogue().as_ptr(), epilogue_ptr);
        for text in ["", "replacement"] {
            assert_eq!(
                part.set_preamble_text_with_new_line_format(text, NewLineFormat::Mixed),
                Err(EncoderOptionsError::InvalidNewLineFormat(
                    NewLineFormat::Mixed
                ))
            );
            assert_eq!(
                part.set_epilogue_text_with_new_line_format(text, NewLineFormat::Mixed),
                Err(EncoderOptionsError::InvalidNewLineFormat(
                    NewLineFormat::Mixed
                ))
            );
        }
        assert_eq!(part.preamble_text().unwrap(), "old preamble\r\n");
        assert_eq!(part.epilogue_text().unwrap(), "\r\nold epilogue\r\n");
        assert_eq!(part.headers(), &original_headers);
        assert_eq!(text_edit_output(&mut entity), wire);
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        part.set_preamble_text("new preamble");
        part.set_epilogue_text("new epilogue");
        assert_eq!(
            text_edit_output(&mut entity),
            b"Content-Type: multipart/mixed; boundary=b\r\n\r\nnew preamble\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b--\r\nnew epilogue\r\n"
        );
    }
}

#[test]
fn changed_epilogue_restores_a_truncated_multipart_end_boundary() {
    // A parsed multipart without an end boundary must gain one when a new
    // epilogue is assigned; otherwise the epilogue would become child content.
    let wire = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody";
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let mut entity = MimeParser::with_storage(Cursor::new(wire.as_slice()), storage)
            .unwrap()
            .parse_entity()
            .unwrap();
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        part.set_epilogue_text("");
        assert_eq!(text_edit_output(&mut entity), wire);
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        part.set_epilogue_text("tail");
        let output = text_edit_output(&mut entity);
        assert_eq!(output, [wire.as_slice(), b"\r\n--b--\r\ntail\r\n"].concat());
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        part.set_epilogue_text("tail\r\n");
        assert_eq!(text_edit_output(&mut entity), output);
        let reparsed = MimeParser::new(output.as_slice()).parse_entity().unwrap();
        let MimeEntity::Multipart(part) = reparsed else {
            panic!()
        };
        assert_eq!(part.epilogue_text().unwrap(), "tail\r\n");
        assert_eq!(part.children().len(), 1);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            b"body"
        );
    }
}

#[test]
fn missing_and_invalid_raw_metadata_is_not_silently_repaired() {
    let mut part: Multipart =
        Multipart::from_parts(HeaderList::new(), Vec::new(), Vec::new(), Vec::new());
    assert_eq!(part.content_type().unwrap(), None);
    assert_eq!(part.boundary().unwrap(), None);
    assert!(matches!(
        part.set_boundary("b"),
        Err(MultipartError::MissingContentType)
    ));
    assert!(part.headers().is_empty());

    for (value, expected) in [
        ("multipart/mixed", "missing"),
        ("multipart/mixed; boundary=\"bad \"", "boundary"),
        ("text/plain; boundary=b", "type"),
        ("invalid", "syntax"),
    ] {
        part.headers_mut()
            .replace_all(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap())
            .unwrap();
        part.headers_mut()
            .add(Header::parse(b"CONTENT-TYPE: multipart/mixed; boundary=valid").unwrap());
        match expected {
            "missing" => assert_eq!(part.boundary().unwrap(), None),
            "boundary" => assert!(matches!(
                part.boundary(),
                Err(MultipartError::InvalidBoundary { .. })
            )),
            "type" => assert!(matches!(
                part.boundary(),
                Err(MultipartError::NotMultipart(_))
            )),
            _ => assert!(matches!(
                part.boundary(),
                Err(MultipartError::ContentType(_))
            )),
        }
    }
}

#[test]
fn detached_metadata_and_parse_options_follow_raw_header_authority() {
    let mut part = multipart();
    let mut snapshot = part.content_type().unwrap().unwrap();
    snapshot.set_boundary(Some("new"));
    assert_eq!(part.boundary().unwrap().as_deref(), Some("outer-boundary"));
    part.set_content_type(&snapshot).unwrap();
    assert_eq!(part.boundary().unwrap().as_deref(), Some("new"));
    part.headers_mut()
        .replace_all(Header::parse(b"Content-Type: multipart/mixed; boundary=two words").unwrap())
        .unwrap();
    assert_eq!(part.boundary().unwrap().as_deref(), Some("two words"));
    let options = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    assert!(part.content_type_with_options(&options).is_err());
}

#[test]
fn checked_edits_preserve_slots_parameters_children_and_unrelated_bytes() {
    let raw = headers(b"X-First: \xff\ncontent-type: multipart/mixed; boundary=old; x=keep\nMalformed\nCONTENT-TYPE: text/plain\nX-Last: last\n");
    let mut part = Multipart::from_parts(
        raw.clone(),
        vec![leaf(b"body")],
        b"pre".to_vec(),
        b"epi".to_vec(),
    );
    let child_ptr = part.children().as_ptr();
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    part.set_boundary_with_options("new:boundary", &options)
        .unwrap();
    assert_eq!(part.headers().len(), 4);
    assert_eq!(part.headers().index_of("Content-Type"), Some(1));
    assert_eq!(part.headers().last_index_of("Content-Type"), Some(1));
    assert_eq!(part.headers().as_slice()[0], raw.as_slice()[0]);
    assert_eq!(part.headers().as_slice()[2], raw.as_slice()[2]);
    assert_eq!(part.headers().as_slice()[3], raw.as_slice()[4]);
    let content_type = part.content_type().unwrap().unwrap();
    assert_eq!(content_type.parameters().get("x"), Some("keep"));
    assert_eq!(content_type.boundary(), Some("new:boundary"));
    assert_eq!(
        part.headers().as_slice()[1],
        content_type.to_header_with_options(&options).unwrap()
    );
    assert_eq!(part.children().as_ptr(), child_ptr);
    assert_eq!(part.preamble(), b"pre");
    assert_eq!(part.epilogue(), b"epi");
    assert_eq!(
        part.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        1
    );
}

#[test]
fn boundary_repairs_and_content_type_replacement_do_not_invent_defaults() {
    let mut part = multipart();
    for value in ["multipart/mixed", "multipart/mixed; boundary=\"bad \""] {
        part.headers_mut()
            .replace_all(Header::parse(format!("Content-Type: {value}").as_bytes()).unwrap())
            .unwrap();
        part.set_boundary("repaired").unwrap();
        assert_eq!(part.boundary().unwrap().as_deref(), Some("repaired"));
    }
    part.headers_mut().remove_all("Content-Type");
    part.headers_mut()
        .add(Header::parse(b"X-Keep: before").unwrap());
    let mut replacement = ContentType::new("MULTIPART", "related").unwrap();
    replacement.set_boundary(Some("replacement"));
    part.set_content_type(&replacement).unwrap();
    assert_eq!(part.headers().index_of("Content-Type"), Some(1));
    assert!(
        part.content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("multipart", "related")
    );
}

#[test]
fn failed_edits_are_atomic_and_preserve_error_sources() {
    let mut part = multipart();
    part.children_mut().push(leaf(b"body"));
    part.set_preamble(b"pre".to_vec());
    part.set_epilogue(b"epi".to_vec());
    let before = part.headers().clone();
    assert!(matches!(
        part.set_boundary("bad\nvalue"),
        Err(MultipartError::InvalidBoundary { offset: 3 })
    ));
    assert!(matches!(
        part.set_content_type(&ContentType::new("text", "plain").unwrap()),
        Err(MultipartError::NotMultipart(_))
    ));
    assert!(matches!(
        part.set_content_type(&ContentType::new("multipart", "mixed").unwrap()),
        Err(MultipartError::MissingBoundary)
    ));
    let error = part
        .set_boundary_with_options(
            "valid",
            &HeaderFormatOptions {
                max_line_length: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, MultipartError::HeaderFormat(_)));
    assert!(error.source().is_some());
    let mut oversized = ContentType::new("multipart", &"x".repeat(100)).unwrap();
    oversized.set_boundary(Some("valid"));
    assert!(matches!(
        part.set_content_type(&oversized),
        Err(MultipartError::HeaderFormat(_))
    ));
    assert_eq!(part.headers(), &before);
    assert_eq!(part.preamble(), b"pre");
    assert_eq!(part.epilogue(), b"epi");
    assert_eq!(
        part.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        1
    );
    assert_eq!(part.children().len(), 1);
}

#[test]
fn child_vector_edits_and_depth_first_leaf_iteration_preserve_order() {
    let mut inner = multipart();
    inner
        .children_mut()
        .extend([leaf(b"inner1"), multipart().into(), leaf(b"inner2")]);
    let mut outer = multipart();
    outer
        .children_mut()
        .extend([leaf(b"first"), inner.into(), leaf(b"last")]);
    outer.children_mut().insert(1, leaf(b"inserted"));
    let removed = outer.children_mut().remove(0);
    assert_eq!(
        removed
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref(),
        b"first"
    );
    outer.children_mut().swap(0, 2);
    let expected: Vec<&[u8]> = vec![b"last", b"inner1", b"inner2", b"inserted"];
    let actual: Vec<&[u8]> = outer
        .leaf_parts()
        .map(|part| part.content().get_ref().get_ref().as_slice())
        .collect();
    assert_eq!(actual, expected);
    let root: MimeEntity = outer.into();
    let mut leaves = root.leaf_parts();
    fn fused<T: FusedIterator>(_: &T) {}
    fused(&leaves);
    assert_eq!(leaves.by_ref().count(), 4);
    assert!(leaves.next().is_none());
    assert!(leaves.next().is_none());
    let leaf = leaf(b"root");
    assert_eq!(leaf.leaf_parts().count(), 1);
}

#[test]
fn deep_traversal_does_not_depend_on_content_type_or_boundary_validity() {
    let mut root = leaf(b"deep");
    for _ in 0..512 {
        let nested = Multipart::from_parts(HeaderList::new(), vec![root], Vec::new(), Vec::new());
        root = nested.into();
    }
    assert_eq!(root.leaf_parts().count(), 1);
    assert_eq!(
        root.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref(),
        b"deep"
    );
    let MimeEntity::Multipart(mut outer) = root else {
        panic!("expected container")
    };
    outer.children_mut().clear();
    assert_eq!(outer.leaf_parts().count(), 0);
}

#[test]
fn text_conversion_retains_body_allocation_and_node_headers_edit_in_place() {
    let text = TextPart::new("html", "<b>hello</b>").unwrap();
    let body_ptr = text.part().content().get_ref().get_ref().as_ptr();
    let mut entity: MimeEntity = text.into();
    entity
        .headers_mut()
        .add(Header::parse(b"X-Leaf: kept").unwrap());
    assert!(entity.headers().contains("X-Leaf"));
    assert_eq!(
        entity
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref()
            .as_ptr(),
        body_ptr
    );
    let MimeEntity::Part(part) = entity else {
        panic!("expected leaf")
    };
    let mut restored = TextPart::from_part(part).unwrap();
    assert_eq!(restored.text().unwrap(), "<b>hello</b>");

    let mut entity: MimeEntity = multipart().into();
    entity
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: text/plain").unwrap())
        .unwrap();
    assert!(matches!(entity, MimeEntity::Multipart(_)));
    assert_eq!(entity.leaf_parts().count(), 0);
}

#[test]
fn ownership_and_metadata_operations_never_touch_streams() {
    struct NoIo;
    impl Read for NoIo {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("unexpected read")
        }
    }
    impl Seek for NoIo {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            panic!("unexpected seek")
        }
    }
    let mut part = Multipart::with_boundary("mixed", "boundary").unwrap();
    part.children_mut()
        .push(MimePart::new(MimeContent::new(NoIo)).into());
    part.set_boundary("replacement").unwrap();
    assert_eq!(part.leaf_parts().count(), 1);
    part.into_parts();

    let mut source = Cursor::new(b"borrowed");
    source.set_position(3);
    let mut part = Multipart::with_boundary("mixed", "boundary").unwrap();
    part.children_mut()
        .push(MimePart::new(MimeContent::new(&mut source)).into());
    let entity = part.children_mut().pop().unwrap();
    drop(part);
    let MimeEntity::Part(leaf) = entity else {
        panic!("expected leaf")
    };
    assert_eq!(leaf.content().get_ref().position(), 3);
    drop(leaf);
    assert_eq!(source.position(), 3);
}

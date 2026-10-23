// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
    iter::FusedIterator,
};

use mimecraft::{
    ContentType, Header, HeaderFormatOptions, HeaderList, HeaderParser, MimeContent, MimeEntity,
    MimePart, Multipart, MultipartError, ParameterComplianceMode, ParameterParseOptions, TextPart,
    encodings::NewLineFormat,
};

fn multipart() -> Multipart {
    Multipart::new("mixed", "outer-boundary").unwrap()
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
    let part: Multipart = Multipart::new("Alternative", "one:two").unwrap();
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
        Multipart::<Cursor<Vec<u8>>>::new("", "b"),
        Err(MultipartError::ContentType(_))
    ));
}

#[test]
fn boundary_validation_checks_the_exact_alphabet_and_length() {
    for length in 1..=70 {
        let boundary = "a".repeat(length);
        let part: Multipart = Multipart::new("mixed", &boundary).unwrap();
        assert_eq!(part.boundary().unwrap().as_deref(), Some(boundary.as_str()));
    }
    for byte in 0u8..=127 {
        let boundary = format!("a{}z", char::from(byte));
        let result = Multipart::<Cursor<Vec<u8>>>::new("mixed", &boundary);
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
            matches!(Multipart::<Cursor<Vec<u8>>>::new("mixed", boundary),
            Err(MultipartError::InvalidBoundary { offset: actual }) if actual == offset)
        );
    }
    assert!(matches!(
        Multipart::<Cursor<Vec<u8>>>::new("mixed", &"x".repeat(71)),
        Err(MultipartError::InvalidBoundary { offset: 70 })
    ));
    let leading: Multipart = Multipart::new("mixed", " leading").unwrap();
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
    let mut part = Multipart::new("mixed", "boundary").unwrap();
    part.children_mut()
        .push(MimePart::new(MimeContent::new(NoIo)).into());
    part.set_boundary("replacement").unwrap();
    assert_eq!(part.leaf_parts().count(), 1);
    part.into_parts();

    let mut source = Cursor::new(b"borrowed");
    source.set_position(3);
    let mut part = Multipart::new("mixed", "boundary").unwrap();
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

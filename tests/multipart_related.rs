// Adapted from MimeKit's MultipartRelatedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    Header, HeaderFormatOptions, HeaderList, MessagePart, MimeContent, MimeEntity, MimeMessage,
    MimePart, Multipart, MultipartError, MultipartRelated, MultipartRelatedError, TextBodyError,
    TextFormat, TextPart, encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn leaf(media_type: &str, id: Option<&str>, label: &str) -> MimeEntity {
    let mut fields =
        headers(format!("Content-Type: {media_type}\r\nX-Label: {label}\r\n").as_bytes());
    if let Some(id) = id {
        fields.add(Header::parse(format!("Content-Id: {id}").as_bytes()).unwrap());
    }
    MimePart::from_parts(
        fields,
        MimeContent::new(Cursor::new(label.as_bytes().to_vec())),
    )
    .into()
}

fn related(parameters: &str) -> MultipartRelated {
    let fields =
        headers(format!("Content-Type: multipart/related; boundary=b{parameters}\r\n").as_bytes());
    MultipartRelated::from_multipart(Multipart::from_parts(
        fields,
        vec![
            leaf("image/png", Some("<image>"), "image"),
            leaf("text/html", Some("(comment) <root> (tail)"), "html"),
            leaf("text/plain", Some("<plain>"), "plain"),
        ],
        b"preamble\r\n".to_vec(),
        b"epilogue".to_vec(),
    ))
    .unwrap()
}

fn label<R>(entity: &MimeEntity<R>) -> String {
    entity.headers().get("X-Label").unwrap().decode_value().text
}

fn parameter(related: &MultipartRelated, name: &str) -> Option<String> {
    related
        .multipart()
        .content_type()
        .unwrap()
        .unwrap()
        .parameters()
        .get(name)
        .map(str::to_owned)
}

#[test]
fn constructors_and_accessors_do_not_require_source_traits() {
    let mut related: MultipartRelated<()> = MultipartRelated::new("boundary").unwrap();
    assert_eq!(related.multipart().headers().len(), 1);
    assert_eq!(related.root_index().unwrap(), None);
    assert!(related.root().unwrap().is_none());
    assert!(related.root_mut().unwrap().is_none());
    assert!(
        related
            .set_root(MessagePart::new(MimeMessage::new()).into())
            .unwrap()
            .is_none()
    );
    assert_eq!(related.root_index().unwrap(), Some(0));
    assert!(matches!(
        related.root().unwrap(),
        Some(MimeEntity::Message(_))
    ));
    let related = MultipartRelated::from_multipart(related.into_multipart()).unwrap();
    let entity: MimeEntity<()> = related.into();
    assert!(matches!(entity, MimeEntity::Multipart(_)));
    assert!(matches!(
        MultipartRelated::<()>::new(""),
        Err(MultipartRelatedError::Multipart(
            MultipartError::InvalidBoundary { .. }
        ))
    ));
}

#[test]
fn wrapping_and_later_access_check_first_type_without_repairing_it() {
    for (raw, expected) in [
        ("", "missing"),
        (
            "Content-Type: invalid\r\nContent-Type: multipart/related\r\n",
            "malformed",
        ),
        (
            "Content-Type: multipart/mixed\r\nContent-Type: multipart/related\r\n",
            "wrong",
        ),
    ] {
        let part: Multipart<()> =
            Multipart::from_parts(headers(raw.as_bytes()), vec![], vec![], vec![]);
        let error = MultipartRelated::from_multipart(part).unwrap_err();
        match expected {
            "missing" => assert!(matches!(error, MultipartRelatedError::MissingContentType)),
            "malformed" => {
                assert!(error.source().is_some());
                assert!(matches!(error, MultipartRelatedError::ContentType(_)));
            }
            _ => assert!(matches!(error, MultipartRelatedError::NotRelated(_))),
        }
    }
    let raw =
        headers(b"Content-Type: MULTIPART/RELATED; start=\"<>\"\r\nContent-Type: invalid\r\n");
    let part: Multipart<()> = Multipart::from_parts(raw.clone(), vec![], vec![], vec![]);
    let mut related = MultipartRelated::from_multipart(part).unwrap();
    assert_eq!(related.multipart().headers(), &raw);
    assert!(matches!(
        related.root_index(),
        Err(MultipartRelatedError::RootSelection(_))
    ));
    related
        .multipart_mut()
        .headers_mut()
        .remove_all("Content-Type");
    assert!(matches!(
        related.root(),
        Err(MultipartRelatedError::MissingContentType)
    ));
    assert!(matches!(
        related.root_mut(),
        Err(MultipartRelatedError::MissingContentType)
    ));
    assert!(matches!(
        related.set_root(MessagePart::new(MimeMessage::new()).into()),
        Err(MultipartRelatedError::MissingContentType)
    ));
}

#[test]
fn selection_shares_start_type_and_first_child_rules_with_body_lookup() {
    for (parameters, index) in [
        ("", 0),
        ("; type=\"TEXT/HTML\"", 1),
        ("; start=\"<root>\"; type=\"image/png\"", 1),
        ("; start=root", 1),
        ("; start=\"prose <root> <plain>\"", 1),
        ("; start=\"<plain>\"", 2),
        ("; start=\"<ROOT>\"; type=\"text/html\"", 0),
        ("; start=\"<absent>\"; type=\"text/html\"", 0),
        ("; type=\"application/absent\"", 0),
    ] {
        let related = related(parameters);
        assert_eq!(related.root_index().unwrap(), Some(index), "{parameters}");
        assert!(std::ptr::eq(
            related.root().unwrap().unwrap(),
            &related.multipart().children()[index]
        ));
        let entity: MimeEntity = related.into();
        assert_eq!(
            entity.text_body_part(TextFormat::Html).unwrap().is_some(),
            index == 1
        );
        assert_eq!(
            entity.text_body_part(TextFormat::Plain).unwrap().is_some(),
            index == 2
        );
    }
}

#[test]
fn mutable_root_access_observes_raw_edits_and_first_matching_id_wins() {
    let mut related = related("; start=\"<root>\"");
    related
        .root_mut()
        .unwrap()
        .unwrap()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Id: <changed>").unwrap())
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(0));
    assert_eq!(parameter(&related, "start").as_deref(), Some("<root>"));
    for child in related.multipart_mut().children_mut() {
        child
            .headers_mut()
            .replace_all(Header::parse(b"Content-Id: <root>").unwrap())
            .unwrap();
    }
    assert_eq!(related.root_index().unwrap(), Some(0));
}

#[test]
fn visited_metadata_errors_are_explicit_and_unneeded_metadata_is_ignored() {
    let malformed = related("; start=\"<>\"");
    assert!(matches!(
        malformed.root(),
        Err(MultipartRelatedError::RootSelection(
            TextBodyError::RelatedStart(_)
        ))
    ));
    let mut related = related("; start=\"<root>\"");
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .replace_all(Header::parse(b"Content-Id: <>").unwrap())
        .unwrap();
    assert!(matches!(
        related.root(),
        Err(MultipartRelatedError::RootSelection(
            TextBodyError::ContentId(_)
        ))
    ));

    let mut content_type = related.multipart().content_type().unwrap().unwrap();
    content_type.parameters_mut().remove("start");
    content_type
        .parameters_mut()
        .set("type", "text/html")
        .unwrap();
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(1));
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    assert!(matches!(
        related.root(),
        Err(MultipartRelatedError::RootSelection(
            TextBodyError::ContentType(_)
        ))
    ));

    content_type.parameters_mut().remove("type");
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(0));
}

#[test]
fn empty_and_first_child_roots_omit_start_and_do_not_require_content_id() {
    let mut related: MultipartRelated = MultipartRelated::new("b").unwrap();
    let mut content_type = related.multipart().content_type().unwrap().unwrap();
    content_type.parameters_mut().set("start", "<>").unwrap();
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    // An empty container does not examine stale selectors when installing a root.
    assert!(
        related
            .set_root(leaf("text/html", None, "first"))
            .unwrap()
            .is_none()
    );
    assert_eq!(parameter(&related, "type").as_deref(), Some("text/html"));
    assert_eq!(parameter(&related, "start"), None);
    let replaced = related
        .set_root(leaf("text/plain", Some("<>"), "new"))
        .unwrap()
        .unwrap();
    assert_eq!(label(&replaced), "first");
    assert_eq!(related.multipart().children().len(), 1);
    assert_eq!(label(related.root().unwrap().unwrap()), "new");
    assert_eq!(parameter(&related, "type").as_deref(), Some("text/plain"));
    assert_eq!(parameter(&related, "start"), None);
    assert_eq!(
        related
            .root()
            .unwrap()
            .unwrap()
            .headers()
            .get("Content-Id")
            .unwrap()
            .raw_value(),
        b" <>\r\n"
    );

    related
        .root_mut()
        .unwrap()
        .unwrap()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Id: <first>").unwrap())
        .unwrap();
    let mut content_type = related.multipart().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("start", "<first>")
        .unwrap();
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    assert!(
        related
            .set_root(leaf("text/html", None, "last"))
            .unwrap()
            .is_some()
    );
    assert_eq!(related.multipart().children().len(), 1);
    assert_eq!(parameter(&related, "start"), None);
}

#[test]
fn unmatched_selector_inserts_instead_of_replacing_the_getters_fallback() {
    for parameters in [
        "",
        "; type=\"image/gif\"",
        "; start=\"<missing>\"; type=\"text/html\"",
    ] {
        let mut related = related(parameters);
        assert_eq!(related.root_index().unwrap(), Some(0));
        assert!(
            related
                .set_root(leaf("text/html", None, "new"))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            related
                .multipart()
                .children()
                .iter()
                .map(label)
                .collect::<Vec<_>>(),
            ["new", "image", "html", "plain"]
        );
        assert_eq!(parameter(&related, "type").as_deref(), Some("text/html"));
        assert_eq!(parameter(&related, "start"), None);
        assert_eq!(related.root_index().unwrap(), Some(0));
    }
}

#[test]
fn non_first_root_replacement_sets_start_and_returns_the_old_child() {
    for parameters in ["; type=\"text/html\"", "; start=\"<root>\""] {
        let mut related = related(parameters);
        let new = leaf("text/plain", Some("(comment) <replacement> (tail)"), "new");
        let root_headers = new.headers().clone();
        let old = related.set_root(new).unwrap().unwrap();
        assert_eq!(label(&old), "html");
        assert_eq!(
            related
                .multipart()
                .children()
                .iter()
                .map(label)
                .collect::<Vec<_>>(),
            ["image", "new", "plain"]
        );
        assert_eq!(related.root_index().unwrap(), Some(1));
        assert_eq!(related.root().unwrap().unwrap().headers(), &root_headers);
        assert_eq!(
            parameter(&related, "start").as_deref(),
            Some("<replacement>")
        );
        assert_eq!(parameter(&related, "type").as_deref(), Some("text/plain"));
    }
}

#[test]
fn required_new_root_metadata_errors_leave_parent_and_children_unchanged() {
    for fault in ["missing-type", "invalid-type", "missing-id", "invalid-id"] {
        let mut related = related("; start=\"<root>\"");
        let mut new = leaf("text/html", Some("<new>"), "new");
        match fault {
            "missing-type" => {
                new.headers_mut().remove_all("Content-Type");
            }
            "invalid-type" => {
                new.headers_mut()
                    .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
                    .unwrap();
                new.headers_mut()
                    .add(Header::parse(b"Content-Type: text/html").unwrap());
            }
            "missing-id" => {
                new.headers_mut().remove_all("Content-Id");
            }
            _ => {
                new.headers_mut()
                    .replace_all(Header::parse(b"Content-Id: <>").unwrap())
                    .unwrap();
                new.headers_mut()
                    .add(Header::parse(b"Content-Id: <valid>").unwrap());
            }
        }
        let before = related.multipart().headers().clone();
        let children = related.multipart().children().as_ptr();
        let error = related.set_root(new).unwrap_err();
        match fault {
            "missing-type" => assert!(matches!(
                error,
                MultipartRelatedError::MissingRootContentType
            )),
            "invalid-type" => {
                assert!(error.source().is_some());
                assert!(matches!(error, MultipartRelatedError::RootContentType(_)));
            }
            "missing-id" => assert!(matches!(error, MultipartRelatedError::MissingRootContentId)),
            _ => {
                assert!(error.source().is_some());
                assert!(matches!(error, MultipartRelatedError::RootContentId(_)));
            }
        }
        assert_eq!(related.multipart().headers(), &before);
        assert_eq!(related.multipart().children().as_ptr(), children);
        assert_eq!(
            related
                .multipart()
                .children()
                .iter()
                .map(label)
                .collect::<Vec<_>>(),
            ["image", "html", "plain"]
        );
    }
}

#[test]
fn boundary_and_format_failures_are_atomic_and_recoverable() {
    for boundary in [None, Some("bad ")] {
        let raw = match boundary {
            None => "Content-Type: multipart/related; type=\"text/html\"\r\n".to_owned(),
            Some(value) => format!(
                "Content-Type: multipart/related; boundary=\"{value}\"; type=\"text/html\"\r\n"
            ),
        };
        let mut related = related("; type=\"text/html\"");
        *related.multipart_mut().headers_mut() = headers(raw.as_bytes());
        assert_eq!(related.root_index().unwrap(), Some(1));
        let before = related.multipart().headers().clone();
        let error = related
            .set_root(leaf("text/html", Some("<new>"), "new"))
            .unwrap_err();
        assert!(error.source().is_some());
        if boundary.is_none() {
            assert!(matches!(
                error,
                MultipartRelatedError::Multipart(MultipartError::MissingBoundary)
            ));
        } else {
            assert!(matches!(
                error,
                MultipartRelatedError::Multipart(MultipartError::InvalidBoundary { .. })
            ));
        }
        assert_eq!(related.multipart().headers(), &before);
        assert_eq!(label(related.root().unwrap().unwrap()), "html");
        related.multipart_mut().set_boundary("repaired").unwrap();
        assert!(
            related
                .set_root(leaf("text/html", Some("<new>"), "new"))
                .unwrap()
                .is_some()
        );
    }
    for options in [
        HeaderFormatOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        HeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        },
    ] {
        let mut related = related("; type=\"text/html\"");
        let before = related.multipart().headers().clone();
        let error = related
            .set_root_with_options(leaf("text/html", Some("<new>"), "new"), &options)
            .unwrap_err();
        assert!(error.source().unwrap().source().is_some());
        assert!(matches!(
            error,
            MultipartRelatedError::Multipart(MultipartError::HeaderFormat(_))
        ));
        assert_eq!(related.multipart().headers(), &before);
        assert_eq!(label(related.root().unwrap().unwrap()), "html");
    }
}

#[test]
fn replacement_preserves_other_metadata_bytes_allocations_and_positions() {
    let mut related = related("; type=\"text/html\"; x=keep; start-info=\"extra\"");
    related
        .multipart_mut()
        .headers_mut()
        .insert(0, Header::parse(b"X-First: \xff").unwrap())
        .unwrap();
    related
        .multipart_mut()
        .headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    related
        .multipart_mut()
        .headers_mut()
        .add(Header::parse(b"X-Last: value").unwrap());
    for child in related.multipart_mut().children_mut() {
        let MimeEntity::Part(part) = child else {
            panic!()
        };
        part.content_mut().get_mut().set_position(2);
    }
    let before = related.multipart().headers().clone();
    let children = related.multipart().children().as_ptr();
    let preamble = related.multipart().preamble().as_ptr();
    let epilogue = related.multipart().epilogue().as_ptr();
    let mut new = leaf("text/html", Some("<new>"), "new body");
    let MimeEntity::Part(part) = &mut new else {
        panic!()
    };
    part.content_mut().get_mut().set_position(3);
    let options = HeaderFormatOptions {
        new_line_format: NewLineFormat::Unix,
        always_quote_parameters: true,
        ..Default::default()
    };
    let old = related
        .set_root_with_options(new, &options)
        .unwrap()
        .unwrap();
    let MimeEntity::Part(old) = old else { panic!() };
    assert_eq!(old.content().get_ref().position(), 2);
    let part = related.multipart();
    assert_eq!(part.children().as_ptr(), children);
    assert_eq!(part.preamble().as_ptr(), preamble);
    assert_eq!(part.epilogue().as_ptr(), epilogue);
    assert_eq!(part.preamble(), b"preamble\r\n");
    assert_eq!(part.epilogue(), b"epilogue");
    assert_eq!(part.headers().get_all("Content-Type").count(), 1);
    assert_eq!(part.headers().index_of("Content-Type"), Some(1));
    assert_eq!(part.headers().as_slice()[0], before.as_slice()[0]);
    assert_eq!(part.headers().as_slice()[2], before.as_slice()[3]);
    let value = part.content_type().unwrap().unwrap();
    assert_eq!(
        part.headers().as_slice()[1],
        value.to_header_with_options(&options).unwrap()
    );
    assert_eq!(value.parameters().get("x"), Some("keep"));
    assert_eq!(value.parameters().get("start-info"), Some("extra"));
    assert_eq!(
        part.leaf_parts()
            .map(|leaf| leaf.content().get_ref().position())
            .collect::<Vec<_>>(),
        [2, 3, 2]
    );
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("root operations must not read child sources");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("root operations must not seek child sources");
    }
}

#[test]
fn borrowed_child_ownership_survives_root_replacement_without_io() {
    let mut first = NoIo;
    let mut second = NoIo;
    let mut related = MultipartRelated::new("b").unwrap();
    related
        .set_root(MimePart::new(MimeContent::new(&mut first)).into())
        .unwrap();
    let old = related
        .set_root(MimePart::new(MimeContent::new(&mut second)).into())
        .unwrap()
        .unwrap();
    assert!(matches!(old, MimeEntity::Part(_)));
    assert!(related.root().unwrap().is_some());
    let entity: MimeEntity<_> = related.into();
    assert_eq!(entity.body_parts().count(), 1);
    assert_eq!(entity.leaf_parts().count(), 1);
}

#[test]
fn nested_alternative_root_round_trips_and_reuses_body_selection() {
    let mut alternative = Multipart::new("alternative", "alt").unwrap();
    alternative
        .headers_mut()
        .add(Header::parse(b"Content-Id: <alternative>").unwrap());
    alternative
        .children_mut()
        .push(TextPart::new("plain", "plain body").unwrap().into());
    alternative
        .children_mut()
        .push(TextPart::new("html", "<p>html body</p>").unwrap().into());
    let mut related = related("; type=\"text/html\"");
    related.set_root(alternative.into()).unwrap();
    assert_eq!(
        parameter(&related, "type").as_deref(),
        Some("multipart/alternative")
    );
    let mut message = MimeMessage::with_body(related.into());
    assert!(message.text_body_part(TextFormat::Plain).unwrap().is_some());
    assert!(message.text_body_part(TextFormat::Html).unwrap().is_some());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert!(loaded.text_body_part(TextFormat::Plain).unwrap().is_some());
    assert!(loaded.text_body_part(TextFormat::Html).unwrap().is_some());
    let (_, Some(MimeEntity::Multipart(part))) = loaded.into_parts() else {
        panic!()
    };
    let related = MultipartRelated::from_multipart(part).unwrap();
    assert_eq!(related.root_index().unwrap(), Some(1));
    assert!(matches!(
        related.root().unwrap(),
        Some(MimeEntity::Multipart(_))
    ));
    let mut reloaded = MimeMessage::with_body(related.into());
    let mut second = Vec::new();
    reloaded.write_to(&mut second).unwrap();
    assert_eq!(second, wire);
}

// Adapted from MimeKit's MultipartRelatedTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, BufRead, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, Header, HeaderFormatOptions, HeaderList, MessageId, MessageIdParseError,
    MessagePart, MimeContent, MimeContentStorage, MimeEntity, MimeMessage, MimeParser, MimePart,
    Multipart, MultipartError, MultipartRelated, MultipartRelatedError, TextBodyError, TextFormat,
    TextPart, encodings::NewLineFormat,
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

fn related_with_resource<R: Read + Seek>(content: MimeContent<R>) -> MultipartRelated<R> {
    let mut related = MultipartRelated::with_boundary("b").unwrap();
    let mut part = MimePart::new(content);
    part.set_content_id(Some("image")).unwrap();
    related.multipart_mut().children_mut().push(part.into());
    related
}

fn uri_header(fields: &mut HeaderList, name: &str, value: &str) {
    fields
        .replace_all(Header::parse(format!("{name}: {value}\r\n").as_bytes()).unwrap())
        .unwrap();
}

#[test]
fn resources_are_referenced_by_content_location() {
    let mut related: MultipartRelated = MultipartRelated::new().unwrap();
    related.set_root(leaf("text/html", None, "html")).unwrap();
    for (media_type, location) in [("image/gif", "empty.gif"), ("image/jpg", "empty.jpg")] {
        let mut resource = leaf(media_type, None, location);
        uri_header(resource.headers_mut(), "Content-Location", location);
        related.multipart_mut().children_mut().push(resource);
    }
    // MimeKit omits start for a first-child root to work around Thunderbird's bug.
    assert_eq!(parameter(&related, "start"), None);
    assert_eq!(parameter(&related, "type").as_deref(), Some("text/html"));
    for (index, location) in ["empty.gif", "empty.jpg"].into_iter().enumerate() {
        assert_eq!(related.index_of_uri(location).unwrap(), Some(index + 1));
        assert!(std::ptr::eq(
            related.get_by_uri(location).unwrap().unwrap(),
            &related.multipart().children()[index + 1],
        ));
        let mut decoded = Vec::new();
        related
            .open_by_uri(location)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, location.as_bytes());
    }
}

#[test]
fn uri_locations_resolve_against_parent_base() {
    for (location, expected) in [
        ("images/logo.png", "http://example.com/mail/images/logo.png"),
        ("../images/logo.png", "http://example.com/images/logo.png"),
        ("/images/logo.png", "http://example.com/images/logo.png"),
        (
            "//cdn.example.com/logo.png",
            "http://cdn.example.com/logo.png",
        ),
        ("?new", "http://example.com/mail/index.html?new"),
        ("#image", "http://example.com/mail/index.html?old#image"),
        ("", "http://example.com/mail/index.html?old"),
        ("?", "http://example.com/mail/index.html?"),
        ("#", "http://example.com/mail/index.html?old#"),
        ("./a/../logo.png", "http://example.com/mail/logo.png"),
        ("../../../logo.png", "http://example.com/logo.png"),
        ("a/%2e%2E/logo.png", "http://example.com/mail/logo.png"),
        (
            "logo%2Epng?q=/../#a/../",
            "http://example.com/mail/logo%2Epng?q=/../#a/../",
        ),
        (
            "https://elsewhere.example/logo.png",
            "https://elsewhere.example/logo.png",
        ),
    ] {
        let mut related = related("");
        uri_header(
            related.multipart_mut().headers_mut(),
            "Content-Base",
            "http://example.com/mail/index.html?old",
        );
        uri_header(
            related.multipart_mut().children_mut()[0].headers_mut(),
            "Content-Location",
            location,
        );
        assert_eq!(
            related.index_of_uri(expected).unwrap(),
            Some(0),
            "{location}"
        );
        assert_eq!(
            related.index_of_uri(location).unwrap(),
            Some(0),
            "{location}"
        );
        let mut decoded = Vec::new();
        related
            .open_by_uri(expected)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, b"image");
    }
}

#[test]
fn uri_base_precedence_and_missing_base_follow_mimekit() {
    let mut related = related("");
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "images/logo.png",
    );
    let absolute = "http://example.com/images/logo.png";
    assert_eq!(related.index_of_uri(absolute).unwrap(), None);
    assert_eq!(related.index_of_uri("images/logo.png").unwrap(), Some(0));
    uri_header(
        related.multipart_mut().headers_mut(),
        "Content-Base",
        "http://example.com/",
    );
    assert_eq!(related.index_of_uri(absolute).unwrap(), Some(0));
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Base",
        "http://child.example/",
    );
    assert_eq!(related.index_of_uri(absolute).unwrap(), None);
    assert_eq!(
        related
            .index_of_uri("http://child.example/images/logo.png")
            .unwrap(),
        Some(0)
    );
    uri_header(
        related.multipart_mut().headers_mut(),
        "Content-Base",
        "invalid base",
    );
    assert_eq!(
        related
            .index_of_uri("http://child.example/images/logo.png")
            .unwrap(),
        Some(0)
    );
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Base",
        "invalid base",
    );
    assert_eq!(related.index_of_uri("images/logo.png").unwrap(), Some(0));
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        absolute,
    );
    assert_eq!(related.index_of_uri(absolute).unwrap(), Some(0));
}

#[test]
fn location_matching_does_not_normalize_uri_spelling() {
    let mut related = related("");
    let location = "http://example.com:80/mail/logo%2Epng?size=1#image";
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        location,
    );
    assert_eq!(related.index_of_uri(location).unwrap(), Some(0));
    for reference in [
        "HTTP://example.com:80/mail/logo%2Epng?size=1#image",
        "http://EXAMPLE.com:80/mail/logo%2Epng?size=1#image",
        "http://example.com/mail/logo%2Epng?size=1#image",
        "http://example.com:80/mail/logo%2epng?size=1#image",
        "http://example.com:80/mail/logo.png?size=1#image",
        "http://example.com:80/mail/logo%2Epng?size=1",
        "http://example.com:80/mail/logo%2Epng?size=2#image",
    ] {
        assert_eq!(
            related.index_of_uri(reference).unwrap(),
            None,
            "{reference}"
        );
    }
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "images/../logo.png",
    );
    assert_eq!(related.index_of_uri("images/../logo.png").unwrap(), Some(0));
    assert_eq!(related.index_of_uri("logo.png").unwrap(), None);
}

#[test]
fn cid_uri_matching_decodes_once_and_ignores_location_headers() {
    let mut related = related("; start=\"<>\"");
    related.multipart_mut().children_mut()[0]
        .set_content_id(Some("image+1%40part@example.com"))
        .unwrap();
    uri_header(
        related.multipart_mut().headers_mut(),
        "Content-Base",
        "invalid base",
    );
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "invalid location",
    );
    for reference in [
        "cid:image+1%2540part%40example.com",
        "CID:image%2b1%2540part@example.com",
    ] {
        assert_eq!(related.index_of_uri(reference).unwrap(), Some(0));
        let mut decoded = Vec::new();
        related
            .open_by_uri(reference)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, b"image");
    }
    for reference in [
        "cid:image+1%40part@example.com",
        "cid:IMAGE+1%2540part@example.com",
        "cid:absent",
    ] {
        assert_eq!(related.index_of_uri(reference).unwrap(), None);
    }
    for (id, reference) in [
        ("part/#?+@example.com", "cid:part%2F%23%3F%2B@example.com"),
        (
            "\"quoted local\"@example.com",
            "cid:%22quoted%20local%22@example.com",
        ),
        ("caf\u{e9}@example.com", "cid:caf%C3%A9@example.com"),
    ] {
        related.multipart_mut().children_mut()[0]
            .set_content_id(Some(id))
            .unwrap();
        assert_eq!(
            related.index_of_uri(reference).unwrap(),
            Some(0),
            "{reference}"
        );
    }
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .remove_all("Content-Id");
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "cid:absent",
    );
    assert_eq!(related.index_of_uri("cid:absent").unwrap(), None);
}

#[test]
fn invalid_uri_and_cid_input_returns_explicit_errors() {
    let related: MultipartRelated<()> = MultipartRelated::with_boundary("b").unwrap();
    for uri in ["bad uri", "%", "%0x", "http://[", "cid:foo%"] {
        let error = related.index_of_uri(uri).unwrap_err();
        assert!(matches!(error, MultipartRelatedError::Uri(_)));
        assert!(error.source().unwrap().is::<fluent_uri::ParseError>());
        assert!(!error.to_string().is_empty());
    }
    for uri in [
        "cid://image",
        "cid:image?query",
        "cid:image#fragment",
        "cid:image%0D%0A",
        "cid:%20image",
        "cid:%3Cimage%3E",
    ] {
        let error = related.index_of_uri(uri).unwrap_err();
        assert!(matches!(error, MultipartRelatedError::InvalidCid));
        assert!(error.source().is_none());
        assert!(!error.to_string().is_empty());
    }
    for uri in ["cid:", "cid:image%00", "cid:im%0D%0Aage"] {
        let error = related.index_of_uri(uri).unwrap_err();
        assert!(
            matches!(error, MultipartRelatedError::ContentId(_)),
            "{uri}: {error}"
        );
        assert!(error.source().unwrap().is::<MessageIdParseError>());
    }
    let error = related.index_of_uri("cid:%FF").unwrap_err();
    assert!(matches!(error, MultipartRelatedError::CidEncoding(_)));
    assert!(error.source().unwrap().is::<std::str::Utf8Error>());
    assert!(!error.to_string().is_empty());
}

#[test]
fn uri_headers_use_first_decoded_value_and_observe_raw_edits() {
    let mut related = related("");
    related.multipart_mut().children_mut()[0].headers_mut().add(
        Header::parse(b"Content-Location: \r\n\t=?us-ascii?q?images/logo=2Epng?=\r\n").unwrap(),
    );
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .add(Header::parse(b"Content-Location: ignored.png").unwrap());
    let before = related.multipart().children()[0].headers().clone();
    assert_eq!(related.index_of_uri("images/logo.png").unwrap(), Some(0));
    assert_eq!(related.index_of_uri("ignored.png").unwrap(), None);
    assert_eq!(related.multipart().children()[0].headers(), &before);
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "edited.png",
    );
    assert_eq!(related.index_of_uri("images/logo.png").unwrap(), None);
    assert_eq!(related.index_of_uri("edited.png").unwrap(), Some(0));
    uri_header(
        related.multipart_mut().headers_mut(),
        "Content-Base",
        "=?us-ascii?q?http://example.com/mail/?=",
    );
    assert_eq!(
        related
            .index_of_uri("http://example.com/mail/edited.png")
            .unwrap(),
        Some(0)
    );
}

#[test]
fn uri_lookup_reports_only_visited_metadata_errors_without_fallback() {
    let mut related = related("");
    for (index, location) in ["first.png", "bad uri", "first.png"]
        .into_iter()
        .enumerate()
    {
        uri_header(
            related.multipart_mut().children_mut()[index].headers_mut(),
            "Content-Location",
            location,
        );
    }
    related.multipart_mut().children_mut()[1]
        .headers_mut()
        .add(Header::parse(b"Content-Location: second.png").unwrap());
    assert_eq!(related.index_of_uri("first.png").unwrap(), Some(0));
    let error = related.index_of_uri("second.png").unwrap_err();
    assert!(matches!(error, MultipartRelatedError::ContentLocation(_)));
    assert!(error.source().unwrap().is::<fluent_uri::ParseError>());
    assert!(!error.to_string().is_empty());
    uri_header(
        related.multipart_mut().headers_mut(),
        "Content-Base",
        "http://example.com/",
    );
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Base",
        "relative/base",
    );
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .add(Header::parse(b"Content-Base: http://example.com/").unwrap());
    let error = related
        .index_of_uri("http://example.com/first.png")
        .unwrap_err();
    assert!(matches!(error, MultipartRelatedError::ContentBase(_)));
    assert!(error.source().unwrap().is::<fluent_uri::ParseError>());
    assert!(!error.to_string().is_empty());
    for base in ["http://example.com/#fragment", "urn:opaque"] {
        uri_header(
            related.multipart_mut().children_mut()[0].headers_mut(),
            "Content-Base",
            base,
        );
        let error = related
            .index_of_uri("http://example.com/first.png")
            .unwrap_err();
        assert!(matches!(error, MultipartRelatedError::UriResolution(_)));
        assert!(
            error
                .source()
                .unwrap()
                .is::<fluent_uri::resolve::ResolveError>()
        );
        assert!(!error.to_string().is_empty());
    }
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Id",
        "<>",
    );
    assert!(matches!(
        related.index_of_uri("cid:plain"),
        Err(MultipartRelatedError::ContentId(_))
    ));
}

#[test]
fn uri_lookup_checks_parent_type_but_not_root_boundary_or_child_type() {
    let mut related = related("; start=\"<>\"; type=\"bad\"");
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Type",
        "invalid",
    );
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Location",
        "image",
    );
    assert_eq!(related.index_of_uri("image").unwrap(), Some(0));
    for value in ["multipart/related", "multipart/related; boundary=\"bad \""] {
        uri_header(related.multipart_mut().headers_mut(), "Content-Type", value);
        assert_eq!(related.index_of_uri("image").unwrap(), Some(0));
    }
    for (raw, kind) in [
        ("", 0),
        ("Content-Type: invalid\r\n", 1),
        ("Content-Type: multipart/mixed\r\n", 2),
    ] {
        *related.multipart_mut().headers_mut() = headers(raw.as_bytes());
        let error = related.index_of_uri("image").unwrap_err();
        match kind {
            0 => assert!(matches!(error, MultipartRelatedError::MissingContentType)),
            1 => assert!(matches!(error, MultipartRelatedError::ContentType(_))),
            _ => assert!(matches!(error, MultipartRelatedError::NotRelated(_))),
        }
    }
}

#[test]
fn uri_lookup_is_shallow_and_nonleaf_opening_never_falls_back() {
    for multipart in [false, true] {
        let mut related: MultipartRelated<NoIo> = MultipartRelated::with_boundary("b").unwrap();
        let mut descendant = MimePart::new(MimeContent::new(NoIo));
        uri_header(descendant.headers_mut(), "Content-Location", "deep.png");
        let mut wrapper: MimeEntity<NoIo> = if multipart {
            let mut nested = Multipart::with_boundary("mixed", "inner").unwrap();
            nested.children_mut().push(descendant.into());
            nested.into()
        } else {
            MessagePart::new(MimeMessage::with_body(descendant.into())).into()
        };
        uri_header(wrapper.headers_mut(), "Content-Location", "resource");
        related.multipart_mut().children_mut().push(wrapper);
        let mut part = MimePart::new(MimeContent::new(NoIo));
        uri_header(part.headers_mut(), "Content-Location", "resource");
        related.multipart_mut().children_mut().push(part.into());
        assert_eq!(related.index_of_uri("deep.png").unwrap(), None);
        assert!(!related.contains_uri("deep.png").unwrap());
        assert_eq!(related.index_of_uri("resource").unwrap(), Some(0));
        assert!(related.contains_uri("resource").unwrap());
        assert!(related.get_by_uri("resource").unwrap().is_some());
        for uri in ["resource", "absent"] {
            let error = related.open_by_uri(uri).err().unwrap();
            assert!(
                matches!(&error, MultipartRelatedError::UriResourceNotFound(value) if value == uri)
            );
            assert!(error.source().is_none());
            assert!(!error.to_string().is_empty());
            assert!(matches!(
                related.open_by_uri_with_metadata(uri),
                Err(MultipartRelatedError::UriResourceNotFound(value)) if value == uri
            ));
        }
    }
}

#[test]
fn uri_open_is_lazy_restarts_and_propagates_source_errors() {
    let mut source = ResourceSource::new();
    {
        let mut related = related_with_resource(MimeContent::with_encoding(
            &mut source,
            ContentEncoding::Base64,
        ));
        uri_header(
            related.multipart_mut().children_mut()[0].headers_mut(),
            "Content-Location",
            "image",
        );
        assert_eq!(related.index_of_uri("image").unwrap(), Some(0));
        assert!(related.get_by_uri("image").unwrap().is_some());
        for seeks in 1..=2 {
            let mut reader = related.open_by_uri("image").unwrap();
            assert_eq!(reader.get_ref().seeks, seeks);
            assert_eq!(reader.get_ref().bytes.position(), 0);
            if seeks == 1 {
                assert_eq!(reader.get_ref().reads, 0);
            }
            let mut byte = [0];
            reader.read_exact(&mut byte).unwrap();
            assert_eq!(&byte, b"C");
        }
    }
    source.fail_seek = true;
    {
        let mut related = related_with_resource(MimeContent::new(&mut source));
        let error = related.open_by_uri("cid:image").err().unwrap();
        assert!(matches!(error, MultipartRelatedError::Io(_)));
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    let mut source = ResourceSource::new();
    source.fail_read_at = Some(1);
    let mut related =
        related_with_resource(MimeContent::with_encoding(source, ContentEncoding::Base64));
    let mut reader = related.open_by_uri("cid:image").unwrap();
    let mut bytes = [0; 3];
    assert_eq!(
        reader.read(&mut bytes).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"Cat");
}

#[test]
fn resource_open_decodes_stored_encoding_without_changing_bytes_or_headers() {
    let id = MessageId::from_value("image").unwrap();
    for (encoding, stored, expected) in [
        (
            ContentEncoding::Base64,
            b"R0lG\r\n".as_slice(),
            b"GIF".as_slice(),
        ),
        (
            ContentEncoding::QuotedPrintable,
            b"GIF=00=FF\r\n",
            b"GIF\0\xff\r\n",
        ),
        (
            ContentEncoding::UuEncode,
            b"begin 644 image\n#1TE&\n`\nend\n",
            b"GIF",
        ),
        (ContentEncoding::Binary, b"\0\xff\r\n", b"\0\xff\r\n"),
        (ContentEncoding::Base64, b"", b""),
        (ContentEncoding::Base64, b"Q", b""),
    ] {
        let mut source = Cursor::new(stored.to_vec());
        source.set_position(stored.len() as u64);
        let mut related = related_with_resource(MimeContent::with_encoding(source, encoding));
        let MimeEntity::Part(part) = &mut related.multipart_mut().children_mut()[0] else {
            panic!()
        };
        part.set_content_transfer_encoding(ContentEncoding::SevenBit);
        part.headers_mut()
            .add(Header::parse(b"Content-Type: invalid").unwrap());
        let before = part.headers().clone();
        for _ in 0..2 {
            let mut decoded = Vec::new();
            related
                .open_by_content_id(&id)
                .unwrap()
                .read_to_end(&mut decoded)
                .unwrap();
            assert_eq!(decoded, expected);
            let MimeEntity::Part(part) = related.get_by_content_id(&id).unwrap().unwrap() else {
                panic!()
            };
            assert_eq!(part.headers(), &before);
            assert_eq!(part.content().encoding(), encoding);
            assert_eq!(part.content().get_ref().get_ref(), stored);
        }
    }
}

#[derive(Debug)]
struct ResourceSource {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    seeks: usize,
    fail_seek: bool,
    fail_read_at: Option<usize>,
}

impl ResourceSource {
    fn new() -> Self {
        Self {
            bytes: Cursor::new(b"Q2F0".to_vec()),
            reads: 0,
            seeks: 0,
            fail_seek: false,
            fail_read_at: None,
        }
    }
}

impl Read for ResourceSource {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.fail_read_at == Some(self.reads) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "resource read"));
        }
        let count = output.len().min(1);
        self.bytes.read(&mut output[..count])
    }
}

impl Seek for ResourceSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "resource seek",
            ));
        }
        self.bytes.seek(position)
    }
}

#[test]
fn resource_open_is_lazy_borrows_the_source_and_restarts_after_partial_reads() {
    let id = MessageId::from_value("image").unwrap();
    let mut source = ResourceSource::new();
    source.bytes.set_position(3);
    {
        let mut related = related_with_resource(MimeContent::with_encoding(
            &mut source,
            ContentEncoding::Base64,
        ));
        {
            let mut reader = related.open_by_content_id(&id).unwrap();
            assert_eq!(reader.get_ref().bytes.position(), 0);
            assert_eq!(reader.get_ref().seeks, 1);
            assert_eq!(reader.get_ref().reads, 0);
            assert_eq!(reader.fill_buf().unwrap(), b"Cat");
            reader.consume(1);
            assert_eq!(reader.get_ref().bytes.position(), 4);
        }
        let mut reader = related.open_by_content_id(&id).unwrap();
        assert_eq!(reader.get_ref().seeks, 2);
        assert_eq!(reader.get_ref().bytes.position(), 0);
        let mut decoded = Vec::new();
        reader.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, b"Cat");
    }
    assert_eq!(source.seeks, 2);
    assert_eq!(source.bytes.position(), 4);
}

#[test]
fn resource_open_propagates_seek_errors_and_reader_preserves_state_after_read_errors() {
    let id = MessageId::from_value("image").unwrap();
    let mut source = ResourceSource::new();
    source.fail_seek = true;
    let mut related = related_with_resource(MimeContent::with_encoding(
        &mut source,
        ContentEncoding::Base64,
    ));
    let error = related.open_by_content_id(&id).err().unwrap();
    assert!(matches!(error, MultipartRelatedError::Io(_)));
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    drop(related);
    assert_eq!(source.reads, 0);
    assert_eq!(source.seeks, 1);

    let mut source = ResourceSource::new();
    source.fail_read_at = Some(3);
    let mut related =
        related_with_resource(MimeContent::with_encoding(source, ContentEncoding::Base64));
    let mut reader = related.open_by_content_id(&id).unwrap();
    let mut buffer = [0; 3];
    assert_eq!(
        reader.read(&mut buffer).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(reader.get_ref().bytes.position(), 2);
    reader.read_exact(&mut buffer).unwrap();
    assert_eq!(&buffer, b"Cat");
}

#[test]
fn missing_nonleaf_and_malformed_resources_fail_without_source_io_or_fallback() {
    let id = MessageId::from_value("image").unwrap();
    let mut related: MultipartRelated<NoIo> = MultipartRelated::with_boundary("b").unwrap();
    let error = related.open_by_content_id(&id).err().unwrap();
    assert!(matches!(error, MultipartRelatedError::ResourceNotFound(ref value) if value == &id));
    assert!(error.source().is_none());
    for multipart in [false, true] {
        let mut container: MimeEntity<NoIo> = if multipart {
            Multipart::with_boundary("mixed", "inner").unwrap().into()
        } else {
            MessagePart::new(MimeMessage::new()).into()
        };
        container.set_content_id(Some(id.as_str())).unwrap();
        related.multipart_mut().children_mut().clear();
        related.multipart_mut().children_mut().push(container);
        let mut part = MimePart::new(MimeContent::new(NoIo));
        part.set_content_id(Some(id.as_str())).unwrap();
        related.multipart_mut().children_mut().push(part.into());
        assert!(matches!(
            related.open_by_content_id(&id).err().unwrap(),
            MultipartRelatedError::ResourceNotFound(_)
        ));
        assert!(related.contains_content_id(&id).unwrap());
        assert!(matches!(
            related.open_by_content_id_with_metadata(&id),
            Err(MultipartRelatedError::ResourceNotFound(_))
        ));
    }
    related.multipart_mut().children_mut().remove(0);
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .remove_all("Content-Id");
    related.multipart_mut().children_mut()[0]
        .headers_mut()
        .add(Header::parse(b"Content-Id: <>").unwrap());
    assert!(matches!(
        related.open_by_content_id(&id).err().unwrap(),
        MultipartRelatedError::ContentId(_)
    ));
    related
        .multipart_mut()
        .headers_mut()
        .remove_all("Content-Type");
    assert!(matches!(
        related.open_by_content_id(&id).err().unwrap(),
        MultipartRelatedError::MissingContentType
    ));
}

#[test]
fn resource_open_preserves_parsed_empty_content_and_missing_separators() {
    let id = MessageId::from_value("image").unwrap();
    for last in [
        "Content-Id: <image>",
        "Content-Id: <image>\r\n",
        "Content-Id: <image>\r\n\r\n",
    ] {
        let wire = format!("Content-Type: multipart/related; boundary=b\r\n\r\n--b \t\r\n{last}");
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser =
                MimeParser::with_storage(Cursor::new(wire.as_bytes()), storage).unwrap();
            let MimeEntity::Multipart(part) = parser.parse_entity().unwrap() else {
                panic!()
            };
            drop(parser);
            let mut related = MultipartRelated::from_multipart(part).unwrap();
            let mut decoded = Vec::new();
            related
                .open_by_content_id(&id)
                .unwrap()
                .read_to_end(&mut decoded)
                .unwrap();
            assert!(decoded.is_empty());
            related
                .open_by_uri("cid:image")
                .unwrap()
                .read_to_end(&mut decoded)
                .unwrap();
            assert!(decoded.is_empty());
            let mut entity: MimeEntity<_> = related.into();
            let mut output = Vec::new();
            entity.write_to(&mut output).unwrap();
            assert_eq!(output, wire.as_bytes());
        }
    }
}

#[test]
fn resources_are_referenced_by_content_id() {
    let mut related: MultipartRelated = MultipartRelated::new().unwrap();
    related.set_root(leaf("text/html", None, "html")).unwrap();
    let ids = [
        MessageId::from_value("empty.gif@example.com").unwrap(),
        MessageId::from_value("empty.jpg@example.com").unwrap(),
    ];
    for (media_type, id) in ["image/gif", "image/jpg"].into_iter().zip(&ids) {
        let mut resource = leaf(media_type, None, media_type);
        resource.set_content_id(Some(id.as_str())).unwrap();
        related.multipart_mut().children_mut().push(resource);
    }
    // MimeKit omits start for a first-child root to work around Thunderbird's bug.
    assert_eq!(parameter(&related, "start"), None);
    assert_eq!(parameter(&related, "type").as_deref(), Some("text/html"));
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(related.index_of_content_id(id).unwrap(), Some(index + 1));
        assert!(std::ptr::eq(
            related.get_by_content_id(id).unwrap().unwrap(),
            &related.multipart().children()[index + 1],
        ));
    }
    let missing = MessageId::from_value("missing").unwrap();
    assert_eq!(related.index_of_content_id(&missing).unwrap(), None);
    assert!(related.get_by_content_id(&missing).unwrap().is_none());
}

#[test]
fn resource_matching_is_case_sensitive_first_match_and_independent_of_root_selectors() {
    let mut related = related("; start=\"<>\"; type=\"invalid\"");
    related
        .multipart_mut()
        .children_mut()
        .push(leaf("image/jpeg", Some("<image>"), "duplicate"));
    related.multipart_mut().children_mut().push(leaf(
        "image/gif",
        Some("<image%40example>"),
        "escaped",
    ));
    let image = MessageId::from_value("image").unwrap();
    assert_eq!(related.index_of_content_id(&image).unwrap(), Some(0));
    assert_eq!(
        label(related.get_by_content_id(&image).unwrap().unwrap()),
        "image"
    );
    let root = MessageId::from_value("<root>").unwrap();
    assert_eq!(related.index_of_content_id(&root).unwrap(), Some(1));
    assert!(related.root_index().is_err());
    for id in ["IMAGE", "image@example"] {
        assert!(
            related
                .get_by_content_id(&MessageId::from_value(id).unwrap())
                .unwrap()
                .is_none()
        );
    }
    let escaped = MessageId::from_value("image%40example").unwrap();
    assert_eq!(related.index_of_content_id(&escaped).unwrap(), Some(4));
    related.multipart_mut().children_mut()[0]
        .set_content_id(Some("changed"))
        .unwrap();
    assert_eq!(related.index_of_content_id(&image).unwrap(), Some(3));
}

#[test]
fn resource_lookup_visits_only_immediate_child_ids_without_source_io() {
    let mut related: MultipartRelated<NoIo> = MultipartRelated::with_boundary("b").unwrap();
    let mut nested: Multipart<NoIo> = Multipart::with_boundary("mixed", "inner").unwrap();
    let mut descendant = MimePart::new(MimeContent::new(NoIo));
    descendant.set_content_id(Some("deep")).unwrap();
    nested.children_mut().push(descendant.into());
    let mut entity: MimeEntity<NoIo> = nested.into();
    entity.set_content_id(Some("nested")).unwrap();
    related.multipart_mut().children_mut().push(entity);
    let mut part = MimePart::new(MimeContent::new(NoIo));
    part.headers_mut()
        .add(Header::parse(b"Content-Type: invalid").unwrap());
    part.set_content_id(Some("leaf")).unwrap();
    related.multipart_mut().children_mut().push(part.into());
    for (id, index) in [("nested", 0), ("leaf", 1)] {
        assert_eq!(
            related
                .index_of_content_id(&MessageId::from_value(id).unwrap())
                .unwrap(),
            Some(index)
        );
    }
    assert!(
        related
            .get_by_content_id(&MessageId::from_value("deep").unwrap())
            .unwrap()
            .is_none()
    );

    let mut related: MultipartRelated<()> = MultipartRelated::with_boundary("b").unwrap();
    let mut wrapper = MessagePart::new(MimeMessage::new());
    wrapper.set_content_id(Some("attached")).unwrap();
    related.multipart_mut().children_mut().push(wrapper.into());
    assert!(matches!(
        related
            .get_by_content_id(&MessageId::from_value("attached").unwrap())
            .unwrap(),
        Some(MimeEntity::Message(_))
    ));
}

#[test]
fn resource_lookup_propagates_visited_id_errors_without_duplicate_fallback() {
    let mut related = related("");
    let mut bad = leaf("image/gif", Some("<>"), "bad");
    bad.headers_mut()
        .add(Header::parse(b"Content-Id: <valid-duplicate>").unwrap());
    related.multipart_mut().children_mut().insert(1, bad);
    let first = MessageId::from_value("image").unwrap();
    assert_eq!(related.index_of_content_id(&first).unwrap(), Some(0));
    for id in ["root", "missing", "valid-duplicate"] {
        let id = MessageId::from_value(id).unwrap();
        let error = related.index_of_content_id(&id).unwrap_err();
        assert!(matches!(error, MultipartRelatedError::ContentId(_)));
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<MessageIdParseError>()
                .is_some()
        );
        assert!(matches!(
            related.get_by_content_id(&id),
            Err(MultipartRelatedError::ContentId(_))
        ));
    }
}

#[test]
fn resource_lookup_checks_authoritative_container_type_but_not_boundary() {
    let id = MessageId::from_value("image").unwrap();
    let mut related = related("");
    for (raw, kind) in [
        ("", 0),
        (
            "Content-Type: invalid\r\nContent-Type: multipart/related\r\n",
            1,
        ),
        (
            "Content-Type: multipart/mixed\r\nContent-Type: multipart/related\r\n",
            2,
        ),
    ] {
        *related.multipart_mut().headers_mut() = headers(raw.as_bytes());
        let error = related.index_of_content_id(&id).unwrap_err();
        match kind {
            0 => assert!(matches!(error, MultipartRelatedError::MissingContentType)),
            1 => assert!(matches!(error, MultipartRelatedError::ContentType(_))),
            _ => assert!(matches!(error, MultipartRelatedError::NotRelated(_))),
        }
    }
    for raw in [
        b"Content-Type: multipart/related\r\n".as_slice(),
        b"Content-Type: multipart/related; boundary=\"bad \"\r\n",
    ] {
        *related.multipart_mut().headers_mut() = headers(raw);
        assert_eq!(related.index_of_content_id(&id).unwrap(), Some(0));
    }
}

#[test]
fn resource_access_preserves_parsed_framing_and_encoded_content() {
    for end in ["", "--b-- \t\r\n"] {
        let wire = format!(
            "Content-Type: multipart/related; boundary=b; start=\"<root>\"\r\n\
Content-Base: http://example.com/mail/\r\n\r\n\
--b \t\r\nContent-Type: text/html\r\nContent-Id: <root>\r\n\r\nhtml\r\n\
--b \t\r\nContent-Type: image/gif\r\nContent-Id: <image>\r\n\
Content-Location: images/logo.gif\r\n\
Content-Transfer-Encoding: base64\r\n\r\nR0lG\r\n{end}"
        );
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser =
                MimeParser::with_storage(Cursor::new(wire.as_bytes()), storage).unwrap();
            let MimeEntity::Multipart(part) = parser.parse_entity().unwrap() else {
                panic!()
            };
            drop(parser);
            let mut related = MultipartRelated::from_multipart(part).unwrap();
            let before = related.multipart().headers().clone();
            let id = MessageId::from_value("image").unwrap();
            for _ in 0..2 {
                assert_eq!(related.index_of_content_id(&id).unwrap(), Some(1));
                assert!(matches!(
                    related.get_by_content_id(&id).unwrap(),
                    Some(MimeEntity::Part(_))
                ));
                assert_eq!(related.root_index().unwrap(), Some(0));
                assert!(related.text_body_part(TextFormat::Html).unwrap().is_some());
                let mut decoded = Vec::new();
                related
                    .open_by_content_id(&id)
                    .unwrap()
                    .read_to_end(&mut decoded)
                    .unwrap();
                assert_eq!(decoded, b"GIF");
                for uri in [
                    "cid:image",
                    "images/logo.gif",
                    "http://example.com/mail/images/logo.gif",
                ] {
                    assert_eq!(related.index_of_uri(uri).unwrap(), Some(1));
                    assert!(related.get_by_uri(uri).unwrap().is_some());
                    decoded.clear();
                    related
                        .open_by_uri(uri)
                        .unwrap()
                        .read_to_end(&mut decoded)
                        .unwrap();
                    assert_eq!(decoded, b"GIF");
                    let mut resource = related.open_by_uri_with_metadata(uri).unwrap();
                    assert_eq!(resource.content_type.mime_type(), "image/gif");
                    assert_eq!(resource.content_type.charset(), None);
                    decoded.clear();
                    resource.reader.read_to_end(&mut decoded).unwrap();
                    assert_eq!(decoded, b"GIF");
                }
                let mut resource = related.open_by_content_id_with_metadata(&id).unwrap();
                assert_eq!(resource.content_type.mime_type(), "image/gif");
                decoded.clear();
                resource.reader.read_to_end(&mut decoded).unwrap();
                assert_eq!(decoded, b"GIF");
                drop(resource);
                assert_eq!(related.multipart().headers(), &before);
            }
            let mut entity: MimeEntity<_> = related.into();
            for _ in 0..2 {
                let mut output = Vec::new();
                entity.write_to(&mut output).unwrap();
                assert_eq!(output, wire.as_bytes());
            }
        }
    }
}

#[test]
fn constructors_and_accessors_do_not_require_source_traits() {
    let mut related: MultipartRelated<()> = MultipartRelated::with_boundary("boundary").unwrap();
    assert_eq!(related.multipart().headers().len(), 1);
    assert_eq!(related.root_index().unwrap(), None);
    assert!(related.root().unwrap().is_none());
    assert!(related.root_mut().unwrap().is_none());
    assert!(related.text_body_part(TextFormat::Html).unwrap().is_none());
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
        MultipartRelated::<()>::with_boundary(""),
        Err(MultipartRelatedError::Multipart(
            MultipartError::InvalidBoundary { .. }
        ))
    ));
}

#[test]
fn default_constructor_generates_only_a_boundary_without_source_traits() {
    let related: MultipartRelated<()> = MultipartRelated::new().unwrap();
    let part = related.multipart();
    let boundary = part.boundary().unwrap().unwrap();
    assert_eq!(boundary.len(), 26);
    assert!(boundary.starts_with("=-"));
    assert!(boundary.ends_with("=="));
    assert_eq!(part.headers().len(), 1);
    assert!(part.children().is_empty());
    assert!(part.preamble().is_empty());
    assert!(part.epilogue().is_empty());
    assert!(related.root().unwrap().is_none());
    let content_type = part.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("multipart", "related"));
    assert_eq!(content_type.parameters().get("type"), None);
    assert_eq!(content_type.parameters().get("start"), None);
    let explicit: MultipartRelated<()> = MultipartRelated::with_boundary(&boundary).unwrap();
    assert_eq!(part.headers(), explicit.multipart().headers());
}

#[test]
fn generated_boundary_survives_root_assignment_and_message_round_trip() {
    let mut related: MultipartRelated = MultipartRelated::new().unwrap();
    let boundary = related.multipart().boundary().unwrap().unwrap();
    related.set_root(leaf("text/html", None, "html")).unwrap();
    assert_eq!(
        related.multipart().boundary().unwrap(),
        Some(boundary.clone())
    );
    let mut message = MimeMessage::with_body(related.into());
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    let loaded = MimeMessage::load(&mut wire.as_slice()).unwrap();
    assert!(loaded.text_body_part(TextFormat::Html).unwrap().is_some());
    let (_, Some(MimeEntity::Multipart(part))) = loaded.into_parts() else {
        panic!()
    };
    let related = MultipartRelated::from_multipart(part).unwrap();
    assert_eq!(related.multipart().boundary().unwrap(), Some(boundary));
    assert_eq!(related.root_index().unwrap(), Some(0));
    assert_eq!(parameter(&related, "type").as_deref(), Some("text/html"));
    assert_eq!(parameter(&related, "start"), None);
    assert_eq!(label(related.root().unwrap().unwrap()), "html");
    let mut reloaded = MimeMessage::with_body(related.into());
    let mut second = Vec::new();
    reloaded.write_to(&mut second).unwrap();
    assert_eq!(second, wire);
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
    let mut related: MultipartRelated = MultipartRelated::with_boundary("b").unwrap();
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
    for fault in ["missing-type", "invalid-type", "invalid-id"] {
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
    let mut related = MultipartRelated::with_boundary("b").unwrap();
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
    let mut alternative = Multipart::with_boundary("alternative", "alt").unwrap();
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
    assert!(related.text_body_part(TextFormat::Plain).unwrap().is_some());
    assert!(related.text_body_part(TextFormat::Html).unwrap().is_some());
    assert!(
        related
            .text_body_part(TextFormat::Enriched)
            .unwrap()
            .is_none()
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

#[test]
fn document_root_generates_an_id_and_reinserts_after_clearing() {
    let mut related: MultipartRelated = MultipartRelated::new().unwrap();
    related.multipart_mut().children_mut().extend([
        leaf("image/gif", Some("gif"), "gif"),
        leaf("image/jpg", Some("jpg"), "jpg"),
        leaf("text/html", Some("html"), "html"),
    ]);
    let mut content_type = related.multipart().content_type().unwrap().unwrap();
    content_type
        .parameters_mut()
        .set("type", "text/html")
        .unwrap();
    content_type
        .parameters_mut()
        .set("start", "<html>")
        .unwrap();
    related
        .multipart_mut()
        .set_content_type(&content_type)
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(2));

    let old = related
        .set_root(leaf("text/html", None, "replacement"))
        .unwrap()
        .unwrap();
    assert_eq!(label(&old), "html");
    assert_eq!(related.multipart().children().len(), 3);
    assert_eq!(related.root_index().unwrap(), Some(2));
    let root = related.root().unwrap().unwrap();
    assert_eq!(label(root), "replacement");
    let id = root.content_id().unwrap().unwrap();
    assert!(!id.as_str().is_empty());
    assert_eq!(MessageId::from_value(id.as_str()).unwrap(), id);
    assert_eq!(
        parameter(&related, "start"),
        Some(format!("<{}>", id.as_str()))
    );
    assert!(related.contains_content_id(&id).unwrap());
    assert!(
        related
            .contains_uri(&format!("cid:{}", id.as_str()))
            .unwrap()
    );

    related.multipart_mut().children_mut().truncate(2);
    related.set_root(old).unwrap();
    assert_eq!(related.multipart().children().len(), 3);
    assert_eq!(related.root_index().unwrap(), Some(0));
    assert_eq!(label(related.root().unwrap().unwrap()), "html");
    // First-child roots omit start to work around Thunderbird bug 471402.
    assert_eq!(parameter(&related, "start"), None);
}

#[test]
fn typed_construction_preserves_generic_constructor_headers_and_children() {
    let mut multipart = Multipart::new("related").unwrap();
    multipart.headers_mut().add(
        Header::parse(b"Content-Description: This is a description of the multipart.").unwrap(),
    );
    multipart.children_mut().extend([
        leaf("text/plain", None, "This is the message body."),
        leaf("image/gif", None, "attachment.gif"),
    ]);
    let before = multipart.headers().clone();
    let children = multipart.children().as_ptr();
    let related = MultipartRelated::from_multipart(multipart).unwrap();
    assert_eq!(related.multipart().headers(), &before);
    assert_eq!(related.multipart().children().as_ptr(), children);
    assert_eq!(related.multipart().children().len(), 2);
    for (child, expected) in related
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
}

#[test]
fn direct_body_selection_searches_only_the_root_and_keeps_attached_roots() {
    let empty: MultipartRelated<()> = MultipartRelated::with_boundary("b").unwrap();
    assert!(empty.text_body_part(TextFormat::Html).unwrap().is_none());
    let mut related = related("; start=\"<missing>\"; type=\"text/html\"");
    assert!(related.text_body_part(TextFormat::Html).unwrap().is_none());
    let mut attached = leaf("text/html", None, "attached html");
    uri_header(attached.headers_mut(), "Content-Disposition", "attachment");
    assert!(attached.text_body_part(TextFormat::Html).unwrap().is_none());
    related.set_root(attached).unwrap();
    let selected = related.text_body_part(TextFormat::Html).unwrap().unwrap();
    let MimeEntity::Part(root) = related.root().unwrap().unwrap() else {
        panic!()
    };
    assert!(std::ptr::eq(selected, root));
    assert!(related.text_body_part(TextFormat::Plain).unwrap().is_none());
    uri_header(
        related.root_mut().unwrap().unwrap().headers_mut(),
        "Content-Type",
        "invalid",
    );
    assert!(matches!(
        related.text_body_part(TextFormat::Html),
        Err(MultipartRelatedError::RootSelection(
            TextBodyError::ContentType(_)
        ))
    ));
}

#[test]
fn mhtml_document_root_by_type_and_image_opening() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("messages")
        .join("multipart-related-mhtml.txt");
    let wire = std::fs::read(path).unwrap();
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let mut parser = MimeParser::with_storage(Cursor::new(wire.as_slice()), storage).unwrap();
        let MimeEntity::Multipart(part) = parser.parse_entity().unwrap() else {
            panic!()
        };
        drop(parser);
        let mut related = MultipartRelated::from_multipart(part).unwrap();
        assert_eq!(related.multipart().children().len(), 2);
        assert_eq!(related.root_index().unwrap(), Some(1));
        let html = related.text_body_part(TextFormat::Html).unwrap().unwrap();
        assert_eq!(
            html.content_type().unwrap().unwrap().charset(),
            Some("utf-8")
        );
        assert!(related.text_body_part(TextFormat::Plain).unwrap().is_none());
        assert!(related.contains_uri("image1").unwrap());
        assert!(!related.contains_uri("absent").unwrap());
        let mut resource = related.open_by_uri_with_metadata("image1").unwrap();
        assert_eq!(resource.content_type.mime_type(), "image/png");
        assert_eq!(resource.content_type.charset(), None);
        let mut png = Vec::new();
        resource.reader.read_to_end(&mut png).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        drop(resource);
        let mut entity: MimeEntity<_> = related.into();
        let mut output = Vec::new();
        entity.write_to(&mut output).unwrap();
        assert_eq!(output, wire);
    }
}

#[test]
fn metadata_opening_preserves_first_header_charset_and_stored_encoding() {
    let id = MessageId::from_value("image").unwrap();
    for content_type in [
        None,
        Some("Text/Html; charset=\"x-unknown\""),
        Some("text/plain; charset=\"\""),
    ] {
        let mut related = related_with_resource(MimeContent::with_encoding(
            ResourceSource::new(),
            ContentEncoding::Base64,
        ));
        let fields = related.multipart_mut().children_mut()[0].headers_mut();
        fields.remove_all("Content-Type");
        if let Some(value) = content_type {
            uri_header(fields, "Content-Type", value);
            fields.add(Header::parse(b"Content-Type: invalid").unwrap());
        }
        uri_header(fields, "Content-Transfer-Encoding", "7bit");
        let before = fields.clone();
        for by_uri in [false, true] {
            let mut resource = if by_uri {
                related.open_by_uri_with_metadata("cid:image")
            } else {
                related.open_by_content_id_with_metadata(&id)
            }
            .unwrap();
            assert_eq!(
                resource.content_type.mime_type(),
                if content_type.is_some_and(|value| value.starts_with("Text/Html")) {
                    "Text/Html"
                } else {
                    "text/plain"
                }
            );
            assert_eq!(
                resource.content_type.charset(),
                match content_type {
                    None => None,
                    Some(value) if value.starts_with("Text/Html") => Some("x-unknown"),
                    Some(_) => Some(""),
                }
            );
            assert_eq!(resource.reader.get_ref().bytes.position(), 0);
            assert_eq!(resource.reader.get_ref().seeks, if by_uri { 2 } else { 1 });
            if !by_uri {
                assert_eq!(resource.reader.get_ref().reads, 0);
            }
            let mut bytes = Vec::new();
            resource.reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"Cat");
            drop(resource);
            assert_eq!(related.multipart().children()[0].headers(), &before);
        }
    }
}

#[test]
fn metadata_and_contains_failures_do_not_fall_back_or_read_sources() {
    let id = MessageId::from_value("image").unwrap();
    let mut related = related_with_resource(MimeContent::new(NoIo));
    let fields = related.multipart_mut().children_mut()[0].headers_mut();
    uri_header(fields, "Content-Type", "invalid");
    fields.add(Header::parse(b"Content-Type: image/png").unwrap());
    assert!(related.contains_uri("cid:image").unwrap());
    assert!(related.contains_content_id(&id).unwrap());
    for error in [
        related.open_by_uri_with_metadata("cid:image").unwrap_err(),
        related.open_by_content_id_with_metadata(&id).unwrap_err(),
    ] {
        assert!(matches!(
            error,
            MultipartRelatedError::ResourceContentType(_)
        ));
        assert!(error.source().is_some());
        assert!(error.to_string().contains("Content-Type"));
    }
    assert!(matches!(
        related.open_by_uri_with_metadata("absent"),
        Err(MultipartRelatedError::UriResourceNotFound(_))
    ));
    let absent = MessageId::from_value("absent").unwrap();
    assert!(!related.contains_content_id(&absent).unwrap());
    assert!(matches!(
        related.open_by_content_id_with_metadata(&absent),
        Err(MultipartRelatedError::ResourceNotFound(_))
    ));
    assert!(matches!(
        related.contains_uri("cid:%"),
        Err(MultipartRelatedError::Uri(_))
    ));
    uri_header(
        related.multipart_mut().children_mut()[0].headers_mut(),
        "Content-Id",
        "<>",
    );
    assert!(matches!(
        related.contains_content_id(&id),
        Err(MultipartRelatedError::ContentId(_))
    ));
    assert!(matches!(
        related.contains_uri("cid:image"),
        Err(MultipartRelatedError::ContentId(_))
    ));
}

#[test]
fn metadata_opening_propagates_seek_and_incremental_read_errors() {
    for by_uri in [false, true] {
        let id = MessageId::from_value("image").unwrap();
        let mut source = ResourceSource::new();
        source.fail_seek = true;
        let mut related = related_with_resource(MimeContent::new(source));
        let error = if by_uri {
            related.open_by_uri_with_metadata("cid:image")
        } else {
            related.open_by_content_id_with_metadata(&id)
        }
        .unwrap_err();
        let MultipartRelatedError::Io(error) = error else {
            panic!()
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        let mut source = ResourceSource::new();
        source.fail_read_at = Some(3);
        let mut related =
            related_with_resource(MimeContent::with_encoding(source, ContentEncoding::Base64));
        let mut resource = if by_uri {
            related.open_by_uri_with_metadata("cid:image")
        } else {
            related.open_by_content_id_with_metadata(&id)
        }
        .unwrap();
        let mut bytes = [0; 3];
        assert_eq!(
            resource.reader.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        resource.reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"Cat");
    }
}

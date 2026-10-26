// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    Header, MessageId, MessageIdDomainError, MimeContent, MimePart, Multipart, MultipartRelated,
    TextPart,
};
use std::io::Cursor;

// A separate integration-test process keeps one-time configuration independent
// of tests that intentionally exercise the unconfigured default.
#[test]
fn startup_configuration_controls_ids_and_related_roots_without_affecting_overrides() {
    let explicit = MessageId::generate_with_domain("override.example").unwrap();
    assert!(explicit.as_str().ends_with("@override.example"));
    assert!(matches!(
        MessageId::set_default_domain("invalid domain"),
        Err(MessageIdDomainError::InvalidDomain(_))
    ));
    MessageId::set_default_domain("B\u{dc}CHER.EXAMPLE.").unwrap();
    assert_eq!(MessageId::default_domain(), "xn--bcher-kva.example");
    for _ in 0..32 {
        assert!(
            MessageId::generate()
                .unwrap()
                .as_str()
                .ends_with("@xn--bcher-kva.example")
        );
    }
    assert_eq!(
        MessageId::set_default_domain("other.example"),
        Err(MessageIdDomainError::AlreadyInitialized)
    );
    let explicit = MessageId::generate_with_domain("override.example").unwrap();
    assert!(explicit.as_str().ends_with("@override.example"));
    assert_eq!(MessageId::default_domain(), "xn--bcher-kva.example");

    let mut multipart = Multipart::with_boundary("related", "b").unwrap();
    multipart
        .headers_mut()
        .replace_all(
            Header::parse(b"Content-Type: multipart/related; boundary=b; type=\"text/html\"")
                .unwrap(),
        )
        .unwrap();
    multipart.children_mut().extend([
        MimePart::new(MimeContent::new(Cursor::new(Vec::new()))).into(),
        TextPart::new("html", "old").unwrap().into(),
    ]);
    let mut related = MultipartRelated::from_multipart(multipart).unwrap();
    related
        .set_root(TextPart::new("html", "new").unwrap().into())
        .unwrap();
    assert_eq!(related.root_index().unwrap(), Some(1));
    let id = related
        .root()
        .unwrap()
        .unwrap()
        .content_id()
        .unwrap()
        .unwrap();
    assert!(id.as_str().ends_with("@xn--bcher-kva.example"));
}

// Adapted from MimeKit's Multipart.Prepare, MessagePart.Prepare, and MimeMessage.Prepare.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    cell::RefCell,
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
    rc::Rc,
};

use mimecraft::{
    BestEncodingError, ContentEncoding, EncodingConstraint, Header, HeaderList, MessagePart,
    MimeContent, MimeEntity, MimeMessage, MimePart, MimePartWriteOptions, MimePrepareError,
    MimeVersion, Multipart, PartPrepareError, TextPart,
    encodings::{InvalidLineLength, NewLineFormat},
};

fn headers(raw: &str) -> HeaderList {
    HeaderList::load(&mut raw.as_bytes()).unwrap()
}

struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("unexpected source read");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("unexpected source seek");
    }
}

fn opaque() -> MimeEntity<NoIo> {
    MimePart::new(MimeContent::new(NoIo)).into()
}

fn assert_options_error(result: Result<(), MimePrepareError>) {
    let error = result.unwrap_err();
    assert!(!error.to_string().is_empty());
    assert!(error.source().unwrap().is::<io::Error>());
    let MimePrepareError::InvalidOptions(inner) = error else {
        panic!("wrong error");
    };
    assert_eq!(inner.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn all_entry_points_validate_options_even_for_empty_or_safe_trees() {
    let mut multipart: Multipart<NoIo> =
        Multipart::from_parts(HeaderList::new(), Vec::new(), Vec::new(), Vec::new());
    let mut wrapper: MessagePart<NoIo> = MessagePart::from_parts(HeaderList::new(), None);
    let mut message: MimeMessage<NoIo> =
        MimeMessage::from_parts(headers("MIME-Version: invalid\r\n"), None);
    let mut entity = opaque();
    for width in [0, 59, 999, usize::MAX] {
        let options = MimePartWriteOptions {
            max_line_length: width,
            ..Default::default()
        };
        assert_options_error(
            multipart.prepare_with_options(EncodingConstraint::SevenBit, &options),
        );
        assert_options_error(wrapper.prepare_with_options(EncodingConstraint::SevenBit, &options));
        assert_options_error(message.prepare_with_options(EncodingConstraint::SevenBit, &options));
        let error = entity
            .prepare_with_options(EncodingConstraint::SevenBit, &options)
            .unwrap_err();
        let MimePrepareError::InvalidOptions(inner) = &error else {
            panic!("wrong error")
        };
        assert!(inner.get_ref().unwrap().is::<InvalidLineLength>());
        assert_options_error(Err(error));
    }
    let mixed = MimePartWriteOptions {
        new_line_format: NewLineFormat::Mixed,
        ..Default::default()
    };
    assert_options_error(multipart.prepare_with_options(EncodingConstraint::None, &mixed));
    assert_options_error(wrapper.prepare_with_options(EncodingConstraint::None, &mixed));
    assert_options_error(message.prepare_with_options(EncodingConstraint::None, &mixed));
    assert_options_error(entity.prepare_with_options(EncodingConstraint::None, &mixed));
    assert_eq!(entity.headers().len(), 1);
    assert_eq!(message.headers(), &headers("MIME-Version: invalid\r\n"));
    multipart.prepare(EncodingConstraint::SevenBit).unwrap();
    wrapper.prepare(EncodingConstraint::SevenBit).unwrap();
    message.prepare(EncodingConstraint::SevenBit).unwrap();
    assert!(multipart.headers().is_empty());
    assert!(wrapper.headers().is_empty());
    assert_eq!(message.headers(), &headers("MIME-Version: invalid\r\n"));
}

#[test]
fn each_entry_point_forwards_the_constraint_to_leaves() {
    for (constraint, expected) in [
        (EncodingConstraint::SevenBit, ContentEncoding::Base64),
        (EncodingConstraint::EightBit, ContentEncoding::Base64),
        (EncodingConstraint::None, ContentEncoding::Binary),
    ] {
        let mut entity = opaque();
        entity.prepare(constraint).unwrap();
        let mut multipart = Multipart::new("mixed", "test").unwrap();
        multipart.children_mut().push(opaque());
        multipart.prepare(constraint).unwrap();
        let mut wrapper = MessagePart::new(MimeMessage::with_body(opaque()));
        wrapper.prepare(constraint).unwrap();
        let mut message = MimeMessage::with_body(opaque());
        message.prepare(constraint).unwrap();
        for leaf in [
            entity.leaf_parts().next().unwrap(),
            multipart.leaf_parts().next().unwrap(),
            wrapper.leaf_parts().next().unwrap(),
            message.leaf_parts().next().unwrap(),
        ] {
            assert_eq!(leaf.content_transfer_encoding().unwrap(), expected);
            assert_eq!(leaf.content().encoding(), ContentEncoding::Unspecified);
        }
        assert!(!entity.headers().contains("MIME-Version"));
        assert!(!multipart.headers().contains("MIME-Version"));
        assert!(!wrapper.headers().contains("MIME-Version"));
        assert_eq!(
            wrapper.message().unwrap().mime_version().unwrap(),
            Some(MimeVersion::V1_0)
        );
        assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
    }
}

#[test]
fn version_insertion_depends_on_existing_root_headers_not_descendants() {
    for root_headers in [
        HeaderList::new(),
        headers("X-Anything: present\r\n"),
        headers("MIME-Version: 9.8\r\n"),
    ] {
        let mut multipart =
            Multipart::from_parts(root_headers.clone(), vec![opaque()], Vec::new(), Vec::new());
        // Descendant headers do not count toward the enclosing message's check.
        multipart
            .children_mut()
            .push(MessagePart::new(MimeMessage::with_body(opaque())).into());
        let mut message = MimeMessage::with_body(multipart.into());
        message.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(
            message.mime_version().unwrap(),
            (!root_headers.is_empty()).then_some(MimeVersion::V1_0)
        );
        assert_eq!(message.body().unwrap().headers(), &root_headers);
        let MimeEntity::Multipart(root) = message.body().unwrap() else {
            unreachable!()
        };
        let MimeEntity::Message(nested) = &root.children()[1] else {
            unreachable!()
        };
        assert_eq!(
            nested.message().unwrap().mime_version().unwrap(),
            Some(MimeVersion::V1_0)
        );
        assert!(!nested.headers().contains("MIME-Version"));
    }
}

#[test]
fn valid_first_versions_preserve_raw_bytes_duplicates_and_other_metadata() {
    for value in ["01 (comment).\r\n\t0", "2.4", "1.0.0", "1.0.0.0"] {
        let raw = format!(
            "X-Before: a\r\nmImE-vErSiOn: {value}\r\nMIME-Version: invalid\r\n\
             Bcc: hidden@example.com\r\nResent-Bcc: other@example.com\r\n"
        );
        let mut message = MimeMessage::from_parts(headers(&raw), Some(opaque()));
        let before = message.headers().clone();
        message.prepare(EncodingConstraint::SevenBit).unwrap();
        message.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(message.headers(), &before);
        assert_eq!(message.headers().get_all("MIME-Version").count(), 2);
    }
    let mut message = MimeMessage::from_parts(
        headers("Bcc: hidden@example.com\r\nResent-Bcc: other@example.com\r\n"),
        Some(opaque()),
    );
    message.prepare(EncodingConstraint::None).unwrap();
    assert_eq!(
        message.headers(),
        &headers(
            "Bcc: hidden@example.com\r\nResent-Bcc: other@example.com\r\nMIME-Version: 1.0\r\n"
        )
    );
    let before = message.headers().clone();
    message.prepare(EncodingConstraint::None).unwrap();
    assert_eq!(message.headers(), &before);
}

#[test]
fn invalid_version_fails_before_body_edits_even_with_empty_root_headers() {
    for body in [
        opaque(),
        Multipart::from_parts(HeaderList::new(), vec![opaque()], Vec::new(), Vec::new()).into(),
    ] {
        let mut message = MimeMessage::from_parts(
            headers("MIME-Version: invalid\r\nMIME-Version: 1.0\r\n"),
            Some(body),
        );
        let before = message.headers().clone();
        let error = message.prepare(EncodingConstraint::SevenBit).unwrap_err();
        assert!(
            error
                .source()
                .unwrap()
                .is::<mimecraft::MimeVersionParseError>()
        );
        assert!(matches!(error, MimePrepareError::MimeVersion(_)));
        assert_eq!(message.headers(), &before);
        assert_eq!(
            message
                .leaf_parts()
                .next()
                .unwrap()
                .content_transfer_encoding()
                .unwrap(),
            ContentEncoding::Unspecified
        );
    }
}

#[test]
fn container_metadata_and_framing_are_not_validated_or_repaired() {
    let raw = headers(
        "Content-Type: invalid\r\nContent-Transfer-Encoding: unknown\r\n\
         Content-Disposition: broken(\r\n",
    );
    let enclosed = MimeMessage::with_body(opaque());
    let wrapper = MessagePart::from_parts(raw.clone(), Some(enclosed));
    let mut multipart = Multipart::from_parts(
        raw.clone(),
        vec![wrapper.into()],
        b"\xfflong preamble".to_vec(),
        b"\0epilogue".to_vec(),
    );
    multipart.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(multipart.headers(), &raw);
    assert_eq!(multipart.preamble(), b"\xfflong preamble");
    assert_eq!(multipart.epilogue(), b"\0epilogue");
    let MimeEntity::Message(wrapper) = &multipart.children()[0] else {
        unreachable!()
    };
    assert_eq!(wrapper.headers(), &raw);
    assert_eq!(
        wrapper.message().unwrap().mime_version().unwrap(),
        Some(MimeVersion::V1_0)
    );
    assert_eq!(
        multipart
            .leaf_parts()
            .next()
            .unwrap()
            .content_transfer_encoding()
            .unwrap(),
        ContentEncoding::Base64
    );
    let mut output = Vec::new();
    assert_eq!(
        multipart.write_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(output.is_empty());
}

struct Source {
    id: usize,
    cursor: Cursor<Vec<u8>>,
    seeks: Rc<RefCell<Vec<usize>>>,
    fail_read: bool,
}

impl Read for Source {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read failed",
            ));
        }
        self.cursor.read(output)
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks.borrow_mut().push(self.id);
        self.cursor.seek(position)
    }
}

fn recorded_leaf(
    id: usize,
    seeks: &Rc<RefCell<Vec<usize>>>,
    fail_read: bool,
) -> MimeEntity<Source> {
    let mut cursor = Cursor::new(b"ascii\n".to_vec());
    cursor.set_position(3);
    let mut content = MimeContent::new(Source {
        id,
        cursor,
        seeks: seeks.clone(),
        fail_read,
    });
    content.set_new_line_format(Some(NewLineFormat::Mixed));
    MimePart::from_parts(
        headers("Content-Type: text/plain\r\nContent-Transfer-Encoding: 8bit\r\n"),
        content,
    )
    .into()
}

#[test]
fn failure_stops_depth_first_traversal_without_rolling_back_prior_changes() {
    let seeks = Rc::new(RefCell::new(Vec::new()));
    let mut inner = Multipart::new("mixed", "inner").unwrap();
    inner.children_mut().extend([
        recorded_leaf(2, &seeks, false),
        recorded_leaf(3, &seeks, true),
    ]);
    let nested = MessagePart::new(MimeMessage::with_body(inner.into()));
    let unvisited = MessagePart::new(MimeMessage::with_body(recorded_leaf(4, &seeks, false)));
    let mut root = Multipart::new("mixed", "outer").unwrap();
    root.children_mut().extend([
        recorded_leaf(1, &seeks, false),
        nested.into(),
        unvisited.into(),
    ]);
    let mut message = MimeMessage::with_body(root.into());
    let error = message.prepare(EncodingConstraint::SevenBit).unwrap_err();
    assert!(error.source().unwrap().is::<PartPrepareError>());
    let MimePrepareError::Part(PartPrepareError::BestEncoding(BestEncodingError::Io(inner))) =
        error
    else {
        panic!("wrong error")
    };
    assert_eq!(inner.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(*seeks.borrow(), [1, 1, 2, 2, 3]);
    assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
    assert_eq!(
        message
            .leaf_parts()
            .map(|part| part.content_transfer_encoding().unwrap())
            .collect::<Vec<_>>(),
        [
            ContentEncoding::SevenBit,
            ContentEncoding::SevenBit,
            ContentEncoding::EightBit,
            ContentEncoding::EightBit
        ]
    );
    assert_eq!(
        message
            .leaf_parts()
            .map(|part| part.content().get_ref().cursor.position())
            .collect::<Vec<_>>(),
        [0, 0, 0, 3]
    );
    let MimeEntity::Multipart(root) = message.body_mut().unwrap() else {
        unreachable!()
    };
    let MimeEntity::Message(unvisited) = &root.children()[2] else {
        unreachable!()
    };
    assert_eq!(unvisited.message().unwrap().mime_version().unwrap(), None);
    let MimeEntity::Message(nested) = &mut root.children_mut()[1] else {
        unreachable!()
    };
    assert_eq!(
        nested.message().unwrap().mime_version().unwrap(),
        Some(MimeVersion::V1_0)
    );
    let MimeEntity::Multipart(inner) = nested.message_mut().unwrap().body_mut().unwrap() else {
        unreachable!()
    };
    let MimeEntity::Part(failing) = &mut inner.children_mut()[1] else {
        unreachable!()
    };
    failing.content_mut().get_mut().fail_read = false;
    seeks.borrow_mut().clear();
    message.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(*seeks.borrow(), [1, 1, 2, 2, 3, 3, 4, 4]);
    for leaf in message.leaf_parts() {
        assert_eq!(
            leaf.content_transfer_encoding().unwrap(),
            ContentEncoding::SevenBit
        );
        assert_eq!(leaf.content().get_ref().cursor.get_ref(), b"ascii\n");
        assert_eq!(leaf.content().encoding(), ContentEncoding::Unspecified);
        assert_eq!(leaf.content().new_line_format(), Some(NewLineFormat::Mixed));
    }
}

#[test]
fn version_insertion_precedes_leaf_metadata_failure_and_retry_observes_raw_edits() {
    let mut message = MimeMessage::with_body(
        MimePart::from_parts(
            headers("Content-Transfer-Encoding: invalid\r\n"),
            MimeContent::new(NoIo),
        )
        .into(),
    );
    let before = message.body().unwrap().headers().clone();
    assert!(matches!(
        message.prepare(EncodingConstraint::SevenBit),
        Err(MimePrepareError::Part(
            PartPrepareError::ContentTransferEncoding(_)
        ))
    ));
    assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
    assert_eq!(message.body().unwrap().headers(), &before);
    message
        .body_mut()
        .unwrap()
        .headers_mut()
        .replace_all(Header::parse(b"Content-Transfer-Encoding: base64\r\n").unwrap())
        .unwrap();
    message.prepare(EncodingConstraint::SevenBit).unwrap();
    message
        .headers_mut()
        .replace_all(Header::parse(b"MIME-Version: bad\r\n").unwrap())
        .unwrap();
    assert!(matches!(
        message.prepare(EncodingConstraint::SevenBit),
        Err(MimePrepareError::MimeVersion(_))
    ));
}

#[test]
fn explicit_line_options_reach_nested_leaves_and_round_trip_through_writing() {
    let input = "x".repeat(180);
    let mut root: Multipart = Multipart::new("mixed", "outer").unwrap();
    root.children_mut().extend([
        TextPart::new("plain", &input).unwrap().into(),
        MessagePart::new(MimeMessage::with_body(
            TextPart::new("plain", &input).unwrap().into(),
        ))
        .into(),
    ]);
    let mut message = MimeMessage::with_body(root.into());
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let options = MimePartWriteOptions {
            new_line_format: format,
            max_line_length: 60,
            ..Default::default()
        };
        message
            .prepare_with_options(EncodingConstraint::SevenBit, &options)
            .unwrap();
        for leaf in message.leaf_parts() {
            assert_eq!(
                leaf.content_transfer_encoding().unwrap(),
                ContentEncoding::QuotedPrintable
            );
        }
        let mut output = Vec::new();
        message
            .write_to_with_options(&mut output, &options)
            .unwrap();
        assert!(
            output.split(|&b| b == b'\n').all(|line| line
                .strip_suffix(b"\r")
                .unwrap_or(line)
                .len()
                <= 60)
        );
        assert!(
            output.split(|&b| b == b'\n').any(|line| line
                .strip_suffix(b"\r")
                .unwrap_or(line)
                .len()
                == 60)
        );
        let parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
        assert_eq!(parsed.mime_version().unwrap(), Some(MimeVersion::V1_0));
        assert_eq!(parsed.leaf_parts().count(), 2);
        for leaf in parsed.leaf_parts() {
            let mut decoded = Vec::new();
            MimeContent::with_encoding(
                Cursor::new(leaf.content().get_ref().get_ref()),
                leaf.content().encoding(),
            )
            .decode_to(&mut decoded)
            .unwrap();
            assert_eq!(decoded, input.as_bytes());
        }
        let MimeEntity::Multipart(root) = parsed.body().unwrap() else {
            unreachable!()
        };
        let MimeEntity::Message(wrapper) = &root.children()[1] else {
            unreachable!()
        };
        assert_eq!(
            wrapper.message().unwrap().mime_version().unwrap(),
            Some(MimeVersion::V1_0)
        );
    }
}

#[test]
fn borrowed_leaf_sources_keep_their_ownership_and_rewind_rules() {
    let mut source = Cursor::new(b"From sender\n".to_vec());
    source.set_position(4);
    {
        let leaf = MimePart::from_parts(
            headers("Content-Type: text/plain\r\n"),
            MimeContent::new(&mut source),
        );
        let mut wrapper = MessagePart::new(MimeMessage::with_body(leaf.into()));
        wrapper.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(
            wrapper
                .leaf_parts()
                .next()
                .unwrap()
                .content_transfer_encoding()
                .unwrap(),
            ContentEncoding::QuotedPrintable
        );
    }
    assert_eq!(source.position(), 0);
    assert_eq!(source.get_ref(), b"From sender\n");
}

#[test]
fn nested_version_failure_preserves_prior_edits_and_leaves_later_siblings_untouched() {
    let invalid_message = MimeMessage::from_parts(headers("MIME-Version: bad\r\n"), Some(opaque()));
    let mut root = Multipart::new("mixed", "outer").unwrap();
    root.children_mut()
        .extend([opaque(), MessagePart::new(invalid_message).into(), opaque()]);
    let mut message = MimeMessage::with_body(root.into());
    let options = MimePartWriteOptions {
        max_line_length: 59,
        ..Default::default()
    };
    assert_options_error(message.prepare_with_options(EncodingConstraint::SevenBit, &options));
    assert!(message.headers().is_empty());
    assert!(
        message
            .leaf_parts()
            .all(|leaf| leaf.content_transfer_encoding() == Ok(ContentEncoding::Unspecified))
    );
    assert!(matches!(
        message.prepare(EncodingConstraint::SevenBit),
        Err(MimePrepareError::MimeVersion(_))
    ));
    assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
    assert_eq!(
        message
            .leaf_parts()
            .map(|leaf| leaf.content_transfer_encoding().unwrap())
            .collect::<Vec<_>>(),
        [
            ContentEncoding::Base64,
            ContentEncoding::Unspecified,
            ContentEncoding::Unspecified
        ]
    );
}

#[test]
fn loading_and_writing_do_not_prepare_implicitly() {
    let raw = b"Subject: unchanged\r\nContent-Type: text/plain\r\n\r\nFrom sender\r\n";
    let mut message = MimeMessage::load(&mut raw.as_slice()).unwrap();
    assert_eq!(message.mime_version().unwrap(), None);
    let before = message.headers().clone();
    let body_before = message.body().unwrap().headers().clone();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(output, raw);
    assert_eq!(message.headers(), &before);
    assert_eq!(message.body().unwrap().headers(), &body_before);
    message.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(message.mime_version().unwrap(), Some(MimeVersion::V1_0));
    assert_eq!(
        message
            .leaf_parts()
            .next()
            .unwrap()
            .content_transfer_encoding()
            .unwrap(),
        ContentEncoding::QuotedPrintable
    );
}

#[test]
fn deep_mixed_trees_prepare_without_recursive_calls() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut root = opaque();
            for depth in 0..4000 {
                root = if depth % 2 == 0 {
                    let mut multipart = Multipart::new("mixed", "same").unwrap();
                    multipart.children_mut().push(root);
                    multipart.into()
                } else {
                    MessagePart::new(MimeMessage::with_body(root)).into()
                };
            }
            let result = root.prepare(EncodingConstraint::SevenBit);
            let encoding = root
                .leaf_parts()
                .next()
                .unwrap()
                .content_transfer_encoding();
            // Model destruction still recurses: dismantle the fixture iteratively
            // before asserting, so a failed preparation does not overflow on drop.
            let mut pending = vec![root];
            let mut versions = 0;
            while let Some(entity) = pending.pop() {
                match entity {
                    MimeEntity::Part(_) => {}
                    MimeEntity::Multipart(multipart) => pending.extend(multipart.into_parts().1),
                    MimeEntity::Message(wrapper) => {
                        if let Some(message) = wrapper.into_parts().1 {
                            versions +=
                                usize::from(message.mime_version() == Ok(Some(MimeVersion::V1_0)));
                            pending.extend(message.into_parts().1);
                        }
                    }
                }
            }
            result.unwrap();
            assert_eq!(encoding.unwrap(), ContentEncoding::Base64);
            assert_eq!(versions, 2000);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn wide_multipart_prepares_every_child() {
    let mut root = Multipart::new("mixed", "wide").unwrap();
    root.children_mut().extend((0..10_000).map(|_| opaque()));
    root.prepare(EncodingConstraint::EightBit).unwrap();
    assert_eq!(root.children().len(), 10_000);
    assert!(
        root.leaf_parts()
            .all(|leaf| leaf.content_transfer_encoding() == Ok(ContentEncoding::Base64))
    );
}

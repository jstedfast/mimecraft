// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufReader, Cursor};

use mimecraft::{
    ContentEncoding, ContentHeaderParseError, ContentType, Header, HeaderList, HeaderParser,
    MessagePart, MessagePartError, MimeContent, MimeEntity, MimeEntityParseOptions, MimeMessage,
    MimePart, MimePartWriteOptions, Multipart, UnknownContentEncoding, encodings::NewLineFormat,
};

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

fn load(raw: &[u8]) -> MimeEntity {
    MimeEntity::load(&mut &raw[..]).unwrap()
}

fn multipart(entity: &MimeEntity) -> &Multipart {
    let MimeEntity::Multipart(part) = entity else {
        panic!("multipart expected")
    };
    part
}

fn attached(entity: &MimeEntity) -> &MessagePart {
    let MimeEntity::Message(part) = entity else {
        panic!("message part expected")
    };
    part
}

fn write(entity: &mut MimeEntity, format: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    entity
        .write_to_with_options(
            &mut output,
            &MimePartWriteOptions {
                new_line_format: format,
                ..Default::default()
            },
        )
        .unwrap();
    output
}

fn implicit(message: Option<MimeMessage>) -> MimeEntity {
    MessagePart::from_parts(HeaderList::new(), message).into()
}

fn binary(bytes: &[u8]) -> MimeEntity {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(bytes.to_vec())));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.into()
}

#[test]
fn untyped_digest_children_are_messages_without_synthesized_headers() {
    for subtype in ["digest", "DiGeSt"] {
        let raw = format!(
            "Content-Type: multipart/{subtype}; boundary=d\r\n\r\n--d\r\nX-Wrapper: keep\r\n\r\nSubject: attached\r\nX-Message: inner\r\n\r\nbody\r\n--d--\r\n"
        );
        let mut entity = load(raw.as_bytes());
        let part = attached(&multipart(&entity).children()[0]);
        assert_eq!(part.headers(), &headers(b"X-Wrapper: keep\r\n"));
        assert!(part.content_type().unwrap().is_none());
        let message = part.message().unwrap();
        assert_eq!(
            message.headers(),
            &headers(b"Subject: attached\r\nX-Message: inner\r\n")
        );
        let MimeEntity::Part(body) = message.body().unwrap() else {
            panic!("the digest default must not apply to the embedded body");
        };
        assert!(body.headers().is_empty());
        assert_eq!(body.content().get_ref().get_ref(), b"body");
        assert_eq!(write(&mut entity, NewLineFormat::Dos), raw.as_bytes());
        assert!(
            attached(&multipart(&entity).children()[0])
                .headers()
                .get("Content-Type")
                .is_none()
        );
    }
}

#[test]
fn explicit_types_override_digest_defaults_and_duplicates_keep_first_header_semantics() {
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n\
--d\r\nContent-Type: text/plain\r\nContent-Type: message/rfc822\r\n\r\nplain\r\n\
--d\r\nContent-Type: message/x-custom\r\n\r\nopaque\r\n\
--d\r\nContent-Type: message/news\r\n\r\nSubject: attached\r\n\r\nbody\r\n\
--d--\r\n";
    let entity = load(raw);
    let children = multipart(&entity).children();
    assert_eq!(children.len(), 3);
    assert!(matches!(children[0], MimeEntity::Part(_)));
    assert_eq!(children[0].headers().len(), 2);
    assert!(matches!(children[1], MimeEntity::Part(_)));
    assert!(
        attached(&children[2])
            .message()
            .unwrap()
            .headers()
            .contains("Subject")
    );
    let bodies: Vec<&[u8]> = entity
        .leaf_parts()
        .map(|part| part.content().get_ref().get_ref().as_slice())
        .collect();
    assert_eq!(bodies, [b"plain".as_slice(), b"opaque", b"body"]);
}

#[test]
fn digest_defaults_do_not_escape_into_other_containers_or_siblings() {
    let raw = b"Content-Type: multipart/mixed; boundary=o\r\n\r\n\
--o\r\nContent-Type: multipart/digest; boundary=d\r\n\r\n\
--d\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n\
--m\r\n\r\nnot a message\r\n--m--\r\n\r\n\
--d\r\n\r\nSubject: attached\r\n\r\ninner\r\n--d--\r\n\r\n\
--o\r\n\r\nordinary sibling\r\n--o--\r\n";
    let mut entity = load(raw);
    let outer = multipart(&entity);
    let digest = multipart(&outer.children()[0]);
    assert!(matches!(
        multipart(&digest.children()[0]).children()[0],
        MimeEntity::Part(_)
    ));
    assert!(matches!(digest.children()[1], MimeEntity::Message(_)));
    assert!(matches!(outer.children()[1], MimeEntity::Part(_)));
    assert_eq!(entity.leaf_parts().count(), 3);
    assert_eq!(write(&mut entity, NewLineFormat::Dos), raw);
}

#[test]
fn encoded_wrappers_and_the_opaque_option_keep_untyped_digest_payloads_unchanged() {
    for (token, encoding) in [
        ("base64", ContentEncoding::Base64),
        ("quoted-printable", ContentEncoding::QuotedPrintable),
        ("x-uuencode", ContentEncoding::UuEncode),
    ] {
        let raw = format!(
            "Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\nContent-Transfer-Encoding: {token}\r\n\r\nencoded bytes\r\n--d--\r\n"
        );
        let mut entity = load(raw.as_bytes());
        let MimeEntity::Part(part) = &multipart(&entity).children()[0] else {
            panic!("encoded wrapper must remain opaque");
        };
        assert_eq!(part.content().encoding(), encoding);
        assert_eq!(part.content().get_ref().get_ref(), b"encoded bytes");
        assert!(part.content_type().unwrap().is_none());
        assert_eq!(write(&mut entity, NewLineFormat::Dos), raw.as_bytes());
    }
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\nnot message headers\r\n--d--\r\n";
    assert_eq!(
        MimeEntity::load(&mut &raw[..]).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut entity = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            parse_message_parts: false,
            max_depth: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let leaf = entity.leaf_parts().next().unwrap();
    assert!(leaf.headers().is_empty());
    assert_eq!(leaf.content().get_ref().get_ref(), b"not message headers");
    assert_eq!(write(&mut entity, NewLineFormat::Dos), raw);
}

#[test]
fn transparent_implicit_wrappers_keep_wrapper_encoding_separate_from_inner_body() {
    for encoding in ["", "7bit", "8bit", "binary"] {
        let cte = if encoding.is_empty() {
            String::new()
        } else {
            format!("Content-Transfer-Encoding: {encoding}\r\n")
        };
        let raw = format!(
            "Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n{cte}\r\nSubject: attached\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--d--\r\n"
        );
        let mut entity = load(raw.as_bytes());
        let part = attached(&multipart(&entity).children()[0]);
        assert!(part.content_type().unwrap().is_none());
        assert_eq!(
            part.leaf_parts().next().unwrap().content().encoding(),
            ContentEncoding::Base64
        );
        assert_eq!(write(&mut entity, NewLineFormat::Dos), raw.as_bytes());
    }
}

#[test]
fn empty_implicit_messages_and_binary_suffixes_round_trip_in_nested_digests() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for suffix in [b"".as_slice(), b"\0\xff\r", b"\n", b"\r\n", b"\r\n\r\n"] {
            let mut inner: Multipart = Multipart::new("digest", "inner").unwrap();
            inner.children_mut().extend([
                implicit(None),
                implicit(Some(MimeMessage::new())),
                implicit(Some(MimeMessage::with_body(binary(suffix)))),
            ]);
            inner.set_epilogue(b"inner epilogue\r".to_vec());
            let mut outer: Multipart = Multipart::new("digest", "outer").unwrap();
            outer.set_preamble(b"preamble\r\n".to_vec());
            outer
                .children_mut()
                .push(implicit(Some(MimeMessage::with_body(inner.into()))));
            let mut root: MimeEntity = outer.into();
            let raw = write(&mut root, format);
            let mut parsed = load(&raw);
            let message = attached(&multipart(&parsed).children()[0])
                .message()
                .unwrap();
            let children = multipart(message.body().unwrap()).children();
            assert!(attached(&children[0]).message().is_none());
            assert!(attached(&children[1]).message().is_some());
            assert_eq!(
                attached(&children[2])
                    .leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .get_ref(),
                suffix
            );
            assert_eq!(write(&mut parsed, format), raw);
        }
    }
}

#[test]
fn detached_implicit_messages_need_an_explicit_type_before_standalone_writing() {
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\nSubject: attached\r\n\r\nbody\r\n--d--\r\n";
    let MimeEntity::Multipart(mut digest) = load(raw) else {
        unreachable!()
    };
    let mut detached = digest.children_mut().pop().unwrap();
    let mut output = Vec::new();
    let error = detached.write_to(&mut output).unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<MessagePartError>(),
        Some(MessagePartError::MissingContentType)
    ));
    assert!(output.is_empty());
    assert!(detached.headers().is_empty());
    let MimeEntity::Message(part) = &mut detached else {
        unreachable!()
    };
    part.set_content_type(&ContentType::new("message", "rfc822").unwrap())
        .unwrap();
    detached.write_to(&mut output).unwrap();
    assert!(matches!(load(&output), MimeEntity::Message(_)));
}

#[test]
fn writer_context_tracks_current_parent_headers_and_does_not_leak_through_messages() {
    let mut digest: Multipart = Multipart::new("digest", "d").unwrap();
    digest.children_mut().push(implicit(None));
    digest
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: multipart/mixed; boundary=d").unwrap())
        .unwrap();
    let error = digest.write_to(&mut Vec::new()).unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<MessagePartError>(),
        Some(MessagePartError::MissingContentType)
    ));
    digest
        .headers_mut()
        .replace_all(Header::parse(b"Content-Type: multipart/digest; boundary=d").unwrap())
        .unwrap();
    assert!(digest.write_to(&mut Vec::new()).is_ok());

    let untyped_body = implicit(None);
    digest.children_mut().clear();
    digest
        .children_mut()
        .push(implicit(Some(MimeMessage::with_body(untyped_body))));
    let error = digest.write_to(&mut Vec::new()).unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<MessagePartError>(),
        Some(MessagePartError::MissingContentType)
    ));

    let mut mixed: Multipart = Multipart::new("mixed", "m").unwrap();
    mixed.children_mut().push(implicit(None));
    digest.children_mut().clear();
    digest.children_mut().push(mixed.into());
    let error = digest.write_to(&mut Vec::new()).unwrap_err();
    assert!(matches!(
        error.get_ref().unwrap().downcast_ref::<MessagePartError>(),
        Some(MessagePartError::MissingContentType)
    ));
}

#[test]
fn explicit_bad_types_and_encodings_do_not_fall_back_to_digest_defaults() {
    for raw_header in [
        "Content-Type: invalid\r\n",
        "Content-Type:\r\nContent-Type: message/rfc822\r\n",
    ] {
        let raw = format!(
            "Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n{raw_header}\r\n\r\n\r\n--d--\r\n"
        );
        let error = MimeEntity::load(&mut raw.as_bytes()).unwrap_err();
        assert!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<ContentHeaderParseError>()
                .is_some()
        );
    }
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\nContent-Transfer-Encoding: unknown\r\n\r\n\r\n\r\n--d--\r\n";
    let error = MimeEntity::load(&mut &raw[..]).unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<UnknownContentEncoding>()
            .is_some()
    );

    for raw_header in [
        b"Content-Type: text/plain\r\n".as_slice(),
        b"Content-Type: invalid\r\n",
        b"Content-Transfer-Encoding: base64\r\n",
        b"Content-Transfer-Encoding: quoted-printable\r\n",
        b"Content-Transfer-Encoding: x-uuencode\r\n",
        b"Content-Transfer-Encoding: unknown\r\n",
    ] {
        let part: MessagePart = MessagePart::from_parts(headers(raw_header), None);
        let mut digest: Multipart = Multipart::new("digest", "d").unwrap();
        digest.children_mut().push(part.into());
        let mut output = Vec::new();
        assert_eq!(
            digest.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            output,
            b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n"
        );
    }
}

#[test]
fn implicit_wrappers_count_toward_shared_limits_and_fragmented_reads_work() {
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\nSubject: attached\r\n\r\nbody\r\n--d--\r\n";
    for capacity in [1, 2, 3, 7, 64] {
        let mut reader = BufReader::with_capacity(capacity, &raw[..]);
        let entity = MimeEntity::load_with_options(
            &mut reader,
            &MimeEntityParseOptions {
                max_depth: 3,
                max_input_bytes: raw.len(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(
            multipart(&entity).children()[0],
            MimeEntity::Message(_)
        ));
    }
    let error = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            max_depth: 2,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("depth"));
    let mut reader = &raw[..];
    let error = MimeEntity::load_with_options(
        &mut reader,
        &MimeEntityParseOptions {
            max_input_bytes: raw.len() - 1,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("byte limit"));
    assert_eq!(reader, b"\n");
    let raw = b"Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\n\r\n--d--\r\n";
    let entity = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            max_depth: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        attached(&multipart(&entity).children()[0])
            .message()
            .is_none()
    );
}

#[test]
fn implicit_messages_keep_ancestor_boundary_checks_and_require_complete_structure() {
    for body in [
        "Subject: missing separator\r\n",
        "Content-Type: multipart/mixed; boundary=inner\r\n\r\n",
        "Content-Type: multipart/mixed; boundary=d\r\n\r\n",
    ] {
        let raw =
            format!("Content-Type: multipart/digest; boundary=d\r\n\r\n--d\r\n\r\n{body}--d--\r\n");
        assert_eq!(
            MimeEntity::load(&mut raw.as_bytes()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    let inner: Multipart = Multipart::new("mixed", "d-long").unwrap();
    let mut digest: Multipart = Multipart::new("digest", "d").unwrap();
    digest
        .children_mut()
        .push(implicit(Some(MimeMessage::with_body(inner.into()))));
    let error = digest.write_to(&mut Vec::new()).unwrap_err();
    assert!(error.to_string().contains("ancestor"));
}

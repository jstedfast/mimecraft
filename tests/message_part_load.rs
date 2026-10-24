// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read};

use mimecraft::{
    ContentEncoding, Header, HeaderList, HeaderParser, HeaderTooLong, MessagePart, MimeContent,
    MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, MimePartWriteOptions, Multipart,
    TextPart, encodings::NewLineFormat,
};

fn load(raw: &[u8]) -> MimeEntity {
    MimeEntity::load(&mut &raw[..]).unwrap()
}

fn attached(entity: &MimeEntity) -> &MessagePart {
    let MimeEntity::Message(part) = entity else {
        panic!("message part expected")
    };
    part
}

fn headers(raw: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser.parse(raw, &mut records).unwrap();
    parser.finish(&mut records).unwrap();
    records.into()
}

fn binary(raw: &[u8]) -> MimeEntity {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(raw.to_vec())));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.into()
}

fn wrap(body: MimeEntity) -> MimeEntity {
    MessagePart::new(MimeMessage::from_parts(
        headers(b"Subject: attached\r\n"),
        Some(body),
    ))
    .into()
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

#[test]
fn recognized_subtypes_and_transparent_encodings_build_structured_messages() {
    assert!(MimeEntityParseOptions::default().parse_message_parts);
    for subtype in [
        "rfc822",
        "NEWS",
        "global",
        "global-headers",
        "external-body",
        "rfc2822",
    ] {
        for encoding in ["", "7bit", "8bit", "binary", "=?us-ascii?q?7bit?="] {
            let cte = if encoding.is_empty() {
                String::new()
            } else {
                format!("Content-Transfer-Encoding: {encoding}\r\n")
            };
            let raw =
                format!("Content-Type: MeSsAgE/{subtype}\r\n{cte}\r\nSubject: inner\r\n\r\nbody");
            let entity = load(raw.as_bytes());
            let part = attached(&entity);
            assert_eq!(
                part.headers().get("Content-Type").unwrap().raw_value(),
                format!(" MeSsAgE/{subtype}\r\n").as_bytes()
            );
            let message = part.message().unwrap();
            assert_eq!(
                message
                    .headers()
                    .get("Subject")
                    .unwrap()
                    .decode_value()
                    .text,
                "inner"
            );
            assert!(message.body().unwrap().headers().is_empty());
            let leaf = message.leaf_parts().next().unwrap();
            assert_eq!(leaf.content().get_ref().get_ref(), b"body");
            assert_eq!(leaf.content().encoding(), ContentEncoding::Unspecified);
            assert_eq!(leaf.content().get_ref().position(), 0);
        }
    }
}

#[test]
fn each_embedded_message_partitions_only_its_own_root_header_block() {
    let raw = concat!(
        "X-Wrapper: keep\r\nContent-Type: message/rfc822\r\n\r\n",
        "Received: first\nContent-Type: message/news\nSubject: outer\n",
        "CONTENT-X: raw\nMIME-Version: 1.0\nReceived: second\nbroken\n\n",
        "Subject: inner\r\nContent-Type: text/plain\r\nX-Message: kept\r\n\r\nhello",
    );
    let entity = load(raw.as_bytes());
    let part = attached(&entity);
    assert!(part.headers().contains("X-Wrapper"));
    let outer = part.message().unwrap();
    assert_eq!(
        outer.headers(),
        &headers(b"Received: first\nSubject: outer\nMIME-Version: 1.0\nReceived: second\nbroken\n")
    );
    let inner_part = attached(outer.body().unwrap());
    assert_eq!(
        inner_part.headers(),
        &headers(b"Content-Type: message/news\nCONTENT-X: raw\n")
    );
    let inner = inner_part.message().unwrap();
    assert_eq!(
        inner.headers(),
        &headers(b"Subject: inner\r\nX-Message: kept\r\n")
    );
    assert_eq!(
        inner.body().unwrap().headers(),
        &headers(b"Content-Type: text/plain\r\n")
    );
    assert_eq!(
        inner
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref(),
        b"hello"
    );
}

#[test]
fn encoded_and_unrecognized_message_types_stay_opaque() {
    for mime_type in [
        "message/rfc822",
        "message/global",
        "message/news",
        "text/rfc822-headers",
        "message/global-headers",
    ] {
        for (token, encoding, body) in [
            (
                "base64",
                ContentEncoding::Base64,
                "U3ViamVjdDogaGkNCg0KaGVsbG8=",
            ),
            (
                "quoted-printable",
                ContentEncoding::QuotedPrintable,
                "Subject: hi\r\n\r\nhello=0A",
            ),
            (
                "x-uuencode",
                ContentEncoding::UuEncode,
                "not a decoded message",
            ),
        ] {
            let raw = format!(
                "Content-Type: {mime_type}\r\nContent-Transfer-Encoding: {token}\r\n\r\n{body}"
            );
            let MimeEntity::Part(part) = load(raw.as_bytes()) else {
                panic!("encoded wrapper must be opaque")
            };
            assert_eq!(part.content().encoding(), encoding);
            assert_eq!(part.content().get_ref().get_ref(), body.as_bytes());
        }
    }
    for mime_type in [
        "message/partial",
        "message/delivery-status",
        "message/x-custom",
    ] {
        let raw = format!("Content-Type: {mime_type}\r\n\r\nnot message headers");
        assert!(matches!(load(raw.as_bytes()), MimeEntity::Part(_)));
    }
    let raw = b"Content-Type: multipart/digest; boundary=b\r\n\r\n--b\r\n\r\nSubject: opaque\r\n\r\nbody\r\n--b--\r\n";
    let entity = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            parse_message_parts: false,
            ..Default::default()
        },
    )
    .unwrap();
    let leaf = entity.leaf_parts().next().unwrap();
    assert_eq!(
        leaf.content().get_ref().get_ref(),
        b"Subject: opaque\r\n\r\nbody"
    );
    assert!(leaf.headers().is_empty());
}

#[test]
fn opaque_option_preserves_invalid_inner_input_without_applying_inner_limits() {
    let raw = b"Content-Type: message/rfc822\r\n\r\nSubject: no separator";
    assert!(
        attached(&load(raw))
            .message()
            .unwrap()
            .headers()
            .contains("Subject")
    );
    let options = MimeEntityParseOptions {
        parse_message_parts: false,
        max_depth: 1,
        ..Default::default()
    };
    let MimeEntity::Part(part) = MimeEntity::load_with_options(&mut &raw[..], &options).unwrap()
    else {
        panic!("opaque leaf expected");
    };
    assert_eq!(part.content().get_ref().get_ref(), b"Subject: no separator");
}

#[test]
fn missing_message_and_present_empty_message_round_trip_at_eof_and_boundaries() {
    for present in [false, true] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let mut part: MessagePart = MessagePart::new(MimeMessage::new());
            if !present {
                part.set_message(None);
            }
            let mut root: MimeEntity = part.into();
            let raw = write(&mut root, format);
            let mut parsed = load(&raw);
            // Without an enclosing boundary, EOF constructs an empty embedded
            // message even if the original wrapper had no payload.
            assert!(attached(&parsed).message().is_some());
            let expected = if present {
                raw.clone()
            } else {
                [
                    raw.as_slice(),
                    if format == NewLineFormat::Dos {
                        b"\r\n"
                    } else {
                        b"\n"
                    },
                ]
                .concat()
            };
            assert_eq!(write(&mut parsed, format), expected);
            let mut multipart: Multipart = Multipart::new("mixed", "outer").unwrap();
            multipart.children_mut().extend([root, binary(b"sibling")]);
            let mut root: MimeEntity = multipart.into();
            let raw = write(&mut root, format);
            let mut parsed = load(&raw);
            let MimeEntity::Multipart(multipart) = &parsed else {
                panic!("multipart expected")
            };
            assert_eq!(
                attached(&multipart.children()[0]).message().is_some(),
                present
            );
            assert_eq!(write(&mut parsed, format), raw);
        }
    }
    let raw = b"Content-Type: message/rfc822\r\n\r\nContent-Type: message/news\r\n\r\n";
    let entity = load(raw);
    let nested = attached(&entity).message().unwrap().body().unwrap();
    assert!(attached(nested).message().is_some());
}

#[test]
fn empty_message_chains_keep_separator_ownership_at_each_nesting_level() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for depth in 0..8 {
            for kind in 0..4 {
                let mut part: MessagePart = MessagePart::new(match kind {
                    2 => MimeMessage::from_parts(headers(b"Subject: empty\r\n"), None),
                    3 => MimeMessage::with_body(Multipart::new("mixed", "inside").unwrap().into()),
                    _ => MimeMessage::new(),
                });
                if kind == 0 {
                    part.set_message(None);
                }
                let mut entity: MimeEntity = part.into();
                for _ in 0..depth {
                    entity = wrap(entity);
                }
                let raw = write(&mut entity, format);
                let expected = if kind == 0 {
                    [
                        raw.as_slice(),
                        if format == NewLineFormat::Dos {
                            b"\r\n"
                        } else {
                            b"\n"
                        },
                    ]
                    .concat()
                } else {
                    raw.clone()
                };
                assert_eq!(write(&mut load(&raw), format), expected);
                let mut parent: Multipart = Multipart::new("mixed", "outer").unwrap();
                parent.children_mut().extend([entity, binary(b"sibling")]);
                let raw = write(&mut parent.into(), format);
                assert_eq!(write(&mut load(&raw), format), raw);
            }
        }
    }
}

#[test]
fn existing_header_separators_can_frame_empty_embedded_bodies() {
    let raw = b"Content-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\nContent-Type: message/rfc822\r\n\r\nSubject: empty\r\n\r\n--o--";
    let MimeEntity::Multipart(parent) = load(raw) else {
        panic!("multipart expected");
    };
    let message = attached(&parent.children()[0]).message().unwrap();
    assert!(message.headers().contains("Subject"));
    assert!(
        message
            .leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .get_ref()
            .is_empty()
    );

    let raw = b"Content-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\nContent-Type: message/rfc822\r\n\r\n--o--";
    let MimeEntity::Multipart(parent) = load(raw) else {
        panic!("multipart expected");
    };
    assert!(attached(&parent.children()[0]).message().is_none());
}

#[test]
fn nested_multipart_and_message_trees_preserve_binary_suffixes_and_epilogues() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for suffix in [b"".as_slice(), b"\0\xff\r", b"\n", b"\r\n", b"\r\n\r\n"] {
            let mut inside: Multipart = Multipart::new("mixed", "inside").unwrap();
            inside.set_preamble(b"pre\r\n".to_vec());
            inside
                .children_mut()
                .extend([wrap(wrap(binary(suffix))), binary(b"second")]);
            inside.set_epilogue(b"tail\r".to_vec());
            let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
            outer
                .children_mut()
                .extend([wrap(inside.into()), wrap(binary(b"sibling"))]);
            outer.set_epilogue(b"end".to_vec());
            let mut root = wrap(outer.into());
            let raw = write(&mut root, format);
            let mut parsed = load(&raw);
            let leaves: Vec<&[u8]> = parsed
                .leaf_parts()
                .map(|part| part.content().get_ref().get_ref().as_slice())
                .collect();
            assert_eq!(leaves, [suffix, b"second", b"sibling"]);
            assert_eq!(write(&mut parsed, format), raw);
        }
    }
}

#[test]
fn newline_hints_exclude_parent_framing_through_message_wrappers() {
    for (body, hint) in [
        (b"".as_slice(), None),
        (b"\r", None),
        (b"\n", Some(NewLineFormat::Unix)),
        (b"\r\n", Some(NewLineFormat::Dos)),
        (b"a\nb\r\n", Some(NewLineFormat::Mixed)),
    ] {
        let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
        outer.children_mut().push(wrap(wrap(binary(body))));
        let mut entity: MimeEntity = outer.into();
        let raw = write(&mut entity, NewLineFormat::Dos);
        let parsed = load(&raw);
        let content = parsed.leaf_parts().next().unwrap().content();
        assert_eq!(content.new_line_format(), hint);
        assert_eq!(content.get_ref().get_ref(), body);
        assert_eq!(content.get_ref().position(), 0);
    }
}

#[test]
fn depth_counts_wrappers_and_bodies_without_resetting_inside_messages() {
    let mut root = binary(b"body");
    for _ in 0..63 {
        root = wrap(root);
    }
    let raw = write(&mut root, NewLineFormat::Dos);
    assert_eq!(load(&raw).leaf_parts().count(), 1);
    let error = MimeEntity::load_with_options(
        &mut raw.as_slice(),
        &MimeEntityParseOptions {
            max_depth: 63,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("depth"));
    let raw = b"Content-Type: message/rfc822\r\n\r\n";
    assert!(
        MimeEntity::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                max_depth: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    let raw = b"Content-Type: message/rfc822\r\n\r\n\r\n";
    assert!(
        MimeEntity::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                max_depth: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    outer.children_mut().push(wrap(binary(b"body")));
    let raw = write(&mut outer.into(), NewLineFormat::Dos);
    assert!(
        MimeEntity::load_with_options(
            &mut raw.as_slice(),
            &MimeEntityParseOptions {
                max_depth: 3,
                ..Default::default()
            }
        )
        .is_ok()
    );
    assert!(
        MimeEntity::load_with_options(
            &mut raw.as_slice(),
            &MimeEntityParseOptions {
                max_depth: 2,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn wire_limits_count_each_input_byte_once_across_messages() {
    let raw = write(&mut wrap(wrap(binary(b"body"))), NewLineFormat::Dos);
    assert!(
        MimeEntity::load_with_options(
            &mut raw.as_slice(),
            &MimeEntityParseOptions {
                max_input_bytes: raw.len(),
                ..Default::default()
            }
        )
        .is_ok()
    );
    for limit in 1..raw.len() {
        let mut input = raw.as_slice();
        let error = MimeEntity::load_with_options(
            &mut input,
            &MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("byte limit"));
        assert_eq!(input, &raw[limit..]);
    }
}

#[test]
fn folded_inner_header_limits_keep_typed_errors() {
    let record = format!("X-Long: {}\r\n continued\r\n", "a".repeat(100));
    let raw = format!("Content-Type: message/rfc822\r\n\r\n{record}\r\nbody");
    assert!(
        MimeEntity::load_with_options(
            &mut raw.as_bytes(),
            &MimeEntityParseOptions {
                max_header_length: record.len(),
                ..Default::default()
            }
        )
        .is_ok()
    );
    let error = MimeEntity::load_with_options(
        &mut raw.as_bytes(),
        &MimeEntityParseOptions {
            max_header_length: record.len() - 1,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<HeaderTooLong>()
            .is_some()
    );
}

#[test]
fn incomplete_inner_headers_and_unclosed_multiparts_recover() {
    for inner in [
        "Subject: no separator",
        "Subject: no separator\r\n",
        "Content-Type: multipart/mixed; boundary=i\r\n\r\n",
        "Content-Type: multipart/mixed; boundary=i\r\n\r\n--i\r\n\r\nbody",
    ] {
        let raw = format!("Content-Type: message/rfc822\r\n\r\n{inner}");
        assert!(attached(&load(raw.as_bytes())).message().is_some());
        let newline = if raw.ends_with('\n') { "" } else { "\r\n" };
        let raw = format!(
            "Content-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\n{raw}{newline}--o--\r\n"
        );
        let MimeEntity::Multipart(parent) = load(raw.as_bytes()) else {
            panic!("multipart expected")
        };
        assert_eq!(parent.children().len(), 1);
        assert!(attached(&parent.children()[0]).message().is_some());
    }
}

#[test]
fn ancestor_boundaries_are_checked_across_message_frames_but_siblings_can_reuse_them() {
    for (outer, inner) in [
        ("b", "b"),
        ("prefix", "prefix-long"),
        ("prefix-long", "prefix"),
    ] {
        let raw = format!(
            "Content-Type: multipart/mixed; boundary={outer}\r\n\r\n--{outer}\r\nContent-Type: message/rfc822\r\n\r\nContent-Type: multipart/mixed; boundary={inner}\r\n\r\n"
        );
        let MimeEntity::Multipart(parent) = load(raw.as_bytes()) else {
            panic!("multipart expected")
        };
        assert_eq!(parent.children().len(), 1);
        assert!(matches!(
            attached(&parent.children()[0]).message().unwrap().body(),
            Some(MimeEntity::Multipart(_))
        ));
    }
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    for _ in 0..2 {
        let mut inner: Multipart = Multipart::new("mixed", "same").unwrap();
        inner.set_epilogue(b"--same\r\nignored marker\r\n".to_vec());
        outer.children_mut().push(wrap(inner.into()));
    }
    let raw = write(&mut outer.into(), NewLineFormat::Dos);
    let mut parsed = load(&raw);
    assert_eq!(write(&mut parsed, NewLineFormat::Dos), raw);
}

#[test]
fn fragmented_interrupted_and_failed_reads_preserve_message_loading_rules() {
    let raw = write(&mut wrap(wrap(binary(b"body\r\n"))), NewLineFormat::Dos);
    for capacity in [1, 2, 3, 7, 64, 4096] {
        let mut source = BufReader::with_capacity(capacity, Source::new(&raw, None));
        let reader: &mut dyn BufRead = &mut source;
        let mut entity = MimeEntity::load(reader).unwrap();
        assert_eq!(write(&mut entity, NewLineFormat::Dos), raw);
    }
    for limit in 0..=raw.len() {
        let mut source = BufReader::with_capacity(7, Source::new(&raw, Some(limit)));
        assert_eq!(
            MimeEntity::load(&mut source).unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }
}

#[test]
fn load_edit_write_updates_inner_text_without_changing_stored_encoding_semantics() {
    let raw = b"Subject: outer\r\nContent-Type: message/rfc822\r\n\r\nSubject: attached\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n";
    let mut message = MimeMessage::load(&mut &raw[..]).unwrap();
    let MimeEntity::Message(part) = message.body_mut().unwrap() else {
        panic!("message part expected")
    };
    let inner = part.message_mut().unwrap();
    let MimeEntity::Part(body) = inner.set_body(None).unwrap() else {
        panic!("text leaf expected")
    };
    assert_eq!(body.content().encoding(), ContentEncoding::Base64);
    let mut text = TextPart::from_part(body).unwrap();
    assert_eq!(text.text().unwrap(), "hello");
    text.set_text("edited").unwrap();
    inner.set_body(Some(text.into()));
    inner
        .headers_mut()
        .replace_all(Header::parse(b"Subject: changed").unwrap())
        .unwrap();
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    let mut parsed = MimeMessage::load(&mut output.as_slice()).unwrap();
    let MimeEntity::Message(part) = parsed.body_mut().unwrap() else {
        panic!("message part expected")
    };
    let inner = part.message_mut().unwrap();
    assert_eq!(
        inner.headers().get("Subject").unwrap().decode_value().text,
        "changed"
    );
    let MimeEntity::Part(body) = inner.set_body(None).unwrap() else {
        panic!("text leaf expected")
    };
    assert_eq!(body.content().encoding(), ContentEncoding::Base64);
    assert_eq!(TextPart::from_part(body).unwrap().text().unwrap(), "edited");
}

struct Source<'a> {
    bytes: &'a [u8],
    position: usize,
    calls: usize,
    fail_after: Option<usize>,
}

impl<'a> Source<'a> {
    fn new(bytes: &'a [u8], fail_after: Option<usize>) -> Self {
        Self {
            bytes,
            position: 0,
            calls: 0,
            fail_after,
        }
    }
}

impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls % 5 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.position) {
            return Err(io::Error::other("read failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.position);
        let count = output
            .len()
            .min(3)
            .min(remaining)
            .min(self.bytes.len() - self.position);
        output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

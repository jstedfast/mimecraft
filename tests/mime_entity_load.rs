// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read};

use mimecraft::{
    ContentEncoding, Header, HeaderTooLong, MimeContent, MimeEntity, MimeEntityParseOptions,
    MimePart, MimePartWriteOptions, Multipart, TextPart, encodings::NewLineFormat,
};

fn load(bytes: &[u8]) -> MimeEntity {
    MimeEntity::load(&mut &bytes[..]).unwrap()
}

fn part(entity: &MimeEntity) -> &MimePart<Cursor<Vec<u8>>> {
    let MimeEntity::Part(part) = entity else {
        panic!("expected a leaf")
    };
    part
}

fn multipart(entity: &MimeEntity) -> &Multipart {
    let MimeEntity::Multipart(part) = entity else {
        panic!("expected a container")
    };
    part
}

fn bytes(entity: &MimeEntity) -> &[u8] {
    part(entity).content().get_ref().get_ref()
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

fn binary(body: &[u8]) -> MimeEntity {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(body.to_vec())));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.into()
}

#[test]
fn leaf_loading_preserves_raw_headers_encoding_bytes_and_zero_position() {
    let raw = b"cOnTeNt-TyPe: text/plain;\r\n\tcharset=utf-8\r\nX-Raw: \xff\r\nX-Raw: duplicate\r\nmalformed\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n";
    let entity = load(raw);
    let leaf = part(&entity);
    assert_eq!(leaf.headers().len(), 5);
    assert_eq!(leaf.headers().get_all("x-raw").count(), 2);
    assert_eq!(
        leaf.headers().get("content-type").unwrap().raw(),
        b"cOnTeNt-TyPe: text/plain;\r\n\tcharset=utf-8\r\n"
    );
    assert_eq!(leaf.content().encoding(), ContentEncoding::Base64);
    assert_eq!(bytes(&entity), b"aGVsbG8=\r\n");
    assert_eq!(leaf.content().new_line_format(), Some(NewLineFormat::Dos));
    assert_eq!(leaf.content().get_ref().position(), 0);
    let MimeEntity::Part(leaf) = entity else {
        unreachable!()
    };
    assert_eq!(TextPart::from_part(leaf).unwrap().text().unwrap(), "hello");
}

#[test]
fn opaque_option_preserves_message_leaves_without_synthesized_headers() {
    for header in [
        "",
        "X-Other: value\r\n",
        "Content-Type: message/rfc822\r\n",
        "Content-Type: message/global\r\n",
    ] {
        let input = format!("{header}\r\nFrom: someone@example.com\r\n\r\nbody");
        let entity = MimeEntity::load_with_options(
            &mut input.as_bytes(),
            &MimeEntityParseOptions {
                parse_message_parts: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(bytes(&entity), b"From: someone@example.com\r\n\r\nbody");
        assert_eq!(
            part(&entity).content().encoding(),
            ContentEncoding::Unspecified
        );
        if !header.starts_with("Content-Type") {
            assert!(entity.headers().get("content-type").is_none());
        }
    }
    assert_eq!(bytes(&load(b"\r\n")), b"");
    assert_eq!(bytes(&load(b"\n")), b"");
}

#[test]
fn newline_hints_describe_stored_body_not_boundary_lines() {
    for (body, hint) in [
        (&b""[..], None),
        (b"bare\r", None),
        (b"x\n", Some(NewLineFormat::Unix)),
        (b"x\r\n", Some(NewLineFormat::Dos)),
        (b"x\r\ny\n", Some(NewLineFormat::Mixed)),
    ] {
        let entity = load(&[b"\r\n".as_slice(), body].concat());
        assert_eq!(part(&entity).content().new_line_format(), hint);
        let wire = [
            b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\n".as_slice(),
            body,
            b"\r\n--b--\r\n",
        ]
        .concat();
        let entity = load(&wire);
        let child = &multipart(&entity).children()[0];
        assert_eq!(bytes(child), body);
        assert_eq!(part(child).content().new_line_format(), hint);
    }
}

#[test]
fn nested_load_write_round_trips_preserve_binary_suffixes_and_epilogues() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for suffix in [&b""[..], b"x", b"x\r", b"x\n", b"x\r\n", b"x\r\n\r\n"] {
            let mut inner: Multipart = Multipart::new("alternative", "inner").unwrap();
            inner.children_mut().push(binary(suffix));
            inner.set_preamble(b"inner preamble\r\n".to_vec());
            inner.set_epilogue(b"inner epilogue\r\n".to_vec());
            let mut root: Multipart = Multipart::new("mixed", "outer").unwrap();
            root.children_mut()
                .extend([inner.into(), binary(b"\0\xff")]);
            root.set_preamble(b"outer preamble\r\n".to_vec());
            root.set_epilogue(b"outer epilogue\r\n".to_vec());
            let wire = write(&mut root.into(), format);
            let mut parsed = load(&wire);
            let outer = multipart(&parsed);
            let inner = multipart(&outer.children()[0]);
            assert_eq!(bytes(&inner.children()[0]), suffix);
            assert_eq!(bytes(&outer.children()[1]), b"\0\xff");
            let newline = if format == NewLineFormat::Dos {
                "\r\n"
            } else {
                "\n"
            };
            assert_eq!(
                inner.preamble(),
                format!("inner preamble{newline}").as_bytes()
            );
            assert_eq!(
                inner.epilogue(),
                format!("inner epilogue{newline}").as_bytes()
            );
            assert_eq!(
                outer.epilogue(),
                format!("outer epilogue{newline}").as_bytes()
            );
            assert_eq!(write(&mut parsed, format), wire);
        }
    }
}

#[test]
fn separators_can_double_as_framing_for_empty_external_children() {
    let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nX: first\r\n\r\n--b\r\n\r\n--b--\r\n";
    let mut entity = load(raw);
    let children = multipart(&entity).children();
    assert_eq!(children.len(), 2);
    assert_eq!(bytes(&children[0]), b"");
    assert_eq!(bytes(&children[1]), b"");
    let normalized = write(&mut entity, NewLineFormat::Dos);
    assert_ne!(normalized, raw);
    assert_eq!(load(&normalized).leaf_parts().count(), 2);
    assert!(
        load(&normalized)
            .leaf_parts()
            .all(|leaf| leaf.content().get_ref().get_ref().is_empty())
    );
}

#[test]
fn empty_containers_and_closing_markers_at_eof_are_supported() {
    for ending in ["", "\r\n", "\n", " \t", " \t\r\n"] {
        let entity = load(
            format!("Content-Type: multipart/mixed; boundary=b\r\n\r\npreamble\n--b--{ending}")
                .as_bytes(),
        );
        let multi = multipart(&entity);
        assert!(multi.children().is_empty());
        assert_eq!(multi.preamble(), b"preamble\n");
        assert_eq!(multi.epilogue(), b"");
    }
    let mut outer: Multipart = Multipart::new("mixed", "outer").unwrap();
    let empty: Multipart = Multipart::new("mixed", "inner").unwrap();
    outer.children_mut().push(empty.into());
    let raw = write(&mut outer.into(), NewLineFormat::Dos);
    let mut loaded = load(&raw);
    assert!(
        multipart(&multipart(&loaded).children()[0])
            .children()
            .is_empty()
    );
    assert_eq!(write(&mut loaded, NewLineFormat::Dos), raw);
}

#[test]
fn delimiters_are_case_sensitive_whole_lines_with_only_space_tab_padding() {
    let raw = concat!(
        "Content-Type: multipart/mixed; boundary=\"a b\"\r\n\r\n",
        "--a b \t\r\n\r\n",
        "--a bx\r\n--a b--x\r\n--a b-\r\n--A b\r\n",
        " --a b\r\nprefix--a b\r--a b\r\n--a b \x0b\r\n",
        "--a b-- \t\r\n",
        "--a b\r\n"
    );
    let entity = load(raw.as_bytes());
    let multi = multipart(&entity);
    assert_eq!(multi.children().len(), 1);
    assert_eq!(
        bytes(&multi.children()[0]),
        b"--a bx\r\n--a b--x\r\n--a b-\r\n--A b\r\n --a b\r\nprefix--a b\r--a b\r\n--a b \x0b"
    );
    assert_eq!(multi.epilogue(), b"--a b\r\n");
}

#[test]
fn every_input_split_and_one_byte_reads_produce_the_same_tree() {
    let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\npre\r\n--b\r\nContent-Transfer-Encoding: binary\r\n\r\n\0\xff\r\n\r\n--b--\r\nepi";
    for split in 1..raw.len() {
        let first = Cursor::new(&raw[..split]);
        let second = Cursor::new(&raw[split..]);
        let mut reader = first.chain(second);
        let mut entity = MimeEntity::load(&mut reader).unwrap();
        assert_eq!(write(&mut entity, NewLineFormat::Dos), raw);
    }
    let mut reader = BufReader::with_capacity(1, ChunkReader::new(raw));
    let source: &mut dyn BufRead = &mut reader;
    assert_eq!(
        write(&mut MimeEntity::load(source).unwrap(), NewLineFormat::Dos),
        raw
    );
}

#[test]
fn truncated_structure_recovers_but_empty_input_is_not_an_entity() {
    assert_eq!(
        MimeEntity::load(&mut b"".as_slice()).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    for input in [
        "X: value",
        "X: value\r\n",
        "\r",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody\r\n--B--\r\n",
    ] {
        assert!(MimeEntity::load(&mut input.as_bytes()).is_ok(), "{input:?}");
    }
    for input in [
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n--b--\r\n",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=c\r\n\r\n--b--\r\n",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=c\r\n\r\n--c\r\n\r\nbody\r\n--b--\r\n",
    ] {
        assert!(MimeEntity::load(&mut input.as_bytes()).is_ok(), "{input:?}");
    }
    let inner_closed_outer_missing = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/mixed; boundary=c\r\n\r\n--c--\r\n";
    assert_eq!(
        multipart(&load(inner_closed_outer_missing))
            .children()
            .len(),
        1
    );
}

#[test]
fn first_metadata_errors_are_not_masked_by_valid_duplicates() {
    let cases: &[(&[u8], &str)] = &[
        (
            b"Content-Type: invalid\r\nContent-Type: text/plain\r\n\r\nbody",
            "type",
        ),
        (
            b"Content-Transfer-Encoding: unknown\r\nContent-Transfer-Encoding: base64\r\n\r\nbody",
            "encoding",
        ),
        (b"Content-Transfer-Encoding:\r\n\r\nbody", "encoding"),
        (b"Content-Type: multipart/mixed\r\n\r\nbody", "boundary"),
        (
            b"Content-Type: multipart/mixed; boundary=\"bad \"\r\n\r\nbody",
            "boundary",
        ),
    ];
    for &(input, kind) in cases {
        let entity = load(input);
        match kind {
            "type" => {
                assert!(
                    entity
                        .headers()
                        .get("Content-Type")
                        .unwrap()
                        .parse_content_type()
                        .is_err()
                );
                assert_eq!(bytes(&entity), b"body");
            }
            "encoding" => {
                assert_eq!(
                    part(&entity).content().encoding(),
                    ContentEncoding::Unspecified
                );
                assert_eq!(bytes(&entity), b"body");
            }
            "boundary" => {
                assert!(multipart(&entity).children().is_empty());
                assert_eq!(multipart(&entity).preamble(), b"body");
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn multipart_loading_recovers_encodings_and_overlapping_boundaries() {
    for encoding in ["base64", "quoted-printable", "x-uuencode", "unknown"] {
        let raw = format!(
            "Content-Type: multipart/mixed; boundary=b\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n--b--\r\n"
        );
        assert!(multipart(&load(raw.as_bytes())).children().is_empty());
    }
    for encoding in ["7bit", "8bit", "binary", "=?us-ascii?q?7bit?="] {
        let raw = format!(
            "Content-Type: multipart/mixed; boundary=b\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n--b--\r\n"
        );
        assert!(matches!(load(raw.as_bytes()), MimeEntity::Multipart(_)));
    }
    for (outer, inner) in [("b", "b"), ("b", "b-long"), ("b-long", "b")] {
        let raw = format!(
            "Content-Type: multipart/mixed; boundary={outer}\r\n\r\n--{outer}\r\nContent-Type: multipart/mixed; boundary={inner}\r\n\r\n"
        );
        assert_eq!(multipart(&load(raw.as_bytes())).children().len(), 1);
    }
    let raw = b"Content-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\nContent-Type: multipart/mixed; boundary=s\r\n\r\n--s--\r\n\r\n--o\r\nContent-Type: multipart/mixed; boundary=s\r\n\r\n--s--\r\n\r\n--o--\r\n";
    assert_eq!(multipart(&load(raw)).children().len(), 2);
}

#[test]
fn input_limit_accepts_exact_size_and_does_not_consume_an_excess_byte() {
    let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b--\r\nend";
    let options = MimeEntityParseOptions {
        max_input_bytes: raw.len(),
        ..Default::default()
    };
    assert!(MimeEntity::load_with_options(&mut &raw[..], &options).is_ok());
    for limit in 1..raw.len() {
        let mut input = &raw[..];
        let error = MimeEntity::load_with_options(
            &mut input,
            &MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{limit}");
        assert!(error.to_string().contains("byte limit"));
        assert_eq!(input, &raw[limit..]);
    }
}

#[test]
fn header_limit_counts_folds_and_retains_typed_error_cause() {
    let raw = b"X: a\r\n b\r\n\r\nbody";
    assert!(
        MimeEntity::load_with_options(
            &mut &raw[..],
            &MimeEntityParseOptions {
                max_header_length: 10,
                ..Default::default()
            }
        )
        .is_ok()
    );
    let error = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            max_header_length: 9,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(
        error
            .get_ref()
            .unwrap()
            .downcast_ref::<HeaderTooLong>()
            .is_some()
    );
}

#[test]
fn depth_limit_counts_root_and_is_checked_before_descendant_headers() {
    let mut root = binary(b"deep");
    for depth in 0..63 {
        let mut parent: Multipart =
            Multipart::new("mixed", &format!("depth-{depth:03}-end")).unwrap();
        parent.children_mut().push(root);
        root = parent.into();
    }
    let raw = write(&mut root, NewLineFormat::Dos);
    assert_eq!(load(&raw).leaf_parts().count(), 1);
    let error = MimeEntity::load_with_options(
        &mut &raw[..],
        &MimeEntityParseOptions {
            max_depth: 63,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("depth"));
    let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody\r\n--b--\r\n";
    let mut input = &raw[..];
    assert!(
        MimeEntity::load_with_options(
            &mut input,
            &MimeEntityParseOptions {
                max_depth: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(input, b"\r\nbody\r\n--b--\r\n");
}

#[test]
fn invalid_options_do_no_io_and_source_failures_never_return_partial_trees() {
    for options in [
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
    ] {
        let mut reader = BufReader::new(ChunkReader::new(b"\r\nbody"));
        assert_eq!(
            MimeEntity::load_with_options(&mut reader, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(reader.get_ref().calls, 0);
    }
    let raw =
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nbody\r\n--b--\r\nepilogue";
    for fail_after in 0..=raw.len() {
        let mut source = ChunkReader::new(raw);
        source.fail_after = Some(fail_after);
        let mut reader = BufReader::with_capacity(7, source);
        assert_eq!(
            MimeEntity::load(&mut reader).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(reader.get_ref().position, fail_after);
    }
}

#[test]
fn stored_transfer_encodings_can_be_read_edited_and_rewritten() {
    for encoding in [
        ContentEncoding::Unspecified,
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut original = TextPart::new("plain", "hello").unwrap();
        original.part_mut().set_content_transfer_encoding(encoding);
        let raw = write(&mut original.into(), NewLineFormat::Dos);
        let MimeEntity::Part(mut loaded) = load(&raw) else {
            unreachable!()
        };
        assert_eq!(loaded.content().encoding(), encoding);
        loaded.set_content_transfer_encoding(ContentEncoding::Base64);
        let mut text = TextPart::from_part(loaded).unwrap();
        assert_eq!(text.text().unwrap(), "hello");
        text.set_text("changed").unwrap();
        text.part_mut()
            .headers_mut()
            .add(Header::parse(b"X-Edited: yes").unwrap());
        let edited = write(&mut text.into(), NewLineFormat::Dos);
        let MimeEntity::Part(loaded) = load(&edited) else {
            unreachable!()
        };
        assert_eq!(
            loaded
                .headers()
                .get("x-edited")
                .unwrap()
                .decode_value()
                .text,
            "yes"
        );
        assert_eq!(loaded.content().encoding(), ContentEncoding::Base64);
        assert_eq!(
            TextPart::from_part(loaded).unwrap().text().unwrap(),
            "changed"
        );
    }
}

#[test]
fn long_binary_lines_and_raw_text_sections_cross_reader_buffers() {
    let payload = b"\0\xffx\r".repeat(16384);
    let mut root: Multipart = Multipart::new("mixed", "b").unwrap();
    root.set_preamble(b"preamble\r\n".repeat(1024));
    root.set_epilogue(b"epilogue\r\n".repeat(1024));
    root.children_mut().push(binary(&payload));
    let wire = write(&mut root.into(), NewLineFormat::Dos);
    let mut source = BufReader::with_capacity(17, ChunkReader::new(&wire));
    let mut entity = MimeEntity::load(&mut source).unwrap();
    assert_eq!(bytes(&multipart(&entity).children()[0]), payload);
    assert_eq!(write(&mut entity, NewLineFormat::Dos), wire);
}

#[test]
fn every_binary_byte_and_nested_bare_cr_epilogues_round_trip_in_both_formats() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut root: Multipart = Multipart::new("mixed", "outer").unwrap();
        for byte in 0..=255 {
            root.children_mut().push(binary(&[byte]));
        }
        let mut inner: Multipart = Multipart::new("mixed", "inner").unwrap();
        inner.set_epilogue(b"tail\r".to_vec());
        root.children_mut().push(inner.into());
        let raw = write(&mut root.into(), format);
        let mut loaded = load(&raw);
        for (byte, child) in multipart(&loaded).children()[..256].iter().enumerate() {
            assert_eq!(bytes(child), [byte as u8]);
        }
        assert_eq!(
            multipart(&multipart(&loaded).children()[256]).epilogue(),
            b"tail\r"
        );
        assert_eq!(write(&mut loaded, format), raw);
    }
}

struct ChunkReader<'a> {
    input: &'a [u8],
    position: usize,
    calls: usize,
    fail_after: Option<usize>,
}

impl<'a> ChunkReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            position: 0,
            calls: 0,
            fail_after: None,
        }
    }
}

impl Read for ChunkReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls % 5 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.position) {
            return Err(io::Error::other("source failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.position);
        let count = output
            .len()
            .min(3)
            .min(remaining)
            .min(self.input.len() - self.position);
        output[..count].copy_from_slice(&self.input[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

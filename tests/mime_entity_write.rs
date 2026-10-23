// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentEncoding, Header, HeaderList, HeaderParser, MimeContent, MimeEntity, MimePart,
    MimePartWriteOptions, Multipart, TextPart, encodings::NewLineFormat,
};

fn container(boundary: &str) -> Multipart {
    Multipart::new("mixed", boundary).unwrap()
}

fn leaf(bytes: &[u8], encoding: ContentEncoding) -> MimeEntity {
    let mut source = Cursor::new(bytes.to_vec());
    source.set_position(2);
    let mut part = MimePart::new(MimeContent::new(source));
    part.set_content_transfer_encoding(encoding);
    part.into()
}

fn wire(part: &mut Multipart, options: &MimePartWriteOptions) -> Vec<u8> {
    let mut output = Vec::new();
    part.write_to_with_options(&mut output, options).unwrap();
    output
}

fn nested() -> Multipart {
    let mut inner = container("inner");
    inner
        .children_mut()
        .push(TextPart::new("plain", "hello").unwrap().into());
    inner.set_epilogue(b"tail".to_vec());
    let mut outer = container("outer");
    outer.set_preamble(b"pre".to_vec());
    outer.children_mut().push(inner.into());
    outer.set_epilogue(b"done".to_vec());
    outer
}

#[test]
fn nested_tree_has_exact_framing_and_root_only_newline_option() {
    let expected = concat!(
        "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
        "pre\r\n--outer\r\n",
        "Content-Type: multipart/mixed; boundary=inner\r\n\r\n",
        "--inner\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n\r\n",
        "hello\r\n--inner--\r\n",
        "tail\r\n--outer--\r\n",
        "done",
    );
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for ensure in [false, true] {
            let mut part = nested();
            let options = MimePartWriteOptions {
                new_line_format: format,
                ensure_new_line: ensure,
                ..Default::default()
            };
            let output = wire(&mut part, &options);
            let mut expected = expected.to_owned();
            if ensure {
                expected.push_str("\r\n");
            }
            if format == NewLineFormat::Unix {
                expected = expected.replace("\r\n", "\n");
            }
            assert_eq!(output, expected.as_bytes());
            assert_eq!(part.preamble(), b"pre");
            assert_eq!(part.epilogue(), b"done");
            assert_eq!(
                part.leaf_parts()
                    .next()
                    .unwrap()
                    .content()
                    .get_ref()
                    .position(),
                0
            );
            assert_eq!(wire(&mut part, &options), output);
        }
    }
}

#[test]
fn delimiter_newline_never_consumes_binary_suffix_bytes_or_empty_bodies() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let newline = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        for bytes in [
            &b""[..],
            b"\0\xff",
            b"body\r",
            b"body\n",
            b"body\r\n",
            b"body\r\n\r\n",
        ] {
            let mut part = container("outer");
            part.children_mut()
                .push(leaf(bytes, ContentEncoding::Binary));
            part.children_mut()
                .push(leaf(b"second", ContentEncoding::Binary));
            let output = wire(
                &mut part,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line: true,
                    ..Default::default()
                },
            );
            let child_headers = format!(
                "Content-Type: application/octet-stream{newline}Content-Transfer-Encoding: binary{newline}{newline}"
            );
            let prefix = format!(
                "Content-Type: multipart/mixed; boundary=outer{newline}{newline}--outer{newline}{child_headers}"
            );
            let delimiter_newline = if format == NewLineFormat::Unix && bytes.ends_with(b"\r") {
                "\r\n"
            } else {
                newline
            };
            let middle = format!("{delimiter_newline}--outer{newline}{child_headers}");
            let ending = format!("second{newline}--outer--{newline}");
            let expected = [
                prefix.as_bytes(),
                bytes,
                middle.as_bytes(),
                ending.as_bytes(),
            ]
            .concat();
            assert_eq!(output, expected);
            let content_start = prefix.len();
            let delimiter_start = content_start + bytes.len();
            assert_eq!(&output[content_start..delimiter_start], bytes);
            assert!(output[delimiter_start..].starts_with(middle.as_bytes()));
        }
    }
}

#[test]
fn empty_containers_and_nonempty_preambles_are_properly_terminated() {
    for preamble in [&b""[..], b"intro", b"intro\n", b"intro\r", b"intro\r\n"] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let newline = if format == NewLineFormat::Dos {
                "\r\n"
            } else {
                "\n"
            };
            let mut part = container("empty");
            part.set_preamble(preamble.to_vec());
            let output = wire(
                &mut part,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ..Default::default()
                },
            );
            let text = if preamble.is_empty() {
                String::new()
            } else {
                format!("intro{newline}")
            };
            assert_eq!(output, format!(
                "Content-Type: multipart/mixed; boundary=empty{newline}{newline}{text}--empty--{newline}"
            ).as_bytes());
        }
    }
    let mut outer = container("outer");
    outer.children_mut().push(container("empty").into());
    assert_eq!(
        wire(&mut outer, &MimePartWriteOptions::default()),
        concat!(
            "Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\n",
            "Content-Type: multipart/mixed; boundary=empty\r\n\r\n--empty--\r\n",
            "\r\n--outer--\r\n"
        )
        .as_bytes()
    );
}

#[test]
fn raw_preamble_and_epilogue_preserve_non_newline_bytes() {
    let mut part = container("b");
    part.set_preamble(b"\xffa\r\nb\nc\rd".to_vec());
    part.set_epilogue(b"\0a\r\nb\nc\rd".to_vec());
    let before = part.headers().clone();
    let output = wire(
        &mut part,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    );
    assert_eq!(
        output,
        b"Content-Type: multipart/mixed; boundary=b\n\n\xffa\nb\nc\rd\n--b--\n\0a\nb\nc\rd"
    );
    assert_eq!(part.headers(), &before);
    assert_eq!(part.preamble(), b"\xffa\r\nb\nc\rd");
    assert_eq!(part.epilogue(), b"\0a\r\nb\nc\rd");
}

#[test]
fn unix_delimiter_guard_handles_short_writes_and_nested_epilogues() {
    let mut inner = container("inner");
    inner
        .children_mut()
        .push(leaf(b"binary\r", ContentEncoding::Binary));
    inner.set_epilogue(b"tail\r".to_vec());
    let mut outer = container("outer");
    outer.children_mut().push(inner.into());
    let options = MimePartWriteOptions {
        new_line_format: NewLineFormat::Unix,
        ..Default::default()
    };
    let expected = wire(&mut outer, &options);
    assert!(
        expected
            .windows(b"binary\r\r\n--inner--\n".len())
            .any(|bytes| bytes == b"binary\r\r\n--inner--\n")
    );
    assert!(expected.ends_with(b"tail\r\r\n--outer--\n"));
    let mut sink = Sink {
        chunk: 1,
        interrupt: true,
        ..Default::default()
    };
    outer.write_to_with_options(&mut sink, &options).unwrap();
    assert_eq!(sink.bytes, expected);
}

#[test]
fn leaf_serialization_is_reused_for_every_transfer_encoding() {
    for encoding in [
        ContentEncoding::Unspecified,
        ContentEncoding::SevenBit,
        ContentEncoding::EightBit,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut single = leaf(b"Cat\n", encoding);
        let mut leaf_output = Vec::new();
        single.write_to(&mut leaf_output).unwrap();
        let mut part = container("b");
        part.children_mut().push(leaf(b"Cat\n", encoding));
        let before = part.children()[0].headers().clone();
        let output = wire(&mut part, &MimePartWriteOptions::default());
        assert_eq!(
            output,
            [
                b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n".as_slice(),
                &leaf_output,
                b"\r\n--b--\r\n"
            ]
            .concat()
        );
        assert_eq!(part.children()[0].headers(), &before);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .get_ref(),
            b"Cat\n"
        );
    }
    let mut source = MimePart::new(MimeContent::with_encoding(
        Cursor::new(b"Q2F0\r\n".to_vec()),
        ContentEncoding::Base64,
    ));
    source.set_content_transfer_encoding(ContentEncoding::QuotedPrintable);
    let mut part = container("b");
    part.children_mut().push(source.into());
    assert!(
        wire(&mut part, &MimePartWriteOptions::default())
            .ends_with(b"\r\n\r\nCat=\r\n\r\n--b--\r\n")
    );
}

#[test]
fn leaf_roots_match_direct_writer_and_multipart_entity_roots_match_containers() {
    let mut entity = leaf(b"hello", ContentEncoding::Base64);
    let mut actual = Vec::new();
    entity.write_to(&mut actual).unwrap();
    let MimeEntity::Part(mut part) = entity else {
        panic!("leaf expected")
    };
    let mut expected = Vec::new();
    part.write_to(&mut expected).unwrap();
    assert_eq!(actual, expected);

    let expected = wire(&mut nested(), &MimePartWriteOptions::default());
    let mut root: MimeEntity = nested().into();
    let mut output = Vec::new();
    let destination: &mut dyn Write = &mut output;
    root.write_to(destination).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn invalid_root_metadata_and_options_fail_before_output_or_flush() {
    for raw in [
        None,
        Some("Content-Type: invalid"),
        Some("Content-Type: text/plain; boundary=b"),
        Some("Content-Type: multipart/mixed"),
        Some("Content-Type: multipart/mixed; boundary=\"bad \""),
    ] {
        let mut part = container("b");
        part.headers_mut().clear();
        if let Some(raw) = raw {
            part.headers_mut()
                .add(Header::parse(raw.as_bytes()).unwrap());
        }
        part.children_mut()
            .push(leaf(b"untouched", ContentEncoding::Binary));
        let mut sink = Sink::default();
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(sink.calls + sink.flushes, 0);
        assert_eq!(
            part.leaf_parts()
                .next()
                .unwrap()
                .content()
                .get_ref()
                .position(),
            2
        );
    }
    for options in [
        MimePartWriteOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 59,
            ..Default::default()
        },
        MimePartWriteOptions {
            max_line_length: 999,
            ..Default::default()
        },
    ] {
        let mut sink = Sink::default();
        assert_eq!(
            nested()
                .write_to_with_options(&mut sink, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
}

#[test]
fn multipart_transfer_encodings_are_checked_using_the_first_header() {
    for encoding in ["7bit", "8bit", "binary", "=?us-ascii?q?7bit?="] {
        let mut part = container("b");
        part.headers_mut().add(
            Header::parse(format!("Content-Transfer-Encoding: {encoding}").as_bytes()).unwrap(),
        );
        assert!(part.write_to(&mut Vec::new()).is_ok());
    }
    for encoding in ["base64", "quoted-printable", "x-uuencode", "unknown", ""] {
        let mut part = container("b");
        part.headers_mut().add(
            Header::parse(format!("Content-Transfer-Encoding: {encoding}").as_bytes()).unwrap(),
        );
        part.headers_mut()
            .add(Header::parse(b"Content-Transfer-Encoding: 7bit").unwrap());
        let mut sink = Sink::default();
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(sink.calls + sink.flushes, 0);
    }
}

#[test]
fn ancestor_boundary_overlap_is_rejected_but_sibling_reuse_is_allowed() {
    for (outer, inner) in [
        ("same", "same"),
        ("outer", "outer-long"),
        ("outer-long", "outer"),
    ] {
        let mut part = container(outer);
        let mut nested = container("distinct");
        nested.children_mut().push(container(inner).into());
        part.children_mut().push(nested.into());
        let mut output = Vec::new();
        let error = part.write_to(&mut output).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("ancestor"));
        assert!(!output.ends_with(format!("--{outer}--\r\n").as_bytes()));
    }
    let mut part = container("outer");
    part.children_mut()
        .extend([container("shared").into(), container("shared").into()]);
    let output = wire(&mut part, &MimePartWriteOptions::default());
    assert_eq!(
        output
            .windows(b"--shared--".len())
            .filter(|bytes| *bytes == b"--shared--")
            .count(),
        2
    );
}

#[test]
fn container_headers_repair_truncation_without_mutating_raw_records() {
    let mut parser = HeaderParser::new();
    let mut records = Vec::new();
    parser
        .parse(
            b"Content-Type: multipart/mixed; boundary=b\nX: unterminated",
            &mut records,
        )
        .unwrap();
    parser.finish(&mut records).unwrap();
    let raw: HeaderList = records.into();
    let mut part: Multipart =
        Multipart::from_parts(raw.clone(), Vec::new(), Vec::new(), Vec::new());
    let output = wire(&mut part, &MimePartWriteOptions::default());
    assert_eq!(
        output,
        b"Content-Type: multipart/mixed; boundary=b\r\nX: unterminated\r\n\r\n--b--\r\n"
    );
    assert_eq!(part.headers(), &raw);
}

#[test]
fn deep_trees_stream_without_recursive_serialization() {
    let mut root = leaf(b"deep", ContentEncoding::Binary);
    for depth in 0..512 {
        let mut part = container(&format!("boundary-{depth:04}-end"));
        part.children_mut().push(root);
        root = part.into();
    }
    let mut sink = Sink {
        chunk: 3,
        interrupt: true,
        ..Default::default()
    };
    root.write_to(&mut sink).unwrap();
    assert!(sink.bytes.ends_with(b"\r\n--boundary-0511-end--\r\n"));
    assert_eq!(
        sink.bytes
            .windows(4)
            .filter(|bytes| *bytes == b"deep")
            .count(),
        1
    );
    assert_eq!(
        root.leaf_parts()
            .next()
            .unwrap()
            .content()
            .get_ref()
            .position(),
        0
    );
}

#[test]
fn write_zero_short_interrupted_and_flush_failures_propagate_without_continuing() {
    let expected = wire(&mut nested(), &MimePartWriteOptions::default());
    for limit in 0..expected.len() {
        let mut part = nested();
        let mut sink = Sink {
            fail_after: Some(limit),
            chunk: 3,
            interrupt: true,
            ..Default::default()
        };
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.bytes, expected[..limit]);
        assert_eq!(wire(&mut part, &MimePartWriteOptions::default()), expected);
    }
    assert_eq!(
        nested()
            .write_to(&mut Sink {
                chunk: 0,
                ..Default::default()
            })
            .unwrap_err()
            .kind(),
        io::ErrorKind::WriteZero
    );
    let mut sink = Sink::default();
    nested().write_to(&mut sink).unwrap();
    for flush in 1..=sink.flushes {
        let mut failing = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            nested().write_to(&mut failing).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(failing.flushes, flush);
    }
}

#[test]
fn child_io_failure_stops_before_following_siblings_and_preserves_borrows() {
    for fail_seek in [None, Some(1), Some(2)] {
        let mut first = Source::new(false);
        let mut failing = Source::new(fail_seek.is_none());
        failing.fail_seek = fail_seek;
        let mut last = Source::new(false);
        {
            let mut root = Multipart::new("mixed", "outer").unwrap();
            for source in [&mut first, &mut failing, &mut last] {
                root.children_mut()
                    .push(MimePart::new(MimeContent::new(source)).into());
            }
            let mut output = Vec::new();
            assert_eq!(
                root.write_to(&mut output).unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert!(!output.ends_with(b"--outer--\r\n"));
        }
        assert_eq!(first.inner.position(), 0);
        assert_eq!(first.seeks, 2);
        assert_eq!(failing.seeks, fail_seek.unwrap_or(1));
        assert_eq!(last.seeks, 0);
        assert_eq!(last.inner.position(), 2);
    }
}

#[test]
fn descendant_metadata_errors_leave_earlier_siblings_written_and_later_sources_untouched() {
    let mut invalid_container = container("inner");
    invalid_container.headers_mut().clear();
    invalid_container
        .children_mut()
        .push(leaf(b"untouched", ContentEncoding::Binary));
    let mut invalid_leaf = leaf(b"untouched", ContentEncoding::Binary);
    invalid_leaf
        .headers_mut()
        .replace_all(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap())
        .unwrap();
    for invalid in [invalid_container.into(), invalid_leaf] {
        let mut root = container("outer");
        root.children_mut().extend([
            leaf(b"first", ContentEncoding::Binary),
            invalid,
            leaf(b"last", ContentEncoding::Binary),
        ]);
        let mut sink = Sink::default();
        assert_eq!(
            root.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(sink.bytes.ends_with(b"\r\n\r\nfirst\r\n--outer\r\n"));
        assert_eq!(
            root.leaf_parts()
                .map(|part| part.content().get_ref().position())
                .collect::<Vec<_>>(),
            [0, 2, 2]
        );
    }
}

#[test]
fn large_raw_sections_and_chunked_binary_sources_cross_filter_buffers() {
    let payload = b"binary\0\r\n\xff".repeat(4096);
    let mut source = Source::new(false);
    source.inner = Cursor::new(payload.clone());
    let mut body = MimePart::new(MimeContent::new(source));
    body.set_content_transfer_encoding(ContentEncoding::Binary);
    let mut root = Multipart::new("mixed", "outer").unwrap();
    root.children_mut().push(body.into());
    root.set_preamble(b"a\r\nb\n".repeat(4096));
    root.set_epilogue(b"a\r\nb\n".repeat(4096));
    let mut sink = Sink {
        chunk: 7,
        interrupt: true,
        ..Default::default()
    };
    root.write_to_with_options(
        &mut sink,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    )
    .unwrap();
    let normalized = b"a\nb\n".repeat(4096);
    assert_eq!(sink.bytes, [
        b"Content-Type: multipart/mixed; boundary=outer\n\n".as_slice(), &normalized,
        b"--outer\nContent-Type: application/octet-stream\nContent-Transfer-Encoding: binary\n\n",
        &payload, b"\n--outer--\n", &normalized
    ].concat());
    let source = root.leaf_parts().next().unwrap().content().get_ref();
    assert_eq!(source.inner.position(), 0);
    assert!(source.reads > payload.len() / 3);
}

struct Source {
    inner: Cursor<Vec<u8>>,
    fail: bool,
    seeks: usize,
    reads: usize,
    fail_seek: Option<usize>,
}

impl Source {
    fn new(fail: bool) -> Self {
        let mut inner = Cursor::new(b"body".to_vec());
        inner.set_position(2);
        Self {
            inner,
            fail,
            seeks: 0,
            reads: 0,
            fail_seek: None,
        }
    }
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail {
            return Err(io::Error::other("read failed"));
        }
        let count = bytes.len().min(3);
        self.inner.read(&mut bytes[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("seek failed"));
        }
        self.inner.seek(pos)
    }
}

struct Sink {
    bytes: Vec<u8>,
    calls: usize,
    flushes: usize,
    chunk: usize,
    interrupt: bool,
    fail_after: Option<usize>,
    fail_flush: Option<usize>,
}

impl Default for Sink {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            calls: 0,
            flushes: 0,
            chunk: usize::MAX,
            interrupt: false,
            fail_after: None,
            fail_flush: None,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_after == Some(self.bytes.len()) {
            return Err(io::Error::other("write failed"));
        }
        let remaining = self
            .fail_after
            .map_or(usize::MAX, |limit| limit - self.bytes.len());
        let count = bytes.len().min(self.chunk).min(remaining);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush == Some(self.flushes) {
            return Err(io::Error::other("flush failed"));
        }
        Ok(())
    }
}

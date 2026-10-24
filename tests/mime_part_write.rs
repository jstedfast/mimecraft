// Includes fixtures adapted from MimeKit's UnitTests/MimePartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use mimecraft::{
    ContentDisposition, ContentEncoding, ContentType, Header, HeaderList, HeaderParser,
    MimeContent, MimePart, MimePartWriteOptions,
    encodings::NewLineFormat,
    filters::{MimeFilter, encoder_filter_with_options},
};

const ENCODINGS: [ContentEncoding; 7] = [
    ContentEncoding::Unspecified,
    ContentEncoding::SevenBit,
    ContentEncoding::EightBit,
    ContentEncoding::Binary,
    ContentEncoding::Base64,
    ContentEncoding::QuotedPrintable,
    ContentEncoding::UuEncode,
];

fn stored(bytes: &[u8], encoding: ContentEncoding, format: NewLineFormat) -> Vec<u8> {
    let mut result = Vec::new();
    let newline: &[u8] = if format == NewLineFormat::Dos {
        b"\r\n"
    } else {
        b"\n"
    };
    if encoding == ContentEncoding::UuEncode {
        result.extend_from_slice(b"begin 0644 original.bin");
        result.extend_from_slice(newline);
    }
    let mut encoder = encoder_filter_with_options(encoding, 78, format).unwrap();
    encoder.filter(bytes, &mut result);
    encoder.finish(&mut result);
    if encoding == ContentEncoding::UuEncode {
        result.extend_from_slice(b"end");
        result.extend_from_slice(newline);
    }
    result
}

fn body(output: &[u8], format: NewLineFormat) -> &[u8] {
    let separator: &[u8] = if format == NewLineFormat::Dos {
        b"\r\n\r\n"
    } else {
        b"\n\n"
    };
    let start = output
        .windows(separator.len())
        .position(|bytes| bytes == separator)
        .unwrap();
    &output[start + separator.len()..]
}

fn serialize(bytes: &[u8], encoding: ContentEncoding, options: &MimePartWriteOptions) -> Vec<u8> {
    let mut part = MimePart::new(MimeContent::new(Cursor::new(bytes)));
    part.set_content_transfer_encoding(encoding);
    let mut output = Vec::new();
    part.write_to_with_options(&mut output, options).unwrap();
    output
}

fn raw_headers(bytes: &[u8]) -> HeaderList {
    let mut parser = HeaderParser::new();
    let mut headers = Vec::new();
    assert_eq!(parser.parse(bytes, &mut headers).unwrap(), bytes.len());
    parser.finish(&mut headers).unwrap();
    headers.into()
}

#[test]
fn complete_part_has_exact_headers_separator_and_encoded_body() {
    for (encoding, expected) in [
        (ContentEncoding::Base64, &b"Q2F0\r\n"[..]),
        (ContentEncoding::QuotedPrintable, b"Cat=\r\n"),
        (
            ContentEncoding::UuEncode,
            b"begin 0644 unknown\r\n#0V%T\r\n`\r\nend\r\n",
        ),
        (ContentEncoding::Binary, b"Cat"),
        (ContentEncoding::SevenBit, b"Cat"),
    ] {
        let output = serialize(b"Cat", encoding, &MimePartWriteOptions::default());
        let mut expected_part = format!(
            "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: {}\r\n\r\n",
            encoding.as_str().unwrap()
        )
        .into_bytes();
        expected_part.extend_from_slice(expected);
        assert_eq!(output, expected_part, "{encoding:?}");
    }
}

#[test]
fn every_stored_and_requested_encoding_pair_round_trips_without_mutation() {
    let original = b"opaque\0 bytes\xff and spaces";
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for source in ENCODINGS {
            for target in ENCODINGS {
                let bytes = stored(original, source, NewLineFormat::Unix);
                let mut stream = Cursor::new(bytes.clone());
                stream.set_position(3);
                let mut content = MimeContent::with_encoding(stream, source);
                content.set_new_line_format(Some(NewLineFormat::Mixed));
                let mut part = MimePart::new(content);
                part.set_content_transfer_encoding(target);
                let before = part.headers().clone();
                let options = MimePartWriteOptions {
                    new_line_format: format,
                    ..Default::default()
                };
                let mut output = Vec::new();
                part.write_to_with_options(&mut output, &options).unwrap();
                assert_eq!(part.headers(), &before);
                assert_eq!(part.content().encoding(), source);
                assert_eq!(part.content().get_ref().position(), 0);
                assert_eq!(part.content().get_ref().get_ref(), &bytes);
                assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Mixed));
                let mut decoded = Vec::new();
                MimeContent::with_encoding(Cursor::new(body(&output, format)), target)
                    .decode_to(&mut decoded)
                    .unwrap();
                assert_eq!(decoded, original, "{source:?} -> {target:?}, {format:?}");
                let mut repeated = Vec::new();
                part.write_to_with_options(&mut repeated, &options).unwrap();
                assert_eq!(repeated, output);
                let mut content_only = Vec::new();
                part.write_content_to_with_options(&mut content_only, &options)
                    .unwrap();
                assert_eq!(content_only, body(&output, format));
                assert_eq!(part.headers(), &before);
                assert_eq!(part.content().get_ref().position(), 0);
                assert_eq!(part.content().get_ref().get_ref(), &bytes);
            }
        }
    }
}

#[test]
fn newline_conversion_and_ensure_newline_do_not_change_binary_or_base64_input() {
    for (format, expected) in [
        (NewLineFormat::Dos, &b"a\r\nb\r\nc\rd\r\n"[..]),
        (NewLineFormat::Unix, &b"a\nb\nc\rd\n"[..]),
    ] {
        let options = MimePartWriteOptions {
            new_line_format: format,
            ensure_new_line: true,
            ..Default::default()
        };
        for encoding in [
            ContentEncoding::Unspecified,
            ContentEncoding::SevenBit,
            ContentEncoding::EightBit,
        ] {
            let output = serialize(b"a\nb\r\nc\rd", encoding, &options);
            assert_eq!(body(&output, format), expected);
        }
        let binary = serialize(b"a\nb\r\nc\rd", ContentEncoding::Binary, &options);
        assert_eq!(body(&binary, format), b"a\nb\r\nc\rd");
        let encoded = serialize(b"a\nb\r\nc\rd", ContentEncoding::Base64, &options);
        let mut decoded = Vec::new();
        MimeContent::with_encoding(Cursor::new(body(&encoded, format)), ContentEncoding::Base64)
            .decode_to(&mut decoded)
            .unwrap();
        assert_eq!(decoded, b"a\nb\r\nc\rd");

        for ending in [&b"x"[..], b"x\r", b"x\n", b"x\r\n"] {
            let output = serialize(ending, ContentEncoding::Unspecified, &options);
            assert_eq!(
                body(&output, format),
                if format == NewLineFormat::Dos {
                    &b"x\r\n"[..]
                } else {
                    b"x\n"
                }
            );
        }
    }
}

#[test]
fn matching_encodings_preserve_payload_spelling_without_double_wrappers() {
    for (encoding, input, expected) in [
        (ContentEncoding::Base64, &b"Q 2F0\n"[..], &b"Q 2F0\r\n"[..]),
        (
            ContentEncoding::QuotedPrintable,
            b"=6c=6f=77\n",
            b"=6c=6f=77\r\n",
        ),
        (
            ContentEncoding::UuEncode,
            b"begin 0600 stored\n#0V%T\n`\nend\n",
            b"begin 0600 stored\r\n#0V%T\r\n`\r\nend\r\n",
        ),
        (ContentEncoding::Binary, b"\0\n\r\n\xff", b"\0\n\r\n\xff"),
    ] {
        let mut part = MimePart::new(MimeContent::with_encoding(Cursor::new(input), encoding));
        part.set_content_transfer_encoding(encoding);
        part.headers_mut()
            .add(Header::parse(b"Content-Disposition: ;").unwrap());
        let mut output = Vec::new();
        part.write_to(&mut output).unwrap();
        assert_eq!(body(&output, NewLineFormat::Dos), expected);
        let mut content_only = Vec::new();
        part.write_content_to(&mut content_only).unwrap();
        assert_eq!(content_only, expected);
    }
}

#[test]
fn empty_bodies_obey_newline_option_for_every_encoding_path() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let newline: &[u8] = if format == NewLineFormat::Dos {
            b"\r\n"
        } else {
            b"\n"
        };
        for source in ENCODINGS {
            for target in ENCODINGS {
                for ensure in [false, true] {
                    let mut part = MimePart::new(MimeContent::with_encoding(
                        Cursor::new(Vec::<u8>::new()),
                        source,
                    ));
                    part.set_content_transfer_encoding(target);
                    let options = MimePartWriteOptions {
                        new_line_format: format,
                        ensure_new_line: ensure,
                        ..Default::default()
                    };
                    let mut output = Vec::new();
                    part.write_to_with_options(&mut output, &options).unwrap();
                    let expected = if target == ContentEncoding::UuEncode && source != target {
                        [
                            b"begin 0644 unknown".as_slice(),
                            newline,
                            b"`",
                            newline,
                            b"end",
                            newline,
                        ]
                        .concat()
                    } else if ensure && target != ContentEncoding::Binary {
                        newline.to_vec()
                    } else {
                        Vec::new()
                    };
                    assert_eq!(
                        body(&output, format),
                        expected,
                        "{source:?} -> {target:?}, {format:?}, {ensure}"
                    );
                    let mut content_only = Vec::new();
                    part.write_content_to_with_options(&mut content_only, &options)
                        .unwrap();
                    assert_eq!(content_only, expected);
                }
            }
        }
    }
}

#[test]
fn uu_filename_precedence_fallback_trim_and_utf8() {
    for (disposition, content_name, expected) in [
        (Some("  caf\u{e9}.txt  "), Some("ignored"), "caf\u{e9}.txt"),
        (None, Some(" type.bin "), "type.bin"),
        (None, None, "unknown"),
        (Some(""), Some("ignored"), ""),
    ] {
        let mut content_type = ContentType::new("application", "octet-stream").unwrap();
        content_type.set_name(content_name);
        let mut part =
            MimePart::with_content_type(&content_type, MimeContent::new(Cursor::new(b"Cat")))
                .unwrap();
        if let Some(name) = disposition {
            let mut value = ContentDisposition::default();
            value.set_file_name(Some(name));
            part.set_content_disposition(Some(&value)).unwrap();
        }
        part.set_content_transfer_encoding(ContentEncoding::UuEncode);
        let mut output = Vec::new();
        part.write_to(&mut output).unwrap();
        assert!(
            body(&output, NewLineFormat::Dos)
                .starts_with(format!("begin 0644 {expected}\r\n").as_bytes())
        );
        let mut content_only = Vec::new();
        part.write_content_to(&mut content_only).unwrap();
        assert_eq!(content_only, body(&output, NewLineFormat::Dos));
    }
}

#[test]
fn headers_normalize_without_mutation_and_repair_record_boundaries() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut headers = raw_headers(b"x-Mixed \t: folded\n\tvalue\r\nBroken: \xff\n");
        headers.add(raw_headers(b"X-Truncated: value").into_vec().remove(0));
        headers.add(raw_headers(b"X-CR: value\r").into_vec().remove(0));
        headers.add(Header::parse(b"Last: ok").unwrap());
        let mut part =
            MimePart::from_parts(headers.clone(), MimeContent::new(Cursor::new(b"body")));
        let options = MimePartWriteOptions {
            new_line_format: format,
            ..Default::default()
        };
        let mut output = Vec::new();
        part.write_to_with_options(&mut output, &options).unwrap();
        let expected = b"x-Mixed \t: folded\n\tvalue\nBroken: \xff\nX-Truncated: value\nX-CR: value\nLast: ok\n\nbody";
        let expected = if format == NewLineFormat::Dos {
            expected
                .iter()
                .flat_map(|&b| {
                    if b == b'\n' {
                        vec![b'\r', b'\n']
                    } else {
                        vec![b]
                    }
                })
                .collect()
        } else {
            expected.to_vec()
        };
        assert_eq!(output, expected);
        assert_eq!(part.headers(), &headers);
        let mut content_only = Vec::new();
        part.write_content_to_with_options(&mut content_only, &options)
            .unwrap();
        assert_eq!(content_only, b"body");
        assert_eq!(part.headers(), &headers);
    }
    let mut part = MimePart::from_parts(HeaderList::new(), MimeContent::new(Cursor::new(b"body")));
    let mut output = Vec::new();
    part.write_to(&mut output).unwrap();
    assert_eq!(output, b"\r\nbody");
    let mut content_only = Vec::new();
    part.write_content_to(&mut content_only).unwrap();
    assert_eq!(content_only, b"body");

    let raw = raw_headers(b"\r\r\nX: value\n");
    let mut part = MimePart::from_parts(raw, MimeContent::new(Cursor::new(b"body")));
    let mut output = Vec::new();
    part.write_to_with_options(
        &mut output,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output, b"\r\r\nX: value\n\nbody");
}

#[test]
fn invalid_options_or_required_metadata_fail_before_any_io() {
    let mut part = MimePart::new(MimeContent::new(Source::new(b"body")));
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
        let mut output = Sink::default();
        let error = part
            .write_to_with_options(&mut output, &options)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.get_ref().is_some());
        assert_eq!(
            part.write_content_to_with_options(&mut output, &options)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(output.calls, 0);
        assert_eq!(output.flushes, 0);
        assert_eq!(part.content().get_ref().seeks, 0);
    }
    for cte in ["", "unknown"] {
        part.headers_mut()
            .replace_all(
                Header::parse(format!("Content-Transfer-Encoding: {cte}").as_bytes()).unwrap(),
            )
            .unwrap();
        let mut output = Sink::default();
        assert_eq!(
            part.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            part.write_content_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(output.calls + output.flushes, 0);
        assert_eq!(part.content().get_ref().seeks, 0);
    }
    part.set_content_transfer_encoding(ContentEncoding::UuEncode);
    for value in [
        "Content-Disposition: ;",
        "Content-Disposition: attachment; filename*=utf-8''bad%0D%0Aend",
        "Content-Disposition: attachment; filename*=utf-8''bad%00name",
    ] {
        part.headers_mut()
            .replace_all(Header::parse(value.as_bytes()).unwrap())
            .unwrap();
        let mut output = Sink::default();
        assert_eq!(
            part.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            part.write_content_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(output.calls + output.flushes, 0);
        assert_eq!(part.content().get_ref().seeks, 0);
    }
    part.set_content_disposition(None).unwrap();
    part.headers_mut()
        .replace_all(Header::parse(b"Content-Type: invalid").unwrap())
        .unwrap();
    let mut output = Sink::default();
    assert_eq!(
        part.write_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        part.write_content_to(&mut output).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(output.calls + output.flushes, 0);
}

#[test]
fn requested_encoder_widths_and_chunked_streaming_work_for_large_bodies() {
    let bytes = vec![0xff; 25001];
    for width in [60, 61, 76, 78, 998] {
        for encoding in [ContentEncoding::Base64, ContentEncoding::QuotedPrintable] {
            let mut source = Source::new(&bytes);
            source.chunk = 7;
            source.interrupt = true;
            let mut part = MimePart::new(MimeContent::new(source));
            part.set_content_transfer_encoding(encoding);
            let mut sink = Sink {
                chunk: 3,
                interrupt: true,
                ..Default::default()
            };
            part.write_to_with_options(
                &mut sink,
                &MimePartWriteOptions {
                    max_line_length: width,
                    ..Default::default()
                },
            )
            .unwrap();
            let encoded = body(&sink.bytes, NewLineFormat::Dos);
            for line in encoded.split(|&b| b == b'\n') {
                assert!(line.strip_suffix(b"\r").unwrap_or(line).len() <= width.min(76));
            }
            let mut decoded = Vec::new();
            MimeContent::with_encoding(Cursor::new(encoded), encoding)
                .decode_to(&mut decoded)
                .unwrap();
            assert_eq!(decoded, bytes);
            assert_eq!(part.content().get_ref().inner.position(), 0);
            assert!(sink.flushes > 0);
            let mut content_only = Sink {
                chunk: 3,
                interrupt: true,
                ..Default::default()
            };
            part.write_content_to_with_options(
                &mut content_only,
                &MimePartWriteOptions {
                    max_line_length: width,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(content_only.bytes, encoded);
            assert_eq!(part.content().get_ref().inner.position(), 0);
        }
    }
}

#[test]
fn io_failures_propagate_and_retry_restarts_the_whole_part() {
    for encoding in [
        ContentEncoding::Unspecified,
        ContentEncoding::Binary,
        ContentEncoding::Base64,
        ContentEncoding::UuEncode,
    ] {
        let expected = serialize(b"body bytes", encoding, &MimePartWriteOptions::default());
        for limit in 0..expected.len() {
            let mut part = MimePart::new(MimeContent::new(Source::new(b"body bytes")));
            part.set_content_transfer_encoding(encoding);
            let mut sink = Sink {
                fail_after: Some(limit),
                ..Default::default()
            };
            assert_eq!(
                part.write_to(&mut sink).unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert_eq!(sink.bytes, expected[..limit]);
            let mut retry = Vec::new();
            part.write_to(&mut retry).unwrap();
            assert_eq!(retry, expected);
        }
    }
    for fail_seek in [1, 2] {
        let mut source = Source::new(b"body bytes");
        source.fail_seek = Some(fail_seek);
        let mut part = MimePart::new(MimeContent::new(source));
        part.set_content_transfer_encoding(ContentEncoding::Base64);
        assert_eq!(
            part.write_to(&mut Vec::new()).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(part.content().get_ref().seeks, fail_seek);
    }
    let mut source = Source::new(b"body bytes");
    source.fail_read = true;
    let mut part = MimePart::new(MimeContent::new(source));
    assert_eq!(
        part.write_to(&mut Vec::new()).unwrap_err().kind(),
        io::ErrorKind::Other
    );
    assert_eq!(part.content().get_ref().seeks, 1);

    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"body bytes")));
    assert_eq!(
        part.write_to(&mut Sink {
            chunk: 0,
            ..Default::default()
        })
        .unwrap_err()
        .kind(),
        io::ErrorKind::WriteZero
    );
    let mut successful = Sink::default();
    part.write_to(&mut successful).unwrap();
    for flush in 1..=successful.flushes {
        let mut sink = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            part.write_to(&mut sink).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(sink.flushes, flush);
    }
}

#[test]
fn content_only_io_failures_propagate_and_retry_restarts_the_body() {
    for encoding in ENCODINGS {
        let full = serialize(b"body bytes", encoding, &MimePartWriteOptions::default());
        let expected = body(&full, NewLineFormat::Dos);
        for limit in 0..expected.len() {
            let mut part = MimePart::new(MimeContent::new(Source::new(b"body bytes")));
            part.set_content_transfer_encoding(encoding);
            let mut sink = Sink {
                fail_after: Some(limit),
                chunk: 3,
                interrupt: true,
                ..Default::default()
            };
            assert_eq!(
                part.write_content_to(&mut sink).unwrap_err().kind(),
                io::ErrorKind::Other
            );
            assert_eq!(sink.bytes, expected[..limit]);
            let mut retry = Vec::new();
            part.write_content_to(&mut retry as &mut dyn Write).unwrap();
            assert_eq!(retry, expected);
            assert_eq!(part.content().get_ref().inner.position(), 0);
        }
    }
    for fail_seek in [1, 2] {
        let mut source = Source::new(b"body");
        source.fail_seek = Some(fail_seek);
        let mut part = MimePart::new(MimeContent::new(source));
        assert_eq!(
            part.write_content_to(&mut Vec::new()).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(part.content().get_ref().seeks, fail_seek);
    }
    let mut source = Source::new(b"body");
    source.fail_read = true;
    let mut part = MimePart::new(MimeContent::new(source));
    assert_eq!(
        part.write_content_to(&mut Vec::new()).unwrap_err().kind(),
        io::ErrorKind::Other
    );
    assert_eq!(part.content().get_ref().seeks, 1);

    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"body")));
    assert_eq!(
        part.write_content_to(&mut Sink {
            chunk: 0,
            ..Default::default()
        })
        .unwrap_err()
        .kind(),
        io::ErrorKind::WriteZero
    );
    let mut sink = Sink::default();
    part.write_content_to(&mut sink).unwrap();
    for flush in 1..=sink.flushes {
        let mut failing = Sink {
            fail_flush: Some(flush),
            ..Default::default()
        };
        assert_eq!(
            part.write_content_to(&mut failing).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(failing.flushes, flush);
    }
}

#[test]
fn content_only_uu_and_binary_output_match_upstream_fixtures() {
    // MimeKit's content-only UU fixture includes the wrapper lines, not headers.
    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"abc")));
    part.set_content_transfer_encoding(ContentEncoding::UuEncode);
    let mut disposition = ContentDisposition::default();
    disposition.set_file_name(Some("file.txt"));
    part.set_content_disposition(Some(&disposition)).unwrap();
    let mut output = Vec::new();
    part.write_content_to_with_options(
        &mut output,
        &MimePartWriteOptions {
            new_line_format: NewLineFormat::Unix,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output, b"begin 0644 file.txt\n#86)C\n`\nend\n");

    let bytes = b"a\rb\nc";
    let mut part = MimePart::new(MimeContent::with_encoding(
        Cursor::new(bytes),
        ContentEncoding::Binary,
    ));
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    let mut output = Vec::new();
    part.write_content_to_with_options(
        &mut output,
        &MimePartWriteOptions {
            ensure_new_line: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(output, bytes);
}

struct Source {
    inner: Cursor<Vec<u8>>,
    chunk: usize,
    interrupt: bool,
    seeks: usize,
    fail_seek: Option<usize>,
    fail_read: bool,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            inner: Cursor::new(bytes.to_vec()),
            chunk: usize::MAX,
            interrupt: false,
            seeks: 0,
            fail_seek: None,
            fail_read: false,
        }
    }
}

impl Read for Source {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_read {
            return Err(io::Error::other("source read failed"));
        }
        let count = buffer.len().min(self.chunk);
        self.inner.read(&mut buffer[..count])
    }
}

impl Seek for Source {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::other("source seek failed"));
        }
        self.inner.seek(position)
    }
}

struct Sink {
    bytes: Vec<u8>,
    chunk: usize,
    interrupt: bool,
    calls: usize,
    flushes: usize,
    fail_after: Option<usize>,
    fail_flush: Option<usize>,
}

impl Default for Sink {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            chunk: usize::MAX,
            interrupt: false,
            calls: 0,
            flushes: 0,
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
            return Err(io::Error::other("sink write failed"));
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
            return Err(io::Error::other("sink flush failed"));
        }
        Ok(())
    }
}

// Adapted from MimeKit's MimeParserTests.cs; excludes LegacyMimeParser.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    HeaderTooLong, MimeContentStorage, MimeEntity, MimeEntityParseOptions, MimeEvent, MimeMessage,
    MimeParser, MimePartWriteOptions, MimePosition, MimeReader, TextFormat,
    encodings::NewLineFormat,
};
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom};

#[derive(Debug)]
struct ShortSource<R> {
    inner: R,
    chunk: usize,
}

impl<R: Read> Read for ShortSource<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.chunk);
        self.inner.read(&mut bytes[..count])
    }
}

impl<R: Seek> Seek for ShortSource<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.inner.seek(from)
    }
}

fn assert_storage_round_trip(
    input: &[u8],
    expected: &[u8],
    mbox: bool,
    chunk: usize,
    options: MimePartWriteOptions,
) {
    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
        let mut parser = MimeParser::with_storage_and_options(
            ShortSource {
                inner: Cursor::new(input),
                chunk,
            },
            storage,
            MimeEntityParseOptions {
                max_header_length: 16 * 1024 * 1024,
                ..Default::default()
            },
        )
        .unwrap();
        let mut message = if mbox {
            parser.parse_mbox_message().unwrap().unwrap()
        } else {
            parser.parse_message().unwrap()
        };
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position().offset, input.len() as u64);
        drop(parser);
        let mut output = Vec::new();
        if mbox {
            output.extend_from_slice(&input[..=input.iter().position(|&b| b == b'\n').unwrap()]);
        }
        message
            .write_to_with_options(&mut output, &options)
            .unwrap();
        assert_round_trip(&format!("{storage:?}, chunk {chunk}"), expected, &output);
    }
}

const DELIMITER_NEW_LINE_HEADERS: &str = "From: mimekit@example.com\nTo: mimekit@example.com\nSubject: delimiter new-line test\nMIME-Version: 1.0\n";

const DELIMITER_NEW_LINE_MESSAGES: &[&str] = &[
    // A nested multipart that is truncated by the outer boundary.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n\n--inner\nContent-Type: text/plain\n\ninner\n--outer\nContent-Type: text/plain\n\nlast\n--outer--\n",
    // A nested multipart that is truncated by the outer boundary and whose last child has no body.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n\n--inner\nContent-Type: text/plain\n--outer\nContent-Type: text/plain\n\nlast\n--outer--\n",
    // A nested multipart that is truncated by the outer boundary right after a boundary marker.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n\n--inner\nContent-Type: text/plain\n\ninner\n--inner\n--outer\nContent-Type: text/plain\n\nlast\n--outer--\n",
    // A nested multipart with only a preamble that is truncated by the outer boundary.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n\npreamble\n--outer\nContent-Type: text/plain\n\nlast\n--outer--\n",
    // A nested multipart with neither a preamble nor any children that is truncated by the outer boundary.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n--outer\nContent-Type: text/plain\n\nlast\n--outer--\n",
    // A nested message/rfc822 whose multipart body is truncated by the outer boundary.
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: message/rfc822\n\nSubject: embedded\nContent-Type: multipart/alternative; boundary=\"inner\"\n\n--inner\nContent-Type: text/plain\n\ninner\n--outer--\n",
    // Children without a body (with and without a blank line after the headers).
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n--b\nContent-Type: text/plain\n\n--b\nContent-Type: message/rfc822\n\n--b--\n",
    // A double boundary.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\n--b\nContent-Type: text/plain\n\nbody\n--b--\n",
    // A multipart truncated by the end of the stream.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody",
    // A multipart truncated by the end of the stream right after a boundary marker.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b",
    // A multipart truncated by the end of the stream whose last child has no body.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b\nContent-Type: text/plain\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b\nContent-Type: text/plain\n\n",
    // A multipart truncated by the end of the stream with only a preamble.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\npreamble\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\npreamble",
    // A close delimiter at EOF, with and without an epilogue or a trailing newline.
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b--",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b--\n\n",
    "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\nContent-Type: text/plain\n\nbody\n--b--\nepilogue",
    "Content-Type: multipart/mixed; boundary=\"outer\"\n\n--outer\nContent-Type: multipart/alternative; boundary=\"inner\"\n\n--inner\nContent-Type: text/plain\n\ninner\n--inner--\n--outer--",
];

fn assert_round_trip(name: &str, expected: &[u8], actual: &[u8]) {
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    if offset == expected.len() && offset == actual.len() {
        return;
    }
    let start = offset.saturating_sub(64);
    panic!(
        "{name}: serialization differs at offset {offset} (expected length {}, actual length {})\nexpected: {:?}\nactual: {:?}",
        expected.len(),
        actual.len(),
        String::from_utf8_lossy(&expected[start..expected.len().min(start + 128)]),
        String::from_utf8_lossy(&actual[start..actual.len().min(start + 128)]),
    );
}

#[test]
fn delimiter_new_line_round_trip() {
    for (index, body) in DELIMITER_NEW_LINE_MESSAGES.iter().enumerate() {
        for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
            let text = format!("{DELIMITER_NEW_LINE_HEADERS}{body}").replace('\n', newline);
            let ensured = if text.ends_with('\n') {
                text.clone()
            } else {
                format!("{text}{newline}")
            };
            for capacity in [1, 7, 4096] {
                let mut parser =
                    MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
                let mut message = parser.parse_message().unwrap();
                assert!(parser.is_end_of_stream());
                assert_eq!(parser.position().offset, text.len() as u64);
                // Make sure that EnsureNewLine did not leave any state behind.
                for ensure_new_line in [false, true, false] {
                    let mut output = Vec::new();
                    message
                        .write_to_with_options(
                            &mut output,
                            &MimePartWriteOptions {
                                new_line_format: format,
                                ensure_new_line,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    let name = format!(
                        "delimiter case {index}, {format:?}, capacity {capacity}, ensure {ensure_new_line}"
                    );
                    assert_round_trip(
                        &name,
                        if ensure_new_line {
                            ensured.as_bytes()
                        } else {
                            text.as_bytes()
                        },
                        &output,
                    );
                    assert_storage_round_trip(
                        text.as_bytes(),
                        if ensure_new_line {
                            ensured.as_bytes()
                        } else {
                            text.as_bytes()
                        },
                        false,
                        capacity,
                        MimePartWriteOptions {
                            new_line_format: format,
                            ensure_new_line,
                            ..Default::default()
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn round_trip_test_data_messages() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data");
    let mut failures = Vec::new();
    for (directory, extensions, count) in [
        ("messages", &["eml", "txt"][..], 33),
        ("compliance", &["eml"][..], 23),
        ("partial", &["eml"][..], 7),
        ("openpgp", &["eml"][..], 1),
        ("dkim", &["msg"][..], 4),
        ("smime", &["txt"][..], 1),
    ] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(directory))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| extensions.contains(&ext))
                    && (directory != "smime"
                        || path
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .ends_with("-signed.txt"))
            })
            .collect();
        files.sort();
        assert_eq!(
            files.len(),
            count,
            "complete upstream {directory} round-trip corpus"
        );
        for path in files {
            let data = std::fs::read(&path).unwrap();
            let format = data
                .iter()
                .take(1024)
                .position(|&byte| byte == b'\n')
                .map_or(NewLineFormat::Dos, |index| {
                    if index > 0 && data[index - 1] == b'\r' {
                        NewLineFormat::Dos
                    } else {
                        NewLineFormat::Unix
                    }
                });
            // The parser skips a leading UTF-8 BOM; it is not reserialized.
            let expected = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
            // Files starting with an mbox From-line must use mbox parsing to
            // retain their envelope marker. Rust exposes that marker via reader
            // events rather than storing it on the message model.
            let mbox = expected.starts_with(b"From ");
            for capacity in [4096, 7, 1] {
                let mut parser = MimeParser::with_options(
                    BufReader::with_capacity(capacity, expected),
                    MimeEntityParseOptions {
                        max_header_length: 16 * 1024 * 1024,
                        ..Default::default()
                    },
                )
                .unwrap();
                let parsed = if mbox {
                    parser
                        .parse_mbox_message()
                        .map(|message| message.expect("one mbox message"))
                } else {
                    parser.parse_message()
                };
                let label = format!("{} / {capacity}", path.display());
                let mut message = match parsed {
                    Ok(message) => message,
                    Err(error) => {
                        failures.push(format!("{label}: parse: {error}"));
                        break;
                    }
                };
                assert!(
                    parser.is_end_of_stream(),
                    "{label}: expected exactly one message"
                );
                assert_eq!(parser.position().offset, expected.len() as u64, "{label}");
                let mut output = Vec::new();
                if mbox {
                    let mut reader = MimeReader::new(expected);
                    reader
                        .read_mbox_message(|event| {
                            if let MimeEvent::MboxMarker { bytes, .. } = event {
                                output.extend_from_slice(bytes);
                            }
                            Ok(())
                        })
                        .unwrap();
                }
                if let Err(error) = message.write_to_with_options(
                    &mut output,
                    &MimePartWriteOptions {
                        new_line_format: format,
                        ..Default::default()
                    },
                ) {
                    failures.push(format!("{label}: write: {error}"));
                    break;
                }
                assert_storage_round_trip(
                    expected,
                    expected,
                    mbox,
                    capacity,
                    MimePartWriteOptions {
                        new_line_format: format,
                        ..Default::default()
                    },
                );
                if output != expected {
                    let offset = expected
                        .iter()
                        .zip(&output)
                        .position(|(a, b)| a != b)
                        .unwrap_or(expected.len().min(output.len()));
                    let start = offset.saturating_sub(32);
                    failures.push(format!(
                        "{label}: offset {offset}, lengths {}/{}\nexpected {:?}\nactual {:?}",
                        expected.len(),
                        output.len(),
                        String::from_utf8_lossy(&expected[start..expected.len().min(offset + 64)]),
                        String::from_utf8_lossy(&output[start..output.len().min(offset + 64)])
                    ));
                    break;
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn parsed_markers_retain_padding_and_original_newlines_with_or_without_logging() {
    let input = b"Content-Type: multipart/mixed; boundary=b\n\n--b \t\nContent-Type: text/plain\n\nbody\n--b--\t \r\n";
    for capacity in [1, 7, 4096] {
        for logged in [false, true] {
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, input.as_slice()));
            let mut entity = if logged {
                parser.parse_entity_with_logger(&mut |_| Ok(())).unwrap()
            } else {
                parser.parse_entity().unwrap()
            };
            for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
                let expected = format!(
                    "Content-Type: multipart/mixed; boundary=b{newline}{newline}--b \t\nContent-Type: text/plain{newline}{newline}body{newline}--b--\t \r\n"
                );
                for ensure_new_line in [false, true, false] {
                    let mut output = Vec::new();
                    entity
                        .write_to_with_options(
                            &mut output,
                            &MimePartWriteOptions {
                                new_line_format: format,
                                ensure_new_line,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    assert_round_trip("raw marker endings", expected.as_bytes(), &output);
                }
            }
        }
    }
}

#[test]
fn truncated_multipart_does_not_append_newlines_to_binary_content() {
    for body in [b"binary\0".as_slice(), b"binary\r", b"binary\n"] {
        let input = [
            b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Transfer-Encoding: binary\r\n\r\n".as_slice(),
            body,
        ].concat();
        let mut entity = MimeParser::new(input.as_slice()).parse_entity().unwrap();
        for ensure_new_line in [false, true, false] {
            let mut output = Vec::new();
            entity
                .write_to_with_options(
                    &mut output,
                    &MimePartWriteOptions {
                        ensure_new_line,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_round_trip("truncated binary tail", &input, &output);
        }
    }
}

#[test]
fn recovered_metadata_is_only_reused_while_unchanged() {
    use mimecraft::Header;

    for (input, replacement) in [
        (
            "Content-Transfer-Encoding: broken\r\n\r\nbody",
            "Content-Transfer-Encoding: different",
        ),
        (
            "Content-Type: multipart/mixed\r\n\r\npreamble",
            "Content-Type: multipart/alternative",
        ),
        (
            "Content-Type: multipart/mixed; boundary=b\r\nContent-Transfer-Encoding: broken\r\n\r\n--b--\r\n",
            "Content-Transfer-Encoding: different",
        ),
    ] {
        let mut entity = MimeParser::new(input.as_bytes()).parse_entity().unwrap();
        let mut output = Vec::new();
        entity.write_to(&mut output).unwrap();
        assert_round_trip("recovered metadata", input.as_bytes(), &output);
        entity
            .headers_mut()
            .replace_all(Header::parse(replacement.as_bytes()).unwrap())
            .unwrap();
        assert_eq!(
            entity.write_to(&mut Vec::new()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn editing_loaded_bodies_and_eof_headers_produces_distinct_records_and_separator() {
    use mimecraft::{Header, MimeContent};
    use std::io::Cursor;

    let mut entity = MimeParser::new(b"Content-Type: text/plain".as_slice())
        .parse_entity()
        .unwrap();
    let MimeEntity::Part(part) = &mut entity else {
        panic!()
    };
    part.set_content(MimeContent::new(Cursor::new(b"new body".to_vec())));
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert_eq!(output, b"Content-Type: text/plain\r\n\r\nnew body");

    let mut entity = MimeParser::new(b"Content-Type: text/plain".as_slice())
        .parse_entity()
        .unwrap();
    entity
        .headers_mut()
        .add(Header::parse(b"X-Added: value").unwrap());
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert_eq!(output, b"Content-Type: text/plain\r\nX-Added: value\r\n");
}

#[test]
fn editing_loaded_multipart_structure_discards_stale_markers() {
    use mimecraft::TextPart;

    for edit_boundary in [false, true] {
        let input = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b \t\r\nContent-Type: text/plain\r\n\r\nbody";
        let mut entity = MimeParser::new(input.as_slice()).parse_entity().unwrap();
        let MimeEntity::Multipart(part) = &mut entity else {
            panic!()
        };
        let boundary = if edit_boundary {
            part.set_boundary("changed").unwrap();
            "changed"
        } else {
            part.children_mut()
                .push(TextPart::new("plain", "added").unwrap().into());
            "b"
        };
        let mut output = Vec::new();
        entity.write_to(&mut output).unwrap();
        assert!(
            !output
                .windows(b"--b \t".len())
                .any(|bytes| bytes == b"--b \t")
        );
        assert!(output.ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
        let MimeEntity::Multipart(reparsed) =
            MimeParser::new(output.as_slice()).parse_entity().unwrap()
        else {
            panic!()
        };
        assert_eq!(reparsed.children().len(), if edit_boundary { 1 } else { 2 });
    }
    let mut entity = MimeParser::new(b"Content-Type: multipart/mixed; boundary=b".as_slice())
        .parse_entity()
        .unwrap();
    let MimeEntity::Multipart(part) = &mut entity else {
        panic!()
    };
    part.set_preamble(b"new preamble".to_vec());
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert_eq!(
        output,
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\nnew preamble\r\n--b--\r\n"
    );
    let MimeEntity::Multipart(part) = &mut entity else {
        panic!()
    };
    part.set_epilogue(b"new epilogue".to_vec());
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert!(output.ends_with(b"new preamble\r\n--b--\r\nnew epilogue"));
}

#[test]
fn editing_recovered_message_requires_a_wrapper_separator() {
    let mut entity = MimeParser::new(b"Content-Type: message/rfc822".as_slice())
        .parse_entity()
        .unwrap();
    let MimeEntity::Message(part) = &mut entity else {
        panic!()
    };
    part.message_mut().unwrap().set_subject("added").unwrap();
    let mut output = Vec::new();
    entity.write_to(&mut output).unwrap();
    assert_eq!(
        output,
        b"Content-Type: message/rfc822\r\n\r\nSubject: added\r\n"
    );
    let MimeEntity::Message(part) = MimeParser::new(output.as_slice()).parse_entity().unwrap()
    else {
        panic!()
    };
    assert!(part.message().unwrap().headers().contains("Subject"));
}

#[test]
fn absent_parsed_content_does_not_gain_encoder_output() {
    use mimecraft::ContentEncoding;

    for encoding in [
        ContentEncoding::Base64,
        ContentEncoding::QuotedPrintable,
        ContentEncoding::UuEncode,
    ] {
        let mut entity = MimeParser::new(b"Content-Type: text/plain\r\n\r\n".as_slice())
            .parse_entity()
            .unwrap();
        let MimeEntity::Part(part) = &mut entity else {
            panic!()
        };
        part.set_content_transfer_encoding(encoding);
        for ensure_new_line in [false, true, false] {
            let options = MimePartWriteOptions {
                ensure_new_line,
                ..Default::default()
            };
            let mut output = Vec::new();
            entity.write_to_with_options(&mut output, &options).unwrap();
            assert!(output.ends_with(b"\r\n\r\n"));
            assert_eq!(
                output.windows(4).position(|bytes| bytes == b"\r\n\r\n"),
                Some(output.len() - 4)
            );
            let mut content = Vec::new();
            entity
                .write_content_to_with_options(&mut content, &options)
                .unwrap();
            assert!(content.is_empty());
        }
    }
}

fn create_message_with_huge_folded_header(lines: usize) -> (String, String, u64) {
    let mut value = String::from(" participant0@example.com");
    for index in 1..lines {
        value.push_str(&format!(",\r\n participant{index}@example.com"));
    }
    value.push_str("\r\n");
    let mut data = format!("From: sender@example.com\r\nTo:{value}");
    let subject_offset = data.len() as u64;
    data.push_str("Subject: huge header\r\n\r\nThis is the body.\r\n");
    (data, value, subject_offset)
}

#[test]
fn huge_folded_header() {
    for lines in [1, 100, 50_000] {
        let (data, value, subject_offset) = create_message_with_huge_folded_header(lines);
        let options = MimeEntityParseOptions {
            max_header_length: 16 * 1024 * 1024,
            ..Default::default()
        };
        for chunk in [7, 4096] {
            let mut parser =
                MimeParser::with_options(BufReader::with_capacity(chunk, data.as_bytes()), options)
                    .unwrap();
            let message = parser.parse_message().unwrap();
            assert!(parser.is_end_of_stream());
            drop(parser);
            assert_huge_folded_header(message, &data, &value, subject_offset, lines);
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let mut parser = MimeParser::with_storage_and_options(
                    ShortSource {
                        inner: Cursor::new(data.as_bytes()),
                        chunk,
                    },
                    storage,
                    options,
                )
                .unwrap();
                let message = parser.parse_message().unwrap();
                assert!(parser.is_end_of_stream());
                drop(parser);
                assert_huge_folded_header(message, &data, &value, subject_offset, lines);
            }
        }
    }
}

fn assert_huge_folded_header<R: Read + Seek>(
    mut message: MimeMessage<R>,
    data: &str,
    value: &str,
    subject_offset: u64,
    lines: usize,
) {
    assert_eq!(message.headers().len(), 3);
    assert_eq!(
        message.headers().get("To").unwrap().raw_value(),
        value.as_bytes()
    );
    assert_eq!(
        message.headers().get("Subject").unwrap().offset(),
        Some(subject_offset)
    );
    assert_eq!(message.subject().unwrap().text, "huge header");
    let to = message.to();
    assert!(to.issues.is_empty());
    assert_eq!(to.addresses.len(), lines);
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_round_trip("huge folded header", data.as_bytes(), &output);
    // The input has no Content-Type. Unlike MimeKit, the raw model does not
    // supply text/plain, so inspect its body without assuming a TextPart.
    let Some(MimeEntity::Part(part)) = message.body_mut() else {
        panic!("expected leaf")
    };
    let mut body = Vec::new();
    part.content_mut().decode_to(&mut body).unwrap();
    assert_eq!(body, b"This is the body.\r\n");
}

#[test]
fn max_header_length_boundary() {
    let (data, value, subject_offset) = create_message_with_huge_folded_header(100);
    let length = b"To:".len() + value.len();
    // A header that is exactly max_header_length bytes long is accepted.
    let options = MimeEntityParseOptions {
        max_header_length: length,
        ..Default::default()
    };
    for chunk in [1, 7, 4096] {
        let mut parser =
            MimeParser::with_options(BufReader::with_capacity(chunk, data.as_bytes()), options)
                .unwrap();
        assert_huge_folded_header(
            parser.parse_message().unwrap(),
            &data,
            &value,
            subject_offset,
            100,
        );
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser = MimeParser::with_storage_and_options(
                ShortSource {
                    inner: Cursor::new(data.as_bytes()),
                    chunk,
                },
                storage,
                options,
            )
            .unwrap();
            assert_huge_folded_header(
                parser.parse_message().unwrap(),
                &data,
                &value,
                subject_offset,
                100,
            );
        }
    }
    // A header that is one byte longer than max_header_length is rejected.
    assert_header_length_failure(data.as_bytes(), length - 1, 26, false);
}

#[test]
fn max_header_length_exceeded() {
    let (data, _, _) = create_message_with_huge_folded_header(1000);
    assert_eq!(
        MimeEntityParseOptions::default().max_header_length,
        16 * 1024
    );
    assert_header_length_failure(data.as_bytes(), 16 * 1024, 26, true);
}

#[test]
fn max_header_length_shorter_than_field_name() {
    let data = b"From: sender@example.com\r\nSubject: hi\r\n\r\nThis is the body.\r\n";
    assert_header_length_failure(data, 4, 0, false);
}

fn assert_excessive_header_length(error: io::Error, offset: u64, limit: usize) {
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let cause = error
        .get_ref()
        .unwrap()
        .downcast_ref::<HeaderTooLong>()
        .unwrap();
    assert_eq!(cause.offset(), offset);
    assert_eq!(cause.limit(), limit);
}

fn assert_header_length_failure(data: &[u8], limit: usize, offset: u64, stop_early: bool) {
    let options = MimeEntityParseOptions {
        max_header_length: limit,
        ..Default::default()
    };
    for chunk in [1, 7, 4096] {
        let mut parser =
            MimeParser::with_options(BufReader::with_capacity(chunk, Cursor::new(data)), options)
                .unwrap();
        assert_excessive_header_length(parser.parse_message().unwrap_err(), offset, limit);
        if stop_early {
            // The parser should stop reading shortly after exceeding the limit.
            assert!(parser.get_ref().get_ref().position() < data.len() as u64);
        }
        assert_eq!(
            parser.parse_message().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser = MimeParser::with_storage_and_options(
                ShortSource {
                    inner: Cursor::new(data),
                    chunk,
                },
                storage,
                options,
            )
            .unwrap();
            assert_excessive_header_length(parser.parse_message().unwrap_err(), offset, limit);
            if stop_early {
                assert!(parser.get_ref().get_ref().position() < data.len() as u64);
            }
            assert_eq!(
                parser.parse_message().unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}

#[test]
fn header_limit_offsets_include_enclosing_framing() {
    let options = MimeEntityParseOptions {
        max_header_length: 64,
        ..Default::default()
    };
    for (prefix, mbox) in [
        (
            b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n".as_slice(),
            false,
        ),
        (
            b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n",
            false,
        ),
        (b"\xef\xbb\xbfFrom sender\r\nSubject: outer\r\n", true),
    ] {
        let data = [prefix, b"X-Long: ", &[b'x'; 65], b"\r\n\r\nbody"].concat();
        for chunk in [1, 7, 4096] {
            let mut parser =
                MimeParser::with_options(BufReader::with_capacity(chunk, data.as_slice()), options)
                    .unwrap();
            let error = if mbox {
                parser.parse_mbox_message().unwrap_err()
            } else {
                parser.parse_message().unwrap_err()
            };
            assert_excessive_header_length(error, prefix.len() as u64, 64);
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let mut source = Cursor::new([b"skip".as_slice(), &data].concat());
                source.set_position(4);
                let mut parser = MimeParser::with_storage_and_options(
                    ShortSource {
                        inner: source,
                        chunk,
                    },
                    storage,
                    options,
                )
                .unwrap();
                let error = if mbox {
                    parser.parse_mbox_message().unwrap_err()
                } else {
                    parser.parse_message().unwrap_err()
                };
                assert_excessive_header_length(error, prefix.len() as u64, 64);
            }
        }
    }
}

#[test]
fn japanese_message() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("messages")
        .join("japanese.txt");
    let data = std::fs::read(path).unwrap();
    for chunk in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(chunk, data.as_slice()));
        assert_japanese_message(parser.parse_message().unwrap());
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser = MimeParser::with_storage(
                ShortSource {
                    inner: Cursor::new(data.as_slice()),
                    chunk,
                },
                storage,
            )
            .unwrap();
            let message = parser.parse_message().unwrap();
            drop(parser);
            assert_japanese_message(message);
        }
    }
}

fn assert_japanese_message<R: Read + Seek>(mut message: MimeMessage<R>) {
    assert_eq!(
        message.subject().unwrap().text,
        "\u{65e5}\u{672c}\u{8a9e}\u{30e1}\u{30fc}\u{30eb}\u{30c6}\u{30b9}\u{30c8} (testing Japanese emails)",
    );
    let expected = concat!(
        "Let's see if both subject and body works fine...\n\n",
        "\u{65e5}\u{672c}\u{8a9e}\u{304c}\n",
        "\u{6b63}\u{5e38}\u{306b}\n",
        "\u{9001}\u{308c}\u{3066}\u{3044}\u{308b}\u{304b}\n",
        "\u{30c6}\u{30b9}\u{30c8}.\n",
    );
    assert_eq!(
        message
            .text_body(TextFormat::Plain)
            .unwrap()
            .unwrap()
            .replace("\r\n", "\n"),
        expected,
    );
}

#[test]
fn unmunged_from_lines() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("mbox")
        .join("unmunged.mbox.txt");
    let original = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    for data in [original.clone(), original.replace('\n', "\r\n")] {
        for chunk in [1, 7, 4096] {
            let mut reader = MimeReader::new(BufReader::with_capacity(chunk, data.as_bytes()));
            let mut markers = Vec::new();
            while reader
                .read_mbox_message(|event| {
                    if let MimeEvent::MboxMarker {
                        bytes,
                        position,
                        end,
                    } = event
                    {
                        assert_eq!(
                            &data.as_bytes()[position.offset as usize..end.offset as usize],
                            bytes
                        );
                        markers.push(bytes.trim_ascii_end().to_vec());
                    }
                    Ok(())
                })
                .unwrap()
            {}
            assert_eq!(
                markers,
                [
                    b"From -".as_slice(),
                    b"From Russia with love",
                    b"From -",
                    b"From Russia with love",
                ]
            );
            assert_unmunged_mbox(MimeParser::new(BufReader::with_capacity(
                chunk,
                data.as_bytes(),
            )));
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let parser = MimeParser::with_storage(
                    ShortSource {
                        inner: Cursor::new(data.as_bytes()),
                        chunk,
                    },
                    storage,
                )
                .unwrap();
                assert_unmunged_mbox(parser);
            }
        }
    }
}

fn assert_unmunged_mbox<R: BufRead, C: Read + Seek>(mut parser: MimeParser<R, C>) {
    let mut count = 0;
    while let Some(mut message) = parser.parse_mbox_message().unwrap() {
        let expected_subject = match count {
            0 => Some("This is a test message with an unmunged From line"),
            2 => Some("Another test message with an unmunged From line"),
            _ => None,
        };
        assert_eq!(
            message.subject().as_ref().map(|value| value.text.as_str()),
            expected_subject
        );
        // Force the parts to write their content streams. Seeking in their
        // content checks that the parser correctly resumes on its shared source.
        message.write_to(&mut io::sink()).unwrap();
        count += 1;
    }
    assert_eq!(count, 4);
    assert!(parser.is_end_of_stream());
}

#[test]
fn issue991() {
    assert_refill_mbox(false);
}

#[test]
fn mbox_with_lines_exceeding_max_smtp_line_length() {
    assert_refill_mbox(true);
}

fn create_refill_mbox(long_lines: bool) -> (Vec<u8>, [usize; 2]) {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const MARKER: &[u8] = b"From -\n";
    let headers = |index| {
        format!(
            "From: mimekit@example.org\r\nTo: mimekit@example.org\r\n\
         Subject: This message contains long lines that will cause ScanContent() to require a buffer refill\r\n\
         Message-Id: <1234567890.{index}@example.org>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n"
        )
    };
    let mut wire = MARKER.to_vec();
    wire.extend_from_slice(headers(0).as_bytes());
    let mut index = 0;
    let first_limit = if long_lines { 4096 - 1001 } else { 4096 };
    while wire.len() <= first_limit {
        let byte = ALPHABET[index % ALPHABET.len()];
        for _ in 0..255 {
            if wire.len() >= first_limit {
                break;
            }
            wire.push(byte);
        }
        if long_lines || wire.len() < 4096 {
            wire.push(b'\n');
        } else {
            // Fake mbox marker that is midline.
            wire.extend_from_slice(MARKER);
        }
        index += 1;
    }
    if long_lines {
        // Write a line exceeding the SMTP maximum that also spans read boundaries.
        let byte = ALPHABET[index % ALPHABET.len()];
        wire.resize(4096, byte);
        wire.extend_from_slice(MARKER);
        index += 1;
        let byte = ALPHABET[index % ALPHABET.len()];
        wire.resize(wire.len() + 255, byte);
        wire.extend_from_slice(MARKER);
        index += 1;
    }
    wire.push(b'\n');
    let first_end = wire.len();
    assert_eq!(&wire[4096..4103], MARKER);
    assert_ne!(wire[4095], b'\n');
    if long_lines {
        assert!(
            wire.split(|&byte| byte == b'\n')
                .any(|line| line.len() > 1000)
        );
    }

    wire.extend_from_slice(MARKER);
    wire.extend_from_slice(headers(1).as_bytes());
    while wire.len() <= first_end + 4096 {
        let byte = ALPHABET[index % ALPHABET.len()];
        wire.resize(wire.len() + 255, byte);
        wire.push(b'\n');
        index += 1;
    }
    wire.push(b'\n');
    let last_end = wire.len();
    (wire, [first_end, last_end])
}

fn assert_refill_mbox(long_lines: bool) {
    let (wire, ends) = create_refill_mbox(long_lines);
    for capacity in [1, 7, 4095, 4096, 4097, 8192] {
        let parser = MimeParser::new(BufReader::with_capacity(capacity, wire.as_slice()));
        assert_refill_mbox_messages(parser, &wire, ends);
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let mut parser = MimeParser::with_storage(
                ShortSource {
                    inner: Cursor::new(wire.as_slice()),
                    chunk: capacity,
                },
                storage,
            )
            .unwrap();
            let shared = parser.get_ref().get_ref().clone();
            drop(parser.replace_source(BufReader::with_capacity(capacity, shared)));
            assert_refill_mbox_messages(parser, &wire, ends);
        }
    }
}

fn assert_refill_mbox_messages<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    wire: &[u8],
    ends: [usize; 2],
) {
    for (index, end) in ends.into_iter().enumerate() {
        let mut message = parser.parse_mbox_message().unwrap().unwrap();
        assert_eq!(
            message.message_id().unwrap().unwrap().as_str(),
            format!("1234567890.{index}@example.org")
        );
        // MimeKit leaves Position at the next marker. Our reader consumes and
        // retains its five-byte prefix as lookahead, outside this message.
        assert_eq!(
            parser.position().offset - parser.pending_input().len() as u64,
            end as u64
        );
        assert_eq!(
            parser.pending_input(),
            if index == 0 { b"From ".as_slice() } else { b"" }
        );
        let start = if index == 0 { 7 } else { ends[0] + 7 };
        let end = end - usize::from(index == 0);
        let expected = &wire[start..end];
        let mut output = Vec::new();
        message
            .write_to_with_options(
                &mut output,
                &MimePartWriteOptions {
                    new_line_format: NewLineFormat::Unix,
                    ..Default::default()
                },
            )
            .unwrap();
        let normalized = std::str::from_utf8(expected).unwrap().replace("\r\n", "\n");
        assert_round_trip("mbox refill", normalized.as_bytes(), &output);
        let body_start = expected
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let Some(MimeEntity::Part(part)) = message.body_mut() else {
            panic!("expected text part")
        };
        let mut body = Vec::new();
        part.content_mut().decode_to(&mut body).unwrap();
        assert_eq!(body, &expected[body_start..]);
    }
    assert!(parser.parse_mbox_message().unwrap().is_none());
    assert!(parser.is_end_of_stream());
}

#[test]
fn header_parser() {
    let data = b"Header-1: value 1\r\nHeader-2: value 2\r\nHeader-3: value 3\r\n\r\n";
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, data.as_slice()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 3);
        for index in 1..=3 {
            let name = format!("Header-{index}");
            assert_eq!(
                headers.get(&name).unwrap().raw_value(),
                format!(" value {index}\r\n").as_bytes()
            );
        }
    }
}

#[test]
fn truncated_invalid_header_after_valid_header() {
    for tail in ["Junk", "Junk\r", "Junk\r\n"] {
        let text = format!("Header-1: value 1\r\n{tail}");
        for capacity in [1, 7, 4096] {
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
            let headers = parser.parse_headers().unwrap();
            assert_eq!(headers.len(), 2);
            assert_eq!(
                headers.get("Header-1").unwrap().decode_value().text,
                "value 1"
            );
            let invalid = &headers.as_slice()[1];
            assert!(invalid.is_invalid());
            assert_eq!(invalid.raw(), tail.as_bytes());
            assert_eq!(invalid.offset(), Some(19));
        }
    }
}

#[test]
fn truncated_header() {
    assert_single_header("Header-1: value 1");
}

#[test]
fn single_header_no_terminator() {
    assert_single_header("Header-1: value 1\r\n");
}

fn assert_single_header(text: &str) {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers.get("Header-1").unwrap().decode_value().text,
            "value 1"
        );
        assert_eq!(headers.as_slice()[0].raw(), text.as_bytes());
        assert_eq!(parser.position().offset, text.len() as u64);
        assert!(parser.is_end_of_stream());
    }
}

#[test]
fn empty_headers() {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, b"\r\n".as_slice()));
        assert!(parser.parse_headers().unwrap().is_empty());
        assert_eq!(parser.position().offset, 2);
    }
}

const BARE_CR_HEADERS: &str = concat!(
    "From: <mimekit@example.com>\r\nTo: <mimekit@example.com>\r\n",
    "Subject: Test of headers ending with bare carriage-return\r\n",
);

#[test]
fn headers_end_with_bare_carriage_return() {
    assert_bare_carriage_return_headers("\r", false);
}

#[test]
fn headers_with_bare_carriage_return() {
    assert_bare_carriage_return_headers(
        "\rYou might expect this to be a body, but it's really an invalid header.\r\n",
        true,
    );
}

fn assert_bare_carriage_return_headers(tail: &str, invalid: bool) {
    let text = format!("{BARE_CR_HEADERS}{tail}");
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), if invalid { 4 } else { 3 });
        for (header, (id, value)) in headers.iter().zip([
            (mimecraft::HeaderId::From, "<mimekit@example.com>"),
            (mimecraft::HeaderId::To, "<mimekit@example.com>"),
            (
                mimecraft::HeaderId::Subject,
                "Test of headers ending with bare carriage-return",
            ),
        ]) {
            assert_eq!(header.id(), id);
            assert_eq!(header.decode_value().text, value);
        }
        if invalid {
            let header = &headers.as_slice()[3];
            assert!(header.is_invalid());
            assert_eq!(header.raw(), tail.as_bytes());
        }
        assert_eq!(parser.position().offset, text.len() as u64);
    }
}

#[test]
fn status_group_parser_returns_null_at_end_of_stream() {
    let text = b"Reporting-MTA: dns; mm1\r\n\r\n";
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_slice()));
        let group = parser.parse_status_group().unwrap().unwrap();
        assert_eq!(group.len(), 1);
        assert_eq!(
            group.get("Reporting-MTA").unwrap().decode_value().text,
            "dns; mm1"
        );
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        // Calling again continues to return None rather than throwing.
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

#[test]
fn status_group_parser() {
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n",
    );
}

#[test]
fn status_group_parser_trailing_lone_carriage_return() {
    // A trailing lone CR is not a status group and must not create an empty group.
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r",
    );
}

#[test]
fn status_group_parser_trailing_blank_lines() {
    // Trailing blank lines must not create an empty status group.
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r\n\r\n",
    );
}

#[test]
fn status_group_parser_leading_blank_lines() {
    assert_status_groups(
        "\r\n\r\n\r\nReporting-MTA: dns; mm1\r\n\r\n\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n",
    );
}

#[test]
fn status_group_parser_mixed_new_line_formats() {
    assert_status_groups(
        "Reporting-MTA: dns; mm1\r\n\n\r\n\nFinal-Recipient: rfc822; user@example.com\n",
    );
}

#[test]
fn status_group_parser_read_one_byte_at_a_time() {
    assert_status_groups(
        "\r\n\r\nReporting-MTA: dns; mm1\r\n\r\n\r\nFinal-Recipient: rfc822; user@example.com\r\n\r\n\r\n",
    );
}

fn assert_status_groups(text: &str) {
    // One-byte reads force CRLF sequences across buffer boundaries.
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        for (field, value) in [
            ("Reporting-MTA", "dns; mm1"),
            ("Final-Recipient", "rfc822; user@example.com"),
        ] {
            let group = parser.parse_status_group().unwrap().unwrap();
            assert_eq!(group.len(), 1);
            assert_eq!(
                std::str::from_utf8(group.get(field).unwrap().raw_value())
                    .unwrap()
                    .trim(),
                value
            );
        }
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        // Repeated EOF calls keep returning None instead of throwing.
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

#[test]
fn status_group_parser_no_content() {
    assert_no_status_groups("");
}

#[test]
fn status_group_parser_single_blank_line() {
    assert_no_status_groups("\r\n");
}

#[test]
fn status_group_parser_only_blank_lines() {
    assert_no_status_groups("\r\n\r\n\r\n");
}

#[test]
fn status_group_parser_only_blank_lines_lf() {
    assert_no_status_groups("\n\n\n");
}

#[test]
fn status_group_parser_only_lone_carriage_return() {
    assert_no_status_groups("\r");
}

fn assert_no_status_groups(text: &str) {
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
        assert!(parser.parse_status_group().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        assert!(parser.parse_status_group().unwrap().is_none());
    }
}

const MESSAGE_PART_MBOX_MARKER: &str = "From sender Sat Jan  1 00:00:00 2000";

fn assert_message_part_text_body(message: &MimeMessage, expected: &[u8]) {
    assert_eq!(message.subject().unwrap().text, "inner");
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected text body")
    };
    assert!(part.content_type().unwrap().is_none());
    assert_eq!(part.content().get_ref().get_ref(), expected);
}

fn message_part_offsets(
    text: &str,
    capacity: usize,
    mbox: bool,
) -> Vec<(MimePosition, Option<MimePosition>)> {
    let mut reader = MimeReader::new(BufReader::with_capacity(capacity, text.as_bytes()));
    let mut offsets = Vec::new();
    let mut record = |event: MimeEvent<'_>| {
        let positions = match event {
            MimeEvent::MboxMarker { position, end, .. } => (position, Some(end)),
            MimeEvent::EntityBegin {
                position,
                headers_end,
                ..
            } => (position, Some(headers_end)),
            MimeEvent::MessageBegin { position }
            | MimeEvent::MessageEnd { position }
            | MimeEvent::EntityEnd { position, .. } => (position, None),
            MimeEvent::Content { .. } => return Ok(()),
            MimeEvent::Headers { .. } => panic!("expected message events"),
        };
        for position in std::iter::once(positions.0).chain(positions.1) {
            assert!(position.offset <= text.len() as u64);
            assert_eq!(
                position.line,
                1 + text.as_bytes()[..position.offset as usize]
                    .iter()
                    .filter(|&&byte| byte == b'\n')
                    .count() as u64
            );
        }
        offsets.push(positions);
        Ok(())
    };
    if mbox {
        while reader.read_mbox_message(&mut record).unwrap() {}
    } else {
        reader.read_message(&mut record).unwrap();
    }
    offsets
}

fn assert_message_part_short_reads(
    text: &str,
    mbox: bool,
    count: usize,
    serializable: bool,
    mut check: impl FnMut(&MimeMessage, usize, MimePosition),
) {
    let mut expected = None;
    // Owned synchronous parsing does not require seeking. Persistent-source
    // and async variants are not available; exercise the same short reads.
    for capacity in [4096, 7, 2, 1] {
        let mut parser = MimeParser::with_options(
            BufReader::with_capacity(capacity, text.as_bytes()),
            MimeEntityParseOptions {
                max_depth: 16,
                ..Default::default()
            },
        )
        .unwrap();
        let mut serialized = Vec::new();
        for index in 0..count {
            assert!(!parser.is_end_of_stream());
            let mut message = if mbox {
                parser.parse_mbox_message().unwrap().unwrap()
            } else {
                parser.parse_message().unwrap()
            };
            check(&message, index, parser.position());
            if serializable {
                let mut output = Vec::new();
                message
                    .write_to_with_options(
                        &mut output,
                        &MimePartWriteOptions {
                            new_line_format: NewLineFormat::Unix,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                serialized.push(output);
            }
        }
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position().offset, text.len() as u64);
        let actual = (serialized, message_part_offsets(text, capacity, mbox));
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected, "capacity {capacity}");
        } else {
            expected = Some(actual);
        }
    }
}

#[test]
fn message_part_immediate_mbox_boundary() {
    for newline in ["\n", "\r\n"] {
        for marker in [
            "From invalid\0preheader",
            "From sender Sat Jan  1 00:00:01 2000",
        ] {
            let prefix = format!(
                "{MESSAGE_PART_MBOX_MARKER}{newline}Content-Type: message/rfc822{newline}{newline}"
            );
            let text =
                format!("{prefix}{marker}{newline}Subject: inner{newline}{newline}body{newline}");
            assert_message_part_short_reads(&text, true, 2, true, |message, index, position| {
                if index == 0 {
                    let MimeEntity::Message(part) = message.body().unwrap() else {
                        panic!("expected message wrapper")
                    };
                    assert!(part.message().is_none());
                    // The logical entity end is at the marker; mimecraft's
                    // consumed position also includes its five-byte lookahead.
                    assert_eq!(position.offset, prefix.len() as u64 + 5);
                } else {
                    assert_message_part_text_body(message, format!("body{newline}").as_bytes());
                }
            });
        }
    }
}

#[test]
fn message_part_immediate_multipart_boundary() {
    // Short reads must not leave the boundary probe with only part of a marker.
    // Preserve the preceding newline so empty-wrapper end offsets stay correct.
    for newline in ["\n", "\r\n"] {
        for closing in [false, true] {
            for length in [1, 140] {
                let boundary = "b".repeat(length);
                let mut text = format!(
                    "Content-Type: multipart/mixed; boundary=\"{boundary}\"{newline}{newline}--{boundary}{newline}Content-Type: message/rfc822{newline}{newline}"
                );
                if !closing {
                    text.push_str(&format!("--{boundary}{newline}Content-Type: text/plain{newline}{newline}sibling{newline}"));
                }
                text.push_str(&format!("--{boundary}--{newline}"));
                // Recovery accepts long boundaries, but mimecraft's writer
                // intentionally rejects them. Only valid boundaries round-trip.
                assert_message_part_short_reads(&text, false, 1, length <= 70, |message, _, _| {
                    let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                        panic!("expected multipart")
                    };
                    assert_eq!(multipart.children().len(), if closing { 1 } else { 2 });
                    let MimeEntity::Message(part) = &multipart.children()[0] else {
                        panic!("expected message wrapper")
                    };
                    assert!(part.message().is_none());
                    if !closing {
                        let MimeEntity::Part(part) = &multipart.children()[1] else {
                            panic!("expected sibling body")
                        };
                        assert_eq!(part.content().get_ref().get_ref(), b"sibling");
                    }
                });
            }
        }
    }
}

#[test]
fn message_part_at_end_of_stream() {
    for newline in ["\n", "\r\n"] {
        for mbox in [false, true] {
            let mut text = format!("Content-Type: message/rfc822{newline}{newline}");
            if mbox {
                text = format!("{MESSAGE_PART_MBOX_MARKER}{newline}{text}");
            }
            assert_message_part_short_reads(&text, mbox, 1, true, |message, _, _| {
                let MimeEntity::Message(part) = message.body().unwrap() else {
                    panic!("expected message wrapper")
                };
                if mbox {
                    assert!(part.message().is_none());
                } else {
                    let inner = part.message().unwrap();
                    assert!(inner.headers().is_empty());
                    let MimeEntity::Part(body) = inner.body().unwrap() else {
                        panic!("expected empty text body")
                    };
                    assert!(body.content().get_ref().get_ref().is_empty());
                }
            });
        }
    }
}

#[test]
fn message_part_embedded_message_short_reads() {
    for newline in ["\n", "\r\n"] {
        for mbox in [false, true] {
            let mut text = format!(
                "Content-Type: message/rfc822{newline}{newline}Subject: inner{newline}{newline}body{newline}"
            );
            if mbox {
                text = format!("{MESSAGE_PART_MBOX_MARKER}{newline}{text}");
            }
            assert_message_part_short_reads(&text, mbox, 1, true, |message, _, _| {
                let MimeEntity::Message(part) = message.body().unwrap() else {
                    panic!("expected message wrapper")
                };
                assert_message_part_text_body(
                    part.message().unwrap(),
                    format!("body{newline}").as_bytes(),
                );
            });
        }
    }
}

struct ThrowingReadStream<'a> {
    bytes: &'a [u8],
    position: usize,
    limit: usize,
    max_chunk: usize,
    failure: &'static str,
}

impl Read for ThrowingReadStream<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.position >= self.limit {
            return Err(io::Error::other(self.failure));
        }
        if self.position == self.bytes.len() {
            return Ok(0);
        }
        let count = output
            .len()
            .min(self.max_chunk)
            .min(self.limit - self.position)
            .min(self.bytes.len() - self.position);
        output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

#[test]
fn message_part_boundary_lookahead_is_bounded() {
    let prefix = format!("{MESSAGE_PART_MBOX_MARKER}\nContent-Type: message/rfc822\n\n");
    let text = format!(
        "{prefix}From {}\nSubject: inner\n\nbody\n",
        "x".repeat(10000)
    );
    for capacity in [1, 7, 4096] {
        let source = ThrowingReadStream {
            bytes: text.as_bytes(),
            position: 0,
            limit: prefix.len() + 128,
            max_chunk: 1,
            failure: "boundary lookahead exceeded limit",
        };
        let mut parser = MimeParser::new(BufReader::with_capacity(capacity, source));
        let message = parser.parse_mbox_message().unwrap().unwrap();
        let MimeEntity::Message(part) = message.body().unwrap() else {
            panic!("expected message wrapper")
        };
        assert!(part.message().is_none());
        assert_eq!(parser.position().offset, prefix.len() as u64 + 5);
        let (source, pending) = parser.into_parts();
        assert_eq!(pending, b"From ");
        assert!(source.get_ref().position <= prefix.len() + 128);
    }
}

#[test]
fn parse_message_recovers_after_read_failure() {
    let headers = b"Subject: broken\r\n\r\n";
    let mut failing = headers.to_vec();
    failing.extend_from_slice(&[b'x'; 4096]);
    let throw_offset = headers.len() + 64;
    let recovered = b"Subject: recovered\r\n\r\nok";
    for capacity in [1, 7, 4096] {
        for logged in [false, true] {
            let source = ThrowingReadStream {
                bytes: &failing,
                position: 0,
                limit: throw_offset,
                max_chunk: 7,
                failure: "Read failed.",
            };
            let mut parser = MimeParser::new(BufReader::with_capacity(capacity, source));
            let error = if logged {
                parser.parse_message_with_logger(&mut |_| Ok(()))
            } else {
                parser.parse_message()
            }
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other);
            assert_eq!(error.to_string(), "Read failed.");
            assert_eq!(parser.position().offset, throw_offset as u64);
            assert!(!parser.is_end_of_stream());
            assert_eq!(
                parser.parse_message().unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            let old = parser.replace_source(BufReader::with_capacity(
                capacity,
                ThrowingReadStream {
                    bytes: recovered,
                    position: 0,
                    limit: usize::MAX,
                    max_chunk: 7,
                    failure: "unexpected read failure",
                },
            ));
            assert_eq!(old.get_ref().position, throw_offset);
            assert_eq!(parser.position(), MimePosition::default());
            let mut message = parser.parse_message().unwrap();
            assert_eq!(message.subject().unwrap().text, "recovered");
            let MimeEntity::Part(part) = message.body().unwrap() else {
                panic!("expected text body")
            };
            assert_eq!(part.content().get_ref().get_ref(), b"ok");
            assert_eq!(parser.position().offset, recovered.len() as u64);
            assert!(parser.is_end_of_stream());
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_round_trip("recovery after body read failure", recovered, &output);
        }
    }
}

#[test]
fn replacing_and_dropping_sources_keeps_owned_messages_readable() {
    use std::io::Cursor;

    let first = b"Subject: first\r\n\r\nfirst body";
    let second = b"Subject: second\r\n\r\nsecond body";
    for capacity in [1, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(
            capacity,
            Cursor::new(first.to_vec()),
        ));
        let mut messages = vec![parser.parse_message().unwrap()];
        assert_eq!(parser.position().offset, first.len() as u64);
        // Adapt SetStream's source replacement to the copying parser. Unlike
        // persistent-source bodies, these models must survive source disposal.
        let old = parser.replace_source(BufReader::with_capacity(
            capacity,
            Cursor::new(second.to_vec()),
        ));
        drop(old);
        assert_eq!(parser.position(), MimePosition::default());
        assert!(!parser.is_end_of_stream());
        messages.push(parser.parse_message().unwrap());
        assert_eq!(parser.position().offset, second.len() as u64);
        drop(parser);
        for (message, expected) in messages
            .iter_mut()
            .zip([first.as_slice(), second.as_slice()])
        {
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_round_trip("owned content after source disposal", expected, &output);
        }
    }
}

#[test]
fn double_boundary_before_part_preserves_raw_boundary() {
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let text = "Content-Type: multipart/mixed; boundary=\"b\"\n\n--b\n--b\nContent-Type: text/plain\n\nbody\n--b--\n".replace('\n', newline);
        for capacity in [1, 7, 4096] {
            for logged in [false, true] {
                let mut parser =
                    MimeParser::new(BufReader::with_capacity(capacity, text.as_bytes()));
                let mut message = if logged {
                    parser.parse_message_with_logger(&mut |_| Ok(()))
                } else {
                    parser.parse_message()
                }
                .unwrap();
                let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                    panic!()
                };
                assert_eq!(multipart.children().len(), 2);
                for (entity, expected) in multipart.children().iter().zip([b"".as_slice(), b"body"])
                {
                    let MimeEntity::Part(part) = entity else {
                        panic!()
                    };
                    assert_eq!(part.content().get_ref().get_ref(), expected);
                }
                let mut output = Vec::new();
                message
                    .write_to_with_options(
                        &mut output,
                        &MimePartWriteOptions {
                            new_line_format: format,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                assert_round_trip("double boundary before part", text.as_bytes(), &output);
                assert!(parser.is_end_of_stream());
            }
        }
    }
}

const TRUNCATED_MULTIPART_BOUNDARY: &str = "----=_NextPart_000_003F_01CE98CE.6E826F90";
const TRUNCATED_PART_HEADERS: &str = "Content-Type: text/plain; charset=utf-8\n";
const TRUNCATED_PART_BODY: &str = "This is the message body.\n";

fn assert_truncated_multipart(subject: &str, body: &str, children: &[(&str, &str)]) {
    let original = format!(
        "From: mimekit@example.com\n\
         To: mimekit@example.com\n\
         Subject: {subject}\n\
         Date: Tue, 12 Nov 2013 09:12:42 -0500\n\
         MIME-Version: 1.0\n\
         Message-ID: <54AD68C9E3B0184CAC6041320424FD1B5B81E74D@localhost.localdomain>\n\
         X-Mailer: Microsoft Office Outlook 12.0\n\
         Content-Type: multipart/mixed;\n\
         \tboundary=\"{TRUNCATED_MULTIPART_BOUNDARY}\"\n\n\n{body}"
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let children: Vec<_> = children
            .iter()
            .map(|(headers, body)| (headers.replace('\n', newline), body.replace('\n', newline)))
            .collect();
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_truncated_multipart(parser, &input, logged);
                assert_truncated_multipart_model(message, &input, &children, format);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_truncated_multipart(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        &message,
                        &input,
                        storage,
                        children.iter().map(|(_, body)| body.as_str()),
                    );
                    assert_truncated_multipart_model(message, &input, &children, format);
                }
            }
        }
    }
}

fn assert_stored_recovery_bodies<'a, R: Read + Seek>(
    message: &MimeMessage<mimecraft::MimeContentStream<R>>,
    input: &str,
    storage: MimeContentStorage,
    expected: impl IntoIterator<Item = &'a str>,
) {
    let mut parts = message.leaf_parts();
    for expected in expected {
        let part = parts.next().expect("missing recovered leaf");
        match part.content().get_ref() {
            mimecraft::MimeContentStream::Memory(content) => {
                assert_eq!(storage, MimeContentStorage::Memory);
                assert_eq!(content.get_ref(), expected.as_bytes());
                assert_eq!(content.position(), 0);
            }
            mimecraft::MimeContentStream::Source(content) => {
                assert_eq!(storage, MimeContentStorage::Source);
                let start = content.start_boundary() as usize;
                let end = content.end_boundary().unwrap() as usize;
                assert_eq!(&input.as_bytes()[start..end], expected.as_bytes());
                assert_eq!(content.position(), 0);
            }
        }
    }
    assert!(parts.next().is_none(), "unexpected recovered leaf");
}

fn parse_truncated_multipart<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) -> MimeMessage<C> {
    let message = if logged {
        parser.parse_message_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_message()
    }
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    // The returned model must retain its body bytes after the parser is dropped.
    message
}

fn assert_truncated_multipart_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    children: &[(String, String)],
    format: NewLineFormat,
) {
    let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
        panic!("expected top-level multipart")
    };
    assert_eq!(multipart.children().len(), children.len());
    assert!(multipart.epilogue().is_empty());
    for (child, (expected_headers, _)) in multipart.children().iter().zip(children) {
        let MimeEntity::Part(part) = child else {
            panic!("expected text/plain leaf")
        };
        let mut headers = Vec::new();
        part.headers().write_raw_to(&mut headers).unwrap();
        assert_eq!(headers, expected_headers.as_bytes());
        if expected_headers.is_empty() {
            // MimeKit exposes empty children as TextPart instances; the Rust
            // raw model keeps their missing Content-Type absent.
            assert!(part.content_type().unwrap().is_none());
        } else {
            let content_type = part.content_type().unwrap().unwrap();
            assert!(content_type.is_mime_type("text", "plain"));
            assert_eq!(content_type.charset(), Some("utf-8"));
            if part.headers().len() == 2 {
                let header = part.headers().iter().nth(1).unwrap();
                assert!(header.is_invalid());
                let offset = expected_headers.find('\n').unwrap() + 1;
                assert_eq!(header.raw_field(), &expected_headers.as_bytes()[offset..]);
                assert!(header.raw_value().is_empty());
            } else {
                assert_eq!(part.headers().len(), 1);
            }
        }
    }
    assert_recovery_serialization(&mut message, input, format);
    // Mutable access invalidates parsed framing, so inspect decoded content
    // only after checking that read-only serialization preserves it.
    let MimeEntity::Multipart(multipart) = message.body_mut().unwrap() else {
        unreachable!()
    };
    for (child, (_, expected)) in multipart.children_mut().iter_mut().zip(children) {
        let MimeEntity::Part(part) = child else {
            unreachable!()
        };
        let mut output = Vec::new();
        part.content_mut().decode_to(&mut output).unwrap();
        assert_eq!(output, expected.as_bytes());
    }
}

fn assert_recovery_serialization<R: Read + Seek>(
    message: &mut MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
) {
    let newline = match format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => unreachable!(),
    };
    let ensured = if input.ends_with('\n') {
        input.to_owned()
    } else {
        format!("{input}{newline}")
    };
    // The output should match the input exactly unless EnsureNewLine is set.
    // Make sure that EnsureNewLine did not leave any state behind.
    for ensure_new_line in [false, true, false] {
        let mut output = Vec::new();
        message
            .write_to_with_options(
                &mut output,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_round_trip(
            "multipart recovery",
            if ensure_new_line {
                ensured.as_bytes()
            } else {
                input.as_bytes()
            },
            &output,
        );
    }
}

#[test]
fn multipart_without_end_boundary() {
    assert_truncated_multipart(
        "test of a multipart without an end boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the first part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the second part.\n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
        ],
    );
}

#[test]
fn multipart_boundary_line_with_trailing_spaces() {
    assert_truncated_multipart(
        "test of a multipart boundary followed by trailing whitespace",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}   \n{TRUNCATED_PART_HEADERS}\n\
             This is the first part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}       \n{TRUNCATED_PART_HEADERS}\n\
             This is the second part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}--  \n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
        ],
    );
}

#[test]
fn multipart_boundary_line_with_trailing_spaces_and_then_more_characters() {
    let first =
        format!("This is the first part.\n\n--{TRUNCATED_MULTIPART_BOUNDARY}       oops, not it.");
    assert_truncated_multipart(
        "test of a multipart boundary followed by trailing whitespace and then more characters",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             {first}\n--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the second part.\n\n--{TRUNCATED_MULTIPART_BOUNDARY}--\n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, &first),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
        ],
    );
}

#[test]
fn multipart_double_boundary() {
    assert_truncated_multipart(
        "test of double multipart boundaries",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the first part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is technically the third part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}--\n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            ("", ""),
            (
                TRUNCATED_PART_HEADERS,
                "This is technically the third part.\n",
            ),
        ],
    );
}

#[test]
fn multipart_double_boundary_end_boundary() {
    assert_truncated_multipart(
        "test of double multipart boundaries and then an end boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the first part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the second part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n--{TRUNCATED_MULTIPART_BOUNDARY}\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}--\n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
            ("", ""),
            ("", ""),
        ],
    );
}

#[test]
fn multipart_double_boundary_eof() {
    assert_truncated_multipart(
        "test of multipart ending with a double boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the first part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             This is the second part.\n\n\
             --{TRUNCATED_MULTIPART_BOUNDARY}\n--{TRUNCATED_MULTIPART_BOUNDARY}\n"
        ),
        &[
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
            ("", ""),
            ("", ""),
        ],
    );
}

#[test]
fn multipart_without_end_boundary_or_trailing_new_line() {
    let original = concat!(
        "From: mimekit@example.com\n",
        "To: mimekit@example.com\n",
        "Subject: test of a truncated multipart w/o a trailing newline\n",
        "MIME-Version: 1.0\n",
        "Content-Type: multipart/mixed; boundary=\"outer\"\n\n",
        "--outer\nContent-Type: text/plain\n\nThis is the first part.\n\n",
        "--outer\nContent-Type: message/rfc822\n\n",
        "From: mimekit@example.com\nSubject: embedded\n",
        "Content-Type: multipart/alternative; boundary=\"inner\"\n\n",
        "--inner\nContent-Type: text/plain\n\nThis is the last part."
    );
    assert_nested_multipart_recovery(original, false);
}

#[test]
fn multipart_double_boundary_parent_boundary() {
    let original = concat!(
        "From: mimekit@example.com\n",
        "To: mimekit@example.com\n",
        "Subject: test of multipart with double boundaries and then a parent boundary\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\n",
        "MIME-Version: 1.0\n",
        "Message-ID: <54AD68C9E3B0184CAC6041320424FD1B5B81E74D@localhost.localdomain>\n",
        "Content-Type: multipart/mixed;\n\tboundary=\"outer-boundary\"\n\n",
        "--outer-boundary\n",
        "Content-Type: multipart/mixed;\n\tboundary=\"inner-boundary\"\n\n",
        "--inner-boundary\nContent-Type: text/plain; charset=utf-8\n\n",
        "This is the first part.\n\n",
        "--inner-boundary\nContent-Type: text/plain; charset=utf-8\n\n",
        "This is the second part.\n\n",
        "--inner-boundary\n--inner-boundary\n",
        "--outer-boundary\nContent-Type: image/jpeg\n\n",
        "<base64 data>\n--outer-boundary--\n"
    );
    assert_nested_multipart_recovery(original, true);
}

fn assert_nested_multipart_recovery(original: &str, parent_boundary: bool) {
    let expected = if parent_boundary {
        vec![
            (TRUNCATED_PART_HEADERS, "This is the first part.\n"),
            (TRUNCATED_PART_HEADERS, "This is the second part.\n"),
            ("", ""),
            ("", ""),
            ("Content-Type: image/jpeg\n", "<base64 data>"),
        ]
    } else {
        vec![
            ("Content-Type: text/plain\n", "This is the first part.\n"),
            ("Content-Type: text/plain\n", "This is the last part."),
        ]
    };
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let expected: Vec<_> = expected
            .iter()
            .map(|(headers, body)| (headers.replace('\n', newline), body.replace('\n', newline)))
            .collect();
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_truncated_multipart(parser, &input, logged);
                assert_nested_multipart_model(message, &input, &expected, format, parent_boundary);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_truncated_multipart(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        &message,
                        &input,
                        storage,
                        expected.iter().map(|(_, body)| body.as_str()),
                    );
                    assert_nested_multipart_model(
                        message,
                        &input,
                        &expected,
                        format,
                        parent_boundary,
                    );
                }
            }
        }
    }
}

fn assert_nested_multipart_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    expected: &[(String, String)],
    format: NewLineFormat,
    parent_boundary: bool,
) {
    let MimeEntity::Multipart(outer) = message.body().unwrap() else {
        panic!("expected outer multipart")
    };
    assert_eq!(outer.children().len(), 2);
    assert!(outer.epilogue().is_empty());
    if parent_boundary {
        assert_eq!(outer.boundary().unwrap().as_deref(), Some("outer-boundary"));
        let MimeEntity::Multipart(inner) = &outer.children()[0] else {
            panic!("expected nested multipart")
        };
        assert_eq!(inner.boundary().unwrap().as_deref(), Some("inner-boundary"));
        assert_eq!(inner.children().len(), 4);
        assert!(
            inner
                .children()
                .iter()
                .all(|child| matches!(child, MimeEntity::Part(_)))
        );
        assert!(inner.epilogue().is_empty());
        let MimeEntity::Part(image) = &outer.children()[1] else {
            panic!("expected following image sibling")
        };
        assert!(
            image
                .content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("image", "jpeg")
        );
    } else {
        assert_eq!(outer.boundary().unwrap().as_deref(), Some("outer"));
        assert!(matches!(outer.children()[0], MimeEntity::Part(_)));
        let MimeEntity::Message(wrapper) = &outer.children()[1] else {
            panic!("expected message/rfc822 wrapper")
        };
        assert!(
            wrapper
                .content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("message", "rfc822")
        );
        let embedded = wrapper.message().unwrap();
        assert_eq!(embedded.subject().unwrap().text, "embedded");
        let MimeEntity::Multipart(inner) = embedded.body().unwrap() else {
            panic!("expected embedded multipart")
        };
        assert!(
            inner
                .content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("multipart", "alternative")
        );
        assert_eq!(inner.boundary().unwrap().as_deref(), Some("inner"));
        assert_eq!(inner.children().len(), 1);
        assert!(matches!(inner.children()[0], MimeEntity::Part(_)));
        assert!(inner.epilogue().is_empty());
    }
    assert_eq!(message.leaf_parts().count(), expected.len());
    for (part, (expected_headers, _)) in message.leaf_parts().zip(expected) {
        let mut headers = Vec::new();
        part.headers().write_raw_to(&mut headers).unwrap();
        assert_eq!(headers, expected_headers.as_bytes());
        if expected_headers.is_empty() {
            assert!(part.content_type().unwrap().is_none());
        } else if expected_headers.starts_with("Content-Type: text/plain") {
            let content_type = part.content_type().unwrap().unwrap();
            assert!(content_type.is_mime_type("text", "plain"));
            assert_eq!(
                content_type.charset(),
                expected_headers
                    .contains("charset=utf-8")
                    .then_some("utf-8")
            );
        }
    }
    assert_recovery_serialization(&mut message, input, format);
    let mut bodies = expected.iter().map(|(_, body)| body.as_str());
    assert_recovery_body_bytes(message.body_mut().unwrap(), &mut bodies);
    assert!(bodies.next().is_none());
}

fn assert_recovery_body_bytes<'a, R: Read + Seek>(
    entity: &mut MimeEntity<R>,
    expected: &mut impl Iterator<Item = &'a str>,
) {
    match entity {
        MimeEntity::Part(part) => {
            let mut output = Vec::new();
            part.content_mut().decode_to(&mut output).unwrap();
            assert_eq!(output, expected.next().unwrap().as_bytes());
        }
        MimeEntity::Multipart(multipart) => {
            for child in multipart.children_mut() {
                assert_recovery_body_bytes(child, expected);
            }
        }
        MimeEntity::Message(wrapper) => {
            let message = wrapper.message_mut().unwrap();
            assert_recovery_body_bytes(message.body_mut().unwrap(), expected);
        }
    }
}

#[test]
fn multipart_without_boundary_parameter() {
    let original = format!(
        "Content-Type: multipart/mixed\n\n\
         --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
         This is the first part.\n\n\
         --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
         This is the second part.\n\n\
         --{TRUNCATED_MULTIPART_BOUNDARY}--\n"
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                assert_boundaryless_multipart(parser, &input, format, logged);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    assert_boundaryless_multipart(parser, &input, format, logged);
                }
            }
        }
    }
}

fn assert_boundaryless_multipart<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    format: NewLineFormat,
    logged: bool,
) {
    let mut entity = if logged {
        parser.parse_entity_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_entity()
    }
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    drop(parser);
    let MimeEntity::Multipart(multipart) = &entity else {
        panic!("expected multipart without a boundary parameter")
    };
    assert_eq!(multipart.boundary().unwrap(), None);
    assert!(multipart.children().is_empty());
    assert_eq!(
        multipart.preamble(),
        &input.as_bytes()[input.find("--").unwrap()..]
    );
    assert!(multipart.epilogue().is_empty());
    for ensure_new_line in [false, true, false] {
        let mut output = Vec::new();
        entity
            .write_to_with_options(
                &mut output,
                &MimePartWriteOptions {
                    new_line_format: format,
                    ensure_new_line,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_round_trip(
            "missing multipart boundary parameter",
            input.as_bytes(),
            &output,
        );
    }
}

#[test]
fn multipart_truncated_at_end_of_first_boundary() {
    assert_truncated_multipart(
        "test of multipart truncated at the end of the first boundary",
        &format!("--{TRUNCATED_MULTIPART_BOUNDARY}"),
        &[],
    );
}

#[test]
fn multipart_truncated_at_end_of_second_boundary() {
    assert_truncated_multipart(
        "test of a multipart truncated at the end of the second boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             {TRUNCATED_PART_BODY}\n--{TRUNCATED_MULTIPART_BOUNDARY}"
        ),
        &[(TRUNCATED_PART_HEADERS, TRUNCATED_PART_BODY)],
    );
}

#[test]
fn multipart_truncated_immediately_after_first_boundary() {
    assert_truncated_multipart(
        "test of multipart truncated immedately after first boundary",
        &format!("--{TRUNCATED_MULTIPART_BOUNDARY}\n"),
        &[("", "")],
    );
}

#[test]
fn multipart_truncated_immediately_after_second_boundary() {
    assert_truncated_multipart(
        "test of a multipart truncated immediately after the second boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             {TRUNCATED_PART_BODY}\n--{TRUNCATED_MULTIPART_BOUNDARY}\n"
        ),
        &[(TRUNCATED_PART_HEADERS, TRUNCATED_PART_BODY), ("", "")],
    );
}

#[test]
fn multipart_boundary_without_trailing_newline() {
    assert_truncated_multipart(
        "test of multipart boundary w/o trailing newline",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
             {TRUNCATED_PART_BODY}\n--{TRUNCATED_MULTIPART_BOUNDARY}"
        ),
        &[(TRUNCATED_PART_HEADERS, TRUNCATED_PART_BODY)],
    );
}

#[test]
fn truncated_multipart_subpart_headers() {
    let headers = TRUNCATED_PART_HEADERS.trim_end_matches('\n');
    assert_truncated_multipart(
        "test of truncated multipart subpart headers",
        &format!("--{TRUNCATED_MULTIPART_BOUNDARY}\n{headers}"),
        &[(headers, "")],
    );
}

#[test]
fn truncated_multipart_subpart_header_field_name() {
    let headers = format!("{TRUNCATED_PART_HEADERS}Content-Dis");
    assert_truncated_multipart(
        "test of truncated multipart subpart header field name",
        &format!("--{TRUNCATED_MULTIPART_BOUNDARY}\n{headers}"),
        &[(&headers, "")],
    );
}

#[test]
fn multipart_subpart_headers_end_with_boundary() {
    assert_truncated_multipart(
        "test of multipart subpart headers ending with a boundary",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\
             --{TRUNCATED_MULTIPART_BOUNDARY}--\n"
        ),
        &[(TRUNCATED_PART_HEADERS, "")],
    );
}

#[test]
fn multipart_subpart_headers_line_starts_with_dash_dash() {
    let headers = format!("{TRUNCATED_PART_HEADERS}--not-the-boundary-muhahaha\n");
    assert_truncated_multipart(
        "test of multipart subpart headers line starting with --",
        &format!(
            "--{TRUNCATED_MULTIPART_BOUNDARY}\n{headers}\n\
             {TRUNCATED_PART_BODY}\n--{TRUNCATED_MULTIPART_BOUNDARY}--\n"
        ),
        &[(&headers, TRUNCATED_PART_BODY)],
    );
}

#[test]
fn multipart_subpart_headers_line_starts_with_dash_dash_eof() {
    let headers = format!("{TRUNCATED_PART_HEADERS}--not-the-boundary-muhahaha");
    assert_truncated_multipart(
        "test of multipart subpart headers line starting with -- <EOF>",
        &format!("--{TRUNCATED_MULTIPART_BOUNDARY}\n{headers}"),
        &[(&headers, "")],
    );
}

#[test]
fn message_rfc822_with_from_marker_before_message() {
    let original = concat!(
        "From -\n",
        "From: mimekit@example.com\n",
        "To: mimekit@example.com\n",
        "Subject: test of message/rfc822 part with a From-marker before the message\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\n",
        "MIME-Version: 1.0\n",
        "Message-ID: <54AD68C9E3B0184CAC6041320424FD1B5B81E74D@localhost.localdomain>\n",
        "X-Mailer: Microsoft Office Outlook 12.0\n",
        "Content-Type: multipart/mixed;\n",
        "\tboundary=\"----=_NextPart_000_003F_01CE98CE.6E826F90\"\n",
        "Content-Length: 420\n\n\n",
        "------=_NextPart_000_003F_01CE98CE.6E826F90\n",
        "Content-Type: text/plain; charset=utf-8\n\n",
        "This is the message body.\n\n",
        "------=_NextPart_000_003F_01CE98CE.6E826F90\n",
        "Content-Type: message/rfc822\n\n",
        "From -\n",
        "From: mimekit@example.com\n",
        "To: mimekit@example.com\n",
        "Subject: embedded message\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\n",
        "MIME-Version: 1.0\n",
        "Content-Type: text/plain; charset=utf-8\n\n",
        "This is the embedded message body.\n",
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        // Like MimeKit's regression, convert newlines without correcting the
        // declared length. It protects the From-line, not an exact body slice.
        let text = original.replace("\r\n", "\n").replace('\n', newline);
        for capacity in [1, 7, 4096] {
            for logged in [false, true] {
                let mut parser = MimeParser::with_options(
                    BufReader::with_capacity(capacity, text.as_bytes()),
                    MimeEntityParseOptions {
                        respect_content_length: true,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut message = if logged {
                    parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
                } else {
                    parser.parse_mbox_message()
                }
                .unwrap()
                .unwrap();
                let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                    panic!()
                };
                assert_eq!(multipart.children().len(), 2);
                let MimeEntity::Part(part) = &multipart.children()[0] else {
                    panic!()
                };
                assert_eq!(
                    part.content().get_ref().get_ref(),
                    format!("This is the message body.{newline}").as_bytes()
                );
                let MimeEntity::Message(wrapper) = &multipart.children()[1] else {
                    panic!()
                };
                let embedded = wrapper.message().unwrap();
                assert_eq!(embedded.subject().unwrap().text, "embedded message");
                let MimeEntity::Part(part) = embedded.body().unwrap() else {
                    panic!()
                };
                assert_eq!(
                    part.content_type().unwrap().unwrap().charset(),
                    Some("utf-8")
                );
                assert_eq!(
                    part.content().get_ref().get_ref(),
                    format!("This is the embedded message body.{newline}").as_bytes()
                );
                let mut output = Vec::new();
                message
                    .write_to_with_options(
                        &mut output,
                        &MimePartWriteOptions {
                            new_line_format: format,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                assert_round_trip(
                    "embedded From-line with Content-Length",
                    &text.as_bytes()["From -".len() + newline.len()..],
                    &output,
                );
                assert!(parser.is_end_of_stream());
                assert!(parser.parse_mbox_message().unwrap().is_none());
            }
        }
    }
}

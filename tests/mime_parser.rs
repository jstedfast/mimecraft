// Adapted from MimeKit's MimeParserTests.cs; excludes LegacyMimeParser.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding, HeaderId, HeaderList, HeaderTooLong, MimeComplianceContext,
    MimeComplianceIssue, MimeComplianceOptions, MimeCompliancePositionKind,
    MimeComplianceViolation, MimeContentStorage, MimeEntity, MimeEntityKind,
    MimeEntityParseOptions, MimeEvent, MimeEventFormat, MimeMessage, MimeParser,
    MimePartWriteOptions, MimePosition, MimeReader, TextFormat, encodings::NewLineFormat,
};
use std::{
    fmt::Write as _,
    io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom},
};

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
fn short_reads() {
    let mbox = concat!(
        "From sender@example.com Sat Jan  1 00:00:00 2022\n",
        "Subject: First\n\nbody one\n\n",
        "From sender@example.com Sat Jan  1 00:00:01 2022\n",
        "Subject: Second\n\nbody two\n\n",
        "From sender@example.com Sat Jan  1 00:00:02 2022\r\n",
        "Subject: Third\r\n\r\nbody three\r\n",
    );
    let multipart = concat!(
        "From: sender@example.org\r\nSubject: Short reads\r\nMIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n",
        "--b\r\nContent-Type: text/plain\r\n\r\nfirst part\r\n",
        "--b\r\n\r\nsecond part\r\n--b--\r\n",
    );
    for (wire, mbox) in [(mbox, true), (multipart, false)] {
        let expected =
            summarize_short_reads(MimeParser::from_slice(wire.as_bytes()), wire, mbox, false);
        assert_eq!(expected.len(), if mbox { 3 } else { 1 });
        if mbox {
            for (summary, subject, body) in [
                (&expected[0], "First", "body one\n"),
                (&expected[1], "Second", "body two\n"),
                (&expected[2], "Third", "body three\r\n"),
            ] {
                assert_eq!(summary.0, subject);
                assert_eq!(summary.1, [body.as_bytes()]);
            }
        } else {
            assert_eq!(expected[0].0, "Short reads");
            assert_eq!(expected[0].1, [b"first part".as_slice(), b"second part"]);
        }
        for logged in [false, true] {
            for chunk in [1, 2, 3, 4, 5, 7, 4096] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, wire.as_bytes()));
                assert_eq!(summarize_short_reads(parser, wire, mbox, logged), expected);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(wire.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    assert_eq!(summarize_short_reads(parser, wire, mbox, logged), expected);
                }
            }
        }
    }
}

type ShortReadSummary = (String, Vec<Vec<u8>>, Vec<u8>);

fn summarize_short_reads<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    wire: &str,
    mbox: bool,
    logged: bool,
) -> Vec<ShortReadSummary> {
    let mut messages = Vec::new();
    loop {
        let message = if mbox {
            if logged {
                parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
            } else {
                parser.parse_mbox_message()
            }
            .unwrap()
        } else {
            Some(
                if logged {
                    parser.parse_message_with_logger(&mut |_| Ok(()))
                } else {
                    parser.parse_message()
                }
                .unwrap(),
            )
        };
        let Some(message) = message else { break };
        messages.push(message);
        if !mbox {
            break;
        }
    }
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, wire.len() as u64);
    drop(parser);
    messages
        .into_iter()
        .map(|mut message| {
            let subject = message.subject().unwrap().text;
            let mut serialized = Vec::new();
            message
                .write_to_with_options(
                    &mut serialized,
                    &MimePartWriteOptions {
                        new_line_format: NewLineFormat::Unix,
                        ..Default::default()
                    },
                )
                .unwrap();
            let parts: Vec<_> = match message.body_mut().unwrap() {
                MimeEntity::Multipart(multipart) => multipart.children_mut().iter_mut().collect(),
                leaf => vec![leaf],
            };
            let bodies = parts
                .into_iter()
                .map(|part| {
                    let MimeEntity::Part(part) = part else {
                        panic!("expected leaf")
                    };
                    let mut decoded = Vec::new();
                    part.content_mut().decode_to(&mut decoded).unwrap();
                    decoded
                })
                .collect();
            (subject, bodies, serialized)
        })
        .collect()
}

#[test]
fn cr_lf_split_across_reads_is_not_bare_linefeed() {
    let body: String = (0..50)
        .map(|i| format!("line number {i} of the body with some text\r\n"))
        .collect();
    let wire = format!(
        "From: sender@example.org\r\nDate: Tue, 12 Nov 2013 09:12:42 -0500\r\n\
         Message-Id: <1@example.org>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain\r\n\r\n{body}"
    );
    assert_split_crlf(MimeParser::from_slice(wire.as_bytes()), &wire, &body);
    for chunk in [1, 2, 3, 5, 7, 11, 13, 4096] {
        assert_split_crlf(
            MimeParser::new(BufReader::with_capacity(chunk, wire.as_bytes())),
            &wire,
            &body,
        );
        for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
            let parser = MimeParser::with_storage(
                ShortSource {
                    inner: Cursor::new(wire.as_bytes()),
                    chunk,
                },
                storage,
            )
            .unwrap();
            assert_split_crlf(parser, &wire, &body);
        }
    }
}

fn assert_split_crlf<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    wire: &str,
    body: &str,
) {
    let mut issues = Vec::new();
    let mut message = parser
        .parse_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, wire.len() as u64);
    drop(parser);
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected text leaf")
    };
    assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Dos));
    assert_recovery_serialization(&mut message, wire, NewLineFormat::Dos);
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some(body)
    );
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
fn carriage_return_only_header_line_round_trip() {
    // A line made up of nothing but carriage returns is an invalid header,
    // not the blank line ending the header block. These fuzzer-found inputs
    // used to change structure on each round trip because the newline filter
    // stripped one of those CRs at a time.
    for (index, input) in [
        "Content-Type: message/rfc822\n\r\r\nContent-Type: multipart/mixed; boundary=x\n\n\n--x\n\r",
        "Content-Type: multipart/mixed; boundary=x\n\n--x\nContent-Type: text/plain\n\r\r\r\nX-Header: value\n\nbody\n--x--\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_structure_round_trips(input, StructureCase::CarriageReturns(index));
    }
}

#[test]
fn empty_multipart_alternative() {
    assert_structure_round_trips(
        include_str!("data/messages/empty-multipart.txt"),
        StructureCase::EmptyAlternative,
    );
}

#[test]
fn multipart_epilogue_with_text() {
    assert_structure_round_trips(
        include_str!("data/messages/epilogue.txt"),
        StructureCase::Epilogue,
    );
}

#[test]
fn issue358() {
    // This message has a badly folded x-microsoft-exchange-diagnostics value.
    // MimeKit's older parser mistook an already-full read-ahead buffer for EOF
    // and aborted header parsing. The MIME headers and body must still be read.
    assert_structure_round_trips(
        include_str!("data/messages/issue358.txt"),
        StructureCase::BadFold,
    );
}

#[test]
fn missing_message_body() {
    assert_structure_round_trips(
        concat!(
            "Date: Sat, 19 Apr 2014 13:13:23 -0700\r\n",
            "From: Jeffrey Stedfast <notifications@github.com>\r\n",
            "Subject: Re: [MimeKit] Allow parsing of message with 0 byte body. (#51)\r\n",
        ),
        StructureCase::MissingBody,
    );
}

#[test]
fn empty_message() {
    assert_structure_round_trips("\r\n", StructureCase::EmptyMessage);
}

#[derive(Clone, Copy)]
enum StructureCase {
    CarriageReturns(usize),
    EmptyAlternative,
    Epilogue,
    BadFold,
    MissingBody,
    EmptyMessage,
}

fn assert_structure_round_trips(original: &str, case: StructureCase) {
    for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
        // Parsed delimiter lines retain their original newlines. Normalize
        // fixture input, not those retained markers, for each output format.
        // Leave the CR-only fuzzer inputs untouched before their first parse.
        let input = if matches!(case, StructureCase::CarriageReturns(_)) {
            original.to_owned()
        } else {
            let newline = if format == NewLineFormat::Dos {
                "\r\n"
            } else {
                "\n"
            };
            original.replace("\r\n", "\n").replace('\n', newline)
        };
        for logged in [false, true] {
            assert_stable_structure(&input, |input| {
                let message =
                    parse_recovery_message(MimeParser::from_slice(input.as_bytes()), input, logged);
                serialize_structure_case(message, input, case, format)
            });
            for chunk in [1, 7, 4096] {
                assert_stable_structure(&input, |input| {
                    let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                    let message = parse_recovery_message(parser, input, logged);
                    serialize_structure_case(message, input, case, format)
                });
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    assert_stable_structure(&input, |input| {
                        let parser = MimeParser::with_storage(
                            ShortSource {
                                inner: Cursor::new(input.as_bytes()),
                                chunk,
                            },
                            storage,
                        )
                        .unwrap();
                        let message = parse_recovery_message(parser, input, logged);
                        serialize_structure_case(message, input, case, format)
                    });
                }
            }
        }
    }
}

fn assert_stable_structure(original: &str, mut round_trip: impl FnMut(&str) -> (String, Vec<u8>)) {
    let mut input = original.to_owned();
    let mut expected = None;
    for round in 0..=3 {
        let (structure, output) = round_trip(&input);
        if let Some(expected) = &expected {
            assert_eq!(&structure, expected, "structure changed on reparse {round}");
            assert_round_trip(
                &format!("output changed after reparse {round}"),
                input.as_bytes(),
                &output,
            );
        } else {
            expected = Some(structure);
        }
        input = String::from_utf8(output).unwrap();
    }
}

fn describe_structure<R>(message: &MimeMessage<R>) -> String {
    let mut output = format!("message headers={}\n", message.headers().len());
    let mut iter = message.iter();
    while let Some(entity) = iter.next() {
        let indent = "  ".repeat(iter.depth().unwrap());
        let content_type = entity
            .headers()
            .get("Content-Type")
            .map(|header| header.parse_content_type().unwrap());
        // MimeKit materializes the default text/plain Content-Type. The Rust
        // model leaves it absent; describe that same implicit leaf type here.
        let mime_type =
            content_type.map_or_else(|| "text/plain".to_owned(), |value| value.mime_type());
        writeln!(
            output,
            "{indent}{mime_type} headers={}",
            entity.headers().len()
        )
        .unwrap();
        if let MimeEntity::Message(part) = entity
            && let Some(message) = part.message()
        {
            writeln!(
                output,
                "{indent}  message headers={}",
                message.headers().len()
            )
            .unwrap();
        }
    }
    output
}

fn serialize_structure_case<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    case: StructureCase,
    format: NewLineFormat,
) -> (String, Vec<u8>) {
    let structure = describe_structure(&message);
    for headers in std::iter::once(message.headers()).chain(message.iter().map(MimeEntity::headers))
    {
        for header in headers {
            let offset = header.offset().unwrap() as usize;
            assert_eq!(
                header.raw(),
                &input.as_bytes()[offset..offset + header.raw().len()]
            );
        }
    }
    match case {
        StructureCase::CarriageReturns(0) => {
            assert_eq!(
                structure,
                "message headers=1\nmessage/rfc822 headers=2\n  message headers=0\n  text/plain headers=0\n"
            );
            let invalid = &message.headers().as_slice()[0];
            assert!(invalid.is_invalid());
            assert_eq!(invalid.raw(), b"\r\r\n");
            let MimeEntity::Message(wrapper) = message.body().unwrap() else {
                panic!("expected message/rfc822 wrapper")
            };
            assert!(matches!(
                wrapper.message().unwrap().body(),
                Some(MimeEntity::Part(_))
            ));
        }
        StructureCase::CarriageReturns(_) => {
            assert_eq!(
                structure,
                "message headers=0\nmultipart/mixed headers=1\n  text/plain headers=3\n"
            );
            let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                panic!("expected multipart/mixed")
            };
            let MimeEntity::Part(part) = &multipart.children()[0] else {
                panic!("expected text/plain child")
            };
            let invalid = &part.headers().as_slice()[1];
            assert!(invalid.is_invalid());
            assert_eq!(invalid.raw(), b"\r\r\r\n");
            assert_eq!(
                part.headers().get("X-Header").unwrap().decode_value().text,
                "value"
            );
        }
        StructureCase::EmptyAlternative => {
            assert_eq!(
                structure,
                "message headers=7\nmultipart/mixed headers=1\n  multipart/alternative headers=1\n  text/plain headers=1\n"
            );
            let MimeEntity::Multipart(outer) = message.body().unwrap() else {
                panic!("expected multipart/mixed")
            };
            assert_eq!(outer.children().len(), 2);
            let MimeEntity::Multipart(alternative) = &outer.children()[0] else {
                panic!("expected multipart/alternative")
            };
            assert!(alternative.children().is_empty());
            assert!(matches!(&outer.children()[1], MimeEntity::Part(_)));
        }
        StructureCase::Epilogue => {
            let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                panic!("expected multipart/mixed")
            };
            assert_eq!(multipart.children().len(), 2);
            let expected = concat!(
                "Peter Urka <pcu@umich.edu>\n",
                "Dept. of Chemistry, Univ. of Michigan\n",
                "Newt-thought is right-thought.  Go Newt!\n\n",
            );
            assert_eq!(
                multipart.epilogue_text().unwrap().replace("\r\n", "\n"),
                expected
            );
            // MimeKit's RawEpilogue includes the closing delimiter's newline.
            // Rust stores that newline with the delimiter, not the epilogue.
            assert!(multipart.epilogue().starts_with(b"Peter Urka"));
            assert_eq!(
                multipart.epilogue(),
                &input.as_bytes()[input.rfind("Peter Urka <pcu@umich.edu>").unwrap()..]
            );
        }
        StructureCase::BadFold => {
            // Make sure the top-level MIME part is multipart/alternative.
            let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
                panic!("expected multipart/alternative")
            };
            assert!(
                multipart
                    .content_type()
                    .unwrap()
                    .unwrap()
                    .is_mime_type("multipart", "alternative")
            );
            assert_eq!(multipart.children().len(), 2);
            for (child, subtype) in multipart.children().iter().zip(["plain", "html"]) {
                let MimeEntity::Part(part) = child else {
                    panic!("expected text/{subtype} child")
                };
                assert!(
                    part.content_type()
                        .unwrap()
                        .unwrap()
                        .is_mime_type("text", subtype)
                );
            }
            let headers = message.headers().as_slice();
            let index = headers
                .iter()
                .position(|header| header.matches_field("x-microsoft-exchange-diagnostics"))
                .unwrap();
            let invalid = &headers[index + 1];
            assert!(invalid.is_invalid());
            assert_eq!(
                invalid.offset(),
                Some(headers[index].offset().unwrap() + headers[index].raw().len() as u64)
            );
            assert!(message.headers().contains("X-Antivirus-Status"));
        }
        StructureCase::MissingBody => {
            assert_eq!(structure, "message headers=3\ntext/plain headers=0\n");
            assert!(matches!(message.body(), Some(MimeEntity::Part(_))));
        }
        StructureCase::EmptyMessage => {
            assert_eq!(structure, "message headers=0\ntext/plain headers=0\n");
            let MimeEntity::Part(part) = message.body().unwrap() else {
                panic!("expected empty leaf")
            };
            assert!(part.content_type().unwrap().is_none());
            assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
        }
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
    if !matches!(case, StructureCase::CarriageReturns(_)) {
        assert_round_trip("structure fixture", input.as_bytes(), &output);
    }
    // Mutable body access discards parsed framing; inspect content only after
    // verifying serialization of the unchanged model.
    if matches!(case, StructureCase::EmptyAlternative) {
        let MimeEntity::Multipart(multipart) = message.body_mut().unwrap() else {
            unreachable!()
        };
        let MimeEntity::Part(part) = &mut multipart.children_mut()[1] else {
            unreachable!()
        };
        let mut decoded = Vec::new();
        part.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap().replace("\r\n", "\n"),
            "This part should be on the same level as the multipart/alternative and not a child.\n"
        );
    }
    if matches!(
        case,
        StructureCase::MissingBody | StructureCase::EmptyMessage
    ) {
        let MimeEntity::Part(part) = message.body_mut().unwrap() else {
            unreachable!()
        };
        let mut decoded = Vec::new();
        part.content_mut().decode_to(&mut decoded).unwrap();
        assert!(decoded.is_empty());
    }
    (structure, output)
}

#[test]
fn deeply_nested_message_rfc822_parts() {
    assert_nested_depth_cases(false, 64, &[64, 65]);
}

#[test]
fn deeply_nested_multiparts() {
    assert_nested_depth_cases(true, 64, &[64, 65]);
}

#[test]
fn max_mime_depth_matches_reader() {
    // The reader's entity callbacks and the parser's model must agree at the
    // depth boundary. Checking only the deepest level and one below it can
    // hide off-by-one errors, so sweep the entire boundary as MimeKit does.
    // Rust counts the root and rejects excess depth instead of returning an
    // opaque truncated tree; zero is an invalid option, not a truncation level.
    for multipart in [false, true] {
        assert_nested_depth_cases(multipart, 5, &[1, 2, 3, 4, 5, 6, 7]);
    }
    let options = MimeEntityParseOptions {
        max_depth: 0,
        ..Default::default()
    };
    assert!(matches!(
        MimeReader::with_options(b"\r\n".as_slice(), options),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput
    ));
    assert!(matches!(
        MimeParser::with_options(b"\r\n".as_slice(), options),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput
    ));
}

#[test]
fn mime_compliance_mime_depth_limit_exceeded() {
    let wire = concat!(
        "From: sender@example.org\r\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\n",
        "Message-Id: <1@example.org>\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"a\"\r\n\r\n",
        "--a\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n",
        "--b\r\nContent-Type: multipart/mixed; boundary=\"c\"\r\n\r\n",
        "--c\r\nContent-Type: text/plain\r\n\r\nhidden\r\n--c--\r\n",
        "--b\r\nContent-Type: message/rfc822\r\n\r\n",
        "From: inner@example.org\r\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\n",
        "Message-Id: <2@example.org>\r\n",
        "Subject: also beyond the limit\r\n\r\nhidden\r\n",
        "--b--\r\n--a--\r\n",
    );
    use MimeEntityKind::{MessagePart, Multipart, Part};
    assert_mixed_depth_cases(
        wire,
        &[(2, Some("--b\r\n")), (3, Some("--c\r\n")), (4, None)],
        &[
            (Multipart, 0),
            (Multipart, 1),
            (Multipart, 2),
            (Part, 3),
            (MessagePart, 2),
            (Part, 3),
        ],
    );
}

#[test]
fn mime_compliance_mime_depth_limit_exceeded_implied_content_type() {
    let wire = concat!(
        "From: sender@example.org\r\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\n",
        "Message-Id: <1@example.org>\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"a\"\r\n\r\n",
        "--a\r\nContent-Type: multipart/digest; boundary=\"b\"\r\n\r\n",
        "--b\r\nX-Implied-Type: message/rfc822\r\n\r\n",
        "From: inner@example.org\r\n",
        "Date: Tue, 12 Nov 2013 09:12:42 -0500\r\n",
        "Message-Id: <2@example.org>\r\n",
        "Subject: beyond the limit\r\n\r\nhidden\r\n",
        "--b--\r\n--a--\r\n",
    );
    // message/rfc822 is implied by multipart/digest. The wrapper charges a
    // level at its opening delimiter, and its embedded root charges another
    // at the first inner header, even without an explicit Content-Type.
    use MimeEntityKind::{MessagePart, Multipart, Part};
    assert_mixed_depth_cases(
        wire,
        &[
            (2, Some("--b\r\n")),
            (3, Some("From: inner@example.org")),
            (4, None),
        ],
        &[(Multipart, 0), (Multipart, 1), (MessagePart, 2), (Part, 3)],
    );
}

fn assert_mixed_depth_cases(
    wire: &str,
    cases: &[(usize, Option<&str>)],
    expected: &[(MimeEntityKind, usize)],
) {
    for &(limit, marker) in cases {
        let options = MimeEntityParseOptions {
            max_depth: limit,
            ..Default::default()
        };
        for logged in [false, true] {
            assert_mixed_depth_parse(
                MimeParser::from_slice_with_options(wire.as_bytes(), options).unwrap(),
                wire,
                marker,
                expected,
                logged,
            );
            for chunk in [1, 7, 4096] {
                assert_mixed_depth_parse(
                    MimeParser::with_options(
                        BufReader::with_capacity(chunk, wire.as_bytes()),
                        options,
                    )
                    .unwrap(),
                    wire,
                    marker,
                    expected,
                    logged,
                );
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    assert_mixed_depth_parse(
                        MimeParser::with_storage_and_options(
                            ShortSource {
                                inner: Cursor::new(wire.as_bytes()),
                                chunk,
                            },
                            storage,
                            options,
                        )
                        .unwrap(),
                        wire,
                        marker,
                        expected,
                        logged,
                    );
                }
            }
        }
    }
}

fn assert_mixed_depth_parse<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    wire: &str,
    marker: Option<&str>,
    expected: &[(MimeEntityKind, usize)],
    logged: bool,
) {
    let mut issues = Vec::new();
    let result = if logged {
        parser.parse_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
    } else {
        parser.parse_message()
    };
    if let Some(marker) = marker {
        // Unlike MimeKit's flattened opaque leaf, Rust's root-inclusive
        // hard limit rejects the parse without returning a partial model.
        let error = result.err().expect("depth limit must reject the model");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "MIME entity depth limit exceeded");
        if logged {
            let offset = wire.find(marker).unwrap();
            assert_eq!(issues.len(), 1, "{issues:?}");
            let issue = &issues[0];
            assert_eq!(
                issue.violation,
                MimeComplianceViolation::MimeDepthLimitExceeded
            );
            assert_eq!(issue.stream_offset, offset as u64);
            assert_eq!(
                issue.line_number,
                1 + wire[..offset].bytes().filter(|&b| b == b'\n').count() as u64
            );
            assert_eq!(issue.column_number, 1);
            assert_eq!(
                issue.position_kind,
                MimeCompliancePositionKind::ElementStart
            );
        }
        assert!(!parser.is_end_of_stream());
        assert_eq!(
            parser.parse_message().err().unwrap().kind(),
            io::ErrorKind::InvalidInput,
        );
    } else {
        // At the limit, the entire tree is retained and nothing is reported.
        assert!(issues.is_empty(), "{issues:?}");
        let mut message = result.unwrap();
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position().offset, wire.len() as u64);
        drop(parser);
        let mut iter = message.iter();
        let mut actual = Vec::new();
        while let Some(entity) = iter.next() {
            let kind = match entity {
                MimeEntity::Multipart(_) => MimeEntityKind::Multipart,
                MimeEntity::Message(_) => MimeEntityKind::MessagePart,
                MimeEntity::Part(_) => MimeEntityKind::Part,
            };
            actual.push((kind, iter.depth().unwrap()));
        }
        assert_eq!(actual, expected);
        assert_recovery_serialization(&mut message, wire, NewLineFormat::Dos);
    }
}

#[test]
fn mime_parser_forwards_reader_properties() {
    let wire = b"From: sender@example.com\r\nThis is not a header\r\n\r\nBody\r\n";
    let options = MimeEntityParseOptions {
        max_depth: 17,
        ..Default::default()
    };
    let compliance = MimeComplianceOptions {
        context: MimeComplianceContext::Storage,
        ..Default::default()
    };
    let mut parser = MimeParser::with_options(wire.as_slice(), options).unwrap();
    parser.set_compliance_options(compliance);
    assert_eq!(parser.options().max_depth, 17);
    assert_eq!(*parser.compliance_options(), compliance);
    assert_eq!(parser.position(), MimePosition::default());
    assert!(!parser.is_end_of_stream());
    let mut issues = Vec::new();
    parser
        .parse_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
        .unwrap();
    assert!(!issues.is_empty());
    assert!(
        issues
            .iter()
            .all(|issue| issue.context == compliance.context)
    );
    assert_eq!(parser.position().offset, wire.len() as u64);
    assert!(parser.is_end_of_stream());
}

struct NestedDepthCase {
    input: String,
    body: String,
    header_offsets: Vec<usize>,
    multipart: bool,
    format: NewLineFormat,
}

fn nested_depth_case(multipart: bool, nesting: usize, format: NewLineFormat) -> NestedDepthCase {
    let newline = if format == NewLineFormat::Dos {
        "\r\n"
    } else {
        "\n"
    };
    let mut input = format!(
        "From: tester1@contoso.com{newline}\
         To: tester1@contoso.com{newline}\
         Subject: test of a deeply nested multipart message{newline}\
         Date: Tue, 12 Nov 2013 09:12:42 -0500{newline}\
         Message-ID: <root@contoso.com>{newline}\
         MIME-Version: 1.0{newline}"
    );
    let mut header_offsets = vec![0];
    for index in 0..nesting {
        if multipart {
            write!(
                input,
                "Content-Type: multipart/mixed;{newline}\
                 \tboundary=\"----=_NextPart_000_{index:04}_01CE98CE.6E826F90\"{newline}\
                 {newline}------=_NextPart_000_{index:04}_01CE98CE.6E826F90{newline}"
            )
            .unwrap();
        } else {
            write!(input, "Content-Type: message/rfc822{newline}{newline}").unwrap();
        }
        header_offsets.push(input.len());
        if !multipart {
            write!(
                input,
                "From: tester1@contoso.com{newline}\
                 To: tester1@contoso.com{newline}\
                 Subject: embedded message {}{newline}\
                 Date: Tue, 12 Nov 2013 08:12:42 -0500{newline}\
                 Message-ID: <embedded-{index}@contoso.com>{newline}\
                 MIME-Version: 1.0{newline}",
                index + 1
            )
            .unwrap();
        }
    }
    write!(
        input,
        "Content-Type: text/plain; charset=\"us-ascii\"{newline}\
         Content-Transfer-Encoding: 7bit{newline}{newline}"
    )
    .unwrap();
    let kind = if multipart { "multipart" } else { "rfc822" };
    let body = format!("This is the innermost part of a deeply nested {kind} message.{newline}");
    input.push_str(&body);
    if multipart {
        input.push_str(newline);
        for index in (0..nesting).rev() {
            write!(
                input,
                "------=_NextPart_000_{index:04}_01CE98CE.6E826F90--{newline}"
            )
            .unwrap();
        }
    }
    NestedDepthCase {
        input,
        body,
        header_offsets,
        multipart,
        format,
    }
}

fn assert_nested_depth_cases(multipart: bool, nesting: usize, limits: &[usize]) {
    for format in [NewLineFormat::Unix, NewLineFormat::Dos] {
        let case = nested_depth_case(multipart, nesting, format);
        for &limit in limits {
            let options = MimeEntityParseOptions {
                max_depth: limit,
                ..Default::default()
            };
            assert_nested_reader_depth(&case, options);
            for logged in [false, true] {
                let parser =
                    MimeParser::from_slice_with_options(case.input.as_bytes(), options).unwrap();
                if let Some(message) = parse_nested_depth_case(parser, &case, limit, logged) {
                    assert_stored_recovery_bodies(
                        message.leaf_parts(),
                        &case.input,
                        MimeContentStorage::Source,
                        [case.body.as_str()],
                    );
                    assert_nested_depth_model(message, &case);
                }
                for chunk in [1, 7, 4096] {
                    let parser = MimeParser::with_options(
                        BufReader::with_capacity(chunk, case.input.as_bytes()),
                        options,
                    )
                    .unwrap();
                    if let Some(message) = parse_nested_depth_case(parser, &case, limit, logged) {
                        assert_nested_depth_model(message, &case);
                    }
                    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                        let parser = MimeParser::with_storage_and_options(
                            ShortSource {
                                inner: Cursor::new(case.input.as_bytes()),
                                chunk,
                            },
                            storage,
                            options,
                        )
                        .unwrap();
                        if let Some(message) = parse_nested_depth_case(parser, &case, limit, logged)
                        {
                            assert_stored_recovery_bodies(
                                message.leaf_parts(),
                                &case.input,
                                storage,
                                [case.body.as_str()],
                            );
                            assert_nested_depth_model(message, &case);
                        }
                    }
                }
            }
        }
    }
}

fn assert_depth_issues(issues: &[MimeComplianceIssue], case: &NestedDepthCase, limit: usize) {
    let depth_issues: Vec<_> = issues
        .iter()
        .filter(|issue| issue.violation == MimeComplianceViolation::MimeDepthLimitExceeded)
        .collect();
    if limit >= case.header_offsets.len() {
        assert!(depth_issues.is_empty());
        return;
    }
    assert_eq!(depth_issues.len(), 1);
    let issue = depth_issues[0];
    let offset = if case.multipart {
        // An opening delimiter establishes the child before its headers are
        // consumed, so the hard limit is reported at that delimiter.
        case.input[..case.header_offsets[limit] - 1]
            .rfind('\n')
            .unwrap()
            + 1
    } else {
        case.header_offsets[limit]
    };
    assert_eq!(issue.stream_offset, offset as u64);
    assert_eq!(
        issue.line_number,
        1 + case.input.as_bytes()[..offset]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count() as u64,
    );
    assert_eq!(issue.column_number, 1);
    assert_eq!(
        issue.position_kind,
        MimeCompliancePositionKind::ElementStart
    );
}

fn assert_nested_reader_depth(case: &NestedDepthCase, options: MimeEntityParseOptions) {
    let mut reader = MimeReader::with_options(case.input.as_bytes(), options).unwrap();
    let mut issues = Vec::new();
    let mut entities = Vec::new();
    let result = reader.read_message_with_logger(
        &mut |issue| {
            issues.push(issue);
            Ok(())
        },
        |event| {
            if let MimeEvent::EntityBegin { kind, position, .. } = event {
                entities.push((kind, position.offset as usize));
            }
            Ok(())
        },
    );
    let expected: Vec<_> = case
        .header_offsets
        .iter()
        .enumerate()
        .take(options.max_depth)
        .map(|(index, &offset)| {
            let kind = if index + 1 == case.header_offsets.len() {
                MimeEntityKind::Part
            } else if case.multipart {
                MimeEntityKind::Multipart
            } else {
                MimeEntityKind::MessagePart
            };
            (kind, offset)
        })
        .collect();
    assert_eq!(entities, expected);
    assert_depth_issues(&issues, case, options.max_depth);
    if options.max_depth < case.header_offsets.len() {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "MIME entity depth limit exceeded");
        assert!(!reader.is_end_of_stream());
        assert_eq!(
            reader
                .read_message(|_| panic!("failed reader must not emit events"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput,
        );
    } else {
        result.unwrap();
        assert!(reader.is_end_of_stream());
        assert_eq!(reader.position().offset, case.input.len() as u64);
    }
}

fn parse_nested_depth_case<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    case: &NestedDepthCase,
    limit: usize,
    logged: bool,
) -> Option<MimeMessage<C>> {
    let mut issues = Vec::new();
    let result = if logged {
        parser.parse_message_with_logger(&mut |issue| {
            issues.push(issue);
            Ok(())
        })
    } else {
        parser.parse_message()
    };
    if logged {
        assert_depth_issues(&issues, case, limit);
    }
    if limit < case.header_offsets.len() {
        let error = result
            .err()
            .expect("excess nesting must not return a partial tree");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "MIME entity depth limit exceeded");
        assert!(!parser.is_end_of_stream());
        assert_eq!(
            parser.parse_message().err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        None
    } else {
        let message = result.unwrap();
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position().offset, case.input.len() as u64);
        Some(message)
    }
}

fn assert_nested_depth_model<R: Read + Seek>(mut message: MimeMessage<R>, case: &NestedDepthCase) {
    let nesting = case.header_offsets.len() - 1;
    assert_eq!(message.headers().len(), 6);
    let mut iter = message.iter();
    let mut count = 0;
    while let Some(entity) = iter.next() {
        assert_eq!(iter.depth(), Some(count));
        match entity {
            MimeEntity::Multipart(part) => {
                assert!(case.multipart && count < nesting);
                assert_eq!(part.children().len(), 1);
                assert!(part.preamble().is_empty());
                assert!(part.epilogue().is_empty());
                assert_eq!(part.headers().len(), 1);
            }
            MimeEntity::Message(part) => {
                assert!(!case.multipart && count < nesting);
                assert_eq!(part.headers().len(), 1);
                let inner = part.message().unwrap();
                assert_eq!(inner.headers().len(), 6);
                assert_eq!(
                    inner.subject().unwrap().text,
                    format!("embedded message {}", count + 1)
                );
                assert_eq!(
                    inner.headers().as_slice()[0].offset(),
                    Some(case.header_offsets[count + 1] as u64),
                );
            }
            MimeEntity::Part(part) => {
                assert_eq!(count, nesting);
                let content_type = part.content_type().unwrap().unwrap();
                assert!(content_type.is_mime_type("text", "plain"));
                assert_eq!(content_type.charset(), Some("us-ascii"));
                assert_eq!(part.content().encoding(), ContentEncoding::SevenBit);
                assert_eq!(part.headers().len(), 2);
            }
        }
        for header in entity.headers() {
            let offset = header.offset().unwrap() as usize;
            assert_eq!(
                header.raw(),
                &case.input.as_bytes()[offset..offset + header.raw().len()]
            );
        }
        count += 1;
    }
    assert_eq!(count, nesting + 1);
    assert_eq!(message.leaf_parts().count(), 1);
    assert_recovery_serialization(&mut message, &case.input, case.format);
    let mut entity = message.body_mut().unwrap();
    loop {
        entity = match entity {
            MimeEntity::Multipart(part) => &mut part.children_mut()[0],
            MimeEntity::Message(part) => part.message_mut().unwrap().body_mut().unwrap(),
            MimeEntity::Part(part) => {
                let mut decoded = Vec::new();
                part.content_mut().decode_to(&mut decoded).unwrap();
                assert_eq!(decoded, case.body.as_bytes());
                break;
            }
        };
    }
}

#[test]
fn invalid_content_type() {
    for mime_type in ["garbage", "!%^#&^!\\t  "] {
        assert_leaf_header_recovery(
            "test of recovery from invalid media-type in Content-Type header",
            &format!("Content-Type: {mime_type}; charset=utf-8\n"),
            |message_headers, part_headers| {
                assert_eq!(message_headers.len(), 7);
                assert_eq!(part_headers.len(), 1);
                let header = part_headers.get("Content-Type").unwrap();
                // The raw Rust model reports invalid metadata instead of
                // exposing MimeKit's application/octet-stream fallback.
                // Recovery must still retain its parameters and payload.
                assert!(header.parse_content_type().is_err());
                assert!(header.decode_value().text.ends_with("; charset=utf-8"));
            },
        );
    }
}

#[test]
fn missing_subtype() {
    let input = include_str!("data/messages/missing-subtype.txt").replace("\r\n", "\n");
    assert_leaf_message_recovery(&input, |message_headers, part_headers| {
        assert_eq!(message_headers.len(), 5);
        assert_eq!(part_headers.len(), 1);
        let header = part_headers.get("Content-Type").unwrap();
        // MimeKit exposes application/octet-stream with the recovered name.
        // Rust's typed getter remains strict; retain the malformed type and
        // its filename parameter in the raw header rather than rewriting it.
        assert!(header.parse_content_type().is_err());
        assert_eq!(
            header.decode_value().text,
            "application-x-gzip; name=\"document.xml.gz\""
        );
    });
}

#[test]
fn header_field_name_begins_with_colon() {
    assert_leaf_header_recovery(
        "test of a header line starting with ':'",
        "Content-Type: text/plain; charset=utf-8\n: What header is this?\n",
        |message_headers, part_headers| {
            assert_eq!(message_headers.len(), 8);
            assert_eq!(part_headers.len(), 1);
            let header = message_headers.as_slice().last().unwrap();
            // FIXME: Should this really be "valid"? Matches MimeKit.
            assert!(!header.is_invalid());
            assert_eq!(header.field(), Some(""));
            assert_eq!(header.decode_value().text, "What header is this?");
            assert_plain_utf8_headers(part_headers);
        },
    );
}

#[test]
fn header_field_name_colon_colon() {
    assert_leaf_header_recovery(
        "test of a Content-Transfer-Encoding header with double ':'s",
        concat!(
            "Content-Type: text/plain; charset=utf-8\n",
            "Content-Transfer-Encoding:: base64\n",
            "Content-Disposition: inline; name=body.txt\n",
        ),
        |message_headers, part_headers| {
            assert_eq!(message_headers.len(), 7);
            assert_eq!(part_headers.len(), 3);
            let header = &part_headers.as_slice()[1];
            assert_eq!(header.id(), HeaderId::ContentTransferEncoding);
            assert!(!header.is_invalid());
            assert_eq!(header.decode_value().text, ": base64");
            assert_plain_utf8_headers(part_headers);
            let disposition = part_headers
                .get("Content-Disposition")
                .unwrap()
                .parse_content_disposition()
                .unwrap();
            assert_eq!(disposition.disposition(), "inline");
            assert_eq!(disposition.parameters().get("name"), Some("body.txt"));
        },
    );
}

fn assert_plain_utf8_headers(headers: &HeaderList) {
    let content_type = headers
        .get("Content-Type")
        .unwrap()
        .parse_content_type()
        .unwrap();
    assert!(content_type.is_mime_type("text", "plain"));
    assert_eq!(content_type.charset(), Some("utf-8"));
}

fn assert_leaf_header_recovery(
    subject: &str,
    headers: &str,
    inspect: fn(&HeaderList, &HeaderList),
) {
    let original = format!(
        "From: mimekit@example.com\n\
         To: mimekit@example.com\n\
         Subject: {subject}\n\
         Date: Tue, 12 Nov 2013 09:12:42 -0500\n\
         MIME-Version: 1.0\n\
         Message-ID: <54AD68C9E3B0184CAC6041320424FD1B5B81E74D@localhost.localdomain>\n\
         X-Mailer: Microsoft Office Outlook 12.0\n\
         {headers}\nThis is the message body.\n"
    );
    assert_leaf_message_recovery(&original, inspect);
}

fn assert_leaf_message_recovery(original: &str, inspect: fn(&HeaderList, &HeaderList)) {
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let (_, body) = input.split_once(&newline.repeat(2)).unwrap();
        for logged in [false, true] {
            let message =
                parse_recovery_message(MimeParser::from_slice(input.as_bytes()), &input, logged);
            assert_stored_recovery_bodies(
                message.leaf_parts(),
                &input,
                MimeContentStorage::Source,
                [body],
            );
            assert_leaf_header_model(message, &input, body, format, inspect);
            for chunk in [1, 2, 7, 4096] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_recovery_message(parser, &input, logged);
                assert_leaf_header_model(message, &input, body, format, inspect);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_recovery_message(parser, &input, logged);
                    assert_stored_recovery_bodies(message.leaf_parts(), &input, storage, [body]);
                    assert_leaf_header_model(message, &input, body, format, inspect);
                }
            }
        }
    }
}

fn assert_leaf_header_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    body: &str,
    format: NewLineFormat,
    inspect: fn(&HeaderList, &HeaderList),
) {
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected top-level leaf")
    };
    inspect(message.headers(), part.headers());
    for header in message.headers().iter().chain(part.headers().iter()) {
        let offset = header.offset().unwrap() as usize;
        assert_eq!(
            header.raw(),
            &input.as_bytes()[offset..offset + header.raw().len()]
        );
    }
    assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
    assert_eq!(part.content().new_line_format(), Some(format));
    if part.headers().contains("Content-Transfer-Encoding") {
        // The typed getter stays strict, while stored content uses MimeKit's
        // unspecified-encoding recovery for the unchanged malformed header.
        assert!(part.content_transfer_encoding().is_err());
    }
    assert_recovery_serialization(&mut message, input, format);
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        unreachable!()
    };
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, body.as_bytes());
}

#[test]
fn garbage_before_message_headers() {
    assert_leading_garbage(
        concat!(
            ">F!&^#%&^\n",
            "From: mimekit@example.com\n",
            "To: mimekit@example.com\n",
            "Subject: This message has garbage before the headers\n",
            "Date: Tue, 12 Nov 2013 09:12:42 -0500\n",
            "MIME-Version: 1.0\n",
            "Content-Type: text/plain; charset=utf-8\n\n",
            "This is the message body.\n",
        ),
        true,
    );
}

#[test]
fn garbage_before_entity_headers() {
    assert_leading_garbage(
        concat!(
            ">F!&^#%&^\n",
            "MIME-Version: 1.0\n",
            "Content-Type: text/plain; charset=utf-8\n\n",
            "This is the message body.\n",
        ),
        false,
    );
}

fn assert_leading_garbage(original: &str, message: bool) {
    for newline in ["\n", "\r\n"] {
        let input = original.replace('\n', newline);
        for logged in [false, true] {
            assert_leading_garbage_error(MimeParser::from_slice(input.as_bytes()), message, logged);
            for chunk in [1, 2, 7, 4096] {
                assert_leading_garbage_error(
                    MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes())),
                    message,
                    logged,
                );
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    assert_leading_garbage_error(parser, message, logged);
                }
            }
        }
    }
}

fn assert_leading_garbage_error<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    message: bool,
    logged: bool,
) {
    let mut logger = |_| Ok(());
    let error = match (message, logged) {
        (true, true) => parser.parse_message_with_logger(&mut logger).err(),
        (true, false) => parser.parse_message().err(),
        (false, true) => parser.parse_entity_with_logger(&mut logger).err(),
        (false, false) => parser.parse_entity().err(),
    }
    .expect("leading garbage must not produce a MIME model");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "invalid first MIME header");
}

#[test]
fn parsing_garbage_mbox() {
    assert_garbage_stream(MimeEventFormat::Mbox);
}

#[test]
fn parsing_garbage_entity() {
    assert_garbage_stream(MimeEventFormat::Entity);
}

#[test]
fn parsing_garbage_message() {
    assert_garbage_stream(MimeEventFormat::Message);
}

fn assert_garbage_stream(format: MimeEventFormat) {
    for newline in ["\n", "\r\n"] {
        let line = format!(
            "This is just a standard test file... nothing to see here. No MIME anywhere to be found{newline}"
        );
        assert_empty_or_garbage_stream(line.repeat(200).as_bytes(), format);
    }
}

#[test]
fn empty_mbox_stream() {
    assert_empty_or_garbage_stream(b"", MimeEventFormat::Mbox);
}

#[test]
fn empty_entity_stream() {
    assert_empty_or_garbage_stream(b"", MimeEventFormat::Entity);
}

#[test]
fn empty_message_stream() {
    assert_empty_or_garbage_stream(b"", MimeEventFormat::Message);
}

fn assert_empty_or_garbage_stream(input: &[u8], format: MimeEventFormat) {
    for logged in [false, true] {
        assert_empty_or_garbage_parser(MimeParser::from_slice(input), input, format, logged);
        for chunk in [1, 2, 7, 4096] {
            assert_empty_or_garbage_parser(
                MimeParser::new(BufReader::with_capacity(chunk, input)),
                input,
                format,
                logged,
            );
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let parser = MimeParser::with_storage(
                    ShortSource {
                        inner: Cursor::new(input),
                        chunk,
                    },
                    storage,
                )
                .unwrap();
                assert_empty_or_garbage_parser(parser, input, format, logged);
            }
        }
    }
}

fn assert_empty_or_garbage_parser<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &[u8],
    format: MimeEventFormat,
    logged: bool,
) {
    let mut logger = |_| Ok(());
    if input.is_empty() && format == MimeEventFormat::Mbox {
        // MimeKit throws at an empty archive. Rust's iterator-style API
        // distinguishes clean exhaustion (None) from malformed mbox input.
        let message = if logged {
            parser.parse_mbox_message_with_logger(&mut logger)
        } else {
            parser.parse_mbox_message()
        }
        .unwrap();
        assert!(message.is_none());
        assert!(parser.parse_mbox_message().unwrap().is_none());
        assert!(parser.is_end_of_stream());
        assert_eq!(parser.position(), MimePosition::default());
        assert!(parser.pending_input().is_empty());
        return;
    }

    let error = match (format, logged) {
        (MimeEventFormat::Entity, false) => parser.parse_entity().err(),
        (MimeEventFormat::Entity, true) => parser.parse_entity_with_logger(&mut logger).err(),
        (MimeEventFormat::Message, false) => parser.parse_message().err(),
        (MimeEventFormat::Message, true) => parser.parse_message_with_logger(&mut logger).err(),
        (MimeEventFormat::Mbox, false) => parser.parse_mbox_message().err(),
        (MimeEventFormat::Mbox, true) => parser.parse_mbox_message_with_logger(&mut logger).err(),
    }
    .expect("empty or garbage input must not produce a MIME model");
    assert_eq!(
        error.kind(),
        if input.is_empty() {
            io::ErrorKind::UnexpectedEof
        } else {
            io::ErrorKind::InvalidData
        }
    );
    if format == MimeEventFormat::Mbox {
        assert_eq!(error.to_string(), "missing mbox marker");
        assert_eq!(
            parser.position(),
            MimePosition {
                offset: input.len() as u64,
                line: 1 + input.iter().filter(|&&byte| byte == b'\n').count() as u64,
            }
        );
    } else if input.is_empty() {
        assert_eq!(parser.position(), MimePosition::default());
    } else {
        assert_eq!(error.to_string(), "invalid first MIME header");
    }
    assert!(!parser.is_end_of_stream());
    let error = match format {
        MimeEventFormat::Entity => parser.parse_entity().err(),
        MimeEventFormat::Message => parser.parse_message().err(),
        MimeEventFormat::Mbox => parser.parse_mbox_message().err(),
    }
    .expect("a failed source must not be reused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn simple_mbox_with_garbage_before_mbox_marker() {
    let archive = include_str!("data/mbox/simple.mbox.txt").replace("\r\n", "\n");
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        for garbage_length in [
            80, 4094, // The initial marker needs more data across the 4096-byte refill.
            4096, 4097, // Marker search resumes in the middle of the garbage line.
            8193, // Midline marker search needs another refill.
        ] {
            let input = format!(
                "{}{newline}{}",
                "X".repeat(garbage_length - newline.len()),
                archive.replace('\n', newline)
            );
            for logged in [false, true] {
                assert_simple_mbox_after_garbage(
                    MimeParser::from_slice(input.as_bytes()),
                    &input,
                    garbage_length,
                    format,
                    logged,
                );
                for chunk in [1, 7, 4096] {
                    assert_simple_mbox_after_garbage(
                        MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes())),
                        &input,
                        garbage_length,
                        format,
                        logged,
                    );
                    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                        let parser = MimeParser::with_storage(
                            ShortSource {
                                inner: Cursor::new(input.as_bytes()),
                                chunk,
                            },
                            storage,
                        )
                        .unwrap();
                        assert_simple_mbox_after_garbage(
                            parser,
                            &input,
                            garbage_length,
                            format,
                            logged,
                        );
                    }
                }
            }
        }
    }
}

fn assert_simple_mbox_after_garbage<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    garbage_length: usize,
    format: NewLineFormat,
    logged: bool,
) {
    let mut messages = Vec::new();
    for _ in 0..3 {
        let message = if logged {
            parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
        } else {
            parser.parse_mbox_message()
        }
        .unwrap()
        .unwrap();
        messages.push(message);
    }
    assert!(parser.parse_mbox_message().unwrap().is_none());
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    drop(parser);
    let newline_len = if format == NewLineFormat::Dos { 2 } else { 1 };
    let markers: Vec<_> = input
        .match_indices("From - ")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(markers[0], garbage_length);
    for (index, mut message) in messages.into_iter().enumerate() {
        let start = markers[index] + "From - ".len() + newline_len;
        let end = markers
            .get(index + 1)
            .map_or(input.len(), |offset| offset - newline_len);
        assert_eq!(message.headers().as_slice()[0].offset(), Some(start as u64));
        for headers in
            std::iter::once(message.headers()).chain(message.iter().map(MimeEntity::headers))
        {
            for header in headers {
                let offset = header.offset().unwrap() as usize;
                assert_eq!(
                    header.raw(),
                    &input.as_bytes()[offset..offset + header.raw().len()]
                );
            }
        }
        assert_recovery_serialization(&mut message, &input[start..end], format);
        let mut entity = message.body_mut().unwrap();
        for _ in 0..3 {
            let MimeEntity::Multipart(multipart) = entity else {
                panic!("expected three nested multiparts")
            };
            assert_eq!(multipart.children().len(), 1);
            entity = &mut multipart.children_mut()[0];
        }
        let MimeEntity::Part(part) = entity else {
            panic!("expected innermost text/plain leaf")
        };
        assert!(
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("text", "plain")
        );
        let mut output = Vec::new();
        part.write_to_with_options(
            &mut output,
            &MimePartWriteOptions {
                new_line_format: NewLineFormat::Unix,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            output.starts_with(b"Content-Type: text/plain\n\n"),
            "headers are not properly terminated"
        );
        let header = &part.headers().as_slice()[0];
        let body_start = header.offset().unwrap() as usize + header.raw().len() + newline_len;
        let mut decoded = Vec::new();
        part.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, &input.as_bytes()[body_start..end]);
    }
}

#[test]
fn really_long_mbox_marker() {
    let marker = format!("From {}", "X".repeat(4092));
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = format!(
            "{marker}{newline}From: sender@example.com{newline}\
             To: recipient@example.com{newline}Subject: test message{newline}\
             {newline}Body text{newline}"
        );
        let body = &input[input.find("Body text").unwrap()..];
        for logged in [false, true] {
            let message = parse_recovery_mbox_message(
                MimeParser::from_slice(input.as_bytes()),
                &input,
                logged,
            );
            assert_stored_recovery_bodies(
                message.leaf_parts(),
                &input,
                MimeContentStorage::Source,
                [body],
            );
            assert_long_marker_model(message, &input, format);
            for chunk in [1, 2, 7, 4095, 4096, 4097, 8192] {
                let message = parse_recovery_mbox_message(
                    MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes())),
                    &input,
                    logged,
                );
                assert_long_marker_model(message, &input, format);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_recovery_mbox_message(parser, &input, logged);
                    assert_stored_recovery_bodies(message.leaf_parts(), &input, storage, [body]);
                    assert_long_marker_model(message, &input, format);
                }
            }
        }
        // Rust exposes the complete marker, including its newline, as an event
        // rather than a MimeParser property or part of the stored message.
        let marker_end = marker.len() + newline.len();
        for chunk in [1, 2, 7, 4095, 4096, 4097, 8192] {
            for limit in [marker_end - 1, marker_end] {
                let mut reader = MimeReader::with_options(
                    BufReader::with_capacity(chunk, input.as_bytes()),
                    MimeEntityParseOptions {
                        max_header_length: limit,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut markers = 0;
                let result = reader.read_mbox_message(|event| {
                    if let MimeEvent::MboxMarker {
                        bytes,
                        position,
                        end,
                    } = event
                    {
                        markers += 1;
                        assert_eq!(bytes, &input.as_bytes()[..marker_end]);
                        assert_eq!(position, MimePosition::default());
                        assert_eq!(
                            end,
                            MimePosition {
                                offset: marker_end as u64,
                                line: 2,
                            }
                        );
                    }
                    Ok(())
                });
                if limit == marker_end {
                    assert!(result.unwrap());
                    assert_eq!(markers, 1);
                    assert!(reader.is_end_of_stream());
                    assert_eq!(reader.position().offset, input.len() as u64);
                } else {
                    let error = result.unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    assert_eq!(error.to_string(), "mbox marker length limit exceeded");
                    assert_eq!(markers, 0);
                    assert_eq!(reader.position().offset, limit as u64);
                    assert!(!reader.is_end_of_stream());
                }
            }
        }
    }
}

fn parse_recovery_mbox_message<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) -> MimeMessage<C> {
    let message = if logged {
        parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_mbox_message()
    }
    .unwrap()
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert!(parser.parse_mbox_message().unwrap().is_none());
    assert!(parser.pending_input().is_empty());
    assert_eq!(parser.position().offset, input.len() as u64);
    message
}

fn assert_long_marker_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
) {
    let start = input.find('\n').unwrap() + 1;
    assert_eq!(message.headers().len(), 3);
    assert_eq!(message.headers().as_slice()[0].offset(), Some(start as u64));
    assert_eq!(message.subject().unwrap().text, "test message");
    for header in message.headers() {
        let offset = header.offset().unwrap() as usize;
        assert_eq!(
            header.raw(),
            &input.as_bytes()[offset..offset + header.raw().len()]
        );
    }
    let body = &input[input.find("Body text").unwrap()..];
    assert_recovery_serialization(&mut message, &input[start..], format);
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        panic!("expected text leaf")
    };
    assert!(part.headers().is_empty());
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, body.as_bytes());
}

#[test]
fn message_byte_order_mark() {
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let body = format!("\u{feff}body{newline}");
        let input = format!("\u{feff}X: y{newline}{newline}{body}");
        for max_input_bytes in [1, 2, 3, input.len() - 1, input.len()] {
            let options = MimeEntityParseOptions {
                max_input_bytes,
                // The BOM counts toward the input budget, not the header length.
                max_header_length: 4 + newline.len(),
                ..Default::default()
            };
            for logged in [false, true] {
                let parser =
                    MimeParser::from_slice_with_options(input.as_bytes(), options).unwrap();
                assert_message_bom_parser(parser, &input, &body, format, logged, max_input_bytes);
                for chunk in [1, 2, 3, 7, 4096] {
                    let parser = MimeParser::with_options(
                        BufReader::with_capacity(chunk, input.as_bytes()),
                        options,
                    )
                    .unwrap();
                    assert_message_bom_parser(
                        parser,
                        &input,
                        &body,
                        format,
                        logged,
                        max_input_bytes,
                    );
                    for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                        let parser = MimeParser::with_storage_and_options(
                            ShortSource {
                                inner: Cursor::new(input.as_bytes()),
                                chunk,
                            },
                            storage,
                            options,
                        )
                        .unwrap();
                        assert_message_bom_parser(
                            parser,
                            &input,
                            &body,
                            format,
                            logged,
                            max_input_bytes,
                        );
                    }
                }
            }
        }
    }
}

fn assert_message_bom_parser<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    body: &str,
    format: NewLineFormat,
    logged: bool,
    limit: usize,
) {
    let result = if logged {
        parser.parse_message_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_message()
    };
    if limit < input.len() {
        let error = result
            .err()
            .expect("BOM bytes must count toward the input limit");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "MIME input byte limit exceeded");
        assert_eq!(parser.position().offset, limit as u64);
        assert!(!parser.is_end_of_stream());
        assert_eq!(
            parser.parse_message().err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        return;
    }
    let mut message = result.unwrap();
    assert_eq!(parser.position().offset, input.len() as u64);
    assert!(parser.is_end_of_stream());
    drop(parser);
    assert_eq!(message.headers().len(), 1);
    let header = message.headers().get("X").unwrap();
    assert_eq!(header.offset(), Some(3));
    assert_eq!(header.decode_value().text, "y");
    // The source BOM is not part of the message model; a BOM inside the
    // content is ordinary data and must survive decoding and writing.
    assert_recovery_serialization(&mut message, &input[3..], format);
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        panic!("expected default text/plain leaf")
    };
    assert!(part.headers().is_empty());
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, body.as_bytes());
}

#[test]
fn partial_byte_order_mark_eof() {
    for prefix in [b"\xef".as_slice(), b"\xef\xbb"] {
        assert_message_bom_failure(prefix, io::ErrorKind::UnexpectedEof, prefix.len());
    }
}

#[test]
fn partial_byte_order_mark() {
    let archive = include_bytes!("data/mbox/simple.mbox.txt");
    for prefix in [b"\xef".as_slice(), b"\xef\xbb"] {
        let input = [prefix, archive].concat();
        assert_message_bom_failure(&input, io::ErrorKind::InvalidData, prefix.len());
    }
}

fn assert_message_bom_failure(input: &[u8], kind: io::ErrorKind, consumed: usize) {
    for logged in [false, true] {
        assert_message_bom_failure_parser(MimeParser::from_slice(input), kind, consumed, logged);
        for chunk in [1, 2, 7, 4096] {
            assert_message_bom_failure_parser(
                MimeParser::new(BufReader::with_capacity(chunk, input)),
                kind,
                consumed,
                logged,
            );
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let parser = MimeParser::with_storage(
                    ShortSource {
                        inner: Cursor::new(input),
                        chunk,
                    },
                    storage,
                )
                .unwrap();
                assert_message_bom_failure_parser(parser, kind, consumed, logged);
            }
        }
    }
}

fn assert_message_bom_failure_parser<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    kind: io::ErrorKind,
    consumed: usize,
    logged: bool,
) {
    let result = if logged {
        parser.parse_message_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_message()
    };
    let error = result
        .err()
        .expect("partial BOM must not produce a message");
    assert_eq!(error.kind(), kind);
    assert_eq!(error.to_string(), "incomplete UTF-8 byte order mark");
    assert_eq!(
        parser.position(),
        MimePosition {
            offset: consumed as u64,
            line: 1
        }
    );
    assert!(!parser.is_end_of_stream());
    assert_eq!(
        parser.parse_message().err().unwrap().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn byte_order_mark_eof() {
    assert_marker_eof(b"\xef\xbb\xbf", false);
}

#[test]
fn byte_order_mark_does_not_change_entity_or_raw_header_parsing() {
    let input = b"\xef\xbb\xbfSubject: test\r\n\r\nbody";
    for chunk in [1, 2, 7, 4096] {
        let mut parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_slice()));
        assert_eq!(
            parser.parse_entity().err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
        let mut parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_slice()));
        let headers = parser.parse_headers().unwrap();
        assert_eq!(headers.len(), 1);
        let header = &headers.as_slice()[0];
        assert!(header.is_invalid());
        assert_eq!(header.offset(), Some(0));
        assert_eq!(header.raw(), b"\xef\xbb\xbfSubject: test\r\n");
    }
}

#[test]
fn message_byte_order_mark_is_not_repeated_for_nested_messages() {
    let input = "\u{feff}Content-Type: message/rfc822\n\n\u{feff}X: y\n\nbody";
    let message = MimeParser::from_slice(input.as_bytes())
        .parse_message()
        .unwrap();
    let MimeEntity::Message(wrapper) = message.body().unwrap() else {
        panic!("expected message/rfc822 wrapper")
    };
    let headers = wrapper.message().unwrap().headers();
    assert_eq!(headers.len(), 1);
    let header = &headers.as_slice()[0];
    assert!(header.is_invalid());
    assert_eq!(header.raw(), b"\xef\xbb\xbfX: y\n");
    assert_eq!(
        header.offset(),
        Some(input.rfind('\u{feff}').unwrap() as u64)
    );
}

#[test]
fn pre_header_buffer_resizes_for_multiple_invalid_leading_headers() {
    let original = concat!(
        "Content-Type: message/rfc822\r\n\r\n",
        ">From first preheader\r\n",
        ">From second preheader line is longer\r\n",
        "From: sender@example.org\r\n",
        "Subject: preheader resize\r\n\r\nbody\r\n",
    );
    assert_leading_marker_recovery(original, true, 2, Some("preheader resize"));
    for padding in [31, 4090, 8193] {
        let input = original.replace(
            ">From second preheader line is longer",
            &format!(
                ">From second preheader line is longer {}",
                "x".repeat(padding)
            ),
        );
        assert_leading_marker_recovery(&input, true, 2, Some("preheader resize"));
    }
}

#[test]
fn munged_mbox_markers_before_headers() {
    assert_leading_marker_recovery(
        "From - \r\n>From munged\r\nSubject: test\r\n\r\nbody\r\n",
        false,
        2,
        Some("test"),
    );
}

#[test]
fn message_stream_with_mbox_marker_followed_by_message() {
    for (tail, subject) in [("Subject: test", Some("test")), ("\r\n", None)] {
        let input = format!("From sender@example.com Mon Jan  1 00:00:00 2024\r\n{tail}");
        assert_leading_marker_recovery(&input, false, 1, subject);
    }
}

fn assert_leading_marker_recovery(
    original: &str,
    embedded: bool,
    invalid_headers: usize,
    subject: Option<&str>,
) {
    let original = original.replace("\r\n", "\n");
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let separator = newline.repeat(2);
        let body = input.rsplit_once(&separator).map_or("", |(_, body)| body);
        for logged in [false, true] {
            let message =
                parse_recovery_message(MimeParser::from_slice(input.as_bytes()), &input, logged);
            assert_stored_recovery_bodies(
                message.leaf_parts(),
                &input,
                MimeContentStorage::Source,
                [body],
            );
            assert_leading_marker_model(
                message,
                &input,
                format,
                embedded,
                invalid_headers,
                subject,
            );
            for chunk in [1, 2, 7, 4096] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_recovery_message(parser, &input, logged);
                assert_leading_marker_model(
                    message,
                    &input,
                    format,
                    embedded,
                    invalid_headers,
                    subject,
                );
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_recovery_message(parser, &input, logged);
                    assert_stored_recovery_bodies(message.leaf_parts(), &input, storage, [body]);
                    assert_leading_marker_model(
                        message,
                        &input,
                        format,
                        embedded,
                        invalid_headers,
                        subject,
                    );
                }
            }
        }
    }
}

fn assert_leading_marker_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
    embedded: bool,
    invalid_headers: usize,
    subject: Option<&str>,
) {
    let newline = if format == NewLineFormat::Dos {
        "\r\n"
    } else {
        "\n"
    };
    let separator = newline.repeat(2);
    let (inner, mut offset) = if embedded {
        assert!(message.headers().is_empty());
        let MimeEntity::Message(wrapper) = message.body().unwrap() else {
            panic!("expected message/rfc822 wrapper")
        };
        assert_eq!(wrapper.headers().len(), 1);
        assert!(
            wrapper
                .content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("message", "rfc822")
        );
        let header = wrapper.headers().get("Content-Type").unwrap();
        assert_eq!(header.offset(), Some(0));
        assert_eq!(
            header.raw(),
            format!("Content-Type: message/rfc822{newline}").as_bytes()
        );
        (
            wrapper.message().unwrap(),
            input.find(&separator).unwrap() + separator.len(),
        )
    } else {
        (&message, 0)
    };
    assert_eq!(
        inner.headers().len(),
        invalid_headers + usize::from(subject.is_some()) + usize::from(embedded)
    );
    assert_eq!(inner.headers().contains("Subject"), subject.is_some());
    if let Some(subject) = subject {
        assert_eq!(inner.subject().unwrap().text, subject);
    }
    if embedded {
        assert_eq!(
            inner.headers().get("From").unwrap().decode_value().text,
            "sender@example.org"
        );
    }
    // MimeKit accumulates invalid leading records in Message.MboxMarker.
    // The Rust raw-header model retains them in order instead, including
    // their newlines, so growing preheaders must not lose or rewrite bytes.
    for (index, header) in inner.headers().iter().enumerate() {
        let line = input[offset..].split_inclusive('\n').next().unwrap();
        assert_eq!(header.offset(), Some(offset as u64));
        assert_eq!(header.raw(), line.as_bytes());
        assert_eq!(header.is_invalid(), index < invalid_headers);
        if header.is_invalid() {
            assert!(header.raw_value().is_empty());
            assert_eq!(header.id(), HeaderId::Unknown);
        }
        offset += line.len();
    }
    let MimeEntity::Part(part) = inner.body().unwrap() else {
        panic!("expected default leaf")
    };
    assert!(part.headers().is_empty());
    assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
    assert_eq!(message.leaf_parts().count(), 1);
    // Unlike the actual marker consumed by mbox iteration, a leading
    // From-line parsed as an invalid header remains in serialized output.
    if input.ends_with('\n') {
        assert_recovery_serialization(&mut message, input, format);
    } else {
        assert_header_only_serialization(&mut message, input, format);
    }
    let mut entity = message.body_mut().unwrap();
    if embedded {
        let MimeEntity::Message(wrapper) = entity else {
            unreachable!()
        };
        entity = wrapper.message_mut().unwrap().body_mut().unwrap();
    }
    let MimeEntity::Part(part) = entity else {
        unreachable!()
    };
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    let body = input.rsplit_once(&separator).map_or("", |(_, body)| body);
    assert_eq!(decoded, body.as_bytes());
}

#[test]
fn double_mbox_marker() {
    let original = concat!(
        "From - \nFrom -\nFrom: sender@example.com\n",
        "To: recipient@example.com\nSubject: test message\n\nBody text\n",
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        for logged in [false, true] {
            assert_double_mbox_marker(
                MimeParser::from_slice(input.as_bytes()),
                &input,
                format,
                logged,
            );
            for chunk in [1, 2, 7, 4096] {
                assert_double_mbox_marker(
                    MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes())),
                    &input,
                    format,
                    logged,
                );
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    assert_double_mbox_marker(parser, &input, format, logged);
                }
            }
        }
    }
}

fn assert_double_mbox_marker<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    format: NewLineFormat,
    logged: bool,
) {
    let first_end = input.find('\n').unwrap() + 1;
    let second_start = first_end + input[first_end..].find('\n').unwrap() + 1;
    let mut messages = Vec::new();
    for (index, end) in [first_end, input.len()].into_iter().enumerate() {
        let message = if logged {
            parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
        } else {
            parser.parse_mbox_message()
        }
        .unwrap()
        .unwrap();
        let pending = if index == 0 { b"From ".as_slice() } else { b"" };
        assert_eq!(parser.pending_input(), pending);
        assert_eq!(parser.position().offset, (end + pending.len()) as u64);
        assert_eq!(parser.is_end_of_stream(), index == 1);
        messages.push(message);
    }
    for _ in 0..2 {
        assert!(parser.parse_mbox_message().unwrap().is_none());
        assert_eq!(parser.position().offset, input.len() as u64);
    }
    drop(parser);

    for (index, mut message) in messages.into_iter().enumerate() {
        let MimeEntity::Part(part) = message.body().unwrap() else {
            panic!("expected default leaf")
        };
        assert!(part.headers().is_empty());
        assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
        let body = if index == 0 {
            assert!(message.headers().is_empty());
            ""
        } else {
            assert_eq!(message.headers().len(), 3);
            assert_eq!(message.subject().unwrap().text, "test message");
            let mut offset = second_start;
            for header in message.headers() {
                assert_eq!(header.offset(), Some(offset as u64));
                assert_eq!(
                    header.raw(),
                    &input.as_bytes()[offset..offset + header.raw().len()]
                );
                offset += header.raw().len();
            }
            &input[input.find("Body text").unwrap()..]
        };
        assert_eq!(message.leaf_parts().count(), 1);
        if index == 0 {
            assert_header_only_serialization(&mut message, "", format);
        } else {
            assert_recovery_serialization(&mut message, &input[second_start..], format);
        }
        let MimeEntity::Part(part) = message.body_mut().unwrap() else {
            unreachable!()
        };
        let mut decoded = Vec::new();
        part.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, body.as_bytes());
    }
}

#[test]
fn invalid_header_after_mbox_marker() {
    for input in [
        "From - \nThis is not a header\nSubject: test\n\nbody\n",
        "From - \n>From munged\nThis is not a header\n",
    ] {
        assert_leading_garbage(input, true);
    }
}

#[test]
fn message_stream_with_only_mbox_markers() {
    for (input, mbox) in [
        ("From sender@example.com Mon Jan  1 00:00:00 2024", false),
        ("From sender@example.com Mon Jan  1 00:00:00 2024\n", false),
        (
            concat!(
                "From sender@example.com Mon Jan  1 00:00:00 2024\n",
                ">From sender@example.com Mon Jan  1 00:00:00 2024\n",
            ),
            false,
        ),
        (
            concat!(
                "From sender@example.com Mon Jan  1 00:00:00 2024\n",
                ">From sender@example.com Mon Jan  1 00:00:00 2024",
            ),
            true,
        ),
    ] {
        for newline in ["\n", "\r\n"] {
            let input = input.replace('\n', newline);
            assert_marker_eof(input.as_bytes(), mbox);
        }
    }
}

#[test]
fn truncated_mbox_marker() {
    assert_marker_eof(b"From <incomplete mbox marker>", true);
}

#[test]
fn truncated_mbox_marker_ending_with_carriage_return() {
    // This used to loop forever in MimeKit because the trailing '\r' was
    // never consumed. Exercise EOF after that byte, including one-byte reads.
    for input in [
        b"From \r".as_slice(),
        b"From <incomplete mbox marker>\r",
        b"From \0: \x01\0\x10\0\r\xff\r\r",
    ] {
        assert_marker_eof(input, true);
    }
}

fn assert_marker_eof(input: &[u8], mbox: bool) {
    for logged in [false, true] {
        assert_marker_eof_parser(MimeParser::from_slice(input), input, mbox, logged);
        for chunk in [1, 2, 7, 4096] {
            assert_marker_eof_parser(
                MimeParser::new(BufReader::with_capacity(chunk, input)),
                input,
                mbox,
                logged,
            );
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let parser = MimeParser::with_storage(
                    ShortSource {
                        inner: Cursor::new(input),
                        chunk,
                    },
                    storage,
                )
                .unwrap();
                assert_marker_eof_parser(parser, input, mbox, logged);
            }
        }
    }
}

fn assert_marker_eof_parser<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &[u8],
    mbox: bool,
    logged: bool,
) {
    let mut logger = |_| Ok(());
    let error = match (mbox, logged) {
        (true, true) => parser.parse_mbox_message_with_logger(&mut logger).err(),
        (true, false) => parser.parse_mbox_message().err(),
        (false, true) => parser.parse_message_with_logger(&mut logger).err(),
        (false, false) => parser.parse_message().err(),
    }
    .expect("a marker without a message must not produce a MIME model");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(
        parser.position(),
        MimePosition {
            offset: input.len() as u64,
            line: 1 + input.iter().filter(|&&byte| byte == b'\n').count() as u64,
        }
    );
    assert!(!parser.is_end_of_stream());
    let error = if mbox {
        parser.parse_mbox_message().err()
    } else {
        parser.parse_message().err()
    }
    .expect("a failed source must not be reused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn mime_part_basic() {
    let original = concat!(
        "Content-Type: application/octet-stream; name=rawData.dat\n",
        "Content-Disposition: inline; filename=rawData.dat\n",
        "Content-Transfer-Encoding: quoted-printable\n\n",
        "This is some raw data.\n",
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let body = format!("This is some raw data.{newline}");
        for logged in [false, true] {
            let entity =
                parse_recovery_entity(MimeParser::from_slice(input.as_bytes()), &input, logged);
            assert_stored_recovery_bodies(
                entity.leaf_parts(),
                &input,
                MimeContentStorage::Source,
                [body.as_str()],
            );
            assert_basic_part_model(entity, &input, &body, format);
            for chunk in [1, 2, 7, 4096] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let entity = parse_recovery_entity(parser, &input, logged);
                assert_basic_part_model(entity, &input, &body, format);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let entity = parse_recovery_entity(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        entity.leaf_parts(),
                        &input,
                        storage,
                        [body.as_str()],
                    );
                    assert_basic_part_model(entity, &input, &body, format);
                }
            }
        }
    }
}

fn parse_recovery_entity<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) -> MimeEntity<C> {
    let entity = if logged {
        parser.parse_entity_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_entity()
    }
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    entity
}

fn assert_basic_part_model<R: Read + Seek>(
    mut entity: MimeEntity<R>,
    input: &str,
    body: &str,
    format: NewLineFormat,
) {
    let MimeEntity::Part(part) = &entity else {
        panic!("expected application/octet-stream leaf")
    };
    assert_eq!(part.headers().len(), 3);
    let content_type = part.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("application", "octet-stream"));
    assert_eq!(content_type.name(), Some("rawData.dat"));
    let disposition = part.content_disposition().unwrap().unwrap();
    assert_eq!(disposition.disposition(), "inline");
    assert_eq!(disposition.file_name(), Some("rawData.dat"));
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::QuotedPrintable
    );
    assert_eq!(part.content().encoding(), ContentEncoding::QuotedPrintable);
    assert_eq!(part.content().new_line_format(), Some(format));
    let mut offset = 0;
    for header in part.headers() {
        assert_eq!(header.offset(), Some(offset as u64));
        assert_eq!(
            header.raw(),
            &input.as_bytes()[offset..offset + header.raw().len()]
        );
        offset += header.raw().len();
    }
    assert_recovery_output(input, format, |output, options| {
        entity.write_to_with_options(output, options)
    });
    let MimeEntity::Part(part) = &mut entity else {
        unreachable!()
    };
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, body.as_bytes());
}

#[test]
fn mime_part_content_with_mixed_line_endings() {
    let body = concat!(
        "This is a normal line of text.\r\n",
        "This line ends with a bare LF.\n",
        "And this line ends with CRLF.\r\n",
    );
    let input = format!(
        "From: mimekit@example.com\r\n\
         To: mimekit@example.com\r\n\
         Subject: content with mixed line endings\r\n\
         Date: Sat, Dec 21 2024 09:12:42 -0500\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}"
    );
    for logged in [false, true] {
        let message =
            parse_recovery_message(MimeParser::from_slice(input.as_bytes()), &input, logged);
        assert_stored_recovery_bodies(
            message.leaf_parts(),
            &input,
            MimeContentStorage::Source,
            [body],
        );
        assert_mixed_line_endings_model(message, &input, body);
        for chunk in [1, 2, 7, 4096] {
            let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
            let message = parse_recovery_message(parser, &input, logged);
            assert_mixed_line_endings_model(message, &input, body);
            for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                let parser = MimeParser::with_storage(
                    ShortSource {
                        inner: Cursor::new(input.as_bytes()),
                        chunk,
                    },
                    storage,
                )
                .unwrap();
                let message = parse_recovery_message(parser, &input, logged);
                assert_stored_recovery_bodies(message.leaf_parts(), &input, storage, [body]);
                assert_mixed_line_endings_model(message, &input, body);
            }
        }
    }
}

fn assert_mixed_line_endings_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    body: &str,
) {
    assert_eq!(message.headers().len(), 5);
    let MimeEntity::Part(part) = message.body().unwrap() else {
        panic!("expected text/plain leaf")
    };
    assert_eq!(part.headers().len(), 1);
    assert_plain_utf8_headers(part.headers());
    assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Mixed));
    let header = part.headers().get("Content-Type").unwrap();
    assert_eq!(header.decode_value().text, "text/plain; charset=utf-8");
    assert_eq!(
        header.offset(),
        Some(input.find("Content-Type:").unwrap() as u64)
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let expected = input.replace("\r\n", "\n").replace('\n', newline);
        assert_recovery_serialization(&mut message, &expected, format);
    }
    // MimeKit's Text getter normalizes to the platform newline. Rust keeps
    // the decoded content's original line endings, independently of writing.
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some(body)
    );
    let MimeEntity::Part(part) = message.body().unwrap() else {
        unreachable!()
    };
    assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Mixed));
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
fn truncated_header_name() {
    for input in ["Header-1", "Header-1\r", "Header-1\n", "Header-1\r\n"] {
        for logged in [false, true] {
            assert_truncated_header_name(MimeParser::from_slice(input.as_bytes()), input, logged);
            for capacity in [1, 2, 7, 4096] {
                assert_truncated_header_name(
                    MimeParser::new(BufReader::with_capacity(capacity, input.as_bytes())),
                    input,
                    logged,
                );
            }
        }
    }
}

fn assert_truncated_header_name<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) {
    let headers = if logged {
        parser.parse_headers_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_headers()
    }
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    // Unlike MimeKit's ParseHeaders, standalone Rust header loading retains
    // an invalid first record, just like HeaderList::load.
    assert_eq!(headers.len(), 1);
    let header = &headers.as_slice()[0];
    assert!(header.is_invalid());
    assert_eq!(header.id(), HeaderId::Unknown);
    assert_eq!(header.offset(), Some(0));
    assert_eq!(header.raw(), input.as_bytes());
    assert!(header.raw_value().is_empty());
    assert_eq!(
        parser.parse_headers().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
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
fn status_group_parser_lone_carriage_return_separator() {
    for newline in ["\n", "\r\n"] {
        let input = format!(
            "Reporting-MTA: dns; mm1{newline}{newline}\
             \rFinal-Recipient: rfc822; user@example.com{newline}"
        );
        for logged in [false, true] {
            assert_lone_cr_status_separator(
                MimeParser::from_slice(input.as_bytes()),
                newline,
                logged,
            );
            for capacity in [1, 2, 7, 4096] {
                assert_lone_cr_status_separator(
                    MimeParser::new(BufReader::with_capacity(capacity, input.as_bytes())),
                    newline,
                    logged,
                );
            }
        }
    }
}

fn assert_lone_cr_status_separator<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    newline: &str,
    logged: bool,
) {
    let group = if logged {
        parser.parse_status_group_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_status_group()
    }
    .unwrap()
    .unwrap();
    assert_eq!(
        parser.position().offset,
        ("Reporting-MTA: dns; mm1".len() + newline.len() * 2) as u64
    );

    // A lone CR is not a blank line, so MimeKit's SkipBlankLines stops here.
    // Rust likewise must not skip it, but retains the resulting malformed
    // record in a raw status group instead of rejecting the header block.
    let malformed = if logged {
        parser.parse_status_group_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_status_group()
    }
    .unwrap()
    .unwrap();
    assert!(parser.is_end_of_stream());
    for _ in 0..2 {
        assert!(parser.parse_status_group().unwrap().is_none());
    }
    drop(parser);
    assert_eq!(group.len(), 1);
    let header = group.get("Reporting-MTA").unwrap();
    assert_eq!(header.offset(), Some(0));
    assert_eq!(header.decode_value().text, "dns; mm1");
    assert_eq!(
        header.raw(),
        format!("Reporting-MTA: dns; mm1{newline}").as_bytes()
    );
    assert_eq!(malformed.len(), 1);
    assert!(!malformed.contains("Final-Recipient"));
    let header = &malformed.as_slice()[0];
    assert!(header.is_invalid());
    assert_eq!(header.id(), HeaderId::Unknown);
    assert_eq!(
        header.offset(),
        Some(("Reporting-MTA: dns; mm1".len() + newline.len() * 2) as u64)
    );
    assert_eq!(
        header.raw(),
        format!("\rFinal-Recipient: rfc822; user@example.com{newline}").as_bytes()
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
                let message = parse_recovery_message(parser, &input, logged);
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
                    let message = parse_recovery_message(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        message.leaf_parts(),
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

fn assert_stored_recovery_bodies<'a, 'b, R: Read + Seek + 'b>(
    mut parts: impl Iterator<Item = &'b mimecraft::MimePart<mimecraft::MimeContentStream<R>>>,
    input: &str,
    storage: MimeContentStorage,
    expected: impl IntoIterator<Item = &'a str>,
) {
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

fn parse_recovery_message<R: BufRead, C: Read + Seek>(
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
    assert_recovery_output(input, format, |output, options| {
        message.write_to_with_options(output, options)
    });
}

fn assert_header_only_serialization<R: Read + Seek>(
    message: &mut MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
) {
    // An absent body writes nothing, even with EnsureNewLine. Do not invent
    // a newline after an incomplete header or retained leading marker.
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
        assert_round_trip("header-only recovery", input.as_bytes(), &output);
    }
}

fn assert_recovery_output(
    input: &str,
    format: NewLineFormat,
    mut write: impl FnMut(&mut Vec<u8>, &MimePartWriteOptions) -> io::Result<()>,
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
        write(
            &mut output,
            &MimePartWriteOptions {
                new_line_format: format,
                ensure_new_line,
                ..Default::default()
            },
        )
        .unwrap();
        assert_round_trip(
            "parser recovery",
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
                let message = parse_recovery_message(parser, &input, logged);
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
                    let message = parse_recovery_message(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        message.leaf_parts(),
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
fn truncated_immediately_after_message_rfc822_headers() {
    assert_empty_message_rfc822(false);
}

#[test]
fn message_rfc822_without_message() {
    assert_empty_message_rfc822(true);
}

fn assert_empty_message_rfc822(closing_boundary: bool) {
    let subject = if closing_boundary {
        "test of message/rfc822 part without a message"
    } else {
        "test of message/rfc822 part truncated immediately after the MIME headers"
    };
    let length = if closing_boundary {
        "Content-Length: 420\n"
    } else {
        ""
    };
    let ending = if closing_boundary {
        format!("--{TRUNCATED_MULTIPART_BOUNDARY}--\n")
    } else {
        String::new()
    };
    let original = format!(
        "From: mimekit@example.com\n\
         To: mimekit@example.com\n\
         Subject: {subject}\n\
         Date: Tue, 12 Nov 2013 09:12:42 -0500\n\
         MIME-Version: 1.0\n\
         Message-ID: <54AD68C9E3B0184CAC6041320424FD1B5B81E74D@localhost.localdomain>\n\
         X-Mailer: Microsoft Office Outlook 12.0\n\
         Content-Type: multipart/mixed;\n\
         \tboundary=\"{TRUNCATED_MULTIPART_BOUNDARY}\"\n{length}\n\n\
         --{TRUNCATED_MULTIPART_BOUNDARY}\n{TRUNCATED_PART_HEADERS}\n\
         {TRUNCATED_PART_BODY}\n\
         --{TRUNCATED_MULTIPART_BOUNDARY}\nContent-Type: message/rfc822\n\n{ending}"
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let body = TRUNCATED_PART_BODY.replace('\n', newline);
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_recovery_message(parser, &input, logged);
                assert_empty_message_rfc822_model(message, &input, format, &body);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_recovery_message(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        message.leaf_parts(),
                        &input,
                        storage,
                        [body.as_str()],
                    );
                    assert_empty_message_rfc822_model(message, &input, format, &body);
                }
            }
        }
    }
}

fn assert_empty_message_rfc822_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
    body: &str,
) {
    let MimeEntity::Multipart(multipart) = message.body().unwrap() else {
        panic!("expected multipart")
    };
    assert_eq!(multipart.children().len(), 2);
    let MimeEntity::Message(wrapper) = &multipart.children()[1] else {
        panic!("expected empty message/rfc822 wrapper")
    };
    assert_eq!(wrapper.headers().len(), 1);
    assert!(
        wrapper
            .content_type()
            .unwrap()
            .unwrap()
            .is_mime_type("message", "rfc822")
    );
    assert!(wrapper.message().is_none());
    assert_eq!(message.leaf_parts().count(), 1);
    assert_recovery_serialization(&mut message, input, format);
    let MimeEntity::Multipart(multipart) = message.body_mut().unwrap() else {
        unreachable!()
    };
    let MimeEntity::Part(part) = &mut multipart.children_mut()[0] else {
        panic!("expected first text/plain child")
    };
    assert_plain_utf8_body(part, body);
}

fn munged_rfc822_message() -> String {
    // MimeKit's inline regression uses this message from its Content-Length
    // corpus, without the extra blank line before the next mbox message.
    let corpus = include_str!("data/mbox/content-length.mbox.txt").replace("\r\n", "\n");
    let start = corpus.find("From - Fri Mar  7 02:51:22 1997\n").unwrap();
    let end = start + corpus[start..].find("\nFrom ").unwrap();
    format!("{}\n", corpus[start..end].trim_end_matches('\n'))
}

#[test]
fn message_rfc822_with_munged_from_marker_before_message() {
    assert_munged_rfc822(&munged_rfc822_message(), true);
}

#[test]
fn message_rfc822_with_munged_from_marker_eof() {
    let original = munged_rfc822_message();
    let end = original
        .find("\nReturn-Path: <blaker@craswell.com>")
        .unwrap();
    assert_munged_rfc822(&original[..end], false);
}

#[test]
fn message_rfc822_with_truncated_munged_from_marker() {
    let original = munged_rfc822_message();
    let end = original.find(">From ").unwrap() + ">From".len();
    assert_munged_rfc822(&original[..end], false);
}

fn assert_munged_rfc822(original: &str, complete: bool) {
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let body = if complete {
            input.rsplit_once(&format!("{newline}{newline}")).unwrap().1
        } else {
            ""
        };
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let message = parse_munged_rfc822(parser, &input, logged);
                assert_munged_rfc822_model(message, &input, format, complete);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let message = parse_munged_rfc822(parser, &input, logged);
                    assert_stored_recovery_bodies(message.leaf_parts(), &input, storage, [body]);
                    assert_munged_rfc822_model(message, &input, format, complete);
                }
            }
        }
    }
}

fn parse_munged_rfc822<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) -> MimeMessage<C> {
    let message = if logged {
        parser.parse_mbox_message_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_mbox_message()
    }
    .unwrap()
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    assert!(parser.parse_mbox_message().unwrap().is_none());
    message
}

fn assert_munged_rfc822_model<R: Read + Seek>(
    mut message: MimeMessage<R>,
    input: &str,
    format: NewLineFormat,
    complete: bool,
) {
    let MimeEntity::Message(wrapper) = message.body().unwrap() else {
        panic!("expected message/rfc822 wrapper")
    };
    let content_type = wrapper.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("message", "rfc822"));
    assert_eq!(content_type.name(), Some("smime18-encrypted.msg"));
    let disposition = wrapper
        .headers()
        .get("Content-Disposition")
        .unwrap()
        .parse_content_disposition()
        .unwrap();
    assert_eq!(disposition.disposition(), "inline");
    assert_eq!(disposition.file_name(), Some("smime18-encrypted.msg"));
    assert_eq!(
        wrapper
            .headers()
            .get("Content-Transfer-Encoding")
            .unwrap()
            .decode_value()
            .text,
        "7bit"
    );
    let embedded = wrapper.message().unwrap();
    // The Rust raw-header model retains the munged From-line as an invalid
    // header rather than exposing MimeKit's separate Message.MboxMarker.
    let marker_start = input.find(">From").unwrap();
    let marker_end = input[marker_start..]
        .find('\n')
        .map_or(input.len(), |offset| marker_start + offset + 1);
    let marker = embedded.headers().iter().next().unwrap();
    assert!(marker.is_invalid());
    assert_eq!(
        marker.raw_field(),
        &input.as_bytes()[marker_start..marker_end]
    );
    assert!(marker.raw_value().is_empty());
    assert_eq!(embedded.headers().len(), if complete { 15 } else { 1 });
    assert_eq!(
        embedded
            .headers()
            .iter()
            .filter(|header| !header.is_invalid())
            .count(),
        if complete { 14 } else { 0 }
    );
    let MimeEntity::Part(part) = embedded.body().unwrap() else {
        panic!("expected embedded leaf")
    };
    assert_eq!(part.headers().len(), if complete { 3 } else { 0 });
    if complete {
        assert!(
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("application", "x-pkcs7-mime")
        );
        assert_eq!(
            part.content().encoding(),
            mimecraft::ContentEncoding::Base64
        );
    } else {
        assert!(part.content_type().unwrap().is_none());
    }
    let expected = &input[input.find('\n').unwrap() + 1..];
    // Top-level mbox markers belong to the parser, not the serialized message.
    if complete {
        assert_recovery_serialization(&mut message, expected, format);
    } else {
        // MimeKit writes the marker verbatim, and null Content writes no body,
        // even with EnsureNewLine. Do not invent a newline after the marker.
        assert_header_only_serialization(&mut message, expected, format);
    }
    let MimeEntity::Message(wrapper) = message.body_mut().unwrap() else {
        unreachable!()
    };
    let MimeEntity::Part(part) = wrapper.message_mut().unwrap().body_mut().unwrap() else {
        unreachable!()
    };
    let mut stored = Vec::new();
    part.content_mut().write_to(&mut stored).unwrap();
    let expected = if complete {
        let newline = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        input.rsplit_once(&format!("{newline}{newline}")).unwrap().1
    } else {
        ""
    };
    assert_eq!(stored, expected.as_bytes());
}

const RFC822_MESSAGE: &str = concat!(
    "From: mimekit@example.com\n",
    "To: mimekit@example.com\n",
    "Subject: embedded message\n",
    "Date: Tue, 12 Nov 2013 09:12:42 -0500\n",
    "MIME-Version: 1.0\n",
    "Content-Type: text/plain; charset=utf-8\n\n",
    "This is the rfc822 message body.\n",
);
const RFC822_BASE64: &str = concat!(
    "RnJvbTogbWltZWtpdEBleGFtcGxlLmNvbQpUbzogbWltZWtpdEBleGFtcGxlLmNvbQpTdWJqZWN0\n",
    "OiBlbWJlZGRlZCBtZXNzYWdlCkRhdGU6IFR1ZSwgMTIgTm92IDIwMTMgMDk6MTI6NDIgLTA1MDAK\n",
    "TUlNRS1WZXJzaW9uOiAxLjAKQ29udGVudC1UeXBlOiB0ZXh0L3BsYWluOyBjaGFyc2V0PXV0Zi04\n",
    "CgpUaGlzIGlzIHRoZSByZmM4MjIgbWVzc2FnZSBib2R5Lgo=\n",
);

#[test]
fn message_rfc822() {
    assert_rfc822_entity("", false);
}

#[test]
fn message_rfc822_with_content_transfer_encoding() {
    assert_rfc822_entity("Content-Transfer-Encoding: \n", false);
}

#[test]
fn message_rfc822_with_content_transfer_encoding_base64() {
    assert_rfc822_entity("Content-Transfer-Encoding: base64\n", true);
}

fn assert_rfc822_entity(encoding_header: &str, encoded: bool) {
    let original = format!(
        "Content-Type: message/rfc822\n{encoding_header}\n{}",
        if encoded {
            RFC822_BASE64
        } else {
            RFC822_MESSAGE
        }
    );
    for (format, newline) in [(NewLineFormat::Unix, "\n"), (NewLineFormat::Dos, "\r\n")] {
        let input = original.replace('\n', newline);
        let stored = if encoded {
            RFC822_BASE64
        } else {
            "This is the rfc822 message body.\n"
        }
        .replace('\n', newline);
        for chunk in [1, 2, 7, 4096] {
            for logged in [false, true] {
                let parser = MimeParser::new(BufReader::with_capacity(chunk, input.as_bytes()));
                let entity = parse_rfc822_entity(parser, &input, logged);
                assert_rfc822_entity_model(entity, &input, format, encoded);
                for storage in [MimeContentStorage::Memory, MimeContentStorage::Source] {
                    let parser = MimeParser::with_storage(
                        ShortSource {
                            inner: Cursor::new(input.as_bytes()),
                            chunk,
                        },
                        storage,
                    )
                    .unwrap();
                    let entity = parse_rfc822_entity(parser, &input, logged);
                    assert_stored_recovery_bodies(
                        entity.leaf_parts(),
                        &input,
                        storage,
                        [stored.as_str()],
                    );
                    assert_rfc822_entity_model(entity, &input, format, encoded);
                }
            }
        }
    }
}

fn parse_rfc822_entity<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    input: &str,
    logged: bool,
) -> MimeEntity<C> {
    let entity = if logged {
        parser.parse_entity_with_logger(&mut |_| Ok(()))
    } else {
        parser.parse_entity()
    }
    .unwrap();
    assert!(parser.is_end_of_stream());
    assert_eq!(parser.position().offset, input.len() as u64);
    entity
}

fn assert_rfc822_entity_model<R: Read + Seek>(
    mut entity: MimeEntity<R>,
    input: &str,
    format: NewLineFormat,
    encoded: bool,
) {
    assert_recovery_output(input, format, |output, options| {
        entity.write_to_with_options(output, options)
    });
    if encoded {
        let MimeEntity::Part(part) = &mut entity else {
            panic!("encoded message/rfc822 must remain an opaque leaf")
        };
        assert!(
            part.content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("message", "rfc822")
        );
        assert_eq!(
            part.content().encoding(),
            mimecraft::ContentEncoding::Base64
        );
        // MimeKit opens the decoded content and parses it as a new message.
        // Changing the wrapper's newlines must not change its encoded payload.
        let mut message = MimeParser::new(part.content_mut().open().unwrap())
            .parse_message()
            .unwrap();
        assert_recovery_serialization(&mut message, RFC822_MESSAGE, NewLineFormat::Unix);
        assert_rfc822_message_body(&mut message, "\n");
        // Reading the decoded view must not prevent a later byte-exact write.
        assert_recovery_output(input, format, |output, options| {
            entity.write_to_with_options(output, options)
        });
    } else {
        let MimeEntity::Message(wrapper) = &mut entity else {
            panic!("expected transparent message/rfc822 wrapper")
        };
        assert!(
            wrapper
                .content_type()
                .unwrap()
                .unwrap()
                .is_mime_type("message", "rfc822")
        );
        let newline = if format == NewLineFormat::Dos {
            "\r\n"
        } else {
            "\n"
        };
        assert_rfc822_message_body(wrapper.message_mut().unwrap(), newline);
    }
}

fn assert_rfc822_message_body<R: Read + Seek>(message: &mut MimeMessage<R>, newline: &str) {
    assert_eq!(message.headers().len(), 5);
    assert_eq!(message.subject().unwrap().text, "embedded message");
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        panic!("expected embedded text/plain leaf")
    };
    assert_plain_utf8_body(part, &format!("This is the rfc822 message body.{newline}"));
}

fn assert_plain_utf8_body<R: Read + Seek>(part: &mut mimecraft::MimePart<R>, body: &str) {
    assert_eq!(part.headers().len(), 1);
    let content_type = part.content_type().unwrap().unwrap();
    assert!(content_type.is_mime_type("text", "plain"));
    assert_eq!(content_type.charset(), Some("utf-8"));
    let mut decoded = Vec::new();
    part.content_mut().decode_to(&mut decoded).unwrap();
    assert_eq!(decoded, body.as_bytes());
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

// Adapted from MimeKit's MimeParserTests.cs; excludes LegacyMimeParser.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeEntity, MimeEntityParseOptions, MimeEvent, MimeMessage, MimeParser, MimePartWriteOptions,
    MimePosition, MimeReader, encodings::NewLineFormat,
};
use std::io::{self, BufReader, Read};

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
}

impl Read for ThrowingReadStream<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.position >= self.limit {
            return Err(io::Error::other("boundary lookahead exceeded limit"));
        }
        if self.position == self.bytes.len() {
            return Ok(0);
        }
        output[0] = self.bytes[self.position];
        self.position += 1;
        Ok(1)
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

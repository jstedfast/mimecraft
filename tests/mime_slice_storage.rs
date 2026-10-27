// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    path::Path,
};

use mimecraft::{
    MimeContentStorage, MimeContentStream, MimeEntity, MimeEntityParseOptions, MimeMessage,
    MimeParser, MimePosition,
};

fn body<R: Read + Seek>(message: &mut MimeMessage<R>) -> &mut R {
    let Some(MimeEntity::Part(part)) = message.body_mut() else {
        panic!("expected leaf body");
    };
    part.content_mut().get_mut()
}

fn read_body<R: Read + Seek>(message: &mut MimeMessage<R>) -> Vec<u8> {
    let source = body(message);
    source.rewind().unwrap();
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn slice_bodies_survive_parser_drop_and_allow_owned_replacements() {
    let original = b"prefixSubject: before\r\nContent-Type: text/plain\r\n\r\noriginal";
    let input = &original[6..];
    let mut parser = MimeParser::from_slice(input);
    assert!(std::ptr::eq(*parser.get_ref(), input));
    assert_eq!(parser.storage(), MimeContentStorage::Source);
    let mut message = parser.parse_message().unwrap();
    assert_eq!(
        parser.position(),
        MimePosition {
            offset: input.len() as u64,
            line: 4
        }
    );
    assert!(parser.is_end_of_stream());
    match body(&mut message) {
        MimeContentStream::Source(range) => {
            assert_eq!(
                range.start_boundary(),
                (input.len() - b"original".len()) as u64
            );
            assert_eq!(range.end_boundary(), Some(input.len() as u64));
            assert_eq!(range.position(), 0);
        }
        MimeContentStream::Memory(_) => panic!("slice bodies must retain source ranges"),
    }
    drop(parser);
    assert_eq!(read_body(&mut message), b"original");
    message.set_subject("after").unwrap();
    *body(&mut message) = MimeContentStream::Memory(Cursor::new(b"replacement".to_vec()));
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    assert_eq!(
        output,
        b"Subject: after\r\nContent-Type: text/plain\r\n\r\nreplacement"
    );
    assert_eq!(&original[original.len() - b"original".len()..], b"original");
}

#[test]
fn slice_mbox_cursors_survive_interleaved_reads_and_source_replacement() {
    let input = b"From first\nSubject: one\n\nfirst\nFrom second\nSubject: two\n\nsecond\nFrom third\n\nthird\n";
    let mut parser = MimeParser::from_slice(input);
    let mut first = parser.parse_mbox_message().unwrap().unwrap();
    let mut byte = [0];
    body(&mut first).read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"f");
    let mut second = parser.parse_mbox_message().unwrap().unwrap();
    body(&mut second).seek(SeekFrom::Start(2)).unwrap();
    body(&mut second).read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"c");
    body(&mut first).read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"i");
    let mut third = parser.parse_mbox_message().unwrap().unwrap();
    assert!(parser.parse_mbox_message().unwrap().is_none());
    assert_eq!(parser.position().offset, input.len() as u64);
    let replacement = b"Subject: new\r\n\r\nnew";
    assert!(parser.replace_source(replacement).is_empty());
    assert!(std::ptr::eq(*parser.get_ref(), replacement.as_slice()));
    assert_eq!(parser.position(), MimePosition::default());
    assert_eq!(parser.storage(), MimeContentStorage::Source);
    let mut new = parser.parse_message().unwrap();
    drop(parser);
    assert_eq!(read_body(&mut first), b"first");
    assert_eq!(read_body(&mut second), b"second");
    assert_eq!(read_body(&mut third), b"third\n");
    assert_eq!(read_body(&mut new), b"new");
}

#[test]
fn slice_parsing_matches_streaming_positions_diagnostics_and_serialization() {
    let startrek = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("benches")
            .join("data")
            .join("messages")
            .join("startrek.eml"),
    )
    .unwrap();
    for input in [
        startrek.as_slice(),
        b"",
        b"Subject: truncated",
        b"Content-Transfer-Encoding: base64\r\n",
        b"Content-Transfer-Encoding: base64\r\n\r\n",
        b"Subject: mixed\nContent-Type: text/plain\r\n\r\nbody\nnext\r\nlast",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\npreamble\r\n--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b-- \t\r\nepilogue",
        b"Content-Type: multipart/mixed; boundary=b\n\n--b\nContent-Type: message/rfc822\n\nSubject: nested\n\nbody\n--b--",
        b"Content-Type: multipart/mixed; boundary=outer\n\n--outer\nContent-Type: multipart/mixed; boundary=inner\n\n--inner\nContent-Type: text/plain\n\nbody\n--outer--",
        b"Content-Type: broken\r\nContent-Transfer-Encoding: unknown\r\n\r\nbody",
    ] {
        for logged in [false, true] {
            let mut sliced = MimeParser::from_slice(input);
            let mut streamed =
                MimeParser::with_storage(Cursor::new(input), MimeContentStorage::Source).unwrap();
            let mut slice_issues = Vec::new();
            let mut stream_issues = Vec::new();
            let actual = if logged {
                sliced.parse_message_with_logger(&mut |issue| {
                    slice_issues.push(issue);
                    Ok(())
                })
            } else {
                sliced.parse_message()
            };
            let expected = if logged {
                streamed.parse_message_with_logger(&mut |issue| {
                    stream_issues.push(issue);
                    Ok(())
                })
            } else {
                streamed.parse_message()
            };
            assert_eq!(sliced.position(), streamed.position());
            assert_eq!(sliced.is_end_of_stream(), streamed.is_end_of_stream());
            assert_eq!(slice_issues, stream_issues);
            let (mut actual, mut expected) = match (actual, expected) {
                (Ok(actual), Ok(expected)) => (actual, expected),
                (Err(actual), Err(expected)) => {
                    assert_eq!(actual.kind(), expected.kind());
                    assert_eq!(actual.to_string(), expected.to_string());
                    continue;
                }
                _ => panic!("slice and streaming parsers disagree on success"),
            };
            assert_eq!(actual.headers(), expected.headers());
            assert_eq!(actual.iter().count(), expected.iter().count());
            for (actual, expected) in actual.iter().zip(expected.iter()) {
                assert_eq!(actual.headers(), expected.headers());
                if let (MimeEntity::Part(actual), MimeEntity::Part(expected)) = (actual, expected) {
                    assert_eq!(actual.content().new_line_format(), expected.content().new_line_format());
                }
            }
            let mut actual_bytes = Vec::new();
            let mut expected_bytes = Vec::new();
            actual.write_to(&mut actual_bytes).unwrap();
            expected.write_to(&mut expected_bytes).unwrap();
            assert_eq!(actual_bytes, expected_bytes);
        }
    }
}

#[test]
fn slice_limits_and_logger_failures_follow_streaming_rules() {
    let invalid = MimeEntityParseOptions {
        max_header_length: 0,
        ..Default::default()
    };
    assert_eq!(
        MimeParser::from_slice_with_options(b"\r\nbody", invalid)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput,
    );
    for limit in [5, 6] {
        let options = MimeEntityParseOptions {
            max_input_bytes: limit,
            ..Default::default()
        };
        let mut parser = MimeParser::from_slice_with_options(b"\r\nbody", options).unwrap();
        assert_eq!(parser.options(), &options);
        assert_eq!(parser.parse_message().is_ok(), limit == 6);
    }
    let mut parser = MimeParser::from_slice(b"Content-Type: broken\r\n\r\nbody");
    let error = parser
        .parse_message_with_logger(&mut |_| Err(io::Error::other("rejected")))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "rejected");
    assert!(parser.parse_message().is_err());
    parser.replace_source(b"\r\nnew");
    let mut message = parser.parse_message().unwrap();
    assert_eq!(read_body(&mut message), b"new");
}

#[test]
fn slice_header_and_entity_operations_preserve_unread_bytes() {
    let input = b"Subject: example\r\n\r\nbody";
    let mut parser = MimeParser::from_slice(input);
    let headers = parser.parse_headers().unwrap();
    assert_eq!(headers.get("Subject").unwrap().raw_value(), b" example\r\n");
    assert_eq!(*parser.get_ref(), b"body");
    assert_eq!(parser.into_inner(), b"body");

    let mut parser = MimeParser::from_slice(b"\r\nfirst");
    let mut first = parser.parse_entity().unwrap();
    parser.replace_source(b"\r\nsecond");
    let mut second = parser.parse_entity().unwrap();
    drop(parser);
    for (entity, expected) in [
        (&mut first, b"\r\nfirst".as_slice()),
        (&mut second, b"\r\nsecond"),
    ] {
        let mut output = Vec::new();
        entity.write_to(&mut output).unwrap();
        assert_eq!(output, expected);
    }
}

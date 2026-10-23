// Adapted from MimeKit's MessagePartialTests.cs and TestData/partial/rfc2046.*.eml.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, Header, HeaderList, MessagePartial, MessagePartialError, MimeContent,
    MimeEntity, MimeEntityParseOptions, MimeMessage, MimePart, encodings::NewLineFormat,
};

fn fields(bytes: &[u8]) -> HeaderList {
    HeaderList::load(&mut &bytes[..]).unwrap()
}

fn fragment(number: u32, total: u32, bytes: &[u8]) -> MessagePartial {
    MessagePartial::with_content(
        "id",
        number,
        total,
        MimeContent::new(Cursor::new(bytes.to_vec())),
    )
    .unwrap()
}

fn raw_fragment<R: Read + Seek>(parameters: &str, source: R) -> MessagePartial<R> {
    MessagePartial::from_part(MimePart::from_parts(
        fields(format!("Content-Type: message/partial{parameters}\r\n").as_bytes()),
        MimeContent::new(source),
    ))
    .unwrap()
}

fn set_parameter<R: Read + Seek>(partial: &mut MessagePartial<R>, name: &str, value: Option<&str>) {
    let mut content_type = partial.part().content_type().unwrap().unwrap();
    if let Some(value) = value {
        content_type.parameters_mut().set(name, value).unwrap();
    } else {
        content_type.parameters_mut().remove(name);
    }
    partial.part_mut().set_content_type(&content_type).unwrap();
}

fn assert_io(error: MessagePartialError, kind: io::ErrorKind) {
    assert!(error.source().is_some());
    assert!(matches!(error, MessagePartialError::Io(inner) if inner.kind() == kind));
}

fn write(message: &mut MimeMessage) -> Vec<u8> {
    let mut wire = Vec::new();
    message.write_to(&mut wire).unwrap();
    wire
}

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("metadata validation must precede reading");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("metadata validation must precede seeking");
    }
}

#[test]
fn constructors_preserve_id_and_only_generate_content_type() {
    for id in [
        "",
        " id with spaces ",
        "AbC@example.com",
        "quoted\";id",
        "caf\u{e9}",
    ] {
        let partial = MessagePartial::new(id, 1, 5).unwrap();
        assert_eq!(partial.id().unwrap().as_deref(), Some(id));
        assert_eq!(partial.number().unwrap(), Some(1));
        assert_eq!(partial.total().unwrap(), Some(5));
        assert_eq!(partial.part().headers().len(), 1);
        assert_eq!(
            partial.part().content().encoding(),
            ContentEncoding::Default
        );
        assert_eq!(partial.part().content().new_line_format(), None);
        assert!(partial.part().content().get_ref().get_ref().is_empty());
    }
    assert!(MessagePartial::new("id", i32::MAX as u32, i32::MAX as u32).is_ok());
    for number in [0, i32::MAX as u32 + 1, u32::MAX] {
        assert!(matches!(MessagePartial::new("id", number, u32::MAX),
            Err(MessagePartialError::InvalidNumber(actual)) if actual == number));
    }
    for total in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert!(matches!(MessagePartial::new("id", 2, total),
            Err(MessagePartialError::InvalidTotal { number: 2, total: actual }) if actual == total));
    }
}

#[test]
fn metadata_and_conversion_do_not_access_borrowed_sources() {
    let mut source = NoIo;
    let partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(&mut source)).unwrap();
    let partial = MessagePartial::from_part(partial.into_part()).unwrap();
    assert_eq!(partial.id().unwrap().as_deref(), Some("id"));
    assert_eq!(partial.number().unwrap(), Some(1));
    let entity: MimeEntity<_> = partial.into();
    assert!(matches!(entity, MimeEntity::Part(_)));
    assert_eq!(entity.body_parts().count(), 1);
    assert_eq!(entity.leaf_parts().count(), 1);

    let mut source = Cursor::new(b"bytes");
    source.set_position(3);
    let mut content = MimeContent::with_encoding(source, ContentEncoding::Base64);
    content.set_new_line_format(Some(NewLineFormat::Unix));
    let partial = MessagePartial::with_content("id", 1, 1, content).unwrap();
    assert_eq!(partial.part().content().get_ref().position(), 3);
    assert_eq!(partial.part().content().encoding(), ContentEncoding::Base64);
    assert_eq!(
        partial.part().content().new_line_format(),
        Some(NewLineFormat::Unix)
    );
}

#[test]
fn wrapping_checks_first_content_type_but_not_parameters() {
    for raw in [
        "Content-Type: MESSAGE/PARTIAL\r\nContent-Type: invalid\r\n",
        "Content-Type: message/partial; number=invalid\r\n",
    ] {
        let part = MimePart::from_parts(fields(raw.as_bytes()), MimeContent::new(NoIo));
        let partial = MessagePartial::from_part(part).unwrap();
        assert_eq!(partial.id().unwrap(), None);
        assert_eq!(partial.part().headers(), &fields(raw.as_bytes()));
    }
    for raw in [
        "",
        "Content-Type: invalid\r\n",
        "Content-Type: text/partial\r\n",
    ] {
        let mut headers = fields(raw.as_bytes());
        if !raw.is_empty() {
            headers.add(Header::parse(b"Content-Type: message/partial").unwrap());
        }
        let error =
            MessagePartial::from_part(MimePart::from_parts(headers, MimeContent::new(NoIo)))
                .unwrap_err();
        match raw {
            "" => assert!(matches!(error, MessagePartialError::MissingContentType)),
            "Content-Type: invalid\r\n" => {
                assert!(error.source().is_some());
                assert!(matches!(error, MessagePartialError::ContentType(_)));
            }
            _ => assert!(matches!(error, MessagePartialError::NotPartial(_))),
        }
    }
    let mut partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(NoIo)).unwrap();
    partial.part_mut().headers_mut().remove_all("Content-Type");
    assert!(matches!(
        partial.id(),
        Err(MessagePartialError::MissingContentType)
    ));
    assert!(matches!(
        partial.number(),
        Err(MessagePartialError::MissingContentType)
    ));
    assert!(matches!(
        partial.total(),
        Err(MessagePartialError::MissingContentType)
    ));
    assert!(matches!(
        MessagePartial::join(&HeaderList::new(), &mut [partial]),
        Err(MessagePartialError::MissingContentType)
    ));
}

#[test]
fn numeric_parameters_distinguish_absence_invalid_values_and_zero() {
    let mut partial = raw_fragment("", NoIo);
    assert_eq!(partial.number().unwrap(), None);
    assert_eq!(partial.total().unwrap(), None);
    for (value, expected) in [
        ("0", 0),
        ("000000000000000000000000000001", 1),
        (" \t5\t ", 5),
        ("2147483647", i32::MAX as u32),
    ] {
        for name in ["number", "total"] {
            set_parameter(&mut partial, name, Some(value));
        }
        assert_eq!(partial.number().unwrap(), Some(expected));
        assert_eq!(partial.total().unwrap(), Some(expected));
    }
    for value in [
        "",
        " ",
        "+1",
        "-1",
        "1 2",
        "1.0",
        "1e1",
        "invalid",
        "2147483648",
        "4294967296",
        "\u{a0}1",
        "\u{661}",
        "1\0",
    ] {
        for name in ["number", "total"] {
            set_parameter(&mut partial, name, Some(value));
            let error = if name == "number" {
                partial.number()
            } else {
                partial.total()
            }
            .unwrap_err();
            assert!(error.source().is_none());
            assert!(
                matches!(error, MessagePartialError::InvalidParameter { name: actual, .. }
                if actual == name),
                "{name}: {value:?}"
            );
        }
    }
    for name in ["number", "total"] {
        set_parameter(&mut partial, name, None);
    }
    assert_eq!(partial.number().unwrap(), None);
    assert_eq!(partial.total().unwrap(), None);
}

#[test]
fn canonical_rfc2046_example_merges_headers_and_preserves_fragment_order() {
    let first = b"X-Weird-Header-1: Foo\r\nFrom: Bill@host.com\r\nTo: joe@otherhost.com\r\n\
Date: Fri, 26 Mar 1993 12:59:38 -0500 (EST)\r\nSubject: Audio mail (part 1 of 2)\r\n\
Message-ID: <id1@host.com>\r\nMIME-Version: 1.0\r\n\
Content-type: message/partial; id=\"ABC@host.com\";\r\n\tnumber=1; total=2\r\n\r\n\
X-Weird-Header-1: Bar\r\nX-Weird-Header-2: Hello\r\nMessage-ID: <anotherid@foo.com>\r\n\
Subject: Audio mail\r\nMIME-Version: 1.0\r\nContent-type: audio/basic\r\n\
Content-transfer-encoding: base64\r\n\r\n... first half of encoded audio data goes here ...\r\n";
    let second = b"From: Bill@host.com\r\nTo: joe@otherhost.com\r\n\
Subject: Audio mail (part 2 of 2)\r\n\
Content-type: message/partial; id=\"ABC@host.com\"; number=2; total=2\r\n\r\n\
... second half of encoded audio data goes here ...\r\n";
    let expected = b"X-Weird-Header-1: Foo\r\nFrom: Bill@host.com\r\nTo: joe@otherhost.com\r\n\
Date: Fri, 26 Mar 1993 12:59:38 -0500 (EST)\r\nSubject: Audio mail\r\n\
Message-ID: <anotherid@foo.com>\r\nMIME-Version: 1.0\r\nContent-type: audio/basic\r\n\
Content-transfer-encoding: base64\r\n\r\n... first half of encoded audio data goes here ...\r\n\
... second half of encoded audio data goes here ...\r\n";
    let mut envelope = HeaderList::new();
    let mut parts = Vec::new();
    for wire in [first.as_slice(), second.as_slice()] {
        let (headers, Some(MimeEntity::Part(part))) =
            MimeMessage::load(&mut &wire[..]).unwrap().into_parts()
        else {
            panic!("opaque fragment")
        };
        if parts.is_empty() {
            envelope = headers;
        }
        parts.push(MessagePartial::from_part(part).unwrap());
    }
    parts.reverse();
    let saved: Vec<_> = parts.iter().map(|p| p.part().headers().clone()).collect();
    let mut joined = MessagePartial::join(&envelope, &mut parts)
        .unwrap()
        .unwrap();
    assert_eq!(write(&mut joined), expected);
    assert_eq!(parts[0].number().unwrap(), Some(2));
    assert_eq!(parts[1].number().unwrap(), Some(1));
    for (part, headers) in parts.iter().zip(saved) {
        assert_eq!(part.part().headers(), &headers);
        assert_eq!(
            part.part().content().get_ref().position(),
            part.part().content().get_ref().get_ref().len() as u64
        );
    }
    assert_eq!(
        write(
            &mut MessagePartial::join(&envelope, &mut parts)
                .unwrap()
                .unwrap()
        ),
        expected
    );
}

#[test]
fn reassembly_handles_every_split_point_including_empty_fragments() {
    let wire = b"Subject: original\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody\0\r\n--b--\r\n";
    for offset in 0..=wire.len() {
        let mut parts = [
            fragment(2, 2, &wire[offset..]),
            fragment(1, 2, &wire[..offset]),
        ];
        let mut message = MessagePartial::join(&HeaderList::new(), &mut parts)
            .unwrap()
            .unwrap();
        assert_eq!(write(&mut message), wire, "split offset {offset}");
        assert!(matches!(message.body(), Some(MimeEntity::Multipart(_))));
    }
}

#[test]
fn header_merge_keeps_raw_duplicates_and_discards_enclosing_content_fields() {
    let envelope = fields(
        b"X-Keep: one\r\nSubject: outer1\r\nFrom: sender\r\n\
Subject: outer2\r\nEncrypted: outer\r\nMIME-Version: outer\r\n\
Content-X: discard\r\nX-Keep: \xff\r\nbad record\r\n",
    );
    let enclosed = b"Subject: first\r\nX-Drop: inner\r\nMessage-ID: <inner>\r\n\
Subject: second\r\n\tfolded\r\nSubject: third\r\nEncrypted: inner\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain\r\nContent-X: inside\r\n\r\nbody";
    let before = envelope.clone();
    let message = MessagePartial::join(&envelope, &mut [fragment(1, 1, enclosed)])
        .unwrap()
        .unwrap();
    assert_eq!(envelope, before);
    assert_eq!(
        message.headers(),
        &fields(
            b"X-Keep: one\r\nSubject: first\r\nFrom: sender\r\n\
Subject: second\r\n\tfolded\r\nEncrypted: inner\r\nMIME-Version: 1.0\r\nX-Keep: \xff\r\n\
bad record\r\nMessage-ID: <inner>\r\nSubject: third\r\n"
        )
    );
    assert_eq!(
        message.body().unwrap().headers(),
        &fields(b"Content-Type: text/plain\r\nContent-X: inside\r\n")
    );

    let message = MessagePartial::join(
        &fields(b"Subject: discarded\r\n"),
        &mut [fragment(1, 1, b"\r\nbody")],
    )
    .unwrap()
    .unwrap();
    assert!(message.headers().is_empty());
}

#[test]
fn stored_encodings_control_each_decoder_independently_of_outer_headers() {
    let mut first = fragment(1, 2, b"Subject: original=0D=0A=0D=0Ahel");
    first.part_mut().set_content(MimeContent::with_encoding(
        Cursor::new(b"Subject: original=0D=0A=0D=0Ahel".to_vec()),
        ContentEncoding::QuotedPrintable,
    ));
    let mut second = fragment(2, 2, b"bG8=");
    second.part_mut().set_content(MimeContent::with_encoding(
        Cursor::new(b"bG8=".to_vec()),
        ContentEncoding::Base64,
    ));
    for part in [&mut first, &mut second] {
        part.part_mut()
            .headers_mut()
            .add(Header::parse(b"Content-Transfer-Encoding: unknown").unwrap());
        part.part_mut().content_mut().get_mut().set_position(2);
    }
    let mut joined = MessagePartial::join(&HeaderList::new(), &mut [second, first])
        .unwrap()
        .unwrap();
    assert_eq!(write(&mut joined), b"Subject: original\r\n\r\nhello");

    // Padding and codec finalization belong to each fragment, not to the chain.
    let mut parts = [fragment(1, 2, b""), fragment(2, 2, b"")];
    for (part, encoded) in parts
        .iter_mut()
        .zip([b"DQo=".as_slice(), b"eA==".as_slice()])
    {
        part.part_mut().set_content(MimeContent::with_encoding(
            Cursor::new(encoded.to_vec()),
            ContentEncoding::Base64,
        ));
    }
    assert_eq!(
        write(
            &mut MessagePartial::join(&HeaderList::new(), &mut parts)
                .unwrap()
                .unwrap()
        ),
        b"\r\nx"
    );
}

#[test]
fn missing_and_invalid_parameters_fail_before_io_even_for_a_single_fragment() {
    for name in ["id", "number", "total"] {
        let mut partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(NoIo)).unwrap();
        set_parameter(&mut partial, name, None);
        assert!(
            matches!(MessagePartial::join(&HeaderList::new(), &mut [partial]),
            Err(MessagePartialError::MissingParameter(actual)) if actual == name)
        );
    }
    for name in ["number", "total"] {
        let mut partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(NoIo)).unwrap();
        set_parameter(&mut partial, name, Some("invalid"));
        assert!(
            matches!(MessagePartial::join(&HeaderList::new(), &mut [partial]),
            Err(MessagePartialError::InvalidParameter { name: actual, .. }) if actual == name)
        );
    }
}

#[test]
fn mismatching_ids_counts_duplicates_and_missing_numbers_fail_before_io() {
    let envelope = HeaderList::new();
    let mut parts = [
        MessagePartial::with_content("id", 1, 2, MimeContent::new(NoIo)).unwrap(),
        MessagePartial::with_content("ID", 2, 2, MimeContent::new(NoIo)).unwrap(),
    ];
    assert!(matches!(
        MessagePartial::join(&envelope, &mut parts),
        Err(MessagePartialError::MismatchedId { index: 1 })
    ));
    set_parameter(&mut parts[1], "id", Some("id"));
    set_parameter(&mut parts[1], "total", Some("3"));
    assert!(matches!(
        MessagePartial::join(&envelope, &mut parts),
        Err(MessagePartialError::CountMismatch {
            expected: 3,
            actual: 2
        })
    ));
    set_parameter(&mut parts[1], "total", Some("2"));
    for (number, expected, actual) in [("1", 2, 1), ("3", 2, 3), ("0", 1, 0)] {
        set_parameter(&mut parts[1], "number", Some(number));
        assert!(matches!(MessagePartial::join(&envelope, &mut parts),
            Err(MessagePartialError::UnexpectedNumber { expected: e, actual: a })
            if (e, a) == (expected, actual)));
    }
}

#[test]
fn earlier_totals_are_ignored_and_final_total_is_required_after_sorting() {
    for earlier_total in [None, Some("invalid"), Some("99")] {
        let mut parts = [fragment(2, 2, b"body"), fragment(1, 2, b"\r\n")];
        set_parameter(&mut parts[1], "total", earlier_total);
        assert!(
            MessagePartial::join(&HeaderList::new(), &mut parts)
                .unwrap()
                .is_some()
        );
        set_parameter(&mut parts[0], "total", None);
        assert!(matches!(
            MessagePartial::join(&HeaderList::new(), &mut parts),
            Err(MessagePartialError::MissingParameter("total"))
        ));
    }
    let mut parts = [fragment(1, 1, b"\r\n")];
    set_parameter(&mut parts[0], "id", Some(""));
    assert!(
        MessagePartial::join(&HeaderList::new(), &mut parts)
            .unwrap()
            .is_some()
    );
    assert!(
        MessagePartial::join(&HeaderList::new(), &mut Vec::<MessagePartial>::new())
            .unwrap()
            .is_none()
    );
}

#[test]
fn parser_limits_apply_to_decoded_combined_message_not_each_fragment() {
    let wire = b"Subject: original\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b--\r\n";
    let header_length = b"Content-Type: multipart/mixed; boundary=b\r\n".len();
    let mut options = MimeEntityParseOptions {
        max_input_bytes: wire.len(),
        max_header_length: header_length,
        max_depth: 2,
        ..Default::default()
    };
    let mut parts = [fragment(1, 2, &wire[..20]), fragment(2, 2, &wire[20..])];
    assert!(
        MessagePartial::join_with_options(&HeaderList::new(), &mut parts, &options)
            .unwrap()
            .is_some()
    );
    for limit in ["bytes", "header", "depth"] {
        let mut reduced = options;
        match limit {
            "bytes" => reduced.max_input_bytes -= 1,
            "header" => reduced.max_header_length -= 1,
            _ => reduced.max_depth -= 1,
        }
        assert_io(
            MessagePartial::join_with_options(&HeaderList::new(), &mut parts, &reduced)
                .unwrap_err(),
            io::ErrorKind::InvalidData,
        );
    }
    options.max_input_bytes = 3;
    let encoded = MimeContent::with_encoding(Cursor::new(b"DQp4"), ContentEncoding::Base64);
    let mut parts = [MessagePartial::with_content("id", 1, 1, encoded).unwrap()];
    assert!(MessagePartial::join_with_options(&HeaderList::new(), &mut parts, &options).is_ok());
    options.max_input_bytes = 2;
    assert_io(
        MessagePartial::join_with_options(&HeaderList::new(), &mut parts, &options).unwrap_err(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn zero_limits_fail_before_opening_sources_and_for_empty_collections() {
    for option in [
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
    ] {
        let partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(NoIo)).unwrap();
        assert_io(
            MessagePartial::join_with_options(&HeaderList::new(), &mut [partial], &option)
                .unwrap_err(),
            io::ErrorKind::InvalidInput,
        );
        assert_io(
            MessagePartial::join_with_options(
                &HeaderList::new(),
                &mut Vec::<MessagePartial>::new(),
                &option,
            )
            .unwrap_err(),
            io::ErrorKind::InvalidInput,
        );
    }
}

#[derive(Debug)]
struct FailingSource {
    bytes: Cursor<Vec<u8>>,
    fail_seek: bool,
    fail_at_end: bool,
    interrupted: bool,
}

impl Read for FailingSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupted) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = self.bytes.read(buffer)?;
        if count == 0 && self.fail_at_end {
            Err(io::Error::other("tail read failure"))
        } else {
            Ok(count)
        }
    }
}

impl Seek for FailingSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.fail_seek {
            Err(io::Error::other("seek failure"))
        } else {
            self.bytes.seek(position)
        }
    }
}

#[test]
fn source_failures_and_incomplete_structures_are_not_hidden() {
    for fail_seek in [true, false] {
        let source = FailingSource {
            bytes: Cursor::new(b"\r\nbody".to_vec()),
            fail_seek,
            fail_at_end: true,
            interrupted: true,
        };
        let partial = MessagePartial::with_content("id", 1, 1, MimeContent::new(source)).unwrap();
        let error = MessagePartial::join(&HeaderList::new(), &mut [partial]).unwrap_err();
        assert!(error.to_string().contains(if fail_seek {
            "seek failure"
        } else {
            "tail read failure"
        }));
        assert_io(error, io::ErrorKind::Other);
    }
    let mut parts: Vec<_> = [b"\r\nbody".as_slice(), b"tail".as_slice()]
        .into_iter()
        .enumerate()
        .map(|(index, bytes)| {
            MessagePartial::with_content(
                "id",
                index as u32 + 1,
                2,
                MimeContent::new(FailingSource {
                    bytes: Cursor::new(bytes.to_vec()),
                    fail_seek: false,
                    fail_at_end: index == 1,
                    interrupted: true,
                }),
            )
            .unwrap()
        })
        .collect();
    assert_io(
        MessagePartial::join(&HeaderList::new(), &mut parts).unwrap_err(),
        io::ErrorKind::Other,
    );
    for wire in [
        b"Subject: no separator".as_slice(),
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\r\nno close".as_slice(),
    ] {
        assert_io(
            MessagePartial::join(&HeaderList::new(), &mut [fragment(1, 1, wire)]).unwrap_err(),
            io::ErrorKind::UnexpectedEof,
        );
    }
}

#[test]
fn opaque_message_option_is_forwarded_to_the_reassembled_parser() {
    let wire = b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nbody";
    let mut parts = [fragment(1, 1, wire)];
    let structured = MessagePartial::join(&HeaderList::new(), &mut parts)
        .unwrap()
        .unwrap();
    assert!(matches!(structured.body(), Some(MimeEntity::Message(_))));
    let opaque = MessagePartial::join_with_options(
        &HeaderList::new(),
        &mut parts,
        &MimeEntityParseOptions {
            parse_message_parts: false,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(opaque.body(), Some(MimeEntity::Part(_))));
}

#[test]
fn reassembled_message_does_not_borrow_fragment_sources() {
    let mut first = Cursor::new(b"Subject: original\r\n\r\n".to_vec());
    let mut second = Cursor::new(b"body".to_vec());
    let mut joined = {
        let mut fragments = [
            MessagePartial::with_content("id", 1, 2, MimeContent::new(&mut first)).unwrap(),
            MessagePartial::with_content("id", 2, 2, MimeContent::new(&mut second)).unwrap(),
        ];
        MessagePartial::join(&HeaderList::new(), &mut fragments)
            .unwrap()
            .unwrap()
    };
    first.get_mut().clear();
    second.get_mut().clear();
    assert_eq!(write(&mut joined), b"Subject: original\r\n\r\nbody");
}

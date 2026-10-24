// Includes cases adapted from MimeKit's UnitTests/MimePartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    BestEncodingError, ContentEncoding, EncodingConstraint, Header, HeaderList, MimeContent,
    MimePart, MimePartWriteOptions, PartPrepareError,
    encodings::{InvalidLineLength, NewLineFormat},
    filters::{MimeFilter, encoder_filter},
};

const CONSTRAINTS: [EncodingConstraint; 3] = [
    EncodingConstraint::SevenBit,
    EncodingConstraint::EightBit,
    EncodingConstraint::None,
];

const ENCODINGS: [ContentEncoding; 7] = [
    ContentEncoding::Unspecified,
    ContentEncoding::SevenBit,
    ContentEncoding::EightBit,
    ContentEncoding::Binary,
    ContentEncoding::Base64,
    ContentEncoding::QuotedPrintable,
    ContentEncoding::UuEncode,
];

fn headers(raw: &str) -> HeaderList {
    HeaderList::load(&mut raw.as_bytes()).unwrap()
}

fn part(media_type: &str, bytes: &[u8]) -> MimePart<Cursor<Vec<u8>>> {
    let mut source = Cursor::new(bytes.to_vec());
    source.set_position(3);
    MimePart::from_parts(
        headers(&format!("Content-Type: {media_type}\r\n")),
        MimeContent::new(source),
    )
}

struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("unexpected source read");
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("unexpected source seek");
    }
}

#[test]
fn safe_requested_encodings_skip_type_parsing_and_source_access() {
    for constraint in CONSTRAINTS {
        for token in [
            "BaSe64; junk",
            "quoted-printable",
            "uuencode",
            "x-uue",
            "binary",
        ] {
            if token == "binary" && constraint != EncodingConstraint::None {
                continue;
            }
            for type_header in ["", "Content-Type: invalid\r\n"] {
                let raw = format!(
                    "X-Before: a\r\n{type_header}content-transfer-encoding: {token}\r\n\
                     Content-Transfer-Encoding: unknown\r\nX-After: b\r\n"
                );
                let mut part = MimePart::from_parts(headers(&raw), MimeContent::new(NoIo));
                let before = part.headers().clone();
                part.prepare(constraint).unwrap();
                part.prepare_with_options(
                    constraint,
                    &MimePartWriteOptions {
                        max_line_length: 60,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(part.headers(), &before);
            }
        }
    }
}

#[test]
fn preparation_matches_upstream_repeated_and_binary_transitions() {
    let mut part = part("application/octet-stream", &[0; 64]);
    part.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Base64
    );
    let before = part.headers().clone();
    // Calling Prepare again must not change an already-safe representation.
    part.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(part.headers(), &before);
    part.set_content_transfer_encoding(ContentEncoding::Binary);
    part.prepare(EncodingConstraint::None).unwrap();
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Binary
    );
    part.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(
        part.content_transfer_encoding().unwrap(),
        ContentEncoding::Base64
    );
    assert_eq!(part.content().get_ref().position(), 3);
    assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
}

#[test]
fn text_preparation_covers_each_requested_encoding_and_constraint() {
    use ContentEncoding::{Base64, Binary, EightBit, QuotedPrintable, SevenBit, Unspecified};
    for (input, recommendations) in [
        (b"".as_slice(), [SevenBit, SevenBit, SevenBit]),
        (b"ASCII\r\n", [SevenBit, SevenBit, SevenBit]),
        (b"\0", [Base64, Base64, Binary]),
        (b"\xff", [Base64, EightBit, EightBit]),
        (
            b"From someone\n",
            [QuotedPrintable, QuotedPrintable, QuotedPrintable],
        ),
    ] {
        for (constraint, recommended) in CONSTRAINTS.into_iter().zip(recommendations) {
            for requested in ENCODINGS {
                let mut part = part("text/plain", input);
                part.set_content_transfer_encoding(requested);
                part.content_mut()
                    .set_new_line_format(Some(NewLineFormat::Mixed));
                let before = part.headers().clone();
                let shortcut = matches!(
                    requested,
                    Base64 | QuotedPrintable | ContentEncoding::UuEncode
                ) || (requested == Binary && constraint == EncodingConstraint::None);
                let expected = if shortcut || (requested == Unspecified && recommended == SevenBit)
                {
                    requested
                } else {
                    recommended
                };
                part.prepare(constraint).unwrap();
                assert_eq!(
                    part.content_transfer_encoding().unwrap(),
                    expected,
                    "{input:?}, {constraint:?}, {requested:?}"
                );
                if expected == requested {
                    assert_eq!(part.headers(), &before);
                }
                assert_eq!(part.content().get_ref().get_ref(), input);
                assert_eq!(part.content().encoding(), Unspecified);
                assert_eq!(part.content().new_line_format(), Some(NewLineFormat::Mixed));
                assert_eq!(
                    part.content().get_ref().position(),
                    if shortcut { 3 } else { 0 }
                );
            }
        }
    }
}

#[test]
fn unchanged_analyzed_encoding_preserves_raw_duplicates() {
    for (token, input, constraint) in [
        (
            "7BIT; junk",
            b"ascii\n".as_slice(),
            EncodingConstraint::SevenBit,
        ),
        ("8bit", b"\xff\n", EncodingConstraint::EightBit),
    ] {
        let mut part = part("text/plain", input);
        part.headers_mut().add(
            Header::parse(format!("content-transfer-encoding:\t{token}\r\n").as_bytes()).unwrap(),
        );
        part.headers_mut()
            .add(Header::parse(b"Content-Transfer-Encoding: invalid\r\n").unwrap());
        let before = part.headers().clone();
        part.prepare(constraint).unwrap();
        assert_eq!(part.headers(), &before);
        assert_eq!(part.content().get_ref().position(), 0);
    }
}

#[test]
fn changed_encoding_replaces_duplicates_at_the_first_slot() {
    let mut part = MimePart::from_parts(
        headers(
            "X-Before: a\r\ncontent-transfer-encoding:\t8BIT\r\n\
                 Content-Type: application/octet-stream\r\nX-Middle: b\r\n\
                 Content-Transfer-Encoding: unknown\r\nX-After: c\r\n",
        ),
        MimeContent::new(NoIo),
    );
    part.prepare(EncodingConstraint::SevenBit).unwrap();
    assert_eq!(
        part.headers(),
        &headers(
            "X-Before: a\r\nContent-Transfer-Encoding: base64\r\n\
         Content-Type: application/octet-stream\r\nX-Middle: b\r\nX-After: c\r\n"
        )
    );
}

#[test]
fn options_and_first_encoding_are_validated_before_type_or_io() {
    for raw in [
        "Content-Transfer-Encoding: base64\r\n",
        "Content-Type: invalid\r\nContent-Transfer-Encoding: invalid\r\n",
        "",
    ] {
        let mut part = MimePart::from_parts(headers(raw), MimeContent::new(NoIo));
        let before = part.headers().clone();
        for limit in [0, 59, 999, usize::MAX] {
            let error = part
                .prepare_with_options(
                    EncodingConstraint::SevenBit,
                    &MimePartWriteOptions {
                        max_line_length: limit,
                        ..Default::default()
                    },
                )
                .unwrap_err();
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_some());
            let PartPrepareError::InvalidOptions(inner) = error else {
                panic!("wrong error")
            };
            assert_eq!(inner.kind(), io::ErrorKind::InvalidInput);
            assert!(
                inner
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<InvalidLineLength>()
                    .is_some()
            );
        }
        assert!(matches!(
            part.prepare_with_options(
                EncodingConstraint::None,
                &MimePartWriteOptions {
                    new_line_format: NewLineFormat::Mixed,
                    ..Default::default()
                }
            ),
            Err(PartPrepareError::InvalidOptions(_))
        ));
        assert_eq!(part.headers(), &before);
    }

    for value in ["unknown", "", "(comment) base64"] {
        let mut part = MimePart::from_parts(
            headers(&format!(
                "Content-Transfer-Encoding: {value}\r\n\
                Content-Transfer-Encoding: base64\r\nContent-Type: invalid\r\n"
            )),
            MimeContent::new(NoIo),
        );
        let before = part.headers().clone();
        let error = part.prepare(EncodingConstraint::None).unwrap_err();
        assert!(matches!(
            error,
            PartPrepareError::ContentTransferEncoding(_)
        ));
        assert!(
            error
                .source()
                .unwrap()
                .is::<mimecraft::UnknownContentEncoding>()
        );
        assert_eq!(part.headers(), &before);
    }
}

#[test]
fn required_type_errors_leave_headers_unchanged_without_source_io() {
    for (raw, missing) in [
        ("Content-Transfer-Encoding: 7bit\r\n", true),
        (
            "Content-Type: invalid\r\nContent-Type: text/plain\r\n",
            false,
        ),
    ] {
        let mut part = MimePart::from_parts(headers(raw), MimeContent::new(NoIo));
        let before = part.headers().clone();
        let error = part.prepare(EncodingConstraint::SevenBit).unwrap_err();
        assert!(error.source().unwrap().is::<BestEncodingError>());
        match error {
            PartPrepareError::BestEncoding(BestEncodingError::MissingContentType) => {
                assert!(missing)
            }
            PartPrepareError::BestEncoding(BestEncodingError::ContentType(_)) => assert!(!missing),
            _ => panic!("wrong error"),
        }
        assert_eq!(part.headers(), &before);
    }
}

#[test]
fn analysis_decodes_stored_content_and_observes_later_edits() {
    let input = b"From someone\n";
    for encoding in ENCODINGS {
        let mut filter = encoder_filter(encoding);
        let mut stored = Vec::new();
        filter.filter(input, &mut stored);
        filter.finish(&mut stored);
        if encoding == ContentEncoding::UuEncode {
            stored = [b"begin 0644 file.txt\r\n".as_slice(), &stored, b"end\r\n"].concat();
        }
        let mut part = MimePart::from_parts(
            headers("Content-Type: message/x-custom\r\n"),
            MimeContent::with_encoding(Cursor::new(stored.clone()), encoding),
        );
        part.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::QuotedPrintable
        );
        assert_eq!(part.content().get_ref().get_ref(), &stored);
        assert_eq!(part.content().encoding(), encoding);

        part.set_content_transfer_encoding(ContentEncoding::Unspecified);
        part.set_content(MimeContent::new(Cursor::new(b"ascii\n".to_vec())));
        part.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::Unspecified
        );
        part.headers_mut()
            .replace_all(Header::parse(b"Content-Type: image/png\r\n").unwrap())
            .unwrap();
        part.prepare(EncodingConstraint::None).unwrap();
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::Binary
        );
    }
}

#[test]
fn explicit_options_control_analysis_and_subsequent_encoder_width() {
    for media_type in ["text/plain", "application/octet-stream"] {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let input = vec![b'x'; 180];
            let mut part = part(media_type, &input);
            let options = MimePartWriteOptions {
                max_line_length: 60,
                new_line_format: format,
                ..Default::default()
            };
            part.prepare_with_options(EncodingConstraint::SevenBit, &options)
                .unwrap();
            let encoding = part.content_transfer_encoding().unwrap();
            assert_eq!(
                encoding,
                if media_type == "text/plain" {
                    ContentEncoding::QuotedPrintable
                } else {
                    ContentEncoding::Base64
                }
            );
            let mut output = Vec::new();
            part.write_content_to_with_options(&mut output, &options)
                .unwrap();
            let longest = output
                .split(|&b| b == b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line).len())
                .max()
                .unwrap();
            assert_eq!(longest, 60);
            let mut decoded = Vec::new();
            MimeContent::with_encoding(Cursor::new(output.clone()), encoding)
                .decode_to(&mut decoded)
                .unwrap();
            assert_eq!(decoded, input);
            let mut complete = Vec::new();
            part.write_to_with_options(&mut complete, &options).unwrap();
            assert!(complete.ends_with(&output));
            let mut default_output = Vec::new();
            part.write_content_to(&mut default_output).unwrap();
            assert!(
                default_output.split(|&b| b == b'\n').any(|line| line
                    .strip_suffix(b"\r")
                    .unwrap_or(line)
                    .len()
                    > 60)
            );
        }
    }
    for width in [60, 78, 998] {
        for length in [width, width + 1] {
            let mut part = part("text/plain", &vec![b'x'; length]);
            part.prepare_with_options(
                EncodingConstraint::SevenBit,
                &MimePartWriteOptions {
                    max_line_length: width,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                part.content_transfer_encoding().unwrap(),
                if length == width {
                    ContentEncoding::Unspecified
                } else {
                    ContentEncoding::QuotedPrintable
                }
            );
        }
    }
}

#[test]
fn safe_encoded_content_is_not_verified_or_rewrapped() {
    let stored = [vec![b'A'; 120], b"\r\n".to_vec()].concat();
    let mut part = MimePart::new(MimeContent::with_encoding(
        Cursor::new(stored.clone()),
        ContentEncoding::Base64,
    ));
    part.set_content_transfer_encoding(ContentEncoding::Base64);
    let options = MimePartWriteOptions {
        max_line_length: 60,
        ..Default::default()
    };
    part.prepare_with_options(EncodingConstraint::SevenBit, &options)
        .unwrap();
    let mut output = Vec::new();
    part.write_content_to_with_options(&mut output, &options)
        .unwrap();
    assert_eq!(output, stored);
}

struct FailingSource {
    cursor: Cursor<Vec<u8>>,
    seeks: usize,
    fail_seek: Option<usize>,
    fail_read: bool,
}

impl Read for FailingSource {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.fail_read {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read failed",
            ));
        }
        self.cursor.read(output)
    }
}

impl Seek for FailingSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.seeks += 1;
        if self.fail_seek == Some(self.seeks) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "seek failed",
            ));
        }
        self.cursor.seek(position)
    }
}

#[test]
fn analysis_io_failures_are_atomic_and_can_be_retried_with_a_borrowed_source() {
    for (fail_seek, fail_read, seeks) in [(Some(1), false, 1), (Some(2), false, 2), (None, true, 1)]
    {
        let mut source = FailingSource {
            cursor: Cursor::new(b"From someone\n".to_vec()),
            seeks: 0,
            fail_seek,
            fail_read,
        };
        let mut part = MimePart::from_parts(
            headers("Content-Type: text/plain\r\nContent-Transfer-Encoding: 8bit\r\n"),
            MimeContent::new(&mut source),
        );
        let before = part.headers().clone();
        let error = part.prepare(EncodingConstraint::SevenBit).unwrap_err();
        assert!(error.source().unwrap().is::<BestEncodingError>());
        let PartPrepareError::BestEncoding(BestEncodingError::Io(inner)) = error else {
            panic!("wrong error");
        };
        assert_eq!(inner.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(part.headers(), &before);
        assert_eq!(part.content().get_ref().seeks, seeks);
        part.content_mut().get_mut().fail_seek = None;
        part.content_mut().get_mut().fail_read = false;
        part.prepare(EncodingConstraint::SevenBit).unwrap();
        assert_eq!(
            part.content_transfer_encoding().unwrap(),
            ContentEncoding::QuotedPrintable
        );
        assert_eq!(part.content().get_ref().cursor.position(), 0);
        assert_eq!(part.content().get_ref().cursor.get_ref(), b"From someone\n");
    }
}

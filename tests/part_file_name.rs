// Adapted from MimeKit's MimePart.FileName and UnitTests/MimePartTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
};

use mimecraft::{
    ContentEncoding, FileNameEditError, Header, HeaderFormatError, HeaderFormatOptions, HeaderList,
    MimeContent, MimeMessage, MimePart, ParameterComplianceMode, ParameterParseOptions, TextPart,
    encodings::NewLineFormat,
};

#[derive(Debug)]
struct NoIo;

impl Read for NoIo {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("filename metadata must not read content")
    }
}

impl Seek for NoIo {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("filename metadata must not seek content")
    }
}

fn headers(raw: &[u8]) -> HeaderList {
    HeaderList::load(&mut &raw[..]).unwrap()
}

fn part(raw: &[u8]) -> MimePart<NoIo> {
    MimePart::from_parts(headers(raw), MimeContent::new(NoIo))
}

#[test]
fn getter_precedence_trimming_and_absence_match_upstream() {
    for (raw, expected) in [
        ("", None),
        ("Content-Disposition: attachment\n", None),
        ("Content-Type: text/plain\n", None),
        (
            "Content-Type: text/plain; name=type.txt\n",
            Some("type.txt"),
        ),
        (
            "Content-Disposition: inline; filename=preferred.txt\nContent-Type: invalid\n",
            Some("preferred.txt"),
        ),
        (
            "Content-Disposition: attachment; filename=\"\"\nContent-Type: text/plain; name=fallback.txt\n",
            Some(""),
        ),
        (
            "Content-Disposition: attachment; filename=\"   \"\nContent-Type: text/plain; name=fallback.txt\n",
            Some(""),
        ),
        (
            "Content-Disposition: attachment\nContent-Type: text/plain; name=\" fallback.txt \"\n",
            Some("fallback.txt"),
        ),
        (
            "Content-Disposition: attachment; filename*=utf-8''%C2%A0caf%C3%A9.txt%C2%A0\n",
            Some("caf\u{e9}.txt"),
        ),
        (
            "Content-Disposition: inline; filename=first.txt\nContent-Disposition: attachment; filename=second.txt\n",
            Some("first.txt"),
        ),
        (
            "Content-Type: text/plain\nContent-Type: text/plain; name=ignored.txt\n",
            None,
        ),
    ] {
        let part = part(raw.as_bytes());
        let before = part.headers().clone();
        assert_eq!(part.file_name().unwrap().as_deref(), expected, "{raw}");
        assert_eq!(part.headers(), &before);
    }
}

#[test]
fn getter_uses_tolerant_parameter_decoding_and_explicit_options() {
    let part = part(b"Content-Disposition: attachment; filename=\"=?utf-8?q?caf=C3=A9.txt?=\"\n");
    assert_eq!(part.file_name().unwrap().as_deref(), Some("caf\u{e9}.txt"));
    let continuation = self::part(b"Content-Disposition: attachment; filename=ignored.txt; filename*1*=.txt; filename*0*=utf-8''caf%C3%A9\n");
    assert_eq!(
        continuation.file_name().unwrap().as_deref(),
        Some("caf\u{e9}.txt")
    );

    let options = ParameterParseOptions {
        compliance_mode: ParameterComplianceMode::Strict,
        ..Default::default()
    };
    for raw in [
        b"Content-Disposition: attachment; filename=two words.txt\n".as_slice(),
        b"Content-Type: text/plain; name=two words.txt\n",
    ] {
        let part = self::part(raw);
        assert_eq!(part.file_name().unwrap().as_deref(), Some("two words.txt"));
        let error = if raw.starts_with(b"Content-Disposition") {
            part.content_disposition_with_options(&options).unwrap_err()
        } else {
            part.content_type_with_options(&options).unwrap_err()
        };
        assert_eq!(part.file_name_with_options(&options), Err(error));
    }
}

#[test]
fn invalid_first_headers_report_original_offsets_without_fallback() {
    for raw in [
        b"Content-Disposition: ;\nContent-Type: text/plain; name=valid.txt\n".as_slice(),
        b"Content-Disposition: ;\nContent-Disposition: attachment; filename=valid.txt\n",
        b"Content-Disposition: inline\nContent-Type: invalid\nContent-Type: text/plain; name=valid.txt\n",
    ] {
        let part = part(raw);
        let before = part.headers().clone();
        let error = match part.content_disposition() {
            Err(error) => error,
            Ok(_) => part.content_type().unwrap_err(),
        };
        assert_eq!(part.file_name(), Err(error));
        assert_eq!(part.headers(), &before);
    }
}

#[test]
fn returned_filenames_are_detached_unsanitized_values_and_raw_edits_are_visible() {
    let mut part = part(b"Content-Disposition: attachment; filename=\"../outside.txt\"\nContent-Type: text/plain; name=fallback.txt\n");
    let saved = part.file_name().unwrap().unwrap();
    assert_eq!(saved, "../outside.txt");
    part.headers_mut()
        .replace_all(
            Header::parse(b"Content-Disposition: inline; filename*=utf-8''bad%00name").unwrap(),
        )
        .unwrap();
    assert_eq!(part.file_name().unwrap().as_deref(), Some("bad\0name"));
    part.headers_mut().remove_all("Content-Disposition");
    assert_eq!(part.file_name().unwrap().as_deref(), Some("fallback.txt"));
    assert_eq!(saved, "../outside.txt");
    part.headers_mut().clear();
    assert_eq!(part.file_name().unwrap(), None);
}

#[test]
fn assignments_update_both_parameters_and_only_create_missing_dispositions() {
    let mut part = part(b"Content-Type: text/plain; charset=utf-8\n");
    part.set_file_name(Some("report.txt")).unwrap();
    assert_eq!(part.file_name().unwrap().as_deref(), Some("report.txt"));
    assert_eq!(
        part.content_type().unwrap().unwrap().name(),
        Some("report.txt")
    );
    assert_eq!(
        part.content_type().unwrap().unwrap().charset(),
        Some("utf-8")
    );
    assert!(part.is_attachment().unwrap());
    assert_eq!(part.headers().len(), 2);

    for token in ["inline", "X-Custom", "ATTACHMENT"] {
        let raw = format!(
            "Content-Type: image/png; x=type\nContent-Disposition: {token}; size=123; x=disposition\n"
        );
        let mut part = self::part(raw.as_bytes());
        part.set_file_name(Some(" image.png ")).unwrap();
        let disposition = part.content_disposition().unwrap().unwrap();
        assert_eq!(disposition.disposition(), token);
        assert_eq!(disposition.file_name(), Some(" image.png "));
        assert_eq!(disposition.parameters().get("size"), Some("123"));
        assert_eq!(disposition.parameters().get("x"), Some("disposition"));
        let content_type = part.content_type().unwrap().unwrap();
        assert_eq!(content_type.name(), Some(" image.png "));
        assert_eq!(content_type.parameters().get("x"), Some("type"));
        assert_eq!(part.file_name().unwrap().as_deref(), Some("image.png"));
    }
}

#[test]
fn empty_names_are_distinct_from_removal_and_removal_preserves_header_fields() {
    let mut part = part(b"Content-Type: application/octet-stream; x=keep\n");
    part.set_file_name(Some("")).unwrap();
    assert_eq!(part.file_name().unwrap().as_deref(), Some(""));
    assert_eq!(
        part.content_disposition().unwrap().unwrap().file_name(),
        Some("")
    );
    assert_eq!(part.content_type().unwrap().unwrap().name(), Some(""));
    part.set_file_name(None).unwrap();
    assert_eq!(part.file_name().unwrap(), None);
    assert!(part.is_attachment().unwrap());
    assert_eq!(part.headers().len(), 2);
    assert_eq!(
        part.content_type().unwrap().unwrap().parameters().get("x"),
        Some("keep")
    );

    for raw in [
        b"Content-Disposition: inline; filename=remove.txt; x=keep\n".as_slice(),
        b"Content-Type: text/plain; name=remove.txt; x=keep\n",
    ] {
        let mut part = self::part(raw);
        part.set_file_name(None).unwrap();
        assert_eq!(part.file_name().unwrap(), None);
        assert_eq!(part.headers().len(), 1);
        assert!(
            part.headers().as_slice()[0]
                .raw()
                .ends_with(b"; x=keep\r\n")
        );
    }
}

#[test]
fn missing_content_type_is_an_explicit_atomic_error_for_assignment() {
    for raw in [
        b"".as_slice(),
        b"Content-Disposition: inline; filename=old.txt\n",
    ] {
        let mut part = part(raw);
        let before = part.headers().clone();
        for name in ["new.txt", ""] {
            let error = part.set_file_name(Some(name)).unwrap_err();
            assert_eq!(error, FileNameEditError::MissingContentType);
            assert!(error.source().is_none());
            assert!(!error.to_string().is_empty());
            assert_eq!(part.headers(), &before);
        }
    }
    let mut part = part(b"X-Keep: raw\n");
    let before = part.headers().clone();
    part.set_file_name_with_options(
        None,
        &HeaderFormatOptions {
            max_line_length: 0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(part.headers(), &before);
}

#[test]
fn changed_and_equal_assignments_canonicalize_both_headers_and_collapse_duplicates() {
    for type_first in [false, true] {
        let type_header = b"content-type: text/plain; name=same.txt\n".as_slice();
        let disposition_header =
            b"content-disposition: (comment) inline; filename=same.txt\n".as_slice();
        let mut raw = b"X-Before: raw\n".to_vec();
        raw.extend_from_slice(if type_first {
            type_header
        } else {
            disposition_header
        });
        raw.extend_from_slice(b"Malformed record\n");
        raw.extend_from_slice(if type_first {
            disposition_header
        } else {
            type_header
        });
        raw.extend_from_slice(
            b"Content-Type: broken\nContent-Disposition: broken syntax\nX-After: raw\n",
        );
        for name in ["same.txt", "new.txt"] {
            let mut part = part(&raw);
            part.set_file_name(Some(name)).unwrap();
            assert_eq!(part.headers().len(), 5);
            assert_eq!(part.headers().as_slice()[0].raw(), b"X-Before: raw\n");
            assert_eq!(part.headers().as_slice()[2].raw(), b"Malformed record\n");
            assert_eq!(part.headers().as_slice()[4].raw(), b"X-After: raw\n");
            let expected_type = format!("Content-Type: text/plain; name={name}\r\n");
            let expected_disposition = format!("Content-Disposition: inline; filename={name}\r\n");
            assert_eq!(
                part.headers().as_slice()[1].raw(),
                if type_first {
                    expected_type.as_bytes()
                } else {
                    expected_disposition.as_bytes()
                }
            );
            assert_eq!(
                part.headers().as_slice()[3].raw(),
                if type_first {
                    expected_disposition.as_bytes()
                } else {
                    expected_type.as_bytes()
                }
            );
        }
    }
}

#[test]
fn invalid_metadata_is_not_replaced_or_partially_edited_even_on_removal() {
    for raw in [
        b"Content-Disposition: ;\nContent-Type: text/plain; name=old.txt\n".as_slice(),
        b"Content-Disposition: inline; filename=old.txt\nContent-Type: invalid\nContent-Type: text/plain\n",
    ] {
        let mut part = part(raw);
        let before = part.headers().clone();
        let expected = match part.content_disposition() {
            Err(error) => FileNameEditError::ContentDisposition(error),
            Ok(_) => FileNameEditError::ContentType(part.content_type().unwrap_err()),
        };
        for value in [Some("new.txt"), None] {
            let error = part.set_file_name(value).unwrap_err();
            assert_eq!(error, expected);
            assert_eq!(error.to_string(), error.source().unwrap().to_string());
            assert_eq!(part.headers(), &before);
        }
    }
}

#[test]
fn both_replacements_are_formatted_before_either_header_changes() {
    for raw in [
        format!(
            "Content-Disposition: inline; filename=old.txt\nContent-Type: application/{}; name=old.txt\n",
            "x".repeat(100)
        ),
        format!(
            "Content-Type: text/plain; name=old.txt\nContent-Disposition: {}; filename=old.txt\n",
            "x".repeat(100)
        ),
        format!(
            "Content-Type: application/{}; name=old.txt\n",
            "x".repeat(100)
        ),
    ] {
        let mut part = part(raw.as_bytes());
        let before = part.headers().clone();
        for value in [Some("new.txt"), None] {
            let error = part.set_file_name(value).unwrap_err();
            assert!(matches!(
                error,
                FileNameEditError::HeaderFormat(HeaderFormatError::LineTooLong { .. })
            ));
            assert_eq!(error.to_string(), error.source().unwrap().to_string());
            assert_eq!(part.headers(), &before);
        }
    }
    for options in [
        HeaderFormatOptions {
            max_line_length: 59,
            ..Default::default()
        },
        HeaderFormatOptions {
            new_line_format: NewLineFormat::Mixed,
            ..Default::default()
        },
    ] {
        let mut part = part(b"Content-Disposition: inline; filename=old.txt\nContent-Type: text/plain; name=old.txt\n");
        let before = part.headers().clone();
        for value in [Some("new.txt"), None] {
            assert!(matches!(
                part.set_file_name_with_options(value, &options),
                Err(FileNameEditError::HeaderFormat(_))
            ));
            assert_eq!(part.headers(), &before);
        }
    }
}

#[test]
fn checked_authoring_preserves_literal_values_and_physical_line_limits() {
    for name in [
        " caf\u{e9}.txt ",
        "long filename that requires folding and encoding with caf\u{e9} and more text.txt",
        "=?utf-8?q?literal.txt?=",
        "report\r\nX-Injected: yes\0.txt",
        "../not-a-safe-path.txt",
    ] {
        for new_line_format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let options = HeaderFormatOptions {
                max_line_length: 60,
                new_line_format,
                always_quote_parameters: true,
            };
            let mut part = part(b"Content-Type: application/octet-stream\n");
            part.set_file_name_with_options(Some(name), &options)
                .unwrap();
            assert_eq!(part.headers().len(), 2);
            assert_eq!(part.content_type().unwrap().unwrap().name(), Some(name));
            assert_eq!(
                part.content_disposition().unwrap().unwrap().file_name(),
                Some(name)
            );
            assert_eq!(part.file_name().unwrap().as_deref(), Some(name.trim()));
            for header in part.headers().iter() {
                assert!(header.raw().is_ascii());
                assert!(!header.raw().contains(&0));
                for line in header.raw().split(|&b| b == b'\n') {
                    assert!(line.strip_suffix(b"\r").unwrap_or(line).len() <= 60);
                }
                if new_line_format == NewLineFormat::Unix {
                    assert!(!header.raw().contains(&b'\r'));
                }
            }
        }
    }
}

#[test]
fn generic_metadata_access_needs_no_source_traits_and_leaves_borrowed_content_untouched() {
    fn edit<R>(part: &mut MimePart<R>) {
        part.set_file_name(Some("file.txt")).unwrap();
        assert_eq!(part.file_name().unwrap().as_deref(), Some("file.txt"));
        part.set_file_name(None).unwrap();
    }
    edit(&mut part(b"Content-Type: application/octet-stream\n"));
    let mut source = Cursor::new(b"Ym9keQ==".to_vec());
    source.set_position(3);
    {
        let mut part = MimePart::new(MimeContent::with_encoding(
            &mut source,
            ContentEncoding::Base64,
        ));
        edit(&mut part);
        assert_eq!(part.content().encoding(), ContentEncoding::Base64);
    }
    assert_eq!(source.position(), 3);
    assert_eq!(source.into_inner(), b"Ym9keQ==");
}

#[test]
fn text_parts_and_serialized_messages_observe_both_filename_parameters() {
    let mut text = TextPart::new("plain", "body").unwrap();
    text.part_mut().set_file_name(Some("body.txt")).unwrap();
    assert_eq!(
        text.part().file_name().unwrap().as_deref(),
        Some("body.txt")
    );
    let mut message = MimeMessage::with_body(text.into());
    let mut output = Vec::new();
    message.write_to(&mut output).unwrap();
    let loaded = MimeMessage::load(&mut output.as_slice()).unwrap();
    let leaf = loaded.leaf_parts().next().unwrap();
    assert_eq!(leaf.file_name().unwrap().as_deref(), Some("body.txt"));
    assert_eq!(
        leaf.content_type().unwrap().unwrap().name(),
        Some("body.txt")
    );
    assert_eq!(loaded.attachments().count(), 1);
}

#[test]
fn uu_writing_rejects_controls_before_filename_trimming() {
    for name in ["\tfile.txt", "file.txt\r\n", "\u{0085}file.txt"] {
        let mut part = MimePart::new(MimeContent::new(NoIo));
        part.set_file_name(Some(name)).unwrap();
        assert_eq!(part.file_name().unwrap().as_deref(), Some("file.txt"));
        part.set_content_transfer_encoding(ContentEncoding::UuEncode);
        let mut output = Vec::new();
        assert_eq!(
            part.write_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.is_empty());
        assert_eq!(
            part.write_content_to(&mut output).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.is_empty());
    }
    let mut part = MimePart::new(MimeContent::new(Cursor::new(b"abc")));
    part.set_file_name(Some(" file.txt ")).unwrap();
    part.set_content_transfer_encoding(ContentEncoding::UuEncode);
    let mut output = Vec::new();
    part.write_content_to(&mut output).unwrap();
    assert_eq!(output, b"begin 0644 file.txt\r\n#86)C\r\n`\r\nend\r\n");
}

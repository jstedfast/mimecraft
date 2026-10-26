// Adapted from MimeKit's UnitTests/AttachmentCollectionTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

use mimecraft::{
    AttachmentError, AttachmentOptions, BodyBuilder, ContentEncoding, ContentType,
    HeaderFormatError, MimeEntity, MimeEntityParseOptions, MimePart, MultipartRelated, TextFormat,
    attachment,
};

const IMAGE: &[u8] = include_bytes!("data/attachments/girl.jpg");
const TEXT: &[u8] = include_bytes!("data/attachments/lorem-ipsum.txt");
const MESSAGE: &[u8] = include_bytes!("data/messages/body.1.txt");

fn part(entity: &MimeEntity) -> &MimePart<Cursor<Vec<u8>>> {
    let MimeEntity::Part(part) = entity else {
        panic!("expected an opaque MIME part")
    };
    part
}

fn assert_metadata(entity: &MimeEntity, media_type: &str, name: &str, linked: bool) {
    let headers = entity.headers();
    let content_type = headers
        .get("Content-Type")
        .unwrap()
        .parse_content_type()
        .unwrap();
    assert_eq!(content_type.mime_type(), media_type);
    assert_eq!(content_type.name(), Some(name));
    let disposition = headers
        .get("Content-Disposition")
        .unwrap()
        .parse_content_disposition()
        .unwrap();
    assert_eq!(
        disposition.disposition(),
        if linked { "inline" } else { "attachment" }
    );
    assert_eq!(disposition.file_name(), Some(name));
    assert!(entity.content_id().unwrap().is_none());
    assert_eq!(headers.contains("Content-Location"), linked);
}

fn assert_raw(entity: &MimeEntity, data: &[u8], encoding: ContentEncoding) {
    let part = part(entity);
    assert_eq!(part.content_transfer_encoding().unwrap(), encoding);
    assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
    assert_eq!(part.content().get_ref().position(), 0);
    assert_eq!(part.content().get_ref().get_ref(), data);
}

fn with_file(content: &[u8], test: impl FnOnce(&Path)) {
    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&directory).unwrap();
    let path = loop {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "mimecraft-attachment-{}-{id}.eml",
            std::process::id()
        ));
        match File::create_new(&path) {
            Ok(mut file) => {
                file.write_all(content).unwrap();
                break path;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("could not create attachment fixture: {error}"),
        }
    };
    test(&path);
    fs::remove_file(path).unwrap();
}

fn assert_file_released(path: &Path) {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    options.share_mode(0);
    drop(options.open(path).unwrap());
}

#[test]
fn upstream_image_byte_reader_and_file_attachments_infer_or_override_types() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("attachments")
        .join("girl.jpg");
    for linked in [false, true] {
        for explicit in [false, true] {
            let options = AttachmentOptions {
                content_type: explicit.then(|| ContentType::new("image", "gif").unwrap()),
                linked_resource: linked,
                ..AttachmentOptions::default()
            };
            let mut reader = IMAGE;
            let entities = [
                attachment::from_bytes_with_options(path.to_str().unwrap(), IMAGE, &options)
                    .unwrap(),
                attachment::from_reader_with_options(path.to_str().unwrap(), &mut reader, &options)
                    .unwrap(),
                attachment::from_file_with_options(&path, &options).unwrap(),
            ];
            assert!(reader.is_empty());
            for entity in entities {
                assert_metadata(
                    &entity,
                    if explicit { "image/gif" } else { "image/jpeg" },
                    "girl.jpg",
                    linked,
                );
                assert_eq!(
                    part(&entity).file_name().unwrap().as_deref(),
                    Some("girl.jpg")
                );
                assert_raw(&entity, IMAGE, ContentEncoding::Base64);
                if linked {
                    assert_eq!(
                        entity
                            .headers()
                            .get("Content-Location")
                            .unwrap()
                            .decode_value()
                            .text,
                        "girl.jpg"
                    );
                }
            }
        }
    }
    for entity in [
        attachment::from_bytes("girl.jpg", IMAGE).unwrap(),
        attachment::from_reader("girl.jpg", &mut &IMAGE[..]).unwrap(),
        attachment::from_file(&path).unwrap(),
    ] {
        assert_metadata(&entity, "image/jpeg", "girl.jpg", false);
        assert_raw(&entity, IMAGE, ContentEncoding::Base64);
    }
}

#[test]
fn upstream_text_fixture_selects_seven_bit_without_guessing_charset() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("attachments")
        .join("lorem-ipsum.txt");
    for entity in [
        attachment::from_bytes("lorem-ipsum.txt", TEXT).unwrap(),
        attachment::from_reader("lorem-ipsum.txt", &mut &TEXT[..]).unwrap(),
        attachment::from_file(path).unwrap(),
    ] {
        assert_metadata(&entity, "text/plain", "lorem-ipsum.txt", false);
        assert_raw(&entity, TEXT, ContentEncoding::SevenBit);
        assert!(
            part(&entity)
                .content_type()
                .unwrap()
                .unwrap()
                .parameters()
                .get("charset")
                .is_none()
        );
    }
}

#[test]
fn text_encoding_thresholds_and_explicit_parameters_preserve_raw_bytes() {
    for (data, expected) in [
        (Vec::new(), ContentEncoding::SevenBit),
        (b"plain\r\n".to_vec(), ContentEncoding::SevenBit),
        (vec![b'x'; 78], ContentEncoding::SevenBit),
        (vec![b'x'; 79], ContentEncoding::QuotedPrintable),
        (
            b"From sender\r\n".to_vec(),
            ContentEncoding::QuotedPrintable,
        ),
        (b"From sender".to_vec(), ContentEncoding::SevenBit),
        (b"a\0b".to_vec(), ContentEncoding::Base64),
        (
            b"hello world caf\xe9\r\n".to_vec(),
            ContentEncoding::QuotedPrintable,
        ),
        (vec![0xff; 20], ContentEncoding::Base64),
    ] {
        let content_type =
            ContentType::parse(b"text/plain; charset=iso-8859-1; format=flowed; name=old").unwrap();
        let options = AttachmentOptions {
            content_type: Some(content_type),
            ..Default::default()
        };
        let entity = attachment::from_bytes_with_options("new.txt", &data, &options).unwrap();
        assert_raw(&entity, &data, expected);
        let actual = part(&entity).content_type().unwrap().unwrap();
        assert_eq!(actual.parameters().get("charset"), Some("iso-8859-1"));
        assert_eq!(actual.parameters().get("format"), Some("flowed"));
        assert_eq!(actual.name(), Some("new.txt"));
        assert_eq!(options.content_type.unwrap().name(), Some("old"));
    }
    for media_type in [
        "application/octet-stream",
        "image/png",
        "message/global",
        "message/delivery-status",
    ] {
        let options = AttachmentOptions {
            content_type: Some(ContentType::parse(media_type.as_bytes()).unwrap()),
            ..Default::default()
        };
        let entity =
            attachment::from_bytes_with_options("opaque.txt", b"plain\r\n", &options).unwrap();
        assert_raw(&entity, b"plain\r\n", ContentEncoding::Base64);
    }
}

fn assert_message(entity: &mut MimeEntity, name: &str, linked: bool) {
    assert_metadata(entity, "message/rfc822", name, linked);
    assert!(!entity.headers().contains("Content-Transfer-Encoding"));
    let MimeEntity::Message(part) = entity else {
        panic!("expected message/rfc822 wrapper")
    };
    let message = part.message_mut().unwrap();
    assert_eq!(
        message
            .headers()
            .get("Subject")
            .unwrap()
            .decode_value()
            .text,
        "MimeMessage.TextBody and HtmlBody tests"
    );
    assert_eq!(
        message.text_body(TextFormat::Plain).unwrap().as_deref(),
        Some("This is the text body.")
    );
}

#[test]
fn upstream_message_attachments_cover_inferred_explicit_and_inline_inputs() {
    for linked in [false, true] {
        for explicit in [false, true] {
            let options = AttachmentOptions {
                content_type: explicit.then(|| {
                    ContentType::parse(b"message/rfc822; name=old; x-extra=value").unwrap()
                }),
                linked_resource: linked,
                ..Default::default()
            };
            for mut entity in [
                attachment::from_bytes_with_options("forwarded.eml", MESSAGE, &options).unwrap(),
                attachment::from_reader_with_options("forwarded.eml", &mut &MESSAGE[..], &options)
                    .unwrap(),
            ] {
                assert_message(&mut entity, "forwarded.eml", linked);
                let content_type = entity
                    .headers()
                    .get("Content-Type")
                    .unwrap()
                    .parse_content_type()
                    .unwrap();
                assert!(content_type.parameters().get("x-extra").is_none());
            }
            with_file(MESSAGE, |path| {
                let mut entity = attachment::from_file_with_options(path, &options).unwrap();
                assert_file_released(path);
                fs::write(path, b"changed").unwrap();
                assert_message(
                    &mut entity,
                    path.file_name().unwrap().to_str().unwrap(),
                    linked,
                );
            });
        }
    }
}

#[test]
fn invalid_messages_return_errors_unless_explicitly_attached_as_binary() {
    // Unlike MimeKit's auto-detected seekable-stream fallback, these helpers
    // require an explicit binary type instead of hiding a message parse error.
    for data in [&[0xff, 0x00, 0xff][..], IMAGE] {
        for explicit in [false, true] {
            let options = AttachmentOptions {
                content_type: explicit.then(|| ContentType::new("message", "rfc822").unwrap()),
                ..Default::default()
            };
            assert!(matches!(
                attachment::from_bytes_with_options("bad.eml", data, &options),
                Err(AttachmentError::Message(_))
            ));
            let mut reader = Cursor::new(data);
            assert!(matches!(
                attachment::from_reader_with_options("bad.eml", &mut reader, &options),
                Err(AttachmentError::Message(_))
            ));
            assert_eq!(reader.position(), data.len() as u64);
            with_file(data, |path| {
                let error = attachment::from_file_with_options(path, &options).unwrap_err();
                assert!(matches!(error, AttachmentError::Message(_)));
                assert!(error.source().is_some());
                assert_file_released(path);
            });
        }
        let options = AttachmentOptions {
            content_type: Some(ContentType::new("application", "octet-stream").unwrap()),
            ..Default::default()
        };
        let entity = attachment::from_bytes_with_options("bad.eml", data, &options).unwrap();
        assert_metadata(&entity, "application/octet-stream", "bad.eml", false);
        assert_raw(&entity, data, ContentEncoding::Base64);
    }
}

#[test]
fn basenames_and_linked_uri_escaping_preserve_literal_filenames() {
    let options = AttachmentOptions {
        linked_resource: true,
        ..Default::default()
    };
    for (path, name, location) in [
        (r"C:\images\girl.JPG", "girl.JPG", "girl.JPG"),
        ("/images/girl.jpg", "girl.jpg", "girl.jpg"),
        (r"mixed\images/girl.jpg", "girl.jpg", "girl.jpg"),
        ("a:b#c?d%20.jpg", "a:b#c?d%20.jpg", "a%3Ab%23c%3Fd%2520.jpg"),
        (
            "caf\u{e9} \u{1f600}.png",
            "caf\u{e9} \u{1f600}.png",
            "caf%C3%A9%20%F0%9F%98%80.png",
        ),
        ("~a-b_c.d", "~a-b_c.d", "~a-b_c.d"),
        (" report.txt ", " report.txt ", "%20report.txt%20"),
    ] {
        let mut entity = attachment::from_bytes_with_options(path, IMAGE, &options).unwrap();
        assert_eq!(
            part(&entity).file_name().unwrap().as_deref(),
            Some(name.trim())
        );
        assert_eq!(
            part(&entity).content_type().unwrap().unwrap().name(),
            Some(name)
        );
        assert_eq!(
            part(&entity)
                .content_disposition()
                .unwrap()
                .unwrap()
                .file_name(),
            Some(name)
        );
        let header = entity.headers().get("Content-Location").unwrap();
        assert_eq!(header.decode_value().text, location);
        assert!(!header.decode_value().text.contains("=?"));
        let mut wire = Vec::new();
        entity.write_to(&mut wire).unwrap();
        let loaded = MimeEntity::load(&mut wire.as_slice()).unwrap();
        assert_eq!(
            part(&loaded).file_name().unwrap().as_deref(),
            Some(name.trim())
        );
        assert_eq!(
            part(&loaded).content_type().unwrap().unwrap().name(),
            Some(name)
        );
        assert_eq!(
            part(&loaded)
                .content_disposition()
                .unwrap()
                .unwrap()
                .file_name(),
            Some(name)
        );
        assert_eq!(
            loaded
                .headers()
                .get("Content-Location")
                .unwrap()
                .decode_value()
                .text,
            location
        );
    }
    let unknown = attachment::from_bytes("file.unknown-mimecraft-extension", b"").unwrap();
    assert_metadata(
        &unknown,
        "application/octet-stream",
        "file.unknown-mimecraft-extension",
        false,
    );
}

#[test]
fn linked_locations_fold_only_before_the_uri_and_enforce_the_physical_limit() {
    let options = AttachmentOptions {
        linked_resource: true,
        ..Default::default()
    };
    for length in [60, 61, 100, 997] {
        let name = "a".repeat(length);
        let mut entity = attachment::from_bytes_with_options(&name, b"", &options).unwrap();
        let mut wire = Vec::new();
        entity.write_to(&mut wire).unwrap();
        let wire_text = String::from_utf8(wire).unwrap();
        let location = wire_text.split("Content-Location:").nth(1).unwrap();
        let expected = if length <= 60 {
            format!(" {name}\r\n")
        } else {
            format!("\r\n\t{name}\r\n")
        };
        assert!(location.starts_with(&expected), "{length}: {location}");
        assert!(wire_text.split("\r\n").all(|line| line.len() <= 998));
        assert_eq!(
            entity
                .headers()
                .get("Content-Location")
                .unwrap()
                .decode_value()
                .text,
            name
        );
    }
    for name in ["a".repeat(998), "\u{e9}".repeat(167)] {
        let error =
            attachment::from_reader_with_options(&name, &mut NeverRead, &options).unwrap_err();
        assert!(matches!(
            error,
            AttachmentError::HeaderFormat(HeaderFormatError::LineTooLong { limit: 998, .. })
        ));
        assert!(error.source().is_some());
    }
}

struct NeverRead;

impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("invalid options or metadata must fail before reading")
    }
}

#[test]
fn invalid_names_limits_and_metadata_fail_before_io() {
    for name in [
        "",
        " ",
        ".",
        "..",
        "/",
        "a/",
        "\\",
        "a\\",
        "a/..",
        "a/\t",
        "bad\r\nInjected: x",
        "nul\0.txt",
        "del\u{7f}.txt",
    ] {
        assert!(matches!(
            attachment::from_bytes(name, b""),
            Err(AttachmentError::InvalidFileName)
        ));
        assert!(matches!(
            attachment::from_reader(name, &mut NeverRead),
            Err(AttachmentError::InvalidFileName)
        ));
    }
    for parse_options in [
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
    ] {
        let options = AttachmentOptions {
            parse_options,
            ..Default::default()
        };
        for error in [
            attachment::from_bytes_with_options("opaque.bin", b"", &options).unwrap_err(),
            attachment::from_reader_with_options("opaque.bin", &mut NeverRead, &options)
                .unwrap_err(),
            attachment::from_file_with_options(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("target")
                    .join("missing-attachment-options.bin"),
                &options,
            )
            .unwrap_err(),
        ] {
            assert!(matches!(error, AttachmentError::Options(_)));
            assert!(error.source().is_some());
        }
    }
    let options = AttachmentOptions {
        content_type: Some(ContentType::new(&"x".repeat(100), "plain").unwrap()),
        ..Default::default()
    };
    assert!(matches!(
        attachment::from_reader_with_options("opaque.bin", &mut NeverRead, &options),
        Err(AttachmentError::HeaderFormat(_))
    ));
    let error = AttachmentError::InvalidFileName;
    assert!(error.source().is_none());
    assert!(!error.to_string().is_empty());
}

#[test]
fn byte_and_reader_limits_accept_exact_eof_and_consume_only_one_excess_byte() {
    for limit in [1, 3, 4096, 4097] {
        let options = AttachmentOptions {
            parse_options: MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            },
            ..Default::default()
        };
        for length in [limit - 1, limit, limit + 1, limit + 10] {
            let data = vec![b'a'; length];
            let mut reader = Cursor::new(&data);
            let byte_result = attachment::from_bytes_with_options("data.bin", &data, &options);
            let reader_result =
                attachment::from_reader_with_options("data.bin", &mut reader, &options);
            if length <= limit {
                assert_raw(&byte_result.unwrap(), &data, ContentEncoding::Base64);
                assert_raw(&reader_result.unwrap(), &data, ContentEncoding::Base64);
                assert_eq!(reader.position(), length as u64);
            } else {
                for error in [byte_result.unwrap_err(), reader_result.unwrap_err()] {
                    assert!(
                        matches!(error, AttachmentError::InputTooLarge { limit: actual } if actual == limit)
                    );
                    assert!(error.source().is_none());
                    assert!(error.to_string().contains(&limit.to_string()));
                }
                assert_eq!(reader.position(), (limit + 1) as u64);
            }
        }
    }
    let options = AttachmentOptions {
        parse_options: MimeEntityParseOptions {
            max_input_bytes: usize::MAX,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_raw(
        &attachment::from_reader_with_options("data.bin", &mut &b"abc"[..], &options).unwrap(),
        b"abc",
        ContentEncoding::Base64,
    );
    with_file(b"abc", |path| {
        for limit in [2, 3] {
            let options = AttachmentOptions {
                content_type: Some(ContentType::new("application", "octet-stream").unwrap()),
                parse_options: MimeEntityParseOptions {
                    max_input_bytes: limit,
                    ..Default::default()
                },
                ..Default::default()
            };
            let result = attachment::from_file_with_options(path, &options);
            assert_file_released(path);
            if limit == 2 {
                assert!(matches!(
                    result,
                    Err(AttachmentError::InputTooLarge { limit: 2 })
                ));
            } else {
                assert_raw(&result.unwrap(), b"abc", ContentEncoding::Base64);
            }
        }
    });
}

struct ShortReader<'a> {
    remaining: &'a [u8],
    interrupt: bool,
    fail_at_end: bool,
}

impl Read for ShortReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.interrupt) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.remaining.is_empty() && self.fail_at_end {
            return Err(io::Error::other("attachment read failure"));
        }
        let count = output.len().min(self.remaining.len()).min(3);
        output[..count].copy_from_slice(&self.remaining[..count]);
        self.remaining = &self.remaining[count..];
        Ok(count)
    }
}

#[test]
fn nonseekable_short_reads_retry_interruptions_and_propagate_failures() {
    let mut reader = ShortReader {
        remaining: b"hello",
        interrupt: true,
        fail_at_end: false,
    };
    assert_raw(
        &attachment::from_reader("data.bin", &mut reader).unwrap(),
        b"hello",
        ContentEncoding::Base64,
    );
    assert!(reader.remaining.is_empty());
    for limit in [5, 10] {
        let mut reader = ShortReader {
            remaining: b"hello",
            interrupt: true,
            fail_at_end: true,
        };
        let options = AttachmentOptions {
            parse_options: MimeEntityParseOptions {
                max_input_bytes: limit,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut builder = BodyBuilder::new();
        let result = attachment::from_reader_with_options("data.bin", &mut reader, &options);
        if let Ok(entity) = result {
            builder.attachments.push(entity);
            panic!("failed attachment must not be returned")
        }
        let error = result.unwrap_err();
        assert!(matches!(error, AttachmentError::Io(_)));
        assert_eq!(
            error.source().unwrap().to_string(),
            "attachment read failure"
        );
        assert!(builder.attachments.is_empty());
        assert!(reader.remaining.is_empty());
    }
    let mut reader = Cursor::new(b"skip:payload".to_vec());
    reader.set_position(5);
    let entity = attachment::from_reader("data.bin", &mut reader).unwrap();
    assert_eq!(reader.position(), 12);
    reader.get_mut().fill(b'x');
    assert_raw(&entity, b"payload", ContentEncoding::Base64);
}

#[test]
fn message_parser_limits_are_propagated_without_binary_fallback() {
    let nested = b"Subject: long subject\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b--\r\n";
    for parse_options in [
        MimeEntityParseOptions {
            max_header_length: 5,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_depth: 1,
            ..Default::default()
        },
    ] {
        let options = AttachmentOptions {
            parse_options,
            ..Default::default()
        };
        let error =
            attachment::from_bytes_with_options("nested.eml", nested, &options).unwrap_err();
        assert!(matches!(error, AttachmentError::Message(_)));
        assert!(error.source().is_some());
    }
    let options = AttachmentOptions {
        parse_options: MimeEntityParseOptions {
            allow_header_only: false,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        attachment::from_bytes_with_options("headers.eml", b"Subject: only\r\n", &options),
        Err(AttachmentError::Message(_))
    ));
    assert!(attachment::from_bytes("headers.eml", b"Subject: only\r\n").is_ok());
}

#[test]
fn helpers_integrate_with_builder_related_lookup_and_wire_round_trips() {
    let options = AttachmentOptions {
        linked_resource: true,
        ..Default::default()
    };
    let name = "logo:#%.png";
    let location = "logo%3A%23%25.png";
    let mut builder = BodyBuilder {
        html_body: Some(format!("<img src=\"{location}\">")),
        ..Default::default()
    };
    builder
        .linked_resources
        .push(attachment::from_bytes_with_options(name, IMAGE, &options).unwrap());
    builder
        .attachments
        .push(attachment::from_bytes("report.bin", b"binary\0\xff").unwrap());
    builder
        .attachments
        .push(attachment::from_bytes("forwarded.eml", MESSAGE).unwrap());
    let mut body = builder.into_message_body().unwrap();
    let mut wire = Vec::new();
    body.write_to(&mut wire).unwrap();
    let mut loaded = MimeEntity::load(&mut wire.as_slice()).unwrap();
    for entity in [&mut body, &mut loaded] {
        let MimeEntity::Multipart(mixed) = entity else {
            panic!("expected mixed")
        };
        assert_eq!(mixed.children().len(), 3);
        let MimeEntity::Multipart(resources) = mixed.children_mut().remove(0) else {
            panic!("expected related")
        };
        let mut related = MultipartRelated::from_multipart(resources).unwrap();
        assert_eq!(related.index_of_uri(location).unwrap(), Some(1));
        let mut decoded = Vec::new();
        related
            .open_by_uri(location)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, IMAGE);
        let MimeEntity::Part(binary) = &mut mixed.children_mut()[0] else {
            panic!("expected binary attachment")
        };
        decoded.clear();
        binary.content_mut().decode_to(&mut decoded).unwrap();
        assert_eq!(decoded, b"binary\0\xff");
        assert_message(&mut mixed.children_mut()[1], "forwarded.eml", false);
    }
}

#[cfg(unix)]
#[test]
fn file_paths_reject_non_unicode_names_without_lossy_conversion() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let path = Path::new(OsStr::from_bytes(b"invalid-\xff.bin"));
    assert!(matches!(
        attachment::from_file(path),
        Err(AttachmentError::InvalidFileName)
    ));
}

#[cfg(windows)]
#[test]
fn file_paths_reject_non_unicode_names_without_lossy_conversion() {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let path = OsString::from_wide(&[0xd800, b'.' as u16, b'b' as u16]);
    assert!(matches!(
        attachment::from_file(Path::new(&path)),
        Err(AttachmentError::InvalidFileName)
    ));
}

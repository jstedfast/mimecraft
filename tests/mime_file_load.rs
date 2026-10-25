// Exercises the file-loading overloads adapted from MimeKit's MimeEntity.cs and MimeMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

use mimecraft::{HeaderTooLong, MimeEntity, MimeEntityParseOptions, MimeMessage, TextFormat};

fn with_file(content: &[u8], test: impl FnOnce(&Path)) {
    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&directory).unwrap();
    let path = loop {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "mimecraft-file-load-{}-{id}.eml",
            std::process::id()
        ));
        match File::create_new(&path) {
            Ok(mut file) => {
                file.write_all(content).unwrap();
                break path;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("could not create test file: {error}"),
        }
    };
    test(&path);
    fs::remove_file(path).unwrap();
}

fn assert_file_released(path: &Path) {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    // Windows can verify that even a read-only handle was released.
    #[cfg(windows)]
    options.share_mode(0);
    drop(options.open(path).unwrap());
}

#[test]
fn file_loaders_copy_nested_bodies_and_release_the_file_before_returning() {
    let wire = b"Subject: file\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\
        --b\r\nContent-Type: message/rfc822\r\n\r\nSubject: attached\r\n\
        Content-Type: text/plain\r\n\r\ninside\r\n--b--\r\n";
    with_file(wire, |path| {
        let options = MimeEntityParseOptions::default();
        let entities = [
            MimeEntity::load_file(path).unwrap(),
            MimeEntity::load_file_with_options(path, &options).unwrap(),
        ];
        let messages = [
            MimeMessage::load_file(path).unwrap(),
            MimeMessage::load_file_with_options(path, &options).unwrap(),
        ];
        assert_file_released(path);
        fs::write(path, b"changed after loading").unwrap();
        for mut entity in entities {
            assert_eq!(entity.headers().get("Subject").unwrap().offset(), Some(0));
            assert_eq!(entity.leaf_parts().count(), 2);
            let mut output = Vec::new();
            entity.write_to(&mut output).unwrap();
            assert_eq!(output, wire);
        }
        for mut message in messages {
            assert!(message.headers().contains("Subject"));
            assert!(!message.headers().contains("Content-Type"));
            assert!(!message.body().unwrap().headers().contains("Subject"));
            assert_eq!(message.leaf_parts().count(), 2);
            let mut output = Vec::new();
            message.write_to(&mut output).unwrap();
            assert_eq!(output, wire);
            assert_eq!(
                message.text_body(TextFormat::Plain).unwrap().as_deref(),
                Some("hello")
            );
        }
    });
}

#[test]
fn file_loaders_match_stream_loading_for_upstream_body_fixtures() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("messages");
    for index in 1..=9 {
        let path = directory.join(format!("body.{index}.txt"));
        let wire = fs::read(&path).unwrap();
        let mut entity = MimeEntity::load_file(&path).unwrap();
        let mut expected = MimeEntity::load(&mut wire.as_slice()).unwrap();
        let mut actual_bytes = Vec::new();
        let mut expected_bytes = Vec::new();
        entity.write_to(&mut actual_bytes).unwrap();
        expected.write_to(&mut expected_bytes).unwrap();
        assert_eq!(actual_bytes, expected_bytes, "entity body.{index}");

        let mut message = MimeMessage::load_file(&path).unwrap();
        let mut expected = MimeMessage::load(&mut wire.as_slice()).unwrap();
        actual_bytes.clear();
        expected_bytes.clear();
        message.write_to(&mut actual_bytes).unwrap();
        expected.write_to(&mut expected_bytes).unwrap();
        assert_eq!(actual_bytes, expected_bytes, "message body.{index}");
    }
}

#[test]
fn file_loaders_apply_nested_message_options() {
    let wire = b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nbody";
    with_file(wire, |path| {
        let options = MimeEntityParseOptions {
            parse_message_parts: false,
            max_depth: 1,
            ..Default::default()
        };
        let entity = MimeEntity::load_file_with_options(path, &options).unwrap();
        assert!(matches!(entity, MimeEntity::Part(_)));
        let message = MimeMessage::load_file_with_options(path, &options).unwrap();
        assert!(matches!(message.body(), Some(MimeEntity::Part(_))));

        let options = MimeEntityParseOptions {
            parse_message_parts: true,
            ..options
        };
        for error in [
            MimeEntity::load_file_with_options(path, &options).unwrap_err(),
            MimeMessage::load_file_with_options(path, &options).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        assert_file_released(path);
    });
}

#[test]
fn file_loaders_enforce_exact_limits_and_preserve_typed_errors() {
    let wire = b"X: a\r\n b\r\n\r\nbody";
    with_file(wire, |path| {
        let options = MimeEntityParseOptions {
            max_input_bytes: wire.len(),
            max_header_length: b"X: a\r\n b\r\n".len(),
            ..Default::default()
        };
        MimeEntity::load_file_with_options(path, &options).unwrap();
        MimeMessage::load_file_with_options(path, &options).unwrap();
        let too_short = MimeEntityParseOptions {
            max_input_bytes: wire.len() - 1,
            ..options
        };
        for error in [
            MimeEntity::load_file_with_options(path, &too_short).unwrap_err(),
            MimeMessage::load_file_with_options(path, &too_short).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        let too_short = MimeEntityParseOptions {
            max_header_length: options.max_header_length - 1,
            ..options
        };
        for error in [
            MimeEntity::load_file_with_options(path, &too_short).unwrap_err(),
            MimeMessage::load_file_with_options(path, &too_short).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<HeaderTooLong>()
                    .is_some()
            );
        }
        assert_file_released(path);
    });
}

#[test]
fn file_loaders_release_files_after_parse_errors_and_recover_header_only_input() {
    for wire in [
        b"".as_slice(),
        b"not a MIME header\r\n",
        b"Subject: header only",
    ] {
        with_file(wire, |path| {
            let options = MimeEntityParseOptions {
                allow_header_only: false,
                ..Default::default()
            };
            let mut input = wire;
            let expected = MimeEntity::load_with_options(&mut input, &options).unwrap_err();
            for error in [
                MimeEntity::load_file_with_options(path, &options).unwrap_err(),
                MimeMessage::load_file_with_options(path, &options).unwrap_err(),
            ] {
                assert_eq!(error.kind(), expected.kind());
                assert_eq!(error.to_string(), expected.to_string());
            }
            assert_file_released(path);
            if wire.starts_with(b"Subject:") {
                MimeEntity::load_file(path).unwrap();
                MimeMessage::load_file(path).unwrap();
            } else {
                assert!(MimeEntity::load_file(path).is_err());
                assert!(MimeMessage::load_file(path).is_err());
            }
            assert_file_released(path);
        });
    }
}

#[test]
fn file_loaders_validate_options_before_resolving_the_path() {
    struct UnusedPath;
    impl AsRef<Path> for UnusedPath {
        fn as_ref(&self) -> &Path {
            panic!("invalid options must fail before resolving the path");
        }
    }
    for options in [
        MimeEntityParseOptions {
            max_depth: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_input_bytes: 0,
            ..Default::default()
        },
        MimeEntityParseOptions {
            max_header_length: 0,
            ..Default::default()
        },
    ] {
        for error in [
            MimeEntity::load_file_with_options(UnusedPath, &options).unwrap_err(),
            MimeMessage::load_file_with_options(UnusedPath, &options).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }
}

#[test]
fn file_loaders_preserve_platform_open_errors() {
    with_file(b"", |path| {
        let missing = path.join("missing.eml");
        for path in [missing.as_path(), Path::new(""), Path::new("\0")] {
            let expected = File::open(path).unwrap_err();
            let options = MimeEntityParseOptions::default();
            for error in [
                MimeEntity::load_file(path).unwrap_err(),
                MimeMessage::load_file(path).unwrap_err(),
                MimeEntity::load_file_with_options(path, &options).unwrap_err(),
                MimeMessage::load_file_with_options(path, &options).unwrap_err(),
            ] {
                assert_eq!(error.kind(), expected.kind());
                assert_eq!(error.raw_os_error(), expected.raw_os_error());
            }
        }
    });
}

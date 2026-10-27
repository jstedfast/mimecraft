// Adapted from MimeKit's Benchmarks/MimeParserBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    fmt::Write as _,
    fs,
    hint::black_box,
    io::{BufRead, BufReader, Cursor, Read, Seek},
    path::{Path, PathBuf},
};

use mimecraft::{MimeContentStorage, MimeEntityParseOptions, MimeParser, MimeReader};

// MimeKit buffers its MemoryStream input internally. Rust takes BufRead
// sources, so include a fresh 4 KiB input buffer in the buffered variants.
const BUFFER_SIZE: usize = 4096;

pub const CASES: [&str; 5] = [
    "StarTrekMessage",
    "ContentLengthMbox",
    "JwzMbox",
    "HeaderStressTest",
    "MixedNested64",
];
pub const NESTED: &str = "NestedRfc822x1000";

pub struct Workload {
    pub data: Vec<u8>,
    pub messages: usize,
    pub mbox: bool,
    pub options: MimeEntityParseOptions,
}

impl Workload {
    pub fn check(&self, result: (usize, u64, bool)) {
        assert_eq!(result, (self.messages, self.data.len() as u64, true));
    }
}

pub fn workload(name: &str) -> Workload {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let messages = root.join("benches").join("data").join("messages");
    let mbox = root.join("tests").join("data").join("mbox");
    let mut options = MimeEntityParseOptions::default();
    let (data, mbox, messages) = match name {
        "StarTrekMessage" => (read_fixture(messages.join("startrek.eml")), false, 1),
        // Like MimeKit's benchmark, use default mbox framing, not the
        // optional RespectContentLength behavior used by its unit test.
        "ContentLengthMbox" => (read_fixture(mbox.join("content-length.mbox.txt")), true, 29),
        "JwzMbox" => (read_fixture(mbox.join("jwz.mbox.txt")), true, 152),
        "HeaderStressTest" => (read_fixture(messages.join("header-stress.eml")), false, 1),
        "MixedNested64" => {
            // The innermost leaf adds one entity to the 64 containers.
            options.max_depth = 65;
            (generate_mixed_nested_message(64), false, 1)
        }
        NESTED => {
            options.max_depth = 1001;
            (generate_nested_rfc822_message(1000), false, 1)
        }
        _ => panic!("unknown MIME benchmark workload: {name}"),
    };
    Workload {
        data,
        messages,
        mbox,
        options,
    }
}

pub fn read(case: &Workload) -> (usize, u64, bool) {
    let source = BufReader::with_capacity(BUFFER_SIZE, black_box(case.data.as_slice()));
    read_messages(
        MimeReader::with_options(source, case.options).unwrap(),
        case.mbox,
    )
}

pub fn read_slice(case: &Workload) -> (usize, u64, bool) {
    read_messages(
        MimeReader::with_options(black_box(case.data.as_slice()), case.options).unwrap(),
        case.mbox,
    )
}

fn read_messages<R: BufRead>(
    mut reader: MimeReader<R>,
    mbox: bool,
) -> (usize, u64, bool) {
    let messages = if mbox {
        let mut messages = 0;
        while reader.read_mbox_message(|_| Ok(())).unwrap() {
            messages += 1;
        }
        messages
    } else {
        reader.read_message(|_| Ok(())).unwrap();
        1
    };
    (
        messages,
        reader.position().offset,
        reader.is_end_of_stream(),
    )
}

pub fn parse_default(case: &Workload) -> (usize, u64, bool) {
    let source = BufReader::with_capacity(BUFFER_SIZE, black_box(case.data.as_slice()));
    parse(
        MimeParser::with_options(source, case.options).unwrap(),
        case.mbox,
    )
}

pub fn parse_source(case: &Workload) -> (usize, u64, bool) {
    // The source-storage constructor includes a shared source and 4 KiB buffer.
    // Cursor borrows the fixture without copying.
    let source = Cursor::new(black_box(case.data.as_slice()));
    let parser =
        MimeParser::with_storage_and_options(source, MimeContentStorage::Source, case.options)
            .unwrap();
    assert_eq!(parser.get_ref().capacity(), BUFFER_SIZE);
    parse(parser, case.mbox)
}

pub fn parse_slice(case: &Workload) -> (usize, u64, bool) {
    let parser =
        MimeParser::from_slice_with_options(black_box(case.data.as_slice()), case.options)
            .unwrap();
    parse(parser, case.mbox)
}

fn parse<R: BufRead, C: Read + Seek>(
    mut parser: MimeParser<R, C>,
    mbox: bool,
) -> (usize, u64, bool) {
    let messages = if mbox {
        let mut messages = 0;
        while let Some(message) = parser.parse_mbox_message().unwrap() {
            black_box(message);
            messages += 1;
        }
        messages
    } else {
        black_box(parser.parse_message().unwrap());
        1
    };
    (
        messages,
        parser.position().offset,
        parser.is_end_of_stream(),
    )
}

fn generate_mixed_nested_message(depth: usize) -> Vec<u8> {
    // A message that alternates between multipart/mixed and message/rfc822
    // parts (each multipart also has a text/plain sibling before and after
    // the nested part).
    let mut builder =
        String::from("From: a@example.org\r\nSubject: nested\r\nMIME-Version: 1.0\r\n");
    for i in 0..depth {
        if i % 2 == 0 {
            write!(
                builder,
                "Content-Type: multipart/mixed; boundary=\"b{i}\"\r\n\r\n"
            )
            .unwrap();
            write!(
                builder,
                "--b{i}\r\nContent-Type: text/plain\r\n\r\nbefore {i}\r\n"
            )
            .unwrap();
            write!(builder, "--b{i}\r\n").unwrap();
        } else {
            builder.push_str("Content-Type: message/rfc822\r\n\r\n");
            write!(builder, "From: a@example.org\r\nSubject: level {i}\r\n").unwrap();
        }
    }
    builder.push_str("Content-Type: text/plain\r\n\r\ninnermost\r\n");
    for i in (0..depth).rev() {
        if i % 2 == 0 {
            write!(
                builder,
                "--b{i}\r\nContent-Type: text/plain\r\n\r\nafter {i}\r\n"
            )
            .unwrap();
            write!(builder, "--b{i}--\r\n").unwrap();
        }
    }
    builder.into_bytes()
}

fn generate_nested_rfc822_message(depth: usize) -> Vec<u8> {
    let mut builder = String::from("From: a@example.org\r\n");
    for _ in 0..depth {
        builder.push_str("Content-Type: message/rfc822\r\n\r\nFrom: a@example.org\r\n");
    }
    builder.push_str("Content-Type: text/plain\r\n\r\ninnermost\r\n");
    builder.into_bytes()
}

fn read_fixture(path: PathBuf) -> Vec<u8> {
    fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "could not read benchmark fixture {}: {error}",
            path.display()
        )
    })
}

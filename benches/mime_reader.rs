// Adapted from MimeKit's Benchmarks/MimeParserBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    fmt::Write as _,
    fs,
    hint::black_box,
    io::BufReader,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use mimecraft::{MimeEntityParseOptions, MimeParser, MimeReader};

// MimeKit buffers its MemoryStream input internally. Rust takes BufRead
// sources, so include a fresh 4 KiB input buffer in each operation instead.
const BUFFER_SIZE: usize = 4096;

fn measure(
    name: &str,
    bytes: usize,
    messages: usize,
    mut operation: impl FnMut() -> (usize, u64, bool),
) {
    // Check message count, complete input consumption, and EOF before timing.
    assert_eq!(operation(), (messages, bytes as u64, true), "{name}");
    for _ in 0..3 {
        black_box(operation());
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            black_box(operation());
            iterations += 1;
        }
        *sample = iterations as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:36} {:9.1} MiB/s {:11.0} messages/s (median of 5)",
        samples[2] * bytes as f64 / 1048576.0,
        samples[2] * messages as f64,
    );
}

fn read(data: &[u8], options: MimeEntityParseOptions, mbox: bool) -> (usize, u64, bool) {
    let source = BufReader::with_capacity(BUFFER_SIZE, black_box(data));
    let mut reader = MimeReader::with_options(source, options).unwrap();
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

fn parse(data: &[u8], options: MimeEntityParseOptions, mbox: bool) -> (usize, u64, bool) {
    let source = BufReader::with_capacity(BUFFER_SIZE, black_box(data));
    let mut parser = MimeParser::with_options(source, options).unwrap();
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

fn main() {
    println!(
        "target: {}-{}, SIMD feature: {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(feature = "simd"),
    );
    println!("MimeKit workloads; input-byte throughput; fixtures loaded/generated outside timing.");
    println!(
        "Fresh 4 KiB buffered sources; reader callbacks are no-ops; parsing includes model construction/drop."
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let messages = root.join("benches").join("data").join("messages");
    let mbox = root.join("tests").join("data").join("mbox");
    let defaults = MimeEntityParseOptions::default();
    let cases = [
        (
            "StarTrekMessage",
            read_fixture(messages.join("startrek.eml")),
            false,
            1,
            defaults,
        ),
        (
            // Like MimeKit's benchmark, use default mbox framing, not the
            // optional RespectContentLength behavior used by its unit test.
            "ContentLengthMbox",
            read_fixture(mbox.join("content-length.mbox.txt")),
            true,
            29,
            defaults,
        ),
        (
            "JwzMbox",
            read_fixture(mbox.join("jwz.mbox.txt")),
            true,
            152,
            defaults,
        ),
        (
            "HeaderStressTest",
            read_fixture(messages.join("header-stress.eml")),
            false,
            1,
            defaults,
        ),
        (
            "MixedNested64",
            generate_mixed_nested_message(64),
            false,
            1,
            // The innermost leaf adds one entity to the 64 containers.
            MimeEntityParseOptions {
                max_depth: 65,
                ..defaults
            },
        ),
    ];
    let nested = generate_nested_rfc822_message(1000);
    for (name, data, mbox, messages, options) in cases {
        measure(&format!("MimeReader_{name}"), data.len(), messages, || {
            read(&data, options, mbox)
        });
        measure(&format!("MimeParser_{name}"), data.len(), messages, || {
            parse(&data, options, mbox)
        });
    }
    // Upstream benchmarks this depth with the reader only, not owned parsing.
    measure("MimeReader_NestedRfc822x1000", nested.len(), 1, || {
        read(
            &nested,
            MimeEntityParseOptions {
                max_depth: 1001,
                ..defaults
            },
            false,
        )
    });
}

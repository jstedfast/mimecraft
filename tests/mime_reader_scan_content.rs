// Adapted from MimeKit's MimeReaderScanContentTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    MimeComplianceIssue, MimeContentKind, MimeEvent, MimeEventFormat, MimeEventParser,
    MimePosition, MimeReader,
};
use std::{
    fs,
    io::{self, BufReader},
    path::Path,
};

#[derive(Debug, PartialEq)]
enum Recorded {
    Event(String),
    Content(MimeContentKind, MimePosition, Vec<u8>),
    Error(io::ErrorKind),
}

// Record callback arguments, including raw bytes and positions, rather than
// comparing only the assembled bodies. Async duplicates are consolidated.
fn record(data: &[u8], mbox: bool, chunk_size: usize, logged: bool) -> Vec<Recorded> {
    record_with_driver(data, mbox, chunk_size, logged, false)
}

fn record_with_driver(
    data: &[u8],
    mbox: bool,
    chunk_size: usize,
    logged: bool,
    byte_fed: bool,
) -> Vec<Recorded> {
    let mut reader = MimeReader::new(BufReader::with_capacity(chunk_size, data));
    let mut events = Vec::new();
    let mut callback = |event: MimeEvent<'_>| {
        events.push(match event {
            MimeEvent::Content {
                kind,
                bytes,
                position,
            } => {
                let start = position.offset as usize;
                assert_eq!(&data[start..start + bytes.len()], bytes);
                Recorded::Content(kind, position, bytes.to_vec())
            }
            MimeEvent::EntityBegin {
                headers,
                kind,
                encoding,
                position,
                headers_end,
            } => Recorded::Event(format!(
                "EntityBegin {:?} {kind:?} {encoding:?} {position:?} {headers_end:?}",
                headers.as_slice()
            )),
            event => Recorded::Event(format!("{event:?}")),
        });
        Ok(())
    };
    let mut logger = |_: MimeComplianceIssue| Ok(());
    let result = if byte_fed {
        let format = if mbox {
            MimeEventFormat::Mbox
        } else {
            MimeEventFormat::Message
        };
        let mut parser: MimeEventParser<'_> = if logged {
            let logger: &mut (dyn mimecraft::MimeComplianceLogger + Send) = &mut logger;
            MimeEventParser::with_logger(format, Default::default(), Default::default(), logger)
                .unwrap()
        } else {
            MimeEventParser::new(format)
        };
        data.chunks(chunk_size)
            .try_for_each(|chunk| parser.parse(chunk, &mut callback).map(|_| ()))
            .and_then(|_| parser.finish(&mut callback))
    } else if mbox {
        loop {
            let result = if logged {
                reader.read_mbox_message_with_logger(&mut logger, &mut callback)
            } else {
                reader.read_mbox_message(&mut callback)
            };
            match result {
                Ok(true) => {}
                Ok(false) => break Ok(()),
                Err(error) => break Err(error),
            }
        }
    } else if logged {
        reader.read_message_with_logger(&mut logger, &mut callback)
    } else {
        reader.read_message(&mut callback)
    };
    if let Err(error) = result {
        events.push(Recorded::Error(error.kind()));
    }
    events
}

fn assert_events_equal(expected: Vec<Recorded>, actual: Vec<Recorded>, label: &str) {
    for (index, (expected, actual)) in expected.iter().zip(&actual).enumerate() {
        if expected != actual {
            let expected: String = format!("{expected:?}").chars().take(400).collect();
            let actual: String = format!("{actual:?}").chars().take(400).collect();
            panic!("{label}: event #{index} differs\nexpected: {expected}\nactual: {actual}");
        }
    }
    assert_eq!(actual.len(), expected.len(), "{label}: event count");
}

#[test]
fn corpus_fast_path_matches_logged_path() {
    for (directory, mbox, count) in [("messages", false, 36), ("mbox", true, 166)] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("data")
            .join(directory);
        let mut files: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .collect();
        files.sort();
        assert_eq!(files.len(), count, "complete upstream {directory} corpus");
        // Upstream intentionally scans every file, including expected-output
        // fixtures that are not MIME input, and compares errors as well.
        for path in files {
            let data = fs::read(&path).unwrap();
            for chunk_size in [1021, 4096] {
                assert_events_equal(
                    record(&data, mbox, chunk_size, false),
                    record(&data, mbox, chunk_size, true),
                    &format!("{} / {chunk_size}", path.display()),
                );
                for logged in [false, true] {
                    assert_events_equal(
                        record(&data, mbox, chunk_size, logged),
                        record_with_driver(&data, mbox, chunk_size, logged, true),
                        &format!(
                            "byte-fed {} / {chunk_size} / logged={logged}",
                            path.display()
                        ),
                    );
                }
            }
        }
    }
}

struct Random(u64);

impl Random {
    // Keep the generator deterministic without a test-only dependency. Its
    // sequence differs from System.Random; the upstream distributions remain.
    fn next(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize % limit
    }

    fn newline(&mut self) -> &'static str {
        if self.next(8) == 0 { "\n" } else { "\r\n" }
    }

    fn body(&mut self, output: &mut String) {
        let lines = [
            "",
            "a",
            "-",
            "--",
            "---",
            "-x",
            "--x",
            "--b",
            "--b--",
            "--b ",
            "--b--  ",
            "--bb",
            "--inner",
            "--inner--",
            "From ",
            "From someone@example.com",
            ">From escaped",
            "Fr",
            "F",
            "From",
            "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5emFiY2RlZmdoaWprbG1u",
            "short line of text",
            &"x".repeat(31),
            &"y".repeat(32),
            &"z".repeat(63),
            &"w".repeat(997),
            &"v".repeat(1100),
            "trailing\r",
            "\r",
        ];
        for _ in 0..self.next(40) {
            output.push_str(if self.next(3) == 0 {
                lines[self.next(lines.len())]
            } else {
                &lines[20][..self.next(lines[20].len())]
            });
            output.push_str(self.newline());
        }
    }

    fn message(&mut self, depth: usize) -> String {
        let nl = self.newline();
        let mut output = format!("From: mimekit@example.org{nl}");
        if depth < 2 && self.next(3) != 0 {
            let boundary = if depth == 0 { "b" } else { "inner" };
            output.push_str(&format!(
                "Content-Type: multipart/mixed; boundary=\"{boundary}\"{nl}{nl}"
            ));
            self.body(&mut output);
            for _ in 0..self.next(4) {
                let padding = if self.next(4) == 0 { "  " } else { "" };
                output.push_str(&format!("--{boundary}{padding}{}", self.newline()));
                if self.next(4) == 0 {
                    output.push_str(&format!("Content-Type: message/rfc822{nl}{nl}"));
                    output.push_str(&self.message(depth + 1));
                } else if self.next(3) == 0 {
                    let child = self.message(depth + 1);
                    // Drop the child's From header using its own newline.
                    output.push_str(&child[child.find('\n').unwrap() + 1..]);
                } else {
                    output.push_str(&format!("Content-Type: text/plain{nl}{nl}"));
                    self.body(&mut output);
                }
            }
            if self.next(4) != 0 {
                output.push_str(&format!("--{boundary}--{}", self.newline()));
                self.body(&mut output);
            }
        } else {
            output.push_str(&format!("Content-Type: text/plain{nl}{nl}"));
            self.body(&mut output);
        }
        output
    }

    fn mbox(&mut self) -> String {
        let mut output = String::new();
        for _ in 0..1 + self.next(3) {
            output.push_str("From mimekit@example.org Fri Jan  1 00:00:00 2026");
            output.push_str(self.newline());
            output.push_str(&self.message(0));
        }
        output
    }
}

#[test]
fn generated_messages_fast_path_matches_logged_path() {
    for mbox in [false, true] {
        let mut random = Random(if mbox { 4242 } else { 2424 });
        for iteration in 0..300 {
            let mut data = if mbox {
                random.mbox()
            } else {
                random.message(0)
            }
            .into_bytes();
            // Randomly truncate messages to exercise end-of-stream paths.
            if random.next(4) == 0 {
                data.truncate(random.next(data.len() + 1));
            }
            let chunk_size = [1, 2, 3, 7, 31, 33, 64, 4096][iteration % 8];
            assert_events_equal(
                record(&data, mbox, chunk_size, false),
                record(&data, mbox, chunk_size, true),
                &format!("message #{iteration}, mbox={mbox}, chunk_size={chunk_size}"),
            );
            for logged in [false, true] {
                assert_events_equal(
                    record(&data, mbox, chunk_size, logged),
                    record_with_driver(&data, mbox, chunk_size, logged, true),
                    &format!(
                        "byte-fed #{iteration}, mbox={mbox}, chunk_size={chunk_size}, logged={logged}"
                    ),
                );
            }
        }
    }
}

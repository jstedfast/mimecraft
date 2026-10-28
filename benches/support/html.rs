// Adapted from MimeKit's Benchmarks/Text/HtmlBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{fs, hint::black_box, path::Path};

use mimecraft::text::HtmlTokenizerReader;

pub const CASES: [&str; 2] = ["test_html", "Gimhae_Kim_clan_utf_8_html"];

pub struct Workload {
    pub html: String,
    tokens: usize,
}

impl Workload {
    pub fn check(&self, result: (usize, u64, bool)) {
        assert_eq!(result, (self.tokens, self.html.len() as u64, true));
    }
}

pub fn workload(name: &str) -> Workload {
    // Token counts come from MimeKit's corresponding .tokens fixtures.
    let (file_name, tokens, bytes) = match name {
        "test_html" => ("test.html", 7972, 133088),
        "Gimhae_Kim_clan_utf_8_html" => ("Gimhae_Kim_clan.utf-8.html", 1911, 59435),
        _ => panic!("unknown HTML benchmark workload: {name}"),
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("data")
        .join("html")
        .join(file_name);
    let data = fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "could not read benchmark fixture {}: {error}",
            path.display()
        )
    });
    // GlobalSetup uses File.ReadAllText, not the Windows-1252 decoder used by
    // MimeKit's tokenizer tests. Strip the UTF-8 BOM and replace malformed UTF-8
    // before timing: test.html has 32 invalid bytes, while the Korean page has a BOM.
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
    let html = String::from_utf8_lossy(data).into_owned();
    assert_eq!(html.len(), bytes, "decoded fixture changed: {file_name}");
    Workload { html, tokens }
}

pub fn read_all_tokens(case: &Workload) -> (usize, u64, bool) {
    // Match the fresh tokenizer over StringReader: source text is already
    // decoded, but scanner setup, owned token allocation, and drop are measured.
    let mut reader = HtmlTokenizerReader::from_text(black_box(&case.html));
    let mut count = 0;
    while let Some(token) = reader.read_next_token().unwrap() {
        black_box(token);
        count += 1;
    }
    (
        count,
        reader.tokenizer().position().offset,
        reader.tokenizer().is_eof(),
    )
}

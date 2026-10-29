// Adapted from MimeKit's Benchmarks/Text/HtmlBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{fs, hint::black_box, path::Path};

use mimecraft::text::{HtmlToText, HtmlTokenizerReader};

pub const CASES: [&str; 2] = ["test_html", "Gimhae_Kim_clan_utf_8_html"];

pub struct Workload {
    pub html: String,
    tokens: usize,
    text_bytes: usize,
    text_hash: u64,
}

impl Workload {
    pub fn check(&self, result: (usize, u64, bool)) {
        assert_eq!(result, (self.tokens, self.html.len() as u64, true));
    }

    pub fn check_conversion(&self) {
        let expected = convert_to_string(self);
        // CRLF output fingerprints from MimeKit's HtmlToText over the same
        // File.ReadAllText-decoded fixtures. FNV-1a is a regression check only.
        let hash = expected.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        assert_eq!((expected.len(), hash), (self.text_bytes, self.text_hash));
        let mut output = Vec::new();
        HtmlToText::new()
            .convert_to(self.html.as_bytes(), &mut output)
            .unwrap();
        assert_eq!(output, expected.as_bytes());
    }
}

pub fn workload(name: &str) -> Workload {
    // Token counts come from MimeKit's corresponding .tokens fixtures.
    let (file_name, tokens, bytes, text_bytes, text_hash) = match name {
        "test_html" => ("test.html", 7972, 133088, 87455, 0x480aab3530ca9933),
        "Gimhae_Kim_clan_utf_8_html" => (
            "Gimhae_Kim_clan.utf-8.html",
            1911,
            59435,
            10359,
            0x9d26b93fe3966209,
        ),
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
    Workload {
        html,
        tokens,
        text_bytes,
        text_hash,
    }
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

// HtmlToText has no upstream benchmark yet. Mirror HtmlToHtml's fresh converter,
// string output and null-writer workloads without changing the tokenizer case.
pub fn convert_to_string(case: &Workload) -> String {
    HtmlToText::new().convert(black_box(&case.html)).unwrap()
}

pub fn convert_to_writer(case: &Workload) {
    HtmlToText::new()
        .convert_to(black_box(case.html.as_bytes()), &mut std::io::sink())
        .unwrap();
}

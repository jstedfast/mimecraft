// Adapted from MimeKit's Benchmarks/Text/RtfConverterBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/rtf.rs"]
mod workloads;

fn measure<T>(operation: &str, case: &workloads::Workload, mut run: impl FnMut() -> T) {
    for _ in 0..3 {
        black_box(run());
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            black_box(run());
            iterations += 1;
        }
        *sample = iterations as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{operation}/{:23} {:9.1} MiB/s {:9.1} documents/s (median of 5)",
        case.name,
        samples[2] * case.input.len() as f64 / 1048576.0,
        samples[2],
    );
}

fn main() {
    println!(
        "target: {}-{}, SIMD feature: {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(feature = "simd"),
    );
    println!("MimeKit generated RTF workloads; generation and correctness checks outside timing.");
    println!("Fresh synchronous tokenizer/converters with 4 KiB input buffering.");
    println!("Owned tokens and string outputs are dropped; Rust does not use a null-writer API.");
    println!("Throughput counts original RTF bytes (UTF-8 bytes for the extracted HTML baseline).");
    let cases = workloads::CASES
        .into_iter()
        .chain(["ExtractedHtml"])
        .map(workloads::workload)
        .collect::<Vec<_>>();
    for case in &cases {
        case.check();
    }
    for case in &cases[..7] {
        measure("RtfTokenizer_ReadAllTokens", case, || {
            workloads::read_all_tokens(case)
        });
        measure("RtfToText", case, || workloads::to_text(case));
        if workloads::HTML_CASES.contains(&case.name) {
            measure("RtfToHtml_Render", case, || workloads::to_html(case));
        }
        if case.extracted_html.is_some() {
            measure("RtfToHtml_Extract", case, || workloads::extract_html(case));
        }
    }
    let html = &cases[7];
    measure("HtmlToHtml", html, || workloads::rewrite_html(html));
}

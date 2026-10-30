// Adapted from MimeKit's Benchmarks/Text/HtmlBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/html.rs"]
mod workloads;

fn measure<T>(operation: &str, name: &str, case: &workloads::Workload, mut run: impl FnMut() -> T) {
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
        "{operation}/{name:27} {:9.1} MiB/s {:9.1} documents/s (median of 5)",
        samples[2] * case.html.len() as f64 / 1048576.0,
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
    println!("MimeKit HTML workloads; fixtures loaded and decoded outside timing.");
    println!("Fresh synchronous tokenizer; owned tokens are counted and dropped, not retained.");
    println!(
        "Conversions use fresh converters; string output is dropped, writer output is discarded."
    );
    println!("Throughput counts decoded UTF-8 input bytes, not generated output.");
    let cases = workloads::CASES.map(|name| (name, workloads::workload(name)));
    for (name, case) in cases {
        // Check token count, complete consumption, EOF and converter agreement outside timing.
        case.check(workloads::read_all_tokens(&case));
        case.check_conversion();
        case.check_rewriting();
        measure("HtmlTokenizer_ReadAllTokens", name, &case, || {
            workloads::read_all_tokens(&case)
        });
        measure("HtmlToText_ConvertToString", name, &case, || {
            workloads::convert_to_string(&case)
        });
        measure("HtmlToText_ConvertToTextWriter", name, &case, || {
            workloads::convert_to_writer(&case)
        });
        measure("HtmlToHtml_ConvertToString", name, &case, || {
            workloads::rewrite_to_string(&case)
        });
        measure("HtmlToHtml_ConvertToTextWriter", name, &case, || {
            workloads::rewrite_to_writer(&case)
        });
        measure("HtmlToHtml_ConvertWithCallback", name, &case, || {
            workloads::rewrite_with_callback(&case)
        });
    }
}

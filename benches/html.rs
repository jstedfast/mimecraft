// Adapted from MimeKit's Benchmarks/Text/HtmlBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/html.rs"]
mod workloads;

fn measure(name: &str, case: &workloads::Workload) {
    // Check the canonical token count, complete consumption, and EOF before timing.
    case.check(workloads::read_all_tokens(case));
    for _ in 0..3 {
        black_box(workloads::read_all_tokens(case));
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            black_box(workloads::read_all_tokens(case));
            iterations += 1;
        }
        *sample = iterations as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "HtmlTokenizer_ReadAllTokens/{name:27} {:9.1} MiB/s {:9.1} documents/s (median of 5)",
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
    println!("Throughput counts decoded UTF-8 input bytes; HtmlToHtml workloads are deferred.");
    let cases = workloads::CASES.map(|name| (name, workloads::workload(name)));
    for (name, case) in cases {
        measure(name, &case);
    }
}

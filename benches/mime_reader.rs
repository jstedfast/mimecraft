// Adapted from MimeKit's Benchmarks/MimeParserBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/mime_reader.rs"]
mod workloads;

fn measure(
    name: &str,
    case: &workloads::Workload,
    mut operation: impl FnMut() -> (usize, u64, bool),
) {
    // Check message count, complete input consumption, and EOF before timing.
    case.check(operation());
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
        samples[2] * case.data.len() as f64 / 1048576.0,
        samples[2] * case.messages as f64,
    );
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
        "Fresh parsers; buffered variants use 4 KiB buffers; reader callbacks are no-ops; parsing includes model construction/drop."
    );
    println!("MimeReader = buffered input; MimeReader_Slice = direct slice input.");
    println!(
        "MimeParser = default copying API; MimeParser_Source = buffered source-backed API; MimeParser_Slice = unbuffered slice-backed API; bodies are not read after parsing."
    );
    let cases = workloads::CASES.map(|name| (name, workloads::workload(name)));
    let nested = workloads::workload(workloads::NESTED);
    for (name, case) in cases {
        measure(&format!("MimeReader_{name}"), &case, || {
            workloads::read(&case)
        });
        measure(&format!("MimeReader_Slice_{name}"), &case, || {
            workloads::read_slice(&case)
        });
        measure(&format!("MimeParser_{name}"), &case, || {
            workloads::parse_default(&case)
        });
        measure(&format!("MimeParser_Source_{name}"), &case, || {
            workloads::parse_source(&case)
        });
        measure(&format!("MimeParser_Slice_{name}"), &case, || {
            workloads::parse_slice(&case)
        });
    }
    // Upstream benchmarks this depth with the reader only, not owned parsing.
    measure("MimeReader_NestedRfc822x1000", &nested, || {
        workloads::read(&nested)
    });
    measure("MimeReader_Slice_NestedRfc822x1000", &nested, || {
        workloads::read_slice(&nested)
    });
}

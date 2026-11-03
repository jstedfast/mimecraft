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
    let compare_compliance = std::env::args().any(|argument| argument == "--compare-compliance");
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
    if compare_compliance {
        println!(
            "Compliance: ALL validators, max_issues_per_violation=1 per message; black-box logger, no output/storage."
        );
        println!(
            "Median of 9 paired samples >= 250 ms per mode, alternating off/on order; fixtures shared between modes."
        );
        println!("case,bytes,messages,off_mib_s,on_mib_s,throughput_loss_pct");
        for name in workloads::CASES.into_iter().chain([workloads::NESTED]) {
            let mut case = workloads::workload(name);
            compare(&format!("MimeReader_{name}"), &mut case, workloads::read);
            compare(
                &format!("MimeReader_Slice_{name}"),
                &mut case,
                workloads::read_slice,
            );
            if name != workloads::NESTED {
                compare(
                    &format!("MimeParser_{name}"),
                    &mut case,
                    workloads::parse_default,
                );
                compare(
                    &format!("MimeParser_Source_{name}"),
                    &mut case,
                    workloads::parse_source,
                );
                compare(
                    &format!("MimeParser_Slice_{name}"),
                    &mut case,
                    workloads::parse_slice,
                );
            }
        }
        return;
    }
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

fn compare(
    name: &str,
    case: &mut workloads::Workload,
    operation: fn(&workloads::Workload) -> (usize, u64, bool),
) {
    for enabled in [false, true] {
        case.compliance = enabled;
        case.check(operation(case));
        for _ in 0..3 {
            black_box(operation(case));
        }
    }
    let mut samples = [[0.0f64; 2]; 9];
    for (sample, pair) in samples.iter_mut().enumerate() {
        for mode in if sample % 2 == 0 { [0, 1] } else { [1, 0] } {
            case.compliance = mode == 1;
            let start = Instant::now();
            let mut iterations = 0;
            while start.elapsed() < Duration::from_millis(250) {
                black_box(operation(case));
                iterations += 1;
            }
            pair[mode] = iterations as f64 / start.elapsed().as_secs_f64();
        }
    }
    let [off, on] = [0, 1].map(|mode| {
        let mut values = samples.map(|pair| pair[mode]);
        values.sort_by(f64::total_cmp);
        values[4] * case.data.len() as f64 / 1048576.0
    });
    println!(
        "{name},{},{},{off:.3},{on:.3},{:.2}",
        case.data.len(),
        case.messages,
        100.0 * (1.0 - on / off),
    );
    case.compliance = false;
}

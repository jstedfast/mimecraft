// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

#[path = "support/tnef.rs"]
mod workloads;

#[path = "support/tnef_properties.rs"]
mod property_workloads;

#[path = "support/tnef_dates.rs"]
mod date_workloads;

#[path = "support/tnef_embedded.rs"]
mod embedded_workloads;

#[path = "support/tnef_conversions.rs"]
mod conversion_workloads;

#[path = "support/tnef_recipients.rs"]
mod recipient_workloads;

#[path = "support/rtf_compressed.rs"]
mod rtf_compressed_workloads;

fn measure(name: &str, size: usize, mut read: impl FnMut()) {
    for _ in 0..3 {
        read();
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut count = 0;
        while start.elapsed() < Duration::from_millis(100) {
            read();
            count += 1;
        }
        *sample = count as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:28} {:9.1} MiB/s {:10.1} documents/s (median of 5)",
        samples[2] * size as f64 / 1048576.0,
        samples[2]
    );
}

fn main() {
    println!("Initial synchronous raw TNEF attribute baseline; no upstream TNEF benchmark exists.");
    println!("Fresh readers, 4 KiB read-ahead; Stream uses a reused 4 KiB stack buffer.");
    println!("Collect includes owned-value allocation/destruction. Skip still computes checksums.");
    println!("Throughput counts original TNEF bytes; setup and correctness checks are not timed.");
    println!("MAPI parsing, conversion, logging, async and sans-I/O are not measured.");
    let cases: Vec<_> = workloads::CASES
        .into_iter()
        .map(workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in workloads::MODES {
            measure(
                &format!("{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(workloads::read(case, mode));
                },
            );
        }
    }
    println!(
        "MAPI properties: generated fixed, multivalue, named, recipient, and binary workloads."
    );
    println!(
        "Skip navigates properties; Stream reads raw variable values; Decode collects native values."
    );
    println!(
        "Property collects/drops each snapshot; Set retains complete property sets or all recipient rows."
    );
    println!(
        "All modes parse names and checksums. No message/attachment models, embedded readers, logging, or MIME conversion."
    );
    let cases: Vec<_> = property_workloads::CASES
        .into_iter()
        .map(property_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in property_workloads::MODES {
            measure(
                &format!("Properties/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(property_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Date interpretation: legacy attributes, AppTime, FILETIME, and mixed multivalues.");
    println!(
        "Includes date validation, typed conversion, checksums, and a logger rejecting unexpected issues."
    );
    let cases: Vec<_> = date_workloads::CASES
        .into_iter()
        .map(date_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Dates/Read/{}", case.name),
            case.input.len(),
            || {
                black_box(date_workloads::read(case));
            },
        );
    }
    println!("Embedded messages: bounded recursive readers, inherited logger, and shared limits.");
    println!("Includes streaming bodies, checksums at every depth, and reader setup/destruction.");
    let cases: Vec<_> = embedded_workloads::CASES
        .into_iter()
        .map(embedded_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Embedded/Read/{}", case.name),
            case.input.len(),
            || {
                black_box(embedded_workloads::read(case));
            },
        );
    }
    println!("Owned access: preloaded snapshots; reader/setup/drop are outside measurement.");
    println!(
        "Equivalent wire-byte throughput, NOT parser throughput; includes binary text decoding."
    );
    let cases: Vec<_> = conversion_workloads::CASES
        .into_iter()
        .map(conversion_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("Conversions/Access/{}", case.name),
            case.input_length,
            || {
                black_box(conversion_workloads::read(case));
            },
        );
    }
    println!("Recipients: 256 owned rows, type flags, name/address fallbacks, and binary text.");
    println!("Load includes parsing, checksums, model allocation/drop, and all four accessors.");
    println!(
        "Access uses preloaded rows; its wire-byte throughput is equivalent, NOT parser throughput."
    );
    let cases: Vec<_> = recipient_workloads::CASES
        .into_iter()
        .map(recipient_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        for mode in recipient_workloads::MODES {
            measure(
                &format!("Recipients/{}/{}", mode.name(), case.name),
                case.input.len(),
                || {
                    black_box(recipient_workloads::read(case, mode));
                },
            );
        }
    }
    println!("Compressed RTF: literals, dictionary overlap/wrap, MELA, and one-byte input.");
    println!(
        "Includes fresh inline dictionary, bounded-chunk output buffers, CRC validation and drop."
    );
    let cases: Vec<_> = rtf_compressed_workloads::CASES
        .into_iter()
        .map(rtf_compressed_workloads::workload)
        .collect();
    for case in &cases {
        case.check();
    }
    for case in &cases {
        measure(
            &format!("RtfCompressed/Decode/{}", case.name),
            case.input.len(),
            || {
                black_box(rtf_compressed_workloads::read(case));
            },
        );
    }
}

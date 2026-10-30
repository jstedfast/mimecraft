// Adapted from MimeKit's Benchmarks/IO/Filters/*Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/timing.rs"]
mod timing;
#[path = "support/filters.rs"]
mod workloads;

fn main() {
    timing::heading();
    println!("Fresh codecs/filters; output allocation/destruction included.");
    println!("BestEncoding performs 100 reset passes per operation; byte rates count all passes.");
    for name in workloads::CASES {
        let case = workloads::workload(name);
        case.check();
        timing::measure(
            &format!("filters/Process/{name}"),
            case.bytes_per_operation(),
            || case.run(),
        );
    }
}

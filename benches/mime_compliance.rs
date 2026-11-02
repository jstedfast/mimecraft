// Adapted from MimeKit's Benchmarks/MimeComplianceValidatorBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

#[path = "support/timing.rs"]
mod timing;
#[path = "support/mime_compliance.rs"]
mod workloads;

fn main() {
    timing::heading();
    println!("Compliance reporting through fresh readers, including parsing/setup/drop.");
    for name in workloads::CASES {
        let case = workloads::workload(name);
        case.check();
        timing::measure(
            &format!("mime/{}/{name}", case.benchmark_name()),
            case.data.len(),
            || {
                black_box(workloads::validate(&case));
            },
        );
    }
}

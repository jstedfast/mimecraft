// Adapted from MimeKit's Benchmarks/MimeMessageBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/timing.rs"]
mod timing;
#[path = "support/message.rs"]
mod workloads;

fn main() {
    timing::heading();
    println!("Reused multipart message; per-leaf transfer encoding reset is included.");
    println!("MiB/s counts logical payload size, not actual bytes read by Prepare.");
    for (name, constraint) in workloads::CASES {
        let case = workloads::workload(constraint);
        case.check();
        timing::measure(
            &format!("message/MimeMessage_Prepare_EncodingConstraint/{name}"),
            case.input_bytes,
            || case.run(),
        );
    }
}

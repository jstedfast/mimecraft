// Adapted from MimeKit's Benchmarks/Text/RtfConverterBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[path = "../benches/support/rtf.rs"]
mod workloads;

#[test]
fn original_generated_workloads_match_upstream_inputs_tokens_and_outputs() {
    for name in workloads::CASES.into_iter().chain(["ExtractedHtml"]) {
        workloads::workload(name).check();
    }
}

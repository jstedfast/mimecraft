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

#[test]
fn sans_io_engine_preserves_generated_workload_token_counts_and_consumption() {
    for name in workloads::CASES {
        let case = workloads::workload(name);
        assert_eq!(
            workloads::read_all_tokens_engine(&case),
            workloads::read_all_tokens(&case),
            "{name}"
        );
    }
}

// Adapted from MimeKit's Benchmarks/Text/HtmlBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's fixture names across measurement engines."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use workloads::Workload;

#[path = "support/html.rs"]
mod workloads;

fn prepare(name: &str) -> Workload {
    let case = workloads::workload(name);
    case.check(workloads::read_all_tokens(&case));
    case
}

// These workloads track allocations as well as throughput: per-character
// allocation matters when processing the HTML body of every server message.
#[metabench::benchmark(READ, "html", "HtmlTokenizer_ReadAllTokens")]
#[bench::test_html(&prepare("test_html"))]
#[bench::Gimhae_Kim_clan_utf_8_html(&prepare("Gimhae_Kim_clan_utf_8_html"))]
fn read_all_tokens(case: &Workload) -> (usize, u64, bool) {
    workloads::read_all_tokens(case)
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(READ.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.html.len() as u64));
        group.bench_function(BenchmarkId::new(READ.benchmark_name(), name), |b| {
            b.iter(|| read_all_tokens(&case));
        });
    }
    group.finish();
}

metabench::main!(criterion = criterion_benchmarks, benchmarks = [READ]);

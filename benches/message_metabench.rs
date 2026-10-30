// Adapted from MimeKit's Benchmarks/MimeMessageBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's constraint names."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use mimecraft::EncodingConstraint;
use workloads::Workload;

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/message.rs"]
mod workloads;

fn prepare(constraint: EncodingConstraint) -> Workload {
    let case = workloads::workload(constraint);
    case.check();
    case
}

#[metabench::benchmark(PREPARE, "message", "MimeMessage_Prepare_EncodingConstraint")]
#[bench::None(&prepare(EncodingConstraint::None))]
#[bench::SevenBit(&prepare(EncodingConstraint::SevenBit))]
#[bench::EightBit(&prepare(EncodingConstraint::EightBit))]
fn prepare_message(case: &Workload) {
    case.run();
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(PREPARE.group_name());
    for (name, constraint) in workloads::CASES {
        let case = prepare(constraint);
        group.throughput(Throughput::Bytes(case.input_bytes as u64));
        group.bench_function(BenchmarkId::new(PREPARE.benchmark_name(), name), |b| {
            b.iter(|| prepare_message(&case))
        });
    }
    group.finish();
}

metabench::main!(criterion = criterion_benchmarks, benchmarks = [PREPARE]);

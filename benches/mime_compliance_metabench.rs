// Adapted from MimeKit's Benchmarks/MimeComplianceValidatorBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's workload names."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use workloads::Workload;

#[path = "support/mime_compliance.rs"]
mod workloads;

fn prepare(name: &str) -> Workload {
    let case = workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(VALIDATE, "mime", "MimeCompliance_ValidateHeaders")]
#[bench::PlainSubject(&prepare("PlainSubject"))]
#[bench::LongPlainSubject(&prepare("LongPlainSubject"))]
#[bench::ShortQ(&prepare("ShortQ"))]
#[bench::ShortB(&prepare("ShortB"))]
#[bench::FoldedQ(&prepare("FoldedQ"))]
#[bench::FoldedB(&prepare("FoldedB"))]
#[bench::PlainDisplayName(&prepare("PlainDisplayName"))]
#[bench::EncodedDisplayNames(&prepare("EncodedDisplayNames"))]
#[bench::Comments(&prepare("Comments"))]
#[bench::QuotedWord(&prepare("QuotedWord"))]
#[bench::MalformedQ(&prepare("MalformedQ"))]
#[bench::MalformedB(&prepare("MalformedB"))]
#[bench::Unterminated(&prepare("Unterminated"))]
#[bench::SparseWords(&prepare("SparseWords"))]
#[bench::ParameterNone(&prepare("ParameterNone"))]
#[bench::ParameterCharset(&prepare("ParameterCharset"))]
#[bench::ParameterFilename(&prepare("ParameterFilename"))]
#[bench::ParameterPair(&prepare("ParameterPair"))]
#[bench::ParameterContinuation(&prepare("ParameterContinuation"))]
#[bench::ParameterRepeated(&prepare("ParameterRepeated"))]
#[bench::ParameterManyNames(&prepare("ParameterManyNames"))]
#[bench::ParameterManySegments(&prepare("ParameterManySegments"))]
fn validate(case: &Workload) -> usize {
    workloads::validate(case)
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(VALIDATE.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.data.len() as u64));
        group.bench_function(BenchmarkId::new(VALIDATE.benchmark_name(), name), |b| {
            b.iter(|| validate(&case))
        });
    }
    group.finish();
}

metabench::main!(criterion = criterion_benchmarks, benchmarks = [VALIDATE]);

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
#[bench::ContentIdSingle(&prepare("ContentIdSingle"))]
#[bench::ContentIdRepeated(&prepare("ContentIdRepeated"))]
#[bench::ContentIdFolded(&prepare("ContentIdFolded"))]
#[bench::ContentIdMany(&prepare("ContentIdMany"))]
#[bench::ContentIdNoLogger(&prepare("ContentIdNoLogger"))]
#[bench::PadBitsBCanonical(&prepare("PadBitsBCanonical"))]
#[bench::PadBitsBOne(&prepare("PadBitsBOne"))]
#[bench::PadBitsBTwo(&prepare("PadBitsBTwo"))]
#[bench::Utf8Valid(&prepare("Utf8Valid"))]
#[bench::Utf8Invalid(&prepare("Utf8Invalid"))]
fn validate(case: &Workload) -> usize {
    workloads::validate(case)
}

#[metabench::benchmark(VALIDATE_BODY, "mime", "MimeCompliance_ValidateBody")]
#[bench::Base64BodyCanonical(&prepare("Base64BodyCanonical"))]
#[bench::Base64BodyPadBits(&prepare("Base64BodyPadBits"))]
#[bench::Base64BodyWhitespace(&prepare("Base64BodyWhitespace"))]
#[bench::Base64BodyNoLogger(&prepare("Base64BodyNoLogger"))]
#[bench::ScanBody7Bit(&prepare("ScanBody7Bit"))]
#[bench::ScanBody8Bit(&prepare("ScanBody8Bit"))]
#[bench::ScanBodyBinary(&prepare("ScanBodyBinary"))]
#[bench::ScanBodyNoLogger(&prepare("ScanBodyNoLogger"))]
fn validate_body(case: &Workload) -> usize {
    workloads::validate(case)
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(VALIDATE.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.data.len() as u64));
        group.bench_function(BenchmarkId::new(case.benchmark_name(), name), |b| {
            b.iter(|| validate(&case))
        });
    }
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [VALIDATE, VALIDATE_BODY]
);

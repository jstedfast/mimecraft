// Adapted from MimeKit's Benchmarks/IO/Filters/*Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's workload names."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use workloads::Workload;

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/filters.rs"]
mod workloads;

fn prepare(name: &'static str) -> Workload {
    let case = workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(PROCESS, "filters", "Process")]
#[bench::ArmoredFromFilter_FromRussiaWithLove(&prepare("ArmoredFromFilter_FromRussiaWithLove"))]
#[bench::MboxFromFilter_FromRussiaWithLove(&prepare("MboxFromFilter_FromRussiaWithLove"))]
#[bench::BestEncoding_LoremIpsum(&prepare("BestEncoding_LoremIpsum"))]
#[bench::BestEncoding_GirlJpeg(&prepare("BestEncoding_GirlJpeg"))]
#[bench::TrailingWhitespace_LoremIpsum(&prepare("TrailingWhitespace_LoremIpsum"))]
#[bench::Dos2Unix_LoremIpsumDos(&prepare("Dos2Unix_LoremIpsumDos"))]
#[bench::Dos2Unix_LoremIpsumUnix(&prepare("Dos2Unix_LoremIpsumUnix"))]
#[bench::Dos2Unix_PathologicalDos(&prepare("Dos2Unix_PathologicalDos"))]
#[bench::Dos2Unix_PathologicalUnix(&prepare("Dos2Unix_PathologicalUnix"))]
#[bench::Unix2Dos_LoremIpsumDos(&prepare("Unix2Dos_LoremIpsumDos"))]
#[bench::Unix2Dos_LoremIpsumUnix(&prepare("Unix2Dos_LoremIpsumUnix"))]
#[bench::Unix2Dos_PathologicalDos(&prepare("Unix2Dos_PathologicalDos"))]
#[bench::Unix2Dos_PathologicalUnix(&prepare("Unix2Dos_PathologicalUnix"))]
#[bench::Base64Encode_60(&prepare("Base64Encode_60"))]
#[bench::Base64Encode_64(&prepare("Base64Encode_64"))]
#[bench::Base64Encode_68(&prepare("Base64Encode_68"))]
#[bench::Base64Encode_72(&prepare("Base64Encode_72"))]
#[bench::Base64Encode_76(&prepare("Base64Encode_76"))]
#[bench::Base64EncodeStream_60(&prepare("Base64EncodeStream_60"))]
#[bench::Base64EncodeStream_64(&prepare("Base64EncodeStream_64"))]
#[bench::Base64EncodeStream_68(&prepare("Base64EncodeStream_68"))]
#[bench::Base64EncodeStream_72(&prepare("Base64EncodeStream_72"))]
#[bench::Base64EncodeStream_76(&prepare("Base64EncodeStream_76"))]
#[bench::QuotedPrintableEncodeStream(&prepare("QuotedPrintableEncodeStream"))]
#[bench::UUEncodeStream(&prepare("UUEncodeStream"))]
#[bench::Base64Decode(&prepare("Base64Decode"))]
#[bench::Base64DecodeStream(&prepare("Base64DecodeStream"))]
#[bench::QuotedPrintableDecode(&prepare("QuotedPrintableDecode"))]
#[bench::QuotedPrintableDecodeStream(&prepare("QuotedPrintableDecodeStream"))]
#[bench::UUDecode(&prepare("UUDecode"))]
#[bench::UUDecodeStream(&prepare("UUDecodeStream"))]
fn process(case: &Workload) {
    case.run();
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(PROCESS.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.bytes_per_operation() as u64));
        group.bench_function(BenchmarkId::new(PROCESS.benchmark_name(), name), |b| {
            b.iter(|| process(&case))
        });
    }
    group.finish();
}

metabench::main!(criterion = criterion_benchmarks, benchmarks = [PROCESS]);

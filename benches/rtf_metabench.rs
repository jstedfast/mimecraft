// Adapted from MimeKit's Benchmarks/Text/RtfConverterBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's workload names across measurement engines."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use workloads::Workload;

#[path = "support/rtf.rs"]
mod workloads;

fn prepare(name: &'static str) -> Workload {
    let case = workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(READ, "rtf", "RtfTokenizer_ReadAllTokens")]
#[bench::WordDocument(&prepare("WordDocument"))]
#[bench::UnicodeDocument(&prepare("UnicodeDocument"))]
#[bench::EncapsulatedHtml(&prepare("EncapsulatedHtml"))]
#[bench::DeepNesting(&prepare("DeepNesting"))]
#[bench::DeepFormattedNesting(&prepare("DeepFormattedNesting"))]
#[bench::LargeFontTable(&prepare("LargeFontTable"))]
#[bench::LargeBinary(&prepare("LargeBinary"))]
fn read_all_tokens(case: &Workload) -> (usize, usize, bool) {
    workloads::read_all_tokens(case)
}

#[metabench::benchmark(TO_TEXT, "rtf", "RtfToText")]
#[bench::WordDocument(&prepare("WordDocument"))]
#[bench::UnicodeDocument(&prepare("UnicodeDocument"))]
#[bench::EncapsulatedHtml(&prepare("EncapsulatedHtml"))]
#[bench::DeepNesting(&prepare("DeepNesting"))]
#[bench::DeepFormattedNesting(&prepare("DeepFormattedNesting"))]
#[bench::LargeFontTable(&prepare("LargeFontTable"))]
#[bench::LargeBinary(&prepare("LargeBinary"))]
fn to_text(case: &Workload) -> String {
    workloads::to_text(case)
}

#[metabench::benchmark(TO_HTML, "rtf", "RtfToHtml_Render")]
#[bench::WordDocument(&prepare("WordDocument"))]
#[bench::UnicodeDocument(&prepare("UnicodeDocument"))]
#[bench::EncapsulatedHtml(&prepare("EncapsulatedHtml"))]
#[bench::DeepFormattedNesting(&prepare("DeepFormattedNesting"))]
fn to_html(case: &Workload) -> String {
    workloads::to_html(case)
}

#[metabench::benchmark(EXTRACT, "rtf", "RtfToHtml_Extract")]
#[bench::EncapsulatedHtml(&prepare("EncapsulatedHtml"))]
fn extract_html(case: &Workload) -> String {
    workloads::extract_html(case)
}

#[metabench::benchmark(REWRITE, "rtf", "HtmlToHtml")]
#[bench::ExtractedHtml(&prepare("ExtractedHtml"))]
fn rewrite_html(case: &Workload) -> String {
    workloads::rewrite_html(case)
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(READ.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(BenchmarkId::new(READ.benchmark_name(), name), |b| {
            b.iter(|| read_all_tokens(&case));
        });
        group.bench_function(BenchmarkId::new(TO_TEXT.benchmark_name(), name), |b| {
            b.iter(|| to_text(&case));
        });
        if workloads::HTML_CASES.contains(&name) {
            group.bench_function(BenchmarkId::new(TO_HTML.benchmark_name(), name), |b| {
                b.iter(|| to_html(&case));
            });
        }
        if case.extracted_html.is_some() {
            group.bench_function(BenchmarkId::new(EXTRACT.benchmark_name(), name), |b| {
                b.iter(|| extract_html(&case));
            });
        }
    }
    let case = prepare("ExtractedHtml");
    group.throughput(Throughput::Bytes(case.input.len() as u64));
    group.bench_function(BenchmarkId::new(REWRITE.benchmark_name(), case.name), |b| {
        b.iter(|| rewrite_html(&case));
    });
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [READ, TO_TEXT, TO_HTML, EXTRACT, REWRITE]
);

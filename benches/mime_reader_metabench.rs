// Adapted from MimeKit's Benchmarks/MimeParserBenchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's workload names across measurement engines."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use mimecraft::{Header, HeaderList};
use std::hint::black_box;
use workloads::Workload;

#[path = "support/mime_reader.rs"]
mod workloads;

fn prepare(name: &str, operation: fn(&Workload) -> (usize, u64, bool)) -> Workload {
    let case = workloads::workload(name);
    case.check(operation(&case));
    case
}

#[metabench::benchmark(READ, "mime", "MimeReader")]
#[bench::StarTrekMessage(&prepare("StarTrekMessage", workloads::read))]
#[bench::ContentLengthMbox(&prepare("ContentLengthMbox", workloads::read))]
#[bench::JwzMbox(&prepare("JwzMbox", workloads::read))]
#[bench::HeaderStressTest(&prepare("HeaderStressTest", workloads::read))]
#[bench::MixedNested64(&prepare("MixedNested64", workloads::read))]
#[bench::NestedRfc822x1000(&prepare(workloads::NESTED, workloads::read))]
fn read(case: &Workload) -> (usize, u64, bool) {
    workloads::read(case)
}

#[metabench::benchmark(READ_SLICE, "mime", "MimeReader_Slice")]
#[bench::StarTrekMessage(&prepare("StarTrekMessage", workloads::read_slice))]
#[bench::ContentLengthMbox(&prepare("ContentLengthMbox", workloads::read_slice))]
#[bench::JwzMbox(&prepare("JwzMbox", workloads::read_slice))]
#[bench::HeaderStressTest(&prepare("HeaderStressTest", workloads::read_slice))]
#[bench::MixedNested64(&prepare("MixedNested64", workloads::read_slice))]
#[bench::NestedRfc822x1000(&prepare(workloads::NESTED, workloads::read_slice))]
fn read_slice(case: &Workload) -> (usize, u64, bool) {
    workloads::read_slice(case)
}

#[metabench::benchmark(PARSE, "mime", "MimeParser")]
#[bench::StarTrekMessage(&prepare("StarTrekMessage", workloads::parse_default))]
#[bench::ContentLengthMbox(&prepare("ContentLengthMbox", workloads::parse_default))]
#[bench::JwzMbox(&prepare("JwzMbox", workloads::parse_default))]
#[bench::HeaderStressTest(&prepare("HeaderStressTest", workloads::parse_default))]
#[bench::MixedNested64(&prepare("MixedNested64", workloads::parse_default))]
fn parse(case: &Workload) -> (usize, u64, bool) {
    workloads::parse_default(case)
}

#[metabench::benchmark(SOURCE, "mime", "MimeParser_Source")]
#[bench::StarTrekMessage(&prepare("StarTrekMessage", workloads::parse_source))]
#[bench::ContentLengthMbox(&prepare("ContentLengthMbox", workloads::parse_source))]
#[bench::JwzMbox(&prepare("JwzMbox", workloads::parse_source))]
#[bench::HeaderStressTest(&prepare("HeaderStressTest", workloads::parse_source))]
#[bench::MixedNested64(&prepare("MixedNested64", workloads::parse_source))]
fn parse_source(case: &Workload) -> (usize, u64, bool) {
    workloads::parse_source(case)
}

#[metabench::benchmark(SLICE, "mime", "MimeParser_Slice")]
#[bench::StarTrekMessage(&prepare("StarTrekMessage", workloads::parse_slice))]
#[bench::ContentLengthMbox(&prepare("ContentLengthMbox", workloads::parse_slice))]
#[bench::JwzMbox(&prepare("JwzMbox", workloads::parse_slice))]
#[bench::HeaderStressTest(&prepare("HeaderStressTest", workloads::parse_slice))]
#[bench::MixedNested64(&prepare("MixedNested64", workloads::parse_slice))]
fn parse_slice(case: &Workload) -> (usize, u64, bool) {
    workloads::parse_slice(case)
}

const LOOKUP_CASES: [&str; 6] = [
    "LowercaseHit",
    "MixedCaseHit",
    "UppercaseHit",
    "MixedCaseMiss",
    "LongLowercaseHit",
    "LongMixedCaseHit",
];
const LOOKUPS: usize = 1000;

struct Lookup {
    headers: HeaderList,
    field: String,
}

fn prepare_lookup(name: &str) -> Lookup {
    let long_field = format!("X-{}", "Long-Header-".repeat(12));
    let mut headers: HeaderList = [
        "From: sender@example.org",
        "To: recipient@example.org",
        "Subject: example",
        "Content-Type: text/plain",
        "Content-Transfer-Encoding: 7bit",
        "Received: first",
        "Received: second",
    ]
    .into_iter()
    .map(|raw| Header::parse(raw.as_bytes()).unwrap())
    .collect();
    headers.add(Header::parse(format!("{long_field}: value").as_bytes()).unwrap());
    let field = match name {
        "LowercaseHit" => "content-type".to_owned(),
        "MixedCaseHit" => "Content-Type".to_owned(),
        "UppercaseHit" => "CONTENT-TYPE".to_owned(),
        "MixedCaseMiss" => "X-Missing".to_owned(),
        "LongLowercaseHit" => long_field.to_ascii_lowercase(),
        "LongMixedCaseHit" => long_field,
        _ => panic!("unknown lookup workload: {name}"),
    };
    let case = Lookup { headers, field };
    assert_eq!(
        lookup(&case),
        if name == "MixedCaseMiss" { 0 } else { LOOKUPS }
    );
    case
}

#[metabench::benchmark(LOOKUP, "headers", "HeaderList_Get")]
#[bench::LowercaseHit(&prepare_lookup("LowercaseHit"))]
#[bench::MixedCaseHit(&prepare_lookup("MixedCaseHit"))]
#[bench::UppercaseHit(&prepare_lookup("UppercaseHit"))]
#[bench::MixedCaseMiss(&prepare_lookup("MixedCaseMiss"))]
#[bench::LongLowercaseHit(&prepare_lookup("LongLowercaseHit"))]
#[bench::LongMixedCaseHit(&prepare_lookup("LongMixedCaseHit"))]
fn lookup(case: &Lookup) -> usize {
    let mut found = 0;
    for _ in 0..LOOKUPS {
        found += black_box(&case.headers)
            .get(black_box(case.field.as_str()))
            .is_some() as usize;
    }
    found
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(READ.group_name());
    for name in workloads::CASES.into_iter().chain([workloads::NESTED]) {
        let case = prepare(name, workloads::read);
        group.throughput(Throughput::Bytes(case.data.len() as u64));
        group.bench_function(BenchmarkId::new(READ.benchmark_name(), name), |b| {
            b.iter(|| read(&case));
        });
        case.check(workloads::read_slice(&case));
        group.bench_function(BenchmarkId::new(READ_SLICE.benchmark_name(), name), |b| {
            b.iter(|| read_slice(&case));
        });
        if name != workloads::NESTED {
            case.check(workloads::parse_default(&case));
            case.check(workloads::parse_source(&case));
            case.check(workloads::parse_slice(&case));
            group.bench_function(BenchmarkId::new(PARSE.benchmark_name(), name), |b| {
                b.iter(|| parse(&case));
            });
            group.bench_function(BenchmarkId::new(SOURCE.benchmark_name(), name), |b| {
                b.iter(|| parse_source(&case));
            });
            group.bench_function(BenchmarkId::new(SLICE.benchmark_name(), name), |b| {
                b.iter(|| parse_slice(&case));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group(LOOKUP.group_name());
    group.throughput(Throughput::Elements(LOOKUPS as u64));
    for name in LOOKUP_CASES {
        let case = prepare_lookup(name);
        group.bench_function(BenchmarkId::new(LOOKUP.benchmark_name(), name), |b| {
            b.iter(|| lookup(&case));
        });
    }
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [READ, READ_SLICE, PARSE, SOURCE, SLICE, LOOKUP]
);

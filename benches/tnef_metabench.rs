// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(non_snake_case, reason = "Case IDs match the standalone benchmark.")]

use criterion::{BenchmarkId, Criterion, Throughput};
use workloads::{Mode, Summary, Workload};

#[path = "support/tnef.rs"]
mod workloads;

#[path = "support/tnef_properties.rs"]
mod property_workloads;

#[path = "support/tnef_dates.rs"]
mod date_workloads;

#[path = "support/tnef_embedded.rs"]
mod embedded_workloads;

#[path = "support/tnef_conversions.rs"]
mod conversion_workloads;

fn prepare_conversions(name: &'static str) -> conversion_workloads::Workload {
    let case = conversion_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(CONVERSIONS, "tnef-conversions", "Access")]
#[bench::Numeric(&prepare_conversions("Numeric"))]
#[bench::Text(&prepare_conversions("Text"))]
#[bench::BinaryText(&prepare_conversions("BinaryText"))]
#[bench::Bytes(&prepare_conversions("Bytes"))]
#[bench::Lookup(&prepare_conversions("Lookup"))]
fn conversions(case: &conversion_workloads::Workload) -> conversion_workloads::Summary {
    conversion_workloads::read(case)
}

fn prepare_embedded(name: &'static str) -> embedded_workloads::Workload {
    let case = embedded_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(EMBEDDED, "tnef-embedded", "Read")]
#[bench::Single(&prepare_embedded("Single"))]
#[bench::Deep(&prepare_embedded("Deep"))]
#[bench::Siblings(&prepare_embedded("Siblings"))]
#[bench::ShortReads(&prepare_embedded("ShortReads"))]
fn embedded(case: &embedded_workloads::Workload) -> embedded_workloads::Summary {
    embedded_workloads::read(case)
}

fn prepare_dates(name: &'static str) -> date_workloads::Workload {
    let case = date_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(DATES, "tnef-dates", "Read")]
#[bench::Legacy(&prepare_dates("Legacy"))]
#[bench::AppTime(&prepare_dates("AppTime"))]
#[bench::SysTime(&prepare_dates("SysTime"))]
#[bench::MultiValue(&prepare_dates("MultiValue"))]
#[bench::ShortReads(&prepare_dates("ShortReads"))]
fn dates(case: &date_workloads::Workload) -> date_workloads::Summary {
    date_workloads::read(case)
}

fn prepare_properties(name: &'static str) -> property_workloads::Workload {
    let case = property_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(PROPERTY_SKIP, "tnef-properties", "Skip")]
#[bench::Fixed(&prepare_properties("Fixed"))]
#[bench::MultiValue(&prepare_properties("MultiValue"))]
#[bench::Named(&prepare_properties("Named"))]
#[bench::Recipients(&prepare_properties("Recipients"))]
#[bench::LargeBinary(&prepare_properties("LargeBinary"))]
#[bench::ShortReads(&prepare_properties("ShortReads"))]
fn property_skip(case: &property_workloads::Workload) -> property_workloads::Summary {
    property_workloads::read(case, property_workloads::Mode::Skip)
}

#[metabench::benchmark(PROPERTY_STREAM, "tnef-properties", "Stream")]
#[bench::Fixed(&prepare_properties("Fixed"))]
#[bench::MultiValue(&prepare_properties("MultiValue"))]
#[bench::Named(&prepare_properties("Named"))]
#[bench::Recipients(&prepare_properties("Recipients"))]
#[bench::LargeBinary(&prepare_properties("LargeBinary"))]
#[bench::ShortReads(&prepare_properties("ShortReads"))]
fn property_stream(case: &property_workloads::Workload) -> property_workloads::Summary {
    property_workloads::read(case, property_workloads::Mode::Stream)
}

#[metabench::benchmark(PROPERTY_DECODE, "tnef-properties", "Decode")]
#[bench::Fixed(&prepare_properties("Fixed"))]
#[bench::MultiValue(&prepare_properties("MultiValue"))]
#[bench::Named(&prepare_properties("Named"))]
#[bench::Recipients(&prepare_properties("Recipients"))]
#[bench::LargeBinary(&prepare_properties("LargeBinary"))]
#[bench::ShortReads(&prepare_properties("ShortReads"))]
fn property_decode(case: &property_workloads::Workload) -> property_workloads::Summary {
    property_workloads::read(case, property_workloads::Mode::Decode)
}

#[metabench::benchmark(PROPERTY_SNAPSHOT, "tnef-properties", "Property")]
#[bench::Fixed(&prepare_properties("Fixed"))]
#[bench::MultiValue(&prepare_properties("MultiValue"))]
#[bench::Named(&prepare_properties("Named"))]
#[bench::Recipients(&prepare_properties("Recipients"))]
#[bench::LargeBinary(&prepare_properties("LargeBinary"))]
#[bench::ShortReads(&prepare_properties("ShortReads"))]
fn property_snapshot(case: &property_workloads::Workload) -> property_workloads::Summary {
    property_workloads::read(case, property_workloads::Mode::Property)
}

#[metabench::benchmark(PROPERTY_SET, "tnef-properties", "Set")]
#[bench::Fixed(&prepare_properties("Fixed"))]
#[bench::MultiValue(&prepare_properties("MultiValue"))]
#[bench::Named(&prepare_properties("Named"))]
#[bench::Recipients(&prepare_properties("Recipients"))]
#[bench::LargeBinary(&prepare_properties("LargeBinary"))]
#[bench::ShortReads(&prepare_properties("ShortReads"))]
fn property_set(case: &property_workloads::Workload) -> property_workloads::Summary {
    property_workloads::read(case, property_workloads::Mode::Set)
}

fn prepare(name: &'static str) -> Workload {
    let case = workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(SKIP, "tnef", "Skip")]
#[bench::Body(&prepare("Body"))]
#[bench::Attachments(&prepare("Attachments"))]
#[bench::Christmas(&prepare("Christmas"))]
#[bench::ManySmall(&prepare("ManySmall"))]
#[bench::LargeValue(&prepare("LargeValue"))]
#[bench::ShortReads(&prepare("ShortReads"))]
fn skip(case: &Workload) -> Summary {
    workloads::read(case, Mode::Skip)
}

#[metabench::benchmark(STREAM, "tnef", "Stream")]
#[bench::Body(&prepare("Body"))]
#[bench::Attachments(&prepare("Attachments"))]
#[bench::Christmas(&prepare("Christmas"))]
#[bench::ManySmall(&prepare("ManySmall"))]
#[bench::LargeValue(&prepare("LargeValue"))]
#[bench::ShortReads(&prepare("ShortReads"))]
fn stream(case: &Workload) -> Summary {
    workloads::read(case, Mode::Stream)
}

#[metabench::benchmark(COLLECT, "tnef", "Collect")]
#[bench::Body(&prepare("Body"))]
#[bench::Attachments(&prepare("Attachments"))]
#[bench::Christmas(&prepare("Christmas"))]
#[bench::ManySmall(&prepare("ManySmall"))]
#[bench::LargeValue(&prepare("LargeValue"))]
#[bench::ShortReads(&prepare("ShortReads"))]
fn collect(case: &Workload) -> Summary {
    workloads::read(case, Mode::Collect)
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tnef");
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| workloads::read(&case, mode));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-properties");
    for name in property_workloads::CASES {
        let case = prepare_properties(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in property_workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| property_workloads::read(&case, mode));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-dates");
    for name in date_workloads::CASES {
        let case = prepare_dates(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(BenchmarkId::new("Read", name), |b| {
            b.iter(|| date_workloads::read(&case));
        });
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-embedded");
    for name in embedded_workloads::CASES {
        let case = prepare_embedded(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(BenchmarkId::new("Read", name), |b| {
            b.iter(|| embedded_workloads::read(&case));
        });
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-conversions");
    for name in conversion_workloads::CASES {
        let case = prepare_conversions(name);
        group.throughput(Throughput::Bytes(case.input_length as u64));
        group.bench_function(BenchmarkId::new("Access", name), |b| {
            b.iter(|| conversion_workloads::read(&case));
        });
    }
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [
        SKIP,
        STREAM,
        COLLECT,
        PROPERTY_SKIP,
        PROPERTY_STREAM,
        PROPERTY_DECODE,
        PROPERTY_SNAPSHOT,
        PROPERTY_SET,
        DATES,
        EMBEDDED,
        CONVERSIONS
    ]
);

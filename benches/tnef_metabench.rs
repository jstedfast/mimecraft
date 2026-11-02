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

#[path = "support/tnef_recipients.rs"]
mod recipient_workloads;

#[path = "support/rtf_compressed.rs"]
mod rtf_compressed_workloads;

#[path = "support/rtf_compressed_writer.rs"]
mod rtf_writer_workloads;

#[path = "support/tnef_writer.rs"]
mod writer_workloads;

#[path = "support/tnef_calendar.rs"]
mod calendar_workloads;

fn prepare_calendar(name: &'static str) -> calendar_workloads::Workload {
    let case = calendar_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(CALENDAR, "tnef-calendar", "Write")]
#[bench::Components(&prepare_calendar("Components"))]
#[bench::EscapedText(&prepare_calendar("EscapedText"))]
#[bench::Parameters(&prepare_calendar("Parameters"))]
#[bench::FoldAscii(&prepare_calendar("FoldAscii"))]
#[bench::FoldUtf8(&prepare_calendar("FoldUtf8"))]
#[bench::ShortWrites(&prepare_calendar("ShortWrites"))]
#[bench::Formatting(&prepare_calendar("Formatting"))]
fn write_calendar(case: &calendar_workloads::Workload) -> calendar_workloads::Summary {
    calendar_workloads::write(case)
}

fn prepare_writer(name: &'static str) -> writer_workloads::Workload {
    let case = writer_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(WRITER, "tnef-writer", "Write")]
#[bench::Typed(&prepare_writer("Typed"))]
#[bench::ManySmall(&prepare_writer("ManySmall"))]
#[bench::LargeValue(&prepare_writer("LargeValue"))]
#[bench::Stream(&prepare_writer("Stream"))]
#[bench::ShortReads(&prepare_writer("ShortReads"))]
#[bench::ShortWrites(&prepare_writer("ShortWrites"))]
#[bench::Buffered(&prepare_writer("Buffered"))]
#[bench::BufferedCursor(&prepare_writer("BufferedCursor"))]
#[bench::BufferedShortChunks(&prepare_writer("BufferedShortChunks"))]
#[bench::BufferedShortReads(&prepare_writer("BufferedShortReads"))]
#[bench::BufferedShortWrites(&prepare_writer("BufferedShortWrites"))]
#[bench::Properties(&prepare_writer("Properties"))]
#[bench::PropertyCopy(&prepare_writer("PropertyCopy"))]
#[bench::PropertyCursor(&prepare_writer("PropertyCursor"))]
#[bench::PropertyBinary(&prepare_writer("PropertyBinary"))]
#[bench::MultiValueProperties(&prepare_writer("MultiValueProperties"))]
#[bench::NamedProperties(&prepare_writer("NamedProperties"))]
#[bench::Recipients(&prepare_writer("Recipients"))]
#[bench::PropertyStream(&prepare_writer("PropertyStream"))]
#[bench::PropertyStreamCursor(&prepare_writer("PropertyStreamCursor"))]
#[bench::PropertyStreamShortChunks(&prepare_writer("PropertyStreamShortChunks"))]
#[bench::PropertyStreamShortWrites(&prepare_writer("PropertyStreamShortWrites"))]
#[bench::MultiValueStreams(&prepare_writer("MultiValueStreams"))]
#[bench::ObjectStream(&prepare_writer("ObjectStream"))]
#[bench::RtfProperty(&prepare_writer("RtfProperty"))]
#[bench::RtfPropertyUncompressed(&prepare_writer("RtfPropertyUncompressed"))]
#[bench::RtfPropertyCursor(&prepare_writer("RtfPropertyCursor"))]
#[bench::RtfPropertyShortChunks(&prepare_writer("RtfPropertyShortChunks"))]
#[bench::RtfPropertyShortWrites(&prepare_writer("RtfPropertyShortWrites"))]
#[bench::RtfMultiValues(&prepare_writer("RtfMultiValues"))]
#[bench::EmbeddedMessage(&prepare_writer("EmbeddedMessage"))]
#[bench::EmbeddedCursor(&prepare_writer("EmbeddedCursor"))]
#[bench::EmbeddedShortWrites(&prepare_writer("EmbeddedShortWrites"))]
#[bench::EmbeddedNested(&prepare_writer("EmbeddedNested"))]
#[bench::EmbeddedCodepage(&prepare_writer("EmbeddedCodepage"))]
#[bench::EmbeddedMany(&prepare_writer("EmbeddedMany"))]
#[bench::PropertySysTime(&prepare_writer("PropertySysTime"))]
#[bench::PropertyAppTime(&prepare_writer("PropertyAppTime"))]
#[bench::PropertyCurrency(&prepare_writer("PropertyCurrency"))]
#[bench::PropertyCurrencyRounded(&prepare_writer("PropertyCurrencyRounded"))]
fn write(case: &writer_workloads::Workload) -> writer_workloads::Summary {
    writer_workloads::write(case)
}

fn prepare_rtf_writer(name: &'static str) -> rtf_writer_workloads::Workload {
    let case = rtf_writer_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(RTF_WRITER, "rtf-compressed", "Encode")]
#[bench::Repetitive(&prepare_rtf_writer("Repetitive"))]
#[bench::Random(&prepare_rtf_writer("Random"))]
#[bench::Overlapping(&prepare_rtf_writer("Overlapping"))]
#[bench::Uncompressed(&prepare_rtf_writer("Uncompressed"))]
#[bench::ShortChunks(&prepare_rtf_writer("ShortChunks"))]
#[bench::ShortWrites(&prepare_rtf_writer("ShortWrites"))]
fn rtf_write(case: &rtf_writer_workloads::Workload) -> rtf_writer_workloads::Summary {
    rtf_writer_workloads::write(case)
}

#[path = "support/tnef_bodies.rs"]
mod body_workloads;

#[path = "support/tnef_attachments.rs"]
mod attachment_workloads;

#[path = "support/tnef_messages.rs"]
mod message_workloads;

fn prepare_message(name: &'static str) -> message_workloads::Workload {
    let case = message_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(MESSAGE_LOAD, "tnef-messages", "Load")]
#[bench::Legacy(&prepare_message("Legacy"))]
#[bench::Mapi(&prepare_message("Mapi"))]
#[bench::Mixed(&prepare_message("Mixed"))]
#[bench::Embedded(&prepare_message("Embedded"))]
#[bench::ShortReads(&prepare_message("ShortReads"))]
fn message_load(case: &message_workloads::Workload) -> message_workloads::Summary {
    message_workloads::read(case, message_workloads::Mode::Load)
}

#[metabench::benchmark(MESSAGE_TRAVERSE, "tnef-messages", "Traverse")]
#[bench::Legacy(&prepare_message("Legacy"))]
#[bench::Mapi(&prepare_message("Mapi"))]
#[bench::Mixed(&prepare_message("Mixed"))]
#[bench::Embedded(&prepare_message("Embedded"))]
#[bench::ShortReads(&prepare_message("ShortReads"))]
fn message_traverse(case: &message_workloads::Workload) -> message_workloads::Summary {
    message_workloads::read(case, message_workloads::Mode::Traverse)
}

fn prepare_attachment(name: &'static str) -> attachment_workloads::Workload {
    let case = attachment_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(ATTACHMENT_LOAD, "tnef-attachments", "Load")]
#[bench::Direct(&prepare_attachment("Direct"))]
#[bench::BinaryText(&prepare_attachment("BinaryText"))]
#[bench::Embedded(&prepare_attachment("Embedded"))]
#[bench::Empty(&prepare_attachment("Empty"))]
fn attachment_load(case: &attachment_workloads::Workload) -> attachment_workloads::Summary {
    attachment_workloads::read(case, attachment_workloads::Mode::Load)
}

#[metabench::benchmark(ATTACHMENT_ACCESS, "tnef-attachments", "Access")]
#[bench::Direct(&prepare_attachment("Direct"))]
#[bench::BinaryText(&prepare_attachment("BinaryText"))]
#[bench::Embedded(&prepare_attachment("Embedded"))]
#[bench::Empty(&prepare_attachment("Empty"))]
fn attachment_access(case: &attachment_workloads::Workload) -> attachment_workloads::Summary {
    attachment_workloads::read(case, attachment_workloads::Mode::Access)
}

#[metabench::benchmark(ATTACHMENT_STREAM, "tnef-attachments", "Stream")]
#[bench::Direct(&prepare_attachment("Direct"))]
#[bench::BinaryText(&prepare_attachment("BinaryText"))]
#[bench::Embedded(&prepare_attachment("Embedded"))]
#[bench::Empty(&prepare_attachment("Empty"))]
fn attachment_stream(case: &attachment_workloads::Workload) -> attachment_workloads::Summary {
    attachment_workloads::read(case, attachment_workloads::Mode::Stream)
}

fn prepare_body(name: &'static str) -> body_workloads::Workload {
    let case = body_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(BODY_CONSTRUCT, "tnef-bodies", "Construct")]
#[bench::String8(&prepare_body("String8"))]
#[bench::Unicode(&prepare_body("Unicode"))]
#[bench::Html(&prepare_body("Html"))]
#[bench::CompressedRtf(&prepare_body("CompressedRtf"))]
#[bench::UncompressedRtf(&prepare_body("UncompressedRtf"))]
fn body_construct(case: &body_workloads::Workload) -> body_workloads::Summary {
    body_workloads::read(case, body_workloads::Mode::Construct)
}

#[metabench::benchmark(BODY_STREAM, "tnef-bodies", "Stream")]
#[bench::String8(&prepare_body("String8"))]
#[bench::Unicode(&prepare_body("Unicode"))]
#[bench::Html(&prepare_body("Html"))]
#[bench::CompressedRtf(&prepare_body("CompressedRtf"))]
#[bench::UncompressedRtf(&prepare_body("UncompressedRtf"))]
fn body_stream(case: &body_workloads::Workload) -> body_workloads::Summary {
    body_workloads::read(case, body_workloads::Mode::Stream)
}

#[metabench::benchmark(BODY_TEXT, "tnef-bodies", "Text")]
#[bench::String8(&prepare_body("String8"))]
#[bench::Unicode(&prepare_body("Unicode"))]
#[bench::Html(&prepare_body("Html"))]
#[bench::CompressedRtf(&prepare_body("CompressedRtf"))]
#[bench::UncompressedRtf(&prepare_body("UncompressedRtf"))]
fn body_text(case: &body_workloads::Workload) -> body_workloads::Summary {
    body_workloads::read(case, body_workloads::Mode::Text)
}

fn prepare_rtf_compressed(name: &'static str) -> rtf_compressed_workloads::Workload {
    let case = rtf_compressed_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(RTF_COMPRESSED, "rtf-compressed", "Decode")]
#[bench::Literals(&prepare_rtf_compressed("Literals"))]
#[bench::References(&prepare_rtf_compressed("References"))]
#[bench::Uncompressed(&prepare_rtf_compressed("Uncompressed"))]
#[bench::ShortChunks(&prepare_rtf_compressed("ShortChunks"))]
fn rtf_compressed(case: &rtf_compressed_workloads::Workload) -> rtf_compressed_workloads::Summary {
    rtf_compressed_workloads::read(case)
}

fn prepare_recipients(name: &'static str) -> recipient_workloads::Workload {
    let case = recipient_workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(RECIPIENT_LOAD, "tnef-recipients", "Load")]
#[bench::Direct(&prepare_recipients("Direct"))]
#[bench::Fallbacks(&prepare_recipients("Fallbacks"))]
#[bench::BinaryText(&prepare_recipients("BinaryText"))]
fn recipient_load(case: &recipient_workloads::Workload) -> recipient_workloads::Summary {
    recipient_workloads::read(case, recipient_workloads::Mode::Load)
}

#[metabench::benchmark(RECIPIENT_ACCESS, "tnef-recipients", "Access")]
#[bench::Direct(&prepare_recipients("Direct"))]
#[bench::Fallbacks(&prepare_recipients("Fallbacks"))]
#[bench::BinaryText(&prepare_recipients("BinaryText"))]
fn recipient_access(case: &recipient_workloads::Workload) -> recipient_workloads::Summary {
    recipient_workloads::read(case, recipient_workloads::Mode::Access)
}

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

#[metabench::benchmark(DETACHED, "tnef-embedded", "Detached")]
#[bench::Single(&prepare_embedded("Single"))]
#[bench::Deep(&prepare_embedded("Deep"))]
#[bench::Siblings(&prepare_embedded("Siblings"))]
#[bench::ShortReads(&prepare_embedded("ShortReads"))]
fn detached(case: &embedded_workloads::Workload) -> embedded_workloads::Summary {
    embedded_workloads::read_detached(case)
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
    let mut group = criterion.benchmark_group("tnef-calendar");
    for name in calendar_workloads::CASES {
        let case = prepare_calendar(name);
        group.throughput(Throughput::Bytes(case.output_length as u64));
        group.bench_function(BenchmarkId::new("Write", name), |b| {
            b.iter(|| calendar_workloads::write(&case));
        });
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-writer");
    for name in writer_workloads::CASES {
        let case = prepare_writer(name);
        group.throughput(Throughput::Bytes(case.output_length as u64));
        group.bench_function(BenchmarkId::new("Write", name), |b| {
            b.iter(|| writer_workloads::write(&case));
        });
    }
    group.finish();
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
        group.bench_function(BenchmarkId::new("Detached", name), |b| {
            b.iter(|| embedded_workloads::read_detached(&case));
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
    let mut group = criterion.benchmark_group("tnef-recipients");
    for name in recipient_workloads::CASES {
        let case = prepare_recipients(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in recipient_workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| recipient_workloads::read(&case, mode));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group("rtf-compressed");
    for name in rtf_compressed_workloads::CASES {
        let case = prepare_rtf_compressed(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(BenchmarkId::new("Decode", name), |b| {
            b.iter(|| rtf_compressed_workloads::read(&case));
        });
    }
    for name in rtf_writer_workloads::CASES {
        let case = prepare_rtf_writer(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(BenchmarkId::new("Encode", name), |b| {
            b.iter(|| rtf_writer_workloads::write(&case));
        });
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-bodies");
    for name in body_workloads::CASES {
        let case = prepare_body(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in body_workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| body_workloads::read(&case, mode));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-attachments");
    for name in attachment_workloads::CASES {
        let case = prepare_attachment(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in attachment_workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| attachment_workloads::read(&case, mode));
            });
        }
    }
    group.finish();
    let mut group = criterion.benchmark_group("tnef-messages");
    for name in message_workloads::CASES {
        let case = prepare_message(name);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        for mode in message_workloads::MODES {
            group.bench_function(BenchmarkId::new(mode.name(), name), |b| {
                b.iter(|| message_workloads::read(&case, mode));
            });
        }
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
        DETACHED,
        CONVERSIONS,
        RECIPIENT_LOAD,
        RECIPIENT_ACCESS,
        RTF_COMPRESSED,
        RTF_WRITER,
        WRITER,
        CALENDAR,
        BODY_CONSTRUCT,
        BODY_STREAM,
        BODY_TEXT,
        ATTACHMENT_LOAD,
        ATTACHMENT_ACCESS,
        ATTACHMENT_STREAM,
        MESSAGE_LOAD,
        MESSAGE_TRAVERSE
    ]
);

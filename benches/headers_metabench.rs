// Adapted from MimeKit's Benchmarks/AddressParserBenchmarks.cs and
// Benchmarks/Utils/Rfc2047Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(
    non_snake_case,
    reason = "Native case IDs preserve MimeKit's workload names."
)]

use criterion::{BenchmarkId, Criterion, Throughput};
use mimecraft::{AddressList, rfc2047::DecodedHeader};
use workloads::Workload;

#[path = "support/fingerprint.rs"]
mod fingerprint;
#[path = "support/headers.rs"]
mod workloads;

fn prepare(name: &'static str) -> Workload {
    let case = workloads::workload(name);
    case.check();
    case
}

#[metabench::benchmark(BYTES, "headers", "AddressParser_ParseBytes")]
#[bench::Single(&prepare("Single"))]
#[bench::Bare(&prepare("Bare"))]
#[bench::Recipients(&prepare("Recipients"))]
#[bench::Utf8(&prepare("Utf8"))]
#[bench::Encoded(&prepare("Encoded"))]
#[bench::MixedEncoded(&prepare("MixedEncoded"))]
#[bench::Groups(&prepare("Groups"))]
#[bench::Broken(&prepare("Broken"))]
#[bench::LongAtoms(&prepare("LongAtoms"))]
#[bench::Quoted(&prepare("Quoted"))]
#[bench::Comment(&prepare("Comment"))]
#[bench::UnicodeAddr(&prepare("UnicodeAddr"))]
#[bench::QuotedLocal(&prepare("QuotedLocal"))]
#[bench::CFWSDomain(&prepare("CFWSDomain"))]
#[bench::TrailingDot(&prepare("TrailingDot"))]
#[bench::Utf8Cjk(&prepare("Utf8Cjk"))]
#[bench::UnknownCharset(&prepare("UnknownCharset"))]
#[bench::QuotedUtf8(&prepare("QuotedUtf8"))]
#[bench::QuotedEncoded(&prepare("QuotedEncoded"))]
#[bench::Utf8Emoji(&prepare("Utf8Emoji"))]
#[bench::EncodedCjk(&prepare("EncodedCjk"))]
#[bench::InvalidEncoded(&prepare("InvalidEncoded"))]
#[bench::LongName(&prepare("LongName"))]
#[bench::LongUtf8(&prepare("LongUtf8"))]
#[bench::LongEquals(&prepare("LongEquals"))]
#[bench::CommentNames(&prepare("CommentNames"))]
#[bench::CommentTabs(&prepare("CommentTabs"))]
#[bench::CommentUtf8(&prepare("CommentUtf8"))]
#[bench::CommentEncoded(&prepare("CommentEncoded"))]
#[bench::LongComment(&prepare("LongComment"))]
#[bench::LongCommentUtf8(&prepare("LongCommentUtf8"))]
#[bench::CommentUtf8Tabs(&prepare("CommentUtf8Tabs"))]
#[bench::CommentEncodedTabs(&prepare("CommentEncodedTabs"))]
#[bench::CommentUnknownCharsetTabs(&prepare("CommentUnknownCharsetTabs"))]
#[bench::LongCommentUtf8Tabs(&prepare("LongCommentUtf8Tabs"))]
#[bench::LongCommentEqualsTabs(&prepare("LongCommentEqualsTabs"))]
#[bench::EscapedName(&prepare("EscapedName"))]
#[bench::FoldedName(&prepare("FoldedName"))]
#[bench::LongQuotedName(&prepare("LongQuotedName"))]
#[bench::LongQuotedUtf8(&prepare("LongQuotedUtf8"))]
#[bench::LongQuotedEquals(&prepare("LongQuotedEquals"))]
fn parse_bytes(case: &Workload) -> AddressList {
    workloads::parse_bytes(case)
}

#[metabench::benchmark(STRING, "headers", "AddressParser_ParseString")]
#[bench::Single(&prepare("Single"))]
#[bench::Bare(&prepare("Bare"))]
#[bench::Recipients(&prepare("Recipients"))]
#[bench::Utf8(&prepare("Utf8"))]
#[bench::Encoded(&prepare("Encoded"))]
#[bench::MixedEncoded(&prepare("MixedEncoded"))]
#[bench::Groups(&prepare("Groups"))]
#[bench::Broken(&prepare("Broken"))]
#[bench::LongAtoms(&prepare("LongAtoms"))]
#[bench::Quoted(&prepare("Quoted"))]
#[bench::Comment(&prepare("Comment"))]
#[bench::UnicodeAddr(&prepare("UnicodeAddr"))]
#[bench::QuotedLocal(&prepare("QuotedLocal"))]
#[bench::CFWSDomain(&prepare("CFWSDomain"))]
#[bench::TrailingDot(&prepare("TrailingDot"))]
#[bench::Utf8Cjk(&prepare("Utf8Cjk"))]
#[bench::UnknownCharset(&prepare("UnknownCharset"))]
#[bench::QuotedUtf8(&prepare("QuotedUtf8"))]
#[bench::QuotedEncoded(&prepare("QuotedEncoded"))]
#[bench::Utf8Emoji(&prepare("Utf8Emoji"))]
#[bench::EncodedCjk(&prepare("EncodedCjk"))]
#[bench::InvalidEncoded(&prepare("InvalidEncoded"))]
#[bench::LongName(&prepare("LongName"))]
#[bench::LongUtf8(&prepare("LongUtf8"))]
#[bench::LongEquals(&prepare("LongEquals"))]
#[bench::CommentNames(&prepare("CommentNames"))]
#[bench::CommentTabs(&prepare("CommentTabs"))]
#[bench::CommentUtf8(&prepare("CommentUtf8"))]
#[bench::CommentEncoded(&prepare("CommentEncoded"))]
#[bench::LongComment(&prepare("LongComment"))]
#[bench::LongCommentUtf8(&prepare("LongCommentUtf8"))]
#[bench::CommentUtf8Tabs(&prepare("CommentUtf8Tabs"))]
#[bench::CommentEncodedTabs(&prepare("CommentEncodedTabs"))]
#[bench::CommentUnknownCharsetTabs(&prepare("CommentUnknownCharsetTabs"))]
#[bench::LongCommentUtf8Tabs(&prepare("LongCommentUtf8Tabs"))]
#[bench::LongCommentEqualsTabs(&prepare("LongCommentEqualsTabs"))]
#[bench::EscapedName(&prepare("EscapedName"))]
#[bench::FoldedName(&prepare("FoldedName"))]
#[bench::LongQuotedName(&prepare("LongQuotedName"))]
#[bench::LongQuotedUtf8(&prepare("LongQuotedUtf8"))]
#[bench::LongQuotedEquals(&prepare("LongQuotedEquals"))]
fn parse_string(case: &Workload) -> AddressList {
    workloads::parse_string(case)
}

#[metabench::benchmark(TEXT, "headers", "Rfc2047_DecodeText")]
#[bench::ILoveMimeSoMuch(&workloads::check_rfc2047())]
fn decode_text(_: &()) -> DecodedHeader {
    workloads::decode_text()
}

#[metabench::benchmark(PHRASE, "headers", "Rfc2047_DecodePhrase")]
#[bench::ILoveMimeSoMuch(&workloads::check_rfc2047())]
fn decode_phrase(_: &()) -> DecodedHeader {
    workloads::decode_phrase()
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(BYTES.group_name());
    for name in workloads::CASES {
        let case = prepare(name);
        group.throughput(Throughput::Bytes(case.text.len() as u64));
        group.bench_function(BenchmarkId::new(BYTES.benchmark_name(), name), |b| {
            b.iter(|| parse_bytes(&case))
        });
        group.bench_function(BenchmarkId::new(STRING.benchmark_name(), name), |b| {
            b.iter(|| parse_string(&case))
        });
    }
    workloads::check_rfc2047();
    group.throughput(Throughput::Bytes(workloads::RFC2047_INPUT.len() as u64));
    group.bench_function(
        BenchmarkId::new(TEXT.benchmark_name(), "ILoveMimeSoMuch"),
        |b| b.iter(|| decode_text(&())),
    );
    group.bench_function(
        BenchmarkId::new(PHRASE.benchmark_name(), "ILoveMimeSoMuch"),
        |b| b.iter(|| decode_phrase(&())),
    );
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [BYTES, STRING, TEXT, PHRASE]
);

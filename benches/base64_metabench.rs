// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use criterion::{BenchmarkId, Criterion, Throughput};

#[path = "support/base64.rs"]
mod workloads;

struct Case {
    input: Vec<u8>,
    output: Vec<u8>,
    chunk_size: usize,
}

fn encoding(chunk_size: usize) -> Case {
    let fixture = workloads::Fixture::new();
    let mut case = Case {
        input: fixture.input,
        output: Vec::with_capacity(fixture.encoded.len()),
        chunk_size,
    };
    workloads::encode(&case.input, chunk_size, &mut case.output);
    assert_eq!(case.output, fixture.encoded);
    case
}

fn decoding(name: &str, chunk_size: usize, matched: bool) -> Case {
    let fixture = workloads::Fixture::new();
    let mut case = Case {
        input: fixture.decode_input(name).to_vec(),
        output: Vec::with_capacity(fixture.input.len()),
        chunk_size,
    };
    if matched {
        workloads::decode_match(&case.input, chunk_size, &mut case.output);
    } else {
        workloads::decode(&case.input, chunk_size, &mut case.output);
    }
    assert_eq!(case.output, fixture.input);
    case
}

#[metabench::benchmark(ENCODE, "base64", "encode")]
#[bench::chunk_1(&mut encoding(1))]
#[bench::chunk_57(&mut encoding(57))]
#[bench::chunk_4096(&mut encoding(4096))]
fn encode(case: &mut Case) {
    workloads::encode(&case.input, case.chunk_size, &mut case.output);
}

#[metabench::benchmark(DECODE, "base64", "decode")]
#[bench::unwrapped_chunk_1(&mut decoding("unwrapped", 1, false))]
#[bench::unwrapped_chunk_78(&mut decoding("unwrapped", 78, false))]
#[bench::unwrapped_chunk_4096(&mut decoding("unwrapped", 4096, false))]
#[bench::mime_crlf_chunk_1(&mut decoding("mime-crlf", 1, false))]
#[bench::mime_crlf_chunk_78(&mut decoding("mime-crlf", 78, false))]
#[bench::mime_crlf_chunk_4096(&mut decoding("mime-crlf", 4096, false))]
#[bench::noisy_chunk_1(&mut decoding("noisy", 1, false))]
#[bench::noisy_chunk_78(&mut decoding("noisy", 78, false))]
#[bench::noisy_chunk_4096(&mut decoding("noisy", 4096, false))]
fn decode(case: &mut Case) {
    workloads::decode(&case.input, case.chunk_size, &mut case.output);
}

#[metabench::benchmark(MATCH, "base64", "match")]
#[bench::unwrapped_chunk_1(&mut decoding("unwrapped", 1, true))]
#[bench::unwrapped_chunk_78(&mut decoding("unwrapped", 78, true))]
#[bench::unwrapped_chunk_4096(&mut decoding("unwrapped", 4096, true))]
#[bench::mime_crlf_chunk_1(&mut decoding("mime-crlf", 1, true))]
#[bench::mime_crlf_chunk_78(&mut decoding("mime-crlf", 78, true))]
#[bench::mime_crlf_chunk_4096(&mut decoding("mime-crlf", 4096, true))]
#[bench::noisy_chunk_1(&mut decoding("noisy", 1, true))]
#[bench::noisy_chunk_78(&mut decoding("noisy", 78, true))]
#[bench::noisy_chunk_4096(&mut decoding("noisy", 4096, true))]
fn decode_match(case: &mut Case) {
    workloads::decode_match(&case.input, case.chunk_size, &mut case.output);
}

fn criterion_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group(ENCODE.group_name());
    for chunk_size in workloads::ENCODE_CHUNKS {
        let mut case = encoding(chunk_size);
        group.throughput(Throughput::Bytes(case.input.len() as u64));
        group.bench_function(
            BenchmarkId::new(ENCODE.benchmark_name(), format!("chunk_{chunk_size}")),
            |b| b.iter(|| encode(&mut case)),
        );
    }
    for name in workloads::DECODE_INPUTS {
        for chunk_size in workloads::DECODE_CHUNKS {
            let id = format!("{}_chunk_{chunk_size}", name.replace('-', "_"));
            let mut case = decoding(name, chunk_size, false);
            group.throughput(Throughput::Bytes(case.input.len() as u64));
            group.bench_function(BenchmarkId::new(DECODE.benchmark_name(), &id), |b| {
                b.iter(|| decode(&mut case));
            });
            let mut case = decoding(name, chunk_size, true);
            group.bench_function(BenchmarkId::new(MATCH.benchmark_name(), &id), |b| {
                b.iter(|| decode_match(&mut case));
            });
        }
    }
    group.finish();
}

metabench::main!(
    criterion = criterion_benchmarks,
    benchmarks = [ENCODE, DECODE, MATCH]
);

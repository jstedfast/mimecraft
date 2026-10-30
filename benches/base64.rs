// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::time::{Duration, Instant};

#[path = "support/base64.rs"]
mod workloads;

fn measure(name: &str, bytes: usize, mut operation: impl FnMut()) {
    for _ in 0..3 {
        operation();
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            operation();
            iterations += 1;
        }
        *sample = bytes as f64 * iterations as f64 / start.elapsed().as_secs_f64() / 1048576.0;
    }
    samples.sort_by(f64::total_cmp);
    println!("{name:42} {:9.1} MiB/s (median of 5)", samples[2]);
}

fn main() {
    println!(
        "target: {}-{}, SIMD feature: {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(feature = "simd")
    );
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    println!(
        "CPU capabilities: SSSE3={}, AVX2={}",
        is_x86_feature_detected!("ssse3"),
        is_x86_feature_detected!("avx2")
    );
    #[cfg(target_arch = "aarch64")]
    println!(
        "CPU capabilities: NEON={}",
        std::arch::is_aarch64_feature_detected!("neon")
    );
    let fixture = workloads::Fixture::new();

    println!("Throughput counts input bytes; output buffers are reused.");
    for chunk_size in workloads::ENCODE_CHUNKS {
        let mut output = Vec::with_capacity(fixture.encoded.len());
        measure(
            &format!("encode/chunk={chunk_size}"),
            fixture.input.len(),
            || {
                workloads::encode(&fixture.input, chunk_size, &mut output);
            },
        );
        assert_eq!(output, fixture.encoded);
    }

    for name in workloads::DECODE_INPUTS {
        let data = fixture.decode_input(name);
        for chunk_size in workloads::DECODE_CHUNKS {
            let mut output = Vec::with_capacity(fixture.input.len());
            measure(
                &format!("decode/{name}/chunk={chunk_size}"),
                data.len(),
                || {
                    workloads::decode(data, chunk_size, &mut output);
                },
            );
            assert_eq!(output, fixture.input);
            measure(
                &format!("match/{name}/chunk={chunk_size}"),
                data.len(),
                || {
                    workloads::decode_match(data, chunk_size, &mut output);
                },
            );
            assert_eq!(output, fixture.input);
        }
    }

    for name in workloads::STREAMING_INPUTS {
        let mut case = workloads::StreamingCase::new(name);
        assert_eq!(case.decode(), case.decoded_length);
        measure(
            &format!("stream/{name}/chunk=4096"),
            case.input.len(),
            || {
                case.decode();
            },
        );
        assert_eq!(case.decode(), case.decoded_length);
    }
}

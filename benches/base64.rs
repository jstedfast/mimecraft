// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use mimecraft::encodings::{Base64Decoder, Base64Encoder};

// Preserve the original match-based decoder as a benchmark baseline, not a
// second production implementation. Keep its streaming state machine unchanged.
#[derive(Default)]
struct MatchDecoder {
    value: u32,
    padding: u8,
    count: u8,
}

impl MatchDecoder {
    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            let rank = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => 0,
                _ => continue,
            };
            self.value = (self.value << 6) | u32::from(rank);
            self.padding = (self.padding << 1) | u8::from(byte == b'=');
            self.count += 1;
            if self.count == 4 {
                if self.padding & 4 == 0 {
                    output.push((self.value >> 16) as u8);
                    if self.padding & 2 == 0 {
                        output.push((self.value >> 8) as u8);
                        if self.padding & 1 == 0 {
                            output.push(self.value as u8);
                        }
                    }
                }
                *self = Self::default();
            }
        }
    }
}

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
    // Deterministic, non-repeating alphabet distribution rather than an all-zero
    // payload that could favor the match decoder's first branch.
    let mut random = 0x1234_5678u32;
    let input: Vec<u8> = (0..1024 * 1024)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random as u8
        })
        .collect();
    let mut encoded = Vec::new();
    let mut encoder = Base64Encoder::new();
    encoder.encode(&input, &mut encoded);
    encoder.finish(&mut encoded);
    let unwrapped: Vec<u8> = encoded
        .iter()
        .copied()
        .filter(|&b| !matches!(b, b'\r' | b'\n'))
        .collect();
    assert!(
        unwrapped
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(b))
    );
    let noisy: Vec<u8> = unwrapped
        .chunks(37)
        .flat_map(|chunk| chunk.iter().copied().chain([b' ', b'\t', b'!', 0xff]))
        .collect();

    println!("Throughput counts input bytes; output buffers are reused.");
    for chunk_size in [1, 57, 4096] {
        let mut output = Vec::with_capacity(encoded.len());
        measure(&format!("encode/chunk={chunk_size}"), input.len(), || {
            output.clear();
            let mut encoder = Base64Encoder::new();
            for chunk in black_box(&input).chunks(chunk_size) {
                encoder.encode(chunk, &mut output);
            }
            encoder.finish(&mut output);
            black_box(&output);
        });
        assert_eq!(output, encoded);
    }

    for (name, data) in [
        ("unwrapped", &unwrapped),
        ("mime-crlf", &encoded),
        ("noisy", &noisy),
    ] {
        for chunk_size in [1, 78, 4096] {
            let mut output = Vec::with_capacity(input.len());
            measure(
                &format!("decode/{name}/chunk={chunk_size}"),
                data.len(),
                || {
                    output.clear();
                    let mut decoder = Base64Decoder::new();
                    for chunk in black_box(data).chunks(chunk_size) {
                        decoder.decode(chunk, &mut output);
                    }
                    black_box(&output);
                },
            );
            assert_eq!(output, input);
            measure(
                &format!("match/{name}/chunk={chunk_size}"),
                data.len(),
                || {
                    output.clear();
                    let mut decoder = MatchDecoder::default();
                    for chunk in black_box(data).chunks(chunk_size) {
                        decoder.decode(chunk, &mut output);
                    }
                    black_box(&output);
                },
            );
            assert_eq!(output, input);
        }
    }
}

// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::encodings::{Base64Decoder, Base64Encoder};

pub const ENCODE_CHUNKS: [usize; 3] = [1, 57, 4096];
pub const DECODE_CHUNKS: [usize; 3] = [1, 78, 4096];
pub const DECODE_INPUTS: [&str; 3] = ["unwrapped", "mime-crlf", "noisy"];

pub struct Fixture {
    pub input: Vec<u8>,
    pub encoded: Vec<u8>,
    unwrapped: Vec<u8>,
    noisy: Vec<u8>,
}

impl Fixture {
    pub fn new() -> Self {
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
        let noisy = unwrapped
            .chunks(37)
            .flat_map(|chunk| chunk.iter().copied().chain([b' ', b'\t', b'!', 0xff]))
            .collect();
        Self {
            input,
            encoded,
            unwrapped,
            noisy,
        }
    }

    pub fn decode_input(&self, name: &str) -> &[u8] {
        match name {
            "unwrapped" => &self.unwrapped,
            "mime-crlf" => &self.encoded,
            "noisy" => &self.noisy,
            _ => panic!("unknown Base64 benchmark input: {name}"),
        }
    }
}

pub fn encode(input: &[u8], chunk_size: usize, output: &mut Vec<u8>) {
    output.clear();
    let mut encoder = Base64Encoder::new();
    for chunk in black_box(input).chunks(chunk_size) {
        encoder.encode(chunk, output);
    }
    encoder.finish(output);
    black_box(output);
}

pub fn decode(input: &[u8], chunk_size: usize, output: &mut Vec<u8>) {
    output.clear();
    let mut decoder = Base64Decoder::new();
    for chunk in black_box(input).chunks(chunk_size) {
        decoder.decode(chunk, output);
    }
    black_box(output);
}

pub fn decode_match(input: &[u8], chunk_size: usize, output: &mut Vec<u8>) {
    output.clear();
    let mut decoder = MatchDecoder::default();
    for chunk in black_box(input).chunks(chunk_size) {
        decoder.decode(chunk, output);
    }
    black_box(output);
}

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

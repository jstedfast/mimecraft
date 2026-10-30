// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::encodings::{Base64Decoder, Base64Encoder};

pub const ENCODE_CHUNKS: [usize; 3] = [1, 57, 4096];
pub const DECODE_CHUNKS: [usize; 3] = [1, 78, 4096];
pub const DECODE_INPUTS: [&str; 3] = ["unwrapped", "mime-crlf", "noisy"];
pub const STREAMING_INPUTS: [&str; 6] = [
    "unwrapped",
    "mime-crlf",
    "mime-lf",
    "short-lines",
    "garbage",
    "small",
];

fn random_bytes(length: usize) -> Vec<u8> {
    let mut random = 0x1234_5678u32;
    (0..length)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            random as u8
        })
        .collect()
}

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
        let input = random_bytes(1024 * 1024);
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

// Adapted from MimeKit's Base64DecoderBenchmarks. Preserve its input sizes,
// layouts and 4096-byte chunks, using our deterministic PRNG instead of .NET's.
pub struct StreamingCase {
    pub input: Vec<u8>,
    output: Vec<u8>,
    decoder: Base64Decoder,
    pub decoded_length: usize,
}

impl StreamingCase {
    pub fn new(name: &str) -> Self {
        let data = random_bytes(if name == "small" { 48 } else { 65536 });
        let mut encoded = Vec::new();
        let mut encoder = Base64Encoder::new();
        encoder.encode(&data, &mut encoded);
        encoder.finish(&mut encoded);
        encoded.retain(|&byte| !matches!(byte, b'\r' | b'\n'));
        let (width, separator): (usize, &[u8]) = match name {
            "mime-crlf" => (76, b"\r\n"),
            "mime-lf" => (76, b"\n"),
            "short-lines" => (7, b" \t\r\n"),
            "garbage" => (23, b"!? \t\r\n"),
            "unwrapped" | "small" => (encoded.len(), b""),
            _ => panic!("unknown Base64 streaming benchmark input: {name}"),
        };
        let mut input = Vec::new();
        for line in encoded.chunks(width) {
            input.extend_from_slice(line);
            input.extend_from_slice(separator);
        }
        let mut case = Self {
            input,
            output: Vec::with_capacity(4096),
            decoder: Base64Decoder::new(),
            decoded_length: data.len(),
        };
        let mut decoded = 0;
        for chunk in case.input.chunks(4096) {
            case.output.clear();
            case.decoder.decode(chunk, &mut case.output);
            assert_eq!(case.output, data[decoded..decoded + case.output.len()]);
            decoded += case.output.len();
        }
        assert_eq!(decoded, case.decoded_length);
        case
    }

    pub fn decode(&mut self) -> usize {
        self.decoder.reset();
        let mut decoded = 0;
        for chunk in black_box(&self.input).chunks(4096) {
            self.output.clear();
            self.decoder.decode(chunk, &mut self.output);
            decoded += self.output.len();
            black_box(&self.output);
        }
        black_box(decoded)
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

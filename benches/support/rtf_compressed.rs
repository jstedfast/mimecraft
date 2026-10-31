// Reference builder adapted from MimeKit's UnitTests/Tnef/RtfCompressedBuilder.cs.
// Original workloads: MimeKit has no compressed-RTF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::tnef::{RtfCompressedToRtf, RtfCompressionMode};

const INITIALIZER: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";

enum Token {
    Literal(u8),
    Reference(usize, usize),
}

/// A deliberately simple reference encoder: callers choose tokens, not a match
/// finder. Keeping the expected bytes separately lets tests synthesize valid
/// and malformed streams without using the production decompressor as oracle.
pub struct Builder {
    tokens: Vec<Token>,
    dictionary: [u8; 4096],
    pub write_offset: usize,
    pub expected: Vec<u8>,
}

impl Builder {
    pub fn new() -> Self {
        let mut dictionary = [0; 4096];
        assert_eq!(INITIALIZER.len(), 207);
        dictionary[..207].copy_from_slice(INITIALIZER);
        Self {
            tokens: Vec::new(),
            dictionary,
            write_offset: 207,
            expected: Vec::new(),
        }
    }

    fn append(&mut self, byte: u8) {
        self.expected.push(byte);
        self.dictionary[self.write_offset] = byte;
        self.write_offset = (self.write_offset + 1) % 4096;
    }

    pub fn literal(&mut self, byte: u8) {
        self.tokens.push(Token::Literal(byte));
        self.append(byte);
    }

    pub fn literals(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.literal(byte);
        }
    }

    pub fn reference(&mut self, offset: usize, length: usize) {
        assert!(offset < 4096 && offset != self.write_offset);
        assert!((2..=17).contains(&length));
        self.tokens.push(Token::Reference(offset, length));
        // References can overlap bytes they produce; each copied byte goes
        // back into the dictionary before the next byte is read.
        for index in 0..length {
            self.append(self.dictionary[(offset + index) % 4096]);
        }
    }

    pub fn end(&mut self) {
        self.tokens.push(Token::Reference(self.write_offset, 2));
    }

    pub fn build(&self) -> Vec<u8> {
        let mut contents = Vec::new();
        for run in self.tokens.chunks(8) {
            let flags = run.iter().enumerate().fold(0u8, |flags, (index, token)| {
                flags
                    | if matches!(token, Token::Reference(..)) {
                        1 << index
                    } else {
                        0
                    }
            });
            contents.push(flags);
            for token in run {
                match *token {
                    Token::Literal(byte) => contents.push(byte),
                    Token::Reference(offset, length) => {
                        contents.push((offset >> 4) as u8);
                        contents.push((((offset & 15) << 4) | (length - 2)) as u8);
                    }
                }
            }
        }
        document(
            &contents,
            self.expected.len() as i32,
            RtfCompressionMode::Compressed,
            crc(&contents),
        )
    }
}

/// Independent bitwise CRC, initialized to zero with no final complement.
pub fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 0 { 0 } else { 0xedb88320 };
        }
    }
    crc
}

pub fn document(contents: &[u8], raw_size: i32, mode: RtfCompressionMode, crc: u32) -> Vec<u8> {
    let mut input = ((contents.len() + 12) as i32).to_le_bytes().to_vec();
    input.extend_from_slice(&raw_size.to_le_bytes());
    input.extend_from_slice(&u32::from(mode).to_le_bytes());
    input.extend_from_slice(&crc.to_le_bytes());
    input.extend_from_slice(contents);
    input
}

pub const CASES: [&str; 4] = ["Literals", "References", "Uncompressed", "ShortChunks"];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    expected: Vec<u8>,
    chunk: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub checksum: u64,
    pub valid: bool,
}

fn accumulate(summary: &mut Summary, output: &[u8]) {
    summary.bytes += output.len();
    summary.checksum = output.iter().fold(summary.checksum, |sum, &byte| {
        sum.wrapping_add(u64::from(byte))
    });
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let (input, expected) = if name == "Uncompressed" {
        let expected = b"{\\rtf1 uncompressed content}\r\n".repeat(2048);
        (
            document(
                &expected,
                expected.len() as i32,
                RtfCompressionMode::Uncompressed,
                0,
            ),
            expected,
        )
    } else {
        let mut builder = Builder::new();
        if name == "Literals" {
            builder.literals(&b"{\\rtf1 literal content}\r\n".repeat(2048));
        } else {
            builder.reference(0, 6);
            builder.literals(b" ab");
            for _ in 0..6000 {
                builder.reference((builder.write_offset + 4096 - 2) % 4096, 17);
            }
            builder.literal(b'}');
        }
        builder.end();
        (builder.build(), builder.expected)
    };
    Workload {
        name,
        input,
        expected,
        chunk: if name == "ShortChunks" { 1 } else { 4096 },
    }
}

pub fn read(case: &Workload) -> Summary {
    let mut filter = RtfCompressedToRtf::new();
    let mut output = Vec::with_capacity(4096);
    let mut summary = Summary::default();
    for input in black_box(&case.input).chunks(case.chunk) {
        filter.filter(input, &mut output);
        accumulate(&mut summary, black_box(&output));
        output.clear();
    }
    filter.finish(&mut output);
    accumulate(&mut summary, black_box(&output));
    filter.validate().expect("valid generated compressed RTF");
    summary.valid = filter.is_valid_crc32();
    summary
}

impl Workload {
    pub fn check(&self) {
        let mut expected = Summary::default();
        accumulate(&mut expected, &self.expected);
        expected.valid = true;
        assert_eq!(read(self), expected, "{}", self.name);
        let mut filter = RtfCompressedToRtf::new();
        let mut output = Vec::new();
        for input in self.input.chunks(self.chunk) {
            filter.filter(input, &mut output);
        }
        filter.finish(&mut output);
        filter.validate().unwrap();
        assert_eq!(output, self.expected, "{}", self.name);
    }
}

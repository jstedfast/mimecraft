// Original workloads based on MimeKit's RtfCompressedWriteStreamTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::rtf_compressed_workloads::crc;
use mimecraft::tnef::{RtfCompressedToRtf, RtfCompressedWriter, RtfCompressionMode};
use std::{
    hint::black_box,
    io::{self, Cursor, Seek, SeekFrom, Write},
};

pub const CASES: [&str; 6] = [
    "Repetitive",
    "Random",
    "Overlapping",
    "Uncompressed",
    "ShortChunks",
    "ShortWrites",
];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    mode: RtfCompressionMode,
    chunk: usize,
    sink_chunk: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub checksum: u64,
}

struct Sink {
    output: Cursor<Vec<u8>>,
    chunk: usize,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.write(&bytes[..bytes.len().min(self.chunk)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

impl Seek for Sink {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.output.seek(position)
    }
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let input = if matches!(name, "Random" | "Overlapping") {
        let mut state = 0x12345678u32;
        (0..24000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                // A small alphabet produces many short and overlapping matches.
                if name == "Overlapping" {
                    b'a' + (state % 3) as u8
                } else {
                    state as u8
                }
            })
            .collect()
    } else {
        let mut text =
            String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0\\fswiss Arial;}}\r\n");
        for line in 0..2000 {
            use std::fmt::Write;
            write!(
                text,
                "\\pard\\plain\\f0\\fs20 Line {line} of the message body.\\par\r\n"
            )
            .unwrap();
        }
        text.push('}');
        text.into_bytes()
    };
    Workload {
        name,
        input,
        mode: if name == "Uncompressed" {
            RtfCompressionMode::Uncompressed
        } else {
            RtfCompressionMode::Compressed
        },
        chunk: if name == "ShortChunks" { 1 } else { 4096 },
        sink_chunk: if name == "ShortWrites" { 7 } else { usize::MAX },
    }
}

fn encode(case: &Workload) -> Vec<u8> {
    let sink = Sink {
        output: Cursor::new(Vec::new()),
        chunk: case.sink_chunk,
    };
    let mut writer = RtfCompressedWriter::new(sink, case.mode).unwrap();
    for bytes in black_box(&case.input).chunks(case.chunk) {
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    writer.into_inner().output.into_inner()
}

fn summarize(bytes: &[u8]) -> Summary {
    Summary {
        bytes: bytes.len(),
        checksum: bytes.iter().fold(0u64, |sum, &byte| {
            sum.wrapping_mul(31).wrapping_add(u64::from(byte))
        }),
    }
}

pub fn write(case: &Workload) -> Summary {
    summarize(black_box(&encode(case)))
}

impl Workload {
    pub fn check(&self) {
        let bytes = encode(self);
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            self.input.len()
        );
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            u32::from(self.mode)
        );
        assert_eq!(
            u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            if self.mode == RtfCompressionMode::Compressed {
                crc(&bytes[16..])
            } else {
                0
            }
        );
        let mut decoder = RtfCompressedToRtf::new();
        let mut decoded = Vec::new();
        for chunk in bytes.chunks(7) {
            decoder.filter(chunk, &mut decoded);
        }
        decoder.validate().unwrap();
        assert_eq!(decoded, self.input);
        if self.mode == RtfCompressionMode::Uncompressed {
            assert_eq!(&bytes[16..], self.input);
        } else if !matches!(self.name, "Random" | "Overlapping") {
            assert!(bytes.len() < self.input.len() / 3);
        }
        assert_eq!(write(self), summarize(&bytes));
    }
}

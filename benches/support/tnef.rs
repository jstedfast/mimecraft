// Original raw-attribute workloads using MimeKit's TNEF fixtures.
// There is no upstream TNEF benchmark class to port.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    io::{self, Read},
};

use mimecraft::tnef::TnefReader;

pub const CASES: [&str; 6] = [
    "Body",
    "Attachments",
    "Christmas",
    "ManySmall",
    "LargeValue",
    "ShortReads",
];

#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Skip,
    Stream,
    Collect,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::Stream => "Stream",
            Self::Collect => "Collect",
        }
    }
}

pub const MODES: [Mode; 3] = [Mode::Skip, Mode::Stream, Mode::Collect];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    chunk: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    pub attributes: usize,
    pub value_bytes: u64,
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
}

impl Read for Source<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.chunk);
        self.input.read(&mut output[..count])
    }
}

pub fn workload(name: &'static str) -> Workload {
    let input = match name {
        "Body" => include_bytes!("../../tests/data/tnef/body.tnef").to_vec(),
        "Attachments" | "ShortReads" => {
            include_bytes!("../../tests/data/tnef/attachments.tnef").to_vec()
        }
        "Christmas" => include_bytes!("../../tests/data/tnef/christmas.tnef").to_vec(),
        "ManySmall" => generated(4096, 32),
        "LargeValue" => generated(1, 1024 * 1024),
        _ => panic!("unknown TNEF workload: {name}"),
    };
    Workload {
        name,
        input,
        chunk: if name == "ShortReads" { 7 } else { usize::MAX },
    }
}

fn generated(count: usize, length: usize) -> Vec<u8> {
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    let value: Vec<_> = (0..length).map(|index| (index & 255) as u8).collect();
    let checksum = value
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
    for _ in 0..count {
        input.push(1);
        input.extend_from_slice(&0x00060000u32.to_le_bytes()); // Owner, a binary attribute.
        input.extend_from_slice(&(length as u32).to_le_bytes());
        input.extend_from_slice(&value);
        input.extend_from_slice(&checksum.to_le_bytes());
    }
    input
}

pub fn read(case: &Workload, mode: Mode) -> Summary {
    let mut reader = TnefReader::new(Source {
        input: black_box(&case.input),
        chunk: case.chunk,
    });
    let mut summary = Summary {
        attributes: 0,
        value_bytes: 0,
    };
    let mut buffer = [0; 4096];
    while reader.read_next().unwrap() {
        summary.attributes += 1;
        match mode {
            Mode::Skip => summary.value_bytes += u64::from(reader.attribute().unwrap().length),
            Mode::Stream => {
                let mut value = reader.open_value().unwrap();
                loop {
                    let read = value.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    black_box(&buffer[..read]);
                    summary.value_bytes += read as u64;
                }
            }
            Mode::Collect => {
                let value = reader.read_value_bytes().unwrap();
                summary.value_bytes += value.len() as u64;
                black_box(value);
            }
        }
    }
    summary
}

impl Workload {
    pub fn check(&self) {
        // A separate wire walk verifies framing, values, and checksums outside
        // timing. Timed runs consume all attributes but do not interpret MAPI.
        assert_eq!(&self.input[..4], &[0x78, 0x9f, 0x3e, 0x22]);
        let mut offset = 6;
        let mut expected = Summary {
            attributes: 0,
            value_bytes: 0,
        };
        let mut reader = TnefReader::new(self.input.as_slice());
        while offset < self.input.len() {
            let header = &self.input[offset..offset + 9];
            let length = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
            let value = &self.input[offset + 9..offset + 9 + length];
            let checksum = value
                .iter()
                .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
            assert_eq!(
                checksum.to_le_bytes(),
                self.input[offset + 9 + length..offset + 11 + length]
            );
            assert!(reader.read_next().unwrap());
            let attribute = reader.attribute().unwrap();
            assert_eq!(attribute.offset, offset as u64);
            assert_eq!(attribute.length as usize, length);
            assert_eq!(
                attribute.tag.0,
                u32::from_le_bytes(header[1..5].try_into().unwrap())
            );
            assert_eq!(reader.read_value_bytes().unwrap(), value);
            expected.attributes += 1;
            expected.value_bytes += length as u64;
            offset += 11 + length;
        }
        assert_eq!(offset, self.input.len());
        assert!(!reader.read_next().unwrap());
        assert!(!reader.is_truncated());
        for mode in MODES {
            assert_eq!(read(self, mode), expected, "{}/{mode:?}", self.name);
        }
    }
}

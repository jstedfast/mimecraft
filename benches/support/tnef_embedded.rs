// Original recursive-reader workloads; MimeKit has no TNEF benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    io::{self, Read},
};

use mimecraft::tnef::{
    TnefAttributeTag as Attribute, TnefEmbeddedReader, TnefPropertyReader, TnefReader,
};

pub const CASES: [&str; 4] = ["Single", "Deep", "Siblings", "ShortReads"];
const IID: [u8; 16] = [7, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46];

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    expected: Summary,
    chunk: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub messages: usize,
    pub attributes: usize,
    pub body_bytes: usize,
    pub body_sum: u64,
}

fn attribute(output: &mut Vec<u8>, tag: Attribute, value: &[u8]) {
    output.push(if tag == Attribute::ATTACHMENT { 2 } else { 1 });
    output.extend_from_slice(&tag.0.to_le_bytes());
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
    let checksum = value
        .iter()
        .fold(0u16, |sum, &b| sum.wrapping_add(u16::from(b)));
    output.extend_from_slice(&checksum.to_le_bytes());
}

fn document(tag: Attribute, value: &[u8]) -> Vec<u8> {
    let mut input = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    attribute(&mut input, tag, value);
    input
}

fn wrap(child: &[u8], count: u32) -> Vec<u8> {
    let mut bytes = count.to_le_bytes().to_vec();
    for _ in 0..count {
        bytes.extend_from_slice(&0x3701000du32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        let length = IID.len() + child.len();
        bytes.extend_from_slice(&(length as u32).to_le_bytes());
        bytes.extend_from_slice(&IID);
        bytes.extend_from_slice(child);
        bytes.extend(std::iter::repeat_n(0, (4 - (length & 3)) & 3));
    }
    document(Attribute::ATTACHMENT, &bytes)
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let siblings = if name == "Siblings" { 16 } else { 1 };
    let depth = if matches!(name, "Deep" | "ShortReads") {
        8
    } else {
        1
    };
    let body: Vec<_> = (0..32768 / siblings).map(|i| (i * 31 + 7) as u8).collect();
    let mut input = document(Attribute::BODY, &body);
    for _ in 0..depth {
        input = wrap(&input, siblings);
    }
    Workload {
        name,
        input,
        expected: Summary {
            messages: 1 + depth as usize * siblings as usize,
            attributes: 1 + depth as usize * siblings as usize,
            body_bytes: 32768,
            // Each complete 256-byte cycle is a permutation of 0..=255.
            body_sum: 32768 * 255 / 2,
        },
        chunk: if name == "ShortReads" { 7 } else { usize::MAX },
    }
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
}

impl Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(self.chunk);
        self.input.read(&mut buffer[..length])
    }
}

fn properties<R: Read>(reader: &mut TnefPropertyReader<'_, R>, result: &mut Summary) {
    while reader.read_next_property().unwrap() {
        assert!(reader.is_embedded_message());
        child(&mut reader.open_embedded_message().unwrap(), result);
    }
}

fn child(reader: &mut TnefEmbeddedReader<'_, '_>, result: &mut Summary) {
    result.messages += 1;
    while reader.read_next().unwrap() {
        result.attributes += 1;
        if reader.attribute().unwrap().tag == Attribute::ATTACHMENT {
            properties(&mut reader.open_properties().unwrap(), result);
        } else {
            assert_eq!(reader.attribute().unwrap().tag, Attribute::BODY);
            let mut stream = reader.open_value().unwrap();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                result.body_bytes += count;
                result.body_sum += buffer[..count].iter().map(|&b| u64::from(b)).sum::<u64>();
            }
        }
    }
}

pub fn read(case: &Workload) -> Summary {
    let mut reader = TnefReader::new(Source {
        input: black_box(&case.input),
        chunk: case.chunk,
    });
    let mut logger = |issue| panic!("invalid embedded benchmark input: {issue:?}");
    let mut result = Summary {
        messages: 1,
        ..Summary::default()
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        result.attributes += 1;
        properties(
            &mut reader.open_properties_with_logger(&mut logger).unwrap(),
            &mut result,
        );
    }
    result
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(read(self), self.expected, "{}", self.name);
    }
}

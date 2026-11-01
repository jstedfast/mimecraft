// Original workloads based on MimeKit's TnefWriterTests attribute cases.
// There is no upstream TNEF writer benchmark class to port.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::io::MemoryBlockStream;
use mimecraft::tnef::{
    TnefAttributeTag as Tag, TnefAttributeWriter, TnefDateTime, TnefDateTimeKind, TnefReader,
    TnefWriter,
};
use std::{
    hint::black_box,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
};

pub const CASES: [&str; 11] = [
    "Typed",
    "ManySmall",
    "LargeValue",
    "Stream",
    "ShortReads",
    "ShortWrites",
    "Buffered",
    "BufferedCursor",
    "BufferedShortChunks",
    "BufferedShortReads",
    "BufferedShortWrites",
];

pub struct Workload {
    pub name: &'static str,
    pub output_length: usize,
    payload: Vec<u8>,
    expected: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub checksum: u64,
}

struct Sink {
    bytes: Vec<u8>,
    chunk: usize,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.chunk);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Source<'a> {
    input: &'a [u8],
    chunk: usize,
}

impl Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(self.chunk);
        self.input.read(&mut buffer[..count])
    }
}

struct Scratch {
    inner: MemoryBlockStream,
    read_chunk: usize,
    write_chunk: usize,
}

impl Read for Scratch {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.read_chunk);
        self.inner.read(&mut bytes[..count])
    }
}

impl Write for Scratch {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner
            .write(&bytes[..bytes.len().min(self.write_chunk)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for Scratch {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

fn buffer_payload<W: Write, B: Read + Write + Seek>(
    mut stream: TnefAttributeWriter<'_, W, B>,
    bytes: &[u8],
    chunk: usize,
) {
    for input in bytes.chunks(chunk) {
        stream.write_all(input).unwrap();
    }
    drop(stream.finish().unwrap());
}

fn attribute(bytes: &mut Vec<u8>, level: u8, tag: Tag, value: &[u8]) {
    bytes.push(level);
    bytes.extend_from_slice(&tag.0.to_le_bytes());
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value);
    let sum = value
        .iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)));
    bytes.extend_from_slice(&sum.to_le_bytes());
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let size = match name {
        "Typed" => 0,
        "ManySmall" => 32,
        _ => 1024 * 1024,
    };
    let payload: Vec<u8> = (0..size).map(|i| (i & 255) as u8).collect();
    let mut expected = vec![0x78, 0x9f, 0x3e, 0x22, 0x34, 0x12];
    attribute(&mut expected, 1, Tag::TNEF_VERSION, &[0, 0, 1, 0]);
    attribute(
        &mut expected,
        1,
        Tag::OEM_CODEPAGE,
        &[0xe4, 4, 0, 0, 0, 0, 0, 0],
    );
    match name {
        "Typed" => {
            attribute(&mut expected, 1, Tag::MESSAGE_CLASS, b"IPM.Note\0");
            attribute(&mut expected, 1, Tag::SUBJECT, b"Subject \xe9\0");
            attribute(&mut expected, 1, Tag::BODY, b"Body text\0");
            attribute(&mut expected, 1, Tag::PRIORITY, &[1, 0]);
            attribute(&mut expected, 1, Tag::AID_OWNER, &(-5i32).to_le_bytes());
            attribute(
                &mut expected,
                1,
                Tag::DATE_SENT,
                &[0xe8, 7, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 2, 0],
            );
        }
        "ManySmall" => {
            for _ in 0..4096 {
                attribute(&mut expected, 1, Tag::BODY, &payload);
            }
        }
        _ => {
            attribute(&mut expected, 2, Tag::ATTACH_RENDER_DATA, &[0; 14]);
            attribute(&mut expected, 2, Tag::ATTACH_DATA, &payload);
        }
    }
    Workload {
        name,
        output_length: expected.len(),
        payload,
        expected,
    }
}

fn encode(case: &Workload) -> Vec<u8> {
    let mut writer = TnefWriter::new(
        Sink {
            bytes: Vec::new(),
            chunk: if case.name == "ShortWrites" {
                7
            } else {
                usize::MAX
            },
        },
        1252,
        0x1234,
    )
    .unwrap();
    match case.name {
        "Typed" => {
            writer.write_string(Tag::MESSAGE_CLASS, "IPM.Note").unwrap();
            writer.write_string(Tag::SUBJECT, "Subject \u{e9}").unwrap();
            writer.write_string(Tag::BODY, "Body text").unwrap();
            writer.write_i32(Tag::PRIORITY, 1).unwrap();
            writer.write_i32(Tag::AID_OWNER, -5).unwrap();
            let date =
                TnefDateTime::new(2024, 1, 2, 3, 4, 5, TnefDateTimeKind::Unspecified).unwrap();
            writer.write_datetime(Tag::DATE_SENT, date).unwrap();
        }
        "ManySmall" => {
            for _ in 0..4096 {
                writer
                    .write_attribute(Tag::BODY, black_box(&case.payload))
                    .unwrap();
            }
        }
        _ => {
            writer
                .write_attribute(Tag::ATTACH_RENDER_DATA, &[0; 14])
                .unwrap();
            if matches!(case.name, "Buffered" | "BufferedShortChunks") {
                buffer_payload(
                    writer.open_attribute(Tag::ATTACH_DATA).unwrap(),
                    black_box(&case.payload),
                    if case.name == "BufferedShortChunks" {
                        1
                    } else {
                        4096
                    },
                );
            } else if case.name == "BufferedCursor" {
                buffer_payload(
                    writer
                        .open_attribute_with_buffer(Tag::ATTACH_DATA, Cursor::new(Vec::new()))
                        .unwrap(),
                    black_box(&case.payload),
                    4096,
                );
            } else if matches!(case.name, "BufferedShortReads" | "BufferedShortWrites") {
                let buffer = Scratch {
                    inner: MemoryBlockStream::new(),
                    read_chunk: if case.name == "BufferedShortReads" {
                        7
                    } else {
                        usize::MAX
                    },
                    write_chunk: if case.name == "BufferedShortWrites" {
                        7
                    } else {
                        usize::MAX
                    },
                };
                buffer_payload(
                    writer
                        .open_attribute_with_buffer(Tag::ATTACH_DATA, buffer)
                        .unwrap(),
                    black_box(&case.payload),
                    4096,
                );
            } else if matches!(case.name, "Stream" | "ShortReads") {
                let mut source = Source {
                    input: black_box(&case.payload),
                    chunk: if case.name == "ShortReads" {
                        7
                    } else {
                        usize::MAX
                    },
                };
                writer
                    .write_attribute_from(Tag::ATTACH_DATA, &mut source, case.payload.len() as u64)
                    .unwrap();
            } else {
                writer
                    .write_attribute(Tag::ATTACH_DATA, black_box(&case.payload))
                    .unwrap();
            }
        }
    }
    writer.finish().unwrap();
    writer.into_inner().bytes
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
        let actual = encode(self);
        assert_eq!(actual, self.expected);
        assert_eq!(actual.len(), self.output_length);
        let mut reader = TnefReader::new(actual.as_slice());
        let mut logger = |issue| panic!("writer benchmark produced nonconformant TNEF: {issue:?}");
        let mut offset = 6;
        while reader.read_next_with_logger(&mut logger).unwrap() {
            let length =
                u32::from_le_bytes(self.expected[offset + 5..offset + 9].try_into().unwrap())
                    as usize;
            let value = reader.read_value_bytes_with_logger(&mut logger).unwrap();
            assert_eq!(value, self.expected[offset + 9..offset + 9 + length]);
            offset += 11 + length;
        }
        assert_eq!(offset, self.expected.len());
        assert_eq!(write(self), summarize(&self.expected));
    }
}

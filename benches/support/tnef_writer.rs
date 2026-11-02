// Original workloads based on MimeKit's TnefWriterTests attribute/property cases.
// There is no upstream TNEF writer benchmark class to port.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::io::MemoryBlockStream;
use mimecraft::tnef::{
    RtfCompressedToRtf, RtfCompressedWriter, RtfCompressionMode, TnefAttributeTag as Tag,
    TnefAttributeWriter, TnefDateTime, TnefDateTimeKind, TnefEmbeddedReader, TnefEmbeddedSink,
    TnefName, TnefNameId, TnefProperty, TnefPropertyTag as P, TnefPropertyType as Type,
    TnefPropertyWriter, TnefReader, TnefValue as Value, TnefWriter,
};
use std::{
    hint::black_box,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
};

pub const CASES: [&str; 36] = [
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
    "Properties",
    "PropertyCopy",
    "PropertyCursor",
    "PropertyBinary",
    "MultiValueProperties",
    "NamedProperties",
    "Recipients",
    "PropertyStream",
    "PropertyStreamCursor",
    "PropertyStreamShortChunks",
    "PropertyStreamShortWrites",
    "MultiValueStreams",
    "ObjectStream",
    "RtfProperty",
    "RtfPropertyUncompressed",
    "RtfPropertyCursor",
    "RtfPropertyShortChunks",
    "RtfPropertyShortWrites",
    "RtfMultiValues",
    "EmbeddedMessage",
    "EmbeddedCursor",
    "EmbeddedShortWrites",
    "EmbeddedNested",
    "EmbeddedCodepage",
    "EmbeddedMany",
];

pub struct Workload {
    pub name: &'static str,
    pub output_length: usize,
    payload: Vec<u8>,
    expected: Vec<u8>,
    value: Value,
    properties: Vec<TnefProperty>,
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
        "Typed" | "Properties" | "PropertyCopy" | "PropertyCursor" | "NamedProperties"
        | "Recipients" => 0,
        "ManySmall" | "MultiValueProperties" | "MultiValueStreams" => 32,
        "EmbeddedNested" | "EmbeddedCodepage" => 4096,
        "EmbeddedMany" => 128,
        _ => 1024 * 1024,
    };
    let payload: Vec<u8> = if name == "RtfMultiValues" {
        b"{\\rtf1 value}".to_vec()
    } else if name.starts_with("Rtf") {
        let mut text = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Arial;}}");
        for _ in 0..500 {
            text.push_str("\\pard Some text that repeats.\\par\r\n");
        }
        text.push('}');
        text.into_bytes()
    } else {
        (0..size).map(|i| (i & 255) as u8).collect()
    };
    let mut expected = vec![0x78, 0x9f, 0x3e, 0x22, 0x34, 0x12];
    attribute(&mut expected, 1, Tag::TNEF_VERSION, &[0, 0, 1, 0]);
    attribute(
        &mut expected,
        1,
        Tag::OEM_CODEPAGE,
        &[0xe4, 4, 0, 0, 0, 0, 0, 0],
    );
    let value = match name {
        name if name.starts_with("Rtf") => {
            // Only expected compressed bytes are prepared here. Measurements
            // encode raw input through the property writer, not a snapshot copy.
            let mut encoder =
                RtfCompressedWriter::new(Cursor::new(Vec::new()), rtf_mode(name)).unwrap();
            encoder.write_all(&payload).unwrap();
            encoder.finish().unwrap();
            Value::Binary(encoder.into_inner().into_inner())
        }
        "MultiValueProperties"
        | "PropertyBinary"
        | "PropertyStream"
        | "PropertyStreamCursor"
        | "PropertyStreamShortChunks"
        | "PropertyStreamShortWrites"
        | "MultiValueStreams" => Value::Binary(payload.clone()),
        "ObjectStream" => Value::Object(payload.clone()),
        "Recipients" => Value::Text("Recipient".into()),
        _ => Value::Null,
    };
    match name {
        name if name.starts_with("Embedded") => {
            let message =
                expected_embedded_message(embedded_depth(name), embedded_codepage(name), &payload);
            let data = expected_embedded_properties(&message, embedded_count(name));
            attribute(&mut expected, 1, Tag::MAPI_PROPERTIES, &data);
        }
        name if name.starts_with("Rtf") => {
            let Value::Binary(encoded) = &value else {
                unreachable!()
            };
            let count = if name == "RtfMultiValues" { 256u32 } else { 1 };
            let mut data = 1u32.to_le_bytes().to_vec();
            let tag = if count == 1 {
                P::RTF_COMPRESSED
            } else {
                P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0))
            };
            data.extend_from_slice(&tag.0.to_le_bytes());
            data.extend_from_slice(&count.to_le_bytes());
            for _ in 0..count {
                data.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
                data.extend_from_slice(encoded);
                data.resize(data.len() + (4 - encoded.len() % 4) % 4, 0);
            }
            attribute(&mut expected, 1, Tag::MAPI_PROPERTIES, &data);
        }
        "Properties"
        | "PropertyCopy"
        | "PropertyCursor"
        | "PropertyBinary"
        | "MultiValueProperties"
        | "NamedProperties"
        | "Recipients"
        | "PropertyStream"
        | "PropertyStreamCursor"
        | "PropertyStreamShortChunks"
        | "PropertyStreamShortWrites"
        | "MultiValueStreams"
        | "ObjectStream" => {
            let mut data = Vec::new();
            let word =
                |data: &mut Vec<u8>, value: u32| data.extend_from_slice(&value.to_le_bytes());
            match name {
                "Properties" | "PropertyCopy" | "PropertyCursor" => {
                    word(&mut data, 4096);
                    for i in 0..4096 {
                        word(&mut data, P::IMPORTANCE.0);
                        word(&mut data, i);
                    }
                }
                "PropertyBinary"
                | "MultiValueProperties"
                | "PropertyStream"
                | "PropertyStreamCursor"
                | "PropertyStreamShortChunks"
                | "PropertyStreamShortWrites"
                | "MultiValueStreams"
                | "ObjectStream" => {
                    let count = if matches!(name, "MultiValueProperties" | "MultiValueStreams") {
                        256
                    } else {
                        1
                    };
                    let kind = if name == "ObjectStream" {
                        Type::OBJECT
                    } else {
                        Type::BINARY
                    };
                    word(&mut data, 1);
                    word(
                        &mut data,
                        P::new(0x6000, Type(kind.0 | if count == 1 { 0 } else { 0x1000 })).0,
                    );
                    word(&mut data, count);
                    for _ in 0..count {
                        word(&mut data, payload.len() as u32);
                        data.extend_from_slice(&payload);
                    }
                }
                "NamedProperties" => {
                    word(&mut data, 256);
                    for i in 0..256 {
                        word(&mut data, P::new(0x8000 + (i % 16) as u16, Type::LONG).0);
                        data.extend_from_slice(&[0x42; 16]);
                        word(&mut data, 0);
                        word(&mut data, i % 16);
                        word(&mut data, i);
                    }
                }
                "Recipients" => {
                    word(&mut data, 512);
                    for _ in 0..512 {
                        word(&mut data, 2);
                        word(&mut data, P::DISPLAY_NAME_W.0);
                        word(&mut data, 1);
                        word(&mut data, 20);
                        data.extend(
                            "Recipient"
                                .encode_utf16()
                                .chain([0])
                                .flat_map(u16::to_le_bytes),
                        );
                        word(&mut data, P::RECIPIENT_TYPE.0);
                        word(&mut data, 1);
                    }
                }
                _ => unreachable!(),
            }
            attribute(
                &mut expected,
                1,
                if name == "Recipients" {
                    Tag::RECIPIENT_TABLE
                } else {
                    Tag::MAPI_PROPERTIES
                },
                &data,
            );
        }
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
    let mut properties = Vec::new();
    if name == "PropertyCopy" {
        let mut reader = TnefReader::new(expected.as_slice());
        while reader.read_next().unwrap() {
            if reader.attribute().unwrap().tag == Tag::MAPI_PROPERTIES {
                let mut source = reader.open_properties().unwrap();
                while source.read_next_property().unwrap() {
                    properties.push(source.read_property().unwrap());
                }
            }
        }
    }
    Workload {
        name,
        output_length: expected.len(),
        payload,
        expected,
        value,
        properties,
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
        name if name.starts_with("Embedded") => {
            if name == "EmbeddedCursor" {
                write_embedded_properties(
                    writer
                        .open_properties_with_buffer(Tag::MAPI_PROPERTIES, Cursor::new(Vec::new()))
                        .unwrap(),
                    case,
                );
            } else if name == "EmbeddedShortWrites" {
                write_embedded_properties(
                    writer
                        .open_properties_with_buffer(
                            Tag::MAPI_PROPERTIES,
                            Scratch {
                                inner: MemoryBlockStream::new(),
                                read_chunk: usize::MAX,
                                write_chunk: 7,
                            },
                        )
                        .unwrap(),
                    case,
                );
            } else {
                write_embedded_properties(
                    writer.open_properties(Tag::MAPI_PROPERTIES).unwrap(),
                    case,
                );
            }
        }
        "Properties"
        | "PropertyCopy"
        | "PropertyCursor"
        | "PropertyBinary"
        | "MultiValueProperties"
        | "NamedProperties"
        | "Recipients"
        | "PropertyStream"
        | "PropertyStreamCursor"
        | "PropertyStreamShortChunks"
        | "PropertyStreamShortWrites"
        | "MultiValueStreams"
        | "ObjectStream"
        | "RtfProperty"
        | "RtfPropertyUncompressed"
        | "RtfPropertyCursor"
        | "RtfPropertyShortChunks"
        | "RtfPropertyShortWrites"
        | "RtfMultiValues" => {
            let tag = if case.name == "Recipients" {
                Tag::RECIPIENT_TABLE
            } else {
                Tag::MAPI_PROPERTIES
            };
            if matches!(
                case.name,
                "PropertyCursor" | "PropertyStreamCursor" | "RtfPropertyCursor"
            ) {
                write_properties(
                    writer
                        .open_properties_with_buffer(tag, Cursor::new(Vec::new()))
                        .unwrap(),
                    case,
                );
            } else if matches!(
                case.name,
                "PropertyStreamShortWrites" | "RtfPropertyShortWrites"
            ) {
                write_properties(
                    writer
                        .open_properties_with_buffer(
                            tag,
                            Scratch {
                                inner: MemoryBlockStream::new(),
                                read_chunk: usize::MAX,
                                write_chunk: 7,
                            },
                        )
                        .unwrap(),
                    case,
                );
            } else {
                write_properties(writer.open_properties(tag).unwrap(), case);
            }
        }
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

fn embedded_depth(name: &str) -> usize {
    if name == "EmbeddedNested" { 8 } else { 1 }
}

fn embedded_count(name: &str) -> u32 {
    if name == "EmbeddedMany" { 64 } else { 1 }
}

fn embedded_codepage(name: &str) -> u32 {
    if name == "EmbeddedCodepage" {
        1251
    } else {
        1252
    }
}

fn embedded_subject(codepage: u32) -> &'static str {
    if codepage == 1251 {
        "\u{438}\u{43d}\u{43d}\u{435}\u{440}"
    } else {
        "inner"
    }
}

fn expected_embedded_properties(message: &[u8], count: u32) -> Vec<u8> {
    let mut data = count.to_le_bytes().to_vec();
    for _ in 0..count {
        data.extend_from_slice(&P::ATTACH_DATA_OBJ.0.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        let length = message.len() + 16;
        data.extend_from_slice(&(length as u32).to_le_bytes());
        data.extend_from_slice(&[7, 3, 2, 0, 0, 0, 0, 0, 0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
        data.extend_from_slice(message);
        data.resize(data.len() + (4 - length % 4) % 4, 0);
    }
    data
}

fn expected_embedded_message(depth: usize, codepage: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x78, 0x9f, 0x3e, 0x22, 0, 0];
    attribute(&mut bytes, 1, Tag::TNEF_VERSION, &[0, 0, 1, 0]);
    let mut page = [0; 8];
    page[..4].copy_from_slice(&codepage.to_le_bytes());
    attribute(&mut bytes, 1, Tag::OEM_CODEPAGE, &page);
    attribute(
        &mut bytes,
        1,
        Tag::SUBJECT,
        if codepage == 1251 {
            &[0xe8, 0xed, 0xed, 0xe5, 0xf0, 0]
        } else {
            b"inner\0"
        },
    );
    if depth > 1 {
        let nested = expected_embedded_message(depth - 1, codepage, payload);
        attribute(
            &mut bytes,
            1,
            Tag::MAPI_PROPERTIES,
            &expected_embedded_properties(&nested, 1),
        );
    } else {
        attribute(&mut bytes, 1, Tag::BODY, payload);
    }
    bytes
}

fn write_embedded_properties<W: Write, B: Read + Write + Seek>(
    mut properties: TnefPropertyWriter<'_, W, B>,
    case: &Workload,
) {
    for _ in 0..embedded_count(case.name) {
        properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
        let mut embedded = properties
            .open_embedded_message_with_codepage(embedded_codepage(case.name))
            .unwrap();
        write_embedded_message(&mut embedded, embedded_depth(case.name), case);
        embedded.finish().unwrap();
    }
    drop(properties.finish().unwrap());
}

fn write_embedded_message(
    writer: &mut TnefWriter<TnefEmbeddedSink<'_>>,
    depth: usize,
    case: &Workload,
) {
    writer
        .write_string(Tag::SUBJECT, black_box(embedded_subject(writer.codepage())))
        .unwrap();
    if depth > 1 {
        let mut properties = writer.open_properties(Tag::MAPI_PROPERTIES).unwrap();
        properties.write_tag(P::ATTACH_DATA_OBJ).unwrap();
        let mut embedded = properties.open_embedded_message().unwrap();
        write_embedded_message(&mut embedded, depth - 1, case);
        embedded.finish().unwrap();
        properties.finish().unwrap();
    } else {
        writer
            .write_attribute(Tag::BODY, black_box(&case.payload))
            .unwrap();
    }
}

fn check_embedded_message(reader: &mut TnefEmbeddedReader<'_, '_>, depth: usize, case: &Workload) {
    let mut tags = Vec::new();
    while reader.read_next().unwrap() {
        let tag = reader.attribute().unwrap().tag;
        tags.push(tag);
        match tag {
            Tag::SUBJECT => assert_eq!(
                reader.read_value_string().unwrap(),
                embedded_subject(embedded_codepage(case.name))
            ),
            Tag::BODY => assert_eq!(reader.read_value_bytes().unwrap(), case.payload),
            Tag::MAPI_PROPERTIES => {
                assert!(depth > 1);
                let mut properties = reader.open_properties().unwrap();
                assert!(properties.read_next_property().unwrap());
                assert_eq!(properties.property().unwrap().tag, P::ATTACH_DATA_OBJ);
                {
                    let mut embedded = properties.open_embedded_message().unwrap();
                    check_embedded_message(&mut embedded, depth - 1, case);
                }
                assert!(!properties.read_next_property().unwrap());
            }
            Tag::OEM_CODEPAGE => {
                let bytes = reader.read_value_bytes().unwrap();
                assert_eq!(
                    u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                    embedded_codepage(case.name)
                );
            }
            Tag::TNEF_VERSION => {}
            _ => panic!("unexpected embedded attribute"),
        }
    }
    assert_eq!(
        tags,
        [
            Tag::TNEF_VERSION,
            Tag::OEM_CODEPAGE,
            Tag::SUBJECT,
            if depth > 1 {
                Tag::MAPI_PROPERTIES
            } else {
                Tag::BODY
            }
        ]
    );
}

fn rtf_mode(name: &str) -> RtfCompressionMode {
    if name == "RtfPropertyUncompressed" {
        RtfCompressionMode::Uncompressed
    } else {
        RtfCompressionMode::Compressed
    }
}

fn write_properties<W: Write, B: Read + Write + Seek>(
    mut properties: TnefPropertyWriter<'_, W, B>,
    case: &Workload,
) {
    match case.name {
        name if name.starts_with("Rtf") => {
            let count = if name == "RtfMultiValues" { 256 } else { 1 };
            properties
                .write_tag(if count == 1 {
                    P::RTF_COMPRESSED
                } else {
                    P::new(0x6000, Type(Type::BINARY.0 | Type::MULTI_VALUED.0))
                })
                .unwrap();
            for _ in 0..count {
                let mut rtf = properties.open_rtf_compressed(rtf_mode(name)).unwrap();
                let chunk = if name == "RtfPropertyShortChunks" {
                    1
                } else {
                    4096
                };
                for input in black_box(&case.payload).chunks(chunk) {
                    rtf.write_all(input).unwrap();
                }
                rtf.finish().unwrap();
            }
        }
        "Properties" | "PropertyCursor" => {
            for i in 0..4096 {
                properties.write_tag(P::IMPORTANCE).unwrap();
                properties.write_value(&Value::I32(black_box(i))).unwrap();
            }
        }
        "PropertyCopy" => {
            for property in &case.properties {
                properties.write_property(black_box(property)).unwrap();
            }
        }
        "PropertyBinary" | "MultiValueProperties" => {
            let count = if case.name == "PropertyBinary" {
                1
            } else {
                256
            };
            properties
                .write_tag(P::new(
                    0x6000,
                    Type(Type::BINARY.0 | if count == 1 { 0 } else { 0x1000 }),
                ))
                .unwrap();
            for _ in 0..count {
                properties.write_value(black_box(&case.value)).unwrap();
            }
        }
        "PropertyStream"
        | "PropertyStreamCursor"
        | "PropertyStreamShortChunks"
        | "PropertyStreamShortWrites"
        | "MultiValueStreams"
        | "ObjectStream" => {
            let count = if case.name == "MultiValueStreams" {
                256
            } else {
                1
            };
            let kind = if case.name == "ObjectStream" {
                Type::OBJECT
            } else {
                Type::BINARY
            };
            properties
                .write_tag(P::new(
                    0x6000,
                    Type(kind.0 | if count == 1 { 0 } else { Type::MULTI_VALUED.0 }),
                ))
                .unwrap();
            for _ in 0..count {
                let mut value = properties.open_value().unwrap();
                let chunk = if case.name == "PropertyStreamShortChunks" {
                    1
                } else {
                    4096
                };
                for input in black_box(&case.payload).chunks(chunk) {
                    value.write_all(input).unwrap();
                }
                value.finish().unwrap();
            }
        }
        "NamedProperties" => {
            for i in 0..256 {
                let name = TnefNameId {
                    guid: [0x42; 16],
                    name: TnefName::Id(i % 16),
                };
                properties.write_named_tag(&name, Type::LONG).unwrap();
                properties.write_value(&Value::I32(black_box(i))).unwrap();
            }
        }
        "Recipients" => {
            for _ in 0..512 {
                properties.begin_row().unwrap();
                properties.write_tag(P::DISPLAY_NAME_W).unwrap();
                properties.write_value(black_box(&case.value)).unwrap();
                properties.write_tag(P::RECIPIENT_TYPE).unwrap();
                properties.write_value(&Value::I32(1)).unwrap();
            }
        }
        _ => unreachable!(),
    }
    drop(properties.finish().unwrap());
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
        if self.name.starts_with("Embedded") {
            let mut reader = TnefReader::new(actual.as_slice());
            let mut count = 0;
            while reader.read_next_with_logger(&mut logger).unwrap() {
                if reader.attribute().unwrap().tag != Tag::MAPI_PROPERTIES {
                    continue;
                }
                let mut properties = reader.open_properties_with_logger(&mut logger).unwrap();
                while properties.read_next_property().unwrap() {
                    assert_eq!(properties.property().unwrap().tag, P::ATTACH_DATA_OBJ);
                    let mut embedded = properties.open_embedded_message().unwrap();
                    check_embedded_message(&mut embedded, embedded_depth(self.name), self);
                    count += 1;
                }
            }
            assert_eq!(count, embedded_count(self.name));
        }
        if self.name.starts_with("Rtf")
            || matches!(
                self.name,
                "Properties"
                    | "PropertyCopy"
                    | "PropertyCursor"
                    | "PropertyBinary"
                    | "MultiValueProperties"
                    | "NamedProperties"
                    | "Recipients"
                    | "PropertyStream"
                    | "PropertyStreamCursor"
                    | "PropertyStreamShortChunks"
                    | "PropertyStreamShortWrites"
                    | "MultiValueStreams"
                    | "ObjectStream"
            )
        {
            let mut reader = TnefReader::new(actual.as_slice());
            while reader.read_next_with_logger(&mut logger).unwrap() {
                if !matches!(
                    reader.attribute().unwrap().tag,
                    Tag::MAPI_PROPERTIES | Tag::RECIPIENT_TABLE
                ) {
                    continue;
                }
                let mut properties = reader.open_properties_with_logger(&mut logger).unwrap();
                let mut rows = 0;
                loop {
                    if properties.is_table() && !properties.read_next_row().unwrap() {
                        break;
                    }
                    rows += 1;
                    let mut count = 0;
                    while properties.read_next_property().unwrap() {
                        let property = properties.read_property().unwrap();
                        match self.name {
                            name if name.starts_with("Rtf") => {
                                assert_eq!(
                                    property.values().len(),
                                    if name == "RtfMultiValues" { 256 } else { 1 }
                                );
                                for value in property.values() {
                                    let Value::Binary(bytes) = value else {
                                        panic!("RTF must be binary")
                                    };
                                    assert_eq!(
                                        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
                                        bytes.len() - 4
                                    );
                                    assert_eq!(
                                        u32::from_le_bytes(bytes[4..8].try_into().unwrap())
                                            as usize,
                                        self.payload.len()
                                    );
                                    assert_eq!(
                                        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                                        u32::from(rtf_mode(name))
                                    );
                                    assert_eq!(
                                        u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
                                        if rtf_mode(name) == RtfCompressionMode::Compressed {
                                            super::rtf_compressed_workloads::crc(&bytes[16..])
                                        } else {
                                            0
                                        }
                                    );
                                    let mut decoder = RtfCompressedToRtf::new();
                                    let mut decoded = Vec::new();
                                    decoder.filter(bytes, &mut decoded);
                                    decoder.validate().unwrap();
                                    assert_eq!(decoded, self.payload);
                                }
                                if rtf_mode(name) == RtfCompressionMode::Compressed
                                    && name != "RtfMultiValues"
                                {
                                    assert!(actual.len() < self.payload.len() / 4);
                                }
                            }
                            "Properties" | "PropertyCopy" | "PropertyCursor" => {
                                assert_eq!(property.tag(), P::IMPORTANCE);
                                assert_eq!(property.value(), Some(&Value::I32(count)));
                            }
                            "NamedProperties" => {
                                assert_eq!(
                                    property.name(),
                                    Some(&TnefNameId {
                                        guid: [0x42; 16],
                                        name: TnefName::Id(count % 16)
                                    })
                                );
                                assert_eq!(property.value(), Some(&Value::I32(count)));
                            }
                            "PropertyBinary"
                            | "MultiValueProperties"
                            | "PropertyStream"
                            | "PropertyStreamCursor"
                            | "PropertyStreamShortChunks"
                            | "PropertyStreamShortWrites"
                            | "MultiValueStreams"
                            | "ObjectStream" => {
                                assert_eq!(
                                    property.values().len(),
                                    if matches!(
                                        self.name,
                                        "MultiValueProperties" | "MultiValueStreams"
                                    ) {
                                        256
                                    } else {
                                        1
                                    }
                                );
                                assert!(property.values().iter().all(|value| value == &self.value));
                            }
                            "Recipients" => {
                                assert_eq!(
                                    property.value(),
                                    Some(if count == 0 {
                                        &self.value
                                    } else {
                                        &Value::I32(1)
                                    })
                                );
                            }
                            _ => unreachable!(),
                        }
                        count += 1;
                    }
                    assert_eq!(
                        count,
                        match self.name {
                            "Properties" | "PropertyCopy" | "PropertyCursor" => 4096,
                            "NamedProperties" => 256,
                            "Recipients" => 2,
                            _ => 1,
                        }
                    );
                    if !properties.is_table() {
                        break;
                    }
                }
                assert_eq!(rows, if self.name == "Recipients" { 512 } else { 1 });
            }
        }
        assert_eq!(write(self), summarize(&self.expected));
    }
}

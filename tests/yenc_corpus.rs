// Adapted from MimeKit's UnitTests/Encodings/YEncTests.cs and YEncodingTests.cs.
// Fixtures are from MimeKit's UnitTests/TestData/yenc; see data/LICENSE-MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Read, Write};

use mimecraft::{
    MimeEntity, MimeMessage,
    encodings::{NewLineFormat, YDecoder, YEncReader, YEncoder},
    filters::{DecoderFilter, Dos2UnixFilter, EncoderFilter},
    io::FilteredWriter,
};

const FIRST: &[u8] = include_bytes!("data/yenc/00000020.ntx");
const SECOND: &[u8] = include_bytes!("data/yenc/00000021.ntx");
const JPEG: &[u8] = include_bytes!("data/yenc/joystick.jpg");
const SIMPLE: &[u8] = include_bytes!("data/yenc/simple.eml");
const STATES: &[u8] = include_bytes!("data/yenc/state-changes.ntx");

struct Chunked<'a>(&'a [u8], usize);

impl Read for Chunked<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.1);
        self.0.read(&mut output[..count])
    }
}

#[test]
fn multipart_corpus_with_strict_reader_and_tolerant_decoder() {
    for chunk in [1, 4096] {
        let mut strict = Vec::new();
        let mut tolerant = Vec::new();
        for (input, part, range, checksum) in [
            (FIRST, 1, 1..=11250, 0xbfae5c0b),
            (SECOND, 2, 11251..=19338, 0xaca76043),
        ] {
            let mut reader = YEncReader::new(Chunked(input, chunk));
            assert!(reader.read_next().unwrap());
            let header = reader.header().unwrap();
            assert_eq!(header.file_name(), "joystick.jpg");
            assert_eq!(header.size(), 19338);
            assert_eq!(header.part_number(), Some(part));
            assert_eq!(header.range(), Some(&range));
            reader.decode_to(&mut strict).unwrap();
            assert_eq!(reader.decoded_size(), range.end() - range.start() + 1);
            assert_eq!(reader.checksum(), Some(checksum));
            assert!(!reader.read_next().unwrap());

            let mut writer =
                FilteredWriter::new(&mut tolerant, DecoderFilter::new(YDecoder::new()));
            for input in input.chunks(chunk) {
                writer.write_all(input).unwrap();
            }
            writer.finish().unwrap();
            assert_eq!(!writer.filter().decoder().checksum(), checksum);
        }
        assert_eq!(strict, JPEG);
        assert_eq!(tolerant, JPEG);
    }
}

#[test]
fn simple_message_decodes_and_reencodes_the_exact_original_body() {
    let mut source = SIMPLE;
    let mut message = MimeMessage::load(&mut source).unwrap();
    let MimeEntity::Part(part) = message.body_mut().unwrap() else {
        panic!("expected a leaf MIME part")
    };
    let mut writer = FilteredWriter::new(Vec::new(), DecoderFilter::new(YDecoder::new()));
    part.content_mut().write_to(&mut writer).unwrap();
    writer.finish().unwrap();
    let decoded = writer.get_ref();
    assert_eq!(decoded.len(), 584);
    assert_eq!(!writer.filter().decoder().checksum(), 0xded29f4f);

    let mut original = FilteredWriter::new(Vec::new(), Dos2UnixFilter::new());
    part.content_mut().write_to(&mut original).unwrap();
    original.finish().unwrap();

    let begin = b"-- \n=ybegin line=128 size=584 name=testfile.txt \n";
    let end = b"=yend size=584 crc32=ded29f4f \n";
    for chunk in [1, 17, 4096] {
        let encoder = YEncoder::with_new_line_format(NewLineFormat::Unix).unwrap();
        let mut writer = FilteredWriter::new(begin.to_vec(), EncoderFilter::new(encoder));
        for input in decoded.chunks(chunk) {
            writer.write_all(input).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(!writer.filter().encoder().checksum(), 0xded29f4f);
        let (mut encoded, _, pending) = writer.into_parts();
        assert!(pending.is_empty());
        encoded.extend_from_slice(end);
        assert_eq!(&encoded, original.get_ref());

        let mut reader = YEncReader::new(Chunked(SIMPLE, chunk));
        assert!(reader.read_next().unwrap());
        assert_eq!(reader.header().unwrap().file_name(), "testfile.txt");
        let mut actual = Vec::new();
        reader.decode_to(&mut actual).unwrap();
        assert_eq!(reader.checksum(), Some(0xded29f4f));
        assert_eq!(&actual, decoded);
        assert!(!reader.read_next().unwrap());
    }
    let mut encoder = YEncoder::with_new_line_format(NewLineFormat::Unix).unwrap();
    let mut encoded = begin.to_vec();
    encoder.encode(decoded, &mut encoded);
    encoder.finish(&mut encoded);
    assert_eq!(!encoder.checksum(), 0xded29f4f);
    encoded.extend_from_slice(end);
    assert_eq!(&encoded, original.get_ref());
}

#[test]
fn tolerant_state_transition_corpus() {
    let mut expected = Vec::new();
    YDecoder::new().decode(SIMPLE, &mut expected);
    assert_eq!(expected.len(), 584);
    for chunk in [1, 17, 4096] {
        let mut writer = FilteredWriter::new(Vec::new(), DecoderFilter::new(YDecoder::new()));
        for input in STATES.chunks(chunk) {
            writer.write_all(input).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(writer.get_ref(), &expected);
        assert_eq!(!writer.filter().decoder().checksum(), 0xded29f4f);
    }
}

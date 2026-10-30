// Adapted from MimeKit's Benchmarks/IO/Filters/*Benchmarks.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    ContentEncoding, EncodingConstraint,
    encodings::{
        Base64Decoder, Base64Encoder, MimeDecoder, QuotedPrintableDecoder, QuotedPrintableEncoder,
        UuDecoder, UuEncoder,
    },
    filters::{
        ArmoredFromFilter, BestEncodingFilter, DecoderFilter, Dos2UnixFilter, EncoderFilter,
        MboxFromFilter, MimeFilter, TrailingWhitespaceFilter, Unix2DosFilter,
    },
    io::{FilteredWriter, MeasuringStream},
};
use std::{hint::black_box, io::Write, path::Path};

use crate::{fingerprint::fingerprint, fixtures};

pub const CASES: [&str; 31] = [
    "ArmoredFromFilter_FromRussiaWithLove",
    "MboxFromFilter_FromRussiaWithLove",
    "BestEncoding_LoremIpsum",
    "BestEncoding_GirlJpeg",
    "TrailingWhitespace_LoremIpsum",
    "Dos2Unix_LoremIpsumDos",
    "Dos2Unix_LoremIpsumUnix",
    "Dos2Unix_PathologicalDos",
    "Dos2Unix_PathologicalUnix",
    "Unix2Dos_LoremIpsumDos",
    "Unix2Dos_LoremIpsumUnix",
    "Unix2Dos_PathologicalDos",
    "Unix2Dos_PathologicalUnix",
    "Base64Encode_60",
    "Base64Encode_64",
    "Base64Encode_68",
    "Base64Encode_72",
    "Base64Encode_76",
    "Base64EncodeStream_60",
    "Base64EncodeStream_64",
    "Base64EncodeStream_68",
    "Base64EncodeStream_72",
    "Base64EncodeStream_76",
    "QuotedPrintableEncodeStream",
    "UUEncodeStream",
    "Base64Decode",
    "Base64DecodeStream",
    "QuotedPrintableDecode",
    "QuotedPrintableDecodeStream",
    "UUDecode",
    "UUDecodeStream",
];

#[derive(Clone, Copy)]
enum Operation {
    ArmoredFrom,
    MboxFrom,
    Best,
    TrailingWhitespace,
    Dos2Unix,
    Unix2Dos,
    Encode(ContentEncoding, usize, bool),
    Decode(ContentEncoding, bool),
}

pub struct Workload {
    pub name: &'static str,
    pub input: Vec<u8>,
    operation: Operation,
    expected: (usize, u64),
}

fn encoder_fixture(name: &str) -> Vec<u8> {
    // MimeKit's photo.jpg and the already ported girl.jpg are byte-identical.
    if name == "photo.jpg" {
        return fixtures::attachment("girl.jpg");
    }
    let expected = match name {
        "photo.b64" => (178296, 0x53047c2aac1b503e),
        "photo.uu" => (182441, 0x716027736df95d4a),
        "wikipedia.txt" => (383, 0x67fcc5078a00dda3),
        "wikipedia.qp" => (435, 0xcd58c91e66076197),
        _ => panic!("unknown encoder fixture: {name}"),
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("data")
        .join("encoders")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(fingerprint(&bytes), expected, "{name}");
    bytes
}

pub fn workload(name: &'static str) -> Workload {
    use Operation::*;
    let (operation, input, expected) = match name {
        "ArmoredFromFilter_FromRussiaWithLove" | "MboxFromFilter_FromRussiaWithLove" => {
            let input =
                b"From Russia with love is one of my favorite James Bond files.\n".repeat(1000);
            assert_eq!(fingerprint(&input), (62000, 0x87cb899a361a3525));
            if name.starts_with("Armored") {
                (ArmoredFrom, input, (64000, 0x33450129fa917295))
            } else {
                (MboxFrom, input, (63000, 0xdd3ba09f226179c5))
            }
        }
        "BestEncoding_LoremIpsum" | "TrailingWhitespace_LoremIpsum" => (
            if name.starts_with("Best") {
                Best
            } else {
                TrailingWhitespace
            },
            fixtures::attachment("lorem-ipsum.txt"),
            (2568, 0x0f79f7ecad15bc49),
        ),
        "BestEncoding_GirlJpeg" => (
            Best,
            fixtures::attachment("girl.jpg"),
            (130292, 0xc21813b85e0e0385),
        ),
        _ if name.starts_with("Dos2Unix_") || name.starts_with("Unix2Dos_") => {
            let to_unix = name.starts_with("Dos2Unix_");
            let input = if name.contains("LoremIpsum") {
                // Preserve upstream's setup quirk: BOTH LoremIpsumDos and
                // LoremIpsumUnix are generated with Unix2DosFilter, not one each.
                let bytes = fixtures::attachment("lorem-ipsum.txt");
                let mut filter = Unix2DosFilter::new();
                let mut output = Vec::new();
                filter.filter(&bytes, &mut output);
                filter.finish(&mut output);
                assert_eq!(fingerprint(&output), (2568, 0x0f79f7ecad15bc49));
                output
            } else if name.ends_with("Dos") {
                b"\r\n".repeat(1024)
            } else {
                vec![b'\n'; 1024]
            };
            let expected = match (name.contains("LoremIpsum"), to_unix) {
                (true, true) => (2529, 0x9505826804a1f61c),
                (true, false) => (2568, 0x0f79f7ecad15bc49),
                (false, true) => (1024, 0xa4c89a5681d70325),
                (false, false) => (2048, 0x97fd4fdaabbfab25),
            };
            (if to_unix { Dos2Unix } else { Unix2Dos }, input, expected)
        }
        _ if name.starts_with("Base64Encode") => {
            let width = name.rsplit('_').next().unwrap().parse().unwrap();
            let expected = match width {
                60 => (179516, 0xf144a326ba0c587e),
                64 => (179154, 0xc6f6427374089059),
                68 => (178834, 0xeae619686e898909),
                72 => (178550, 0xcf4158c87f8cc559),
                76 => (178296, 0x53047c2aac1b503e),
                _ => panic!("unknown Base64 width: {width}"),
            };
            (
                Encode(ContentEncoding::Base64, width, name.contains("Stream")),
                encoder_fixture("photo.jpg"),
                expected,
            )
        }
        "QuotedPrintableEncodeStream" => (
            Encode(ContentEncoding::QuotedPrintable, 76, true),
            encoder_fixture("wikipedia.txt"),
            (435, 0xcd58c91e66076197),
        ),
        "UUEncodeStream" => (
            Encode(ContentEncoding::UuEncode, 76, true),
            encoder_fixture("photo.jpg"),
            (182415, 0x8c00a6d3cf5fb77d),
        ),
        "Base64Decode" | "Base64DecodeStream" => (
            Decode(ContentEncoding::Base64, name.ends_with("Stream")),
            encoder_fixture("photo.b64"),
            (130292, 0xc21813b85e0e0385),
        ),
        "QuotedPrintableDecode" | "QuotedPrintableDecodeStream" => (
            Decode(ContentEncoding::QuotedPrintable, name.ends_with("Stream")),
            encoder_fixture("wikipedia.qp"),
            (383, 0x67fcc5078a00dda3),
        ),
        "UUDecode" | "UUDecodeStream" => (
            Decode(ContentEncoding::UuEncode, name.ends_with("Stream")),
            encoder_fixture("photo.uu"),
            (130292, 0xc21813b85e0e0385),
        ),
        _ => panic!("unknown filter workload: {name}"),
    };
    Workload {
        name,
        input,
        operation,
        expected,
    }
}

impl Workload {
    pub fn bytes_per_operation(&self) -> usize {
        self.input.len()
            * if matches!(self.operation, Operation::Best) {
                100
            } else {
                1
            }
    }

    fn stream<W: Write>(&self, sink: W) -> W {
        use Operation::*;
        match self.operation {
            ArmoredFrom => filter_to(&self.input, ArmoredFromFilter::new(), sink, 1024),
            MboxFrom => filter_to(&self.input, MboxFromFilter::new(), sink, 1024),
            TrailingWhitespace => {
                filter_to(&self.input, TrailingWhitespaceFilter::new(), sink, 4096)
            }
            Dos2Unix => filter_to(&self.input, Dos2UnixFilter::new(), sink, 4096),
            Unix2Dos => filter_to(&self.input, Unix2DosFilter::new(), sink, 4096),
            Encode(ContentEncoding::Base64, width, _) => filter_to(
                &self.input,
                EncoderFilter::new(Base64Encoder::with_line_length(width).unwrap()),
                sink,
                4096,
            ),
            Encode(ContentEncoding::QuotedPrintable, _, _) => filter_to(
                &self.input,
                EncoderFilter::new(QuotedPrintableEncoder::new()),
                sink,
                4096,
            ),
            Encode(ContentEncoding::UuEncode, _, _) => filter_to(
                &self.input,
                EncoderFilter::new(UuEncoder::new()),
                sink,
                4096,
            ),
            Decode(ContentEncoding::Base64, _) => filter_to(
                &self.input,
                DecoderFilter::new(Base64Decoder::new()),
                sink,
                4096,
            ),
            Decode(ContentEncoding::QuotedPrintable, _) => filter_to(
                &self.input,
                DecoderFilter::new(QuotedPrintableDecoder::new()),
                sink,
                4096,
            ),
            Decode(ContentEncoding::UuEncode, _) => filter_to(
                &self.input,
                DecoderFilter::new(UuDecoder::new()),
                sink,
                4096,
            ),
            _ => panic!("not a stream-filter workload: {}", self.name),
        }
    }

    fn direct(&self) -> Vec<u8> {
        use Operation::*;
        match self.operation {
            Encode(ContentEncoding::Base64, width, false) => {
                let mut encoder = Base64Encoder::with_line_length(width).unwrap();
                // Allocation belongs to the operation, as in upstream's
                // EstimateOutputLength + new byte[] path, not fixture setup.
                let encoded_length = self.input.len().div_ceil(3) * 4;
                let mut output =
                    Vec::with_capacity(encoded_length + encoded_length.div_ceil(width) * 2);
                encoder.encode(black_box(&self.input), &mut output);
                encoder.finish(&mut output);
                output
            }
            Decode(ContentEncoding::Base64, false) => decode(&self.input, Base64Decoder::new()),
            Decode(ContentEncoding::QuotedPrintable, false) => {
                decode(&self.input, QuotedPrintableDecoder::new())
            }
            Decode(ContentEncoding::UuEncode, false) => decode(&self.input, UuDecoder::new()),
            _ => panic!("not a direct-codec workload: {}", self.name),
        }
    }

    pub fn run(&self) {
        use Operation::*;
        match self.operation {
            Best => {
                let mut filter = BestEncodingFilter::new();
                let mut output = Vec::new();
                for _ in 0..100 {
                    for chunk in black_box(&self.input).chunks(4096) {
                        output.clear();
                        filter.filter(chunk, &mut output);
                        black_box(&output);
                    }
                    filter.finish(&mut output);
                    // The analyzer has no emitted result in the upstream
                    // benchmark. Keep its accumulated state observable before reset.
                    black_box(&filter);
                    filter.reset();
                }
            }
            ArmoredFrom | MboxFrom => {
                black_box(self.stream(Vec::new()));
            }
            Encode(_, _, false) | Decode(_, false) => {
                black_box(self.direct());
            }
            _ => {
                black_box(self.stream(MeasuringStream::new()));
            }
        }
    }

    pub fn check(&self) {
        use Operation::*;
        if matches!(self.operation, Best) {
            let expected = if self.name.ends_with("GirlJpeg") {
                ContentEncoding::Base64
            } else {
                ContentEncoding::SevenBit
            };
            let mut filter = BestEncodingFilter::new();
            for _ in 0..100 {
                let mut output = Vec::new();
                for chunk in self.input.chunks(4096) {
                    filter.filter(chunk, &mut output);
                }
                filter.finish(&mut output);
                assert_eq!(output, self.input);
                for constraint in [
                    EncodingConstraint::None,
                    EncodingConstraint::SevenBit,
                    EncodingConstraint::EightBit,
                ] {
                    assert_eq!(filter.best_encoding(constraint), expected);
                }
                filter.reset();
                assert_eq!(
                    filter.best_encoding(EncodingConstraint::SevenBit),
                    ContentEncoding::SevenBit
                );
            }
        } else {
            let output = match self.operation {
                Encode(_, _, false) | Decode(_, false) => self.direct(),
                _ => self.stream(Vec::new()),
            };
            assert_eq!(fingerprint(&output), self.expected, "{} output", self.name);
            assert_eq!(
                self.stream(MeasuringStream::new()).len(),
                output.len() as u64
            );
        }
    }
}

fn filter_to<W: Write, F: MimeFilter>(input: &[u8], filter: F, sink: W, chunk_size: usize) -> W {
    let mut writer = FilteredWriter::new(sink, filter);
    for chunk in black_box(input).chunks(chunk_size) {
        writer.write_all(chunk).unwrap();
    }
    writer.finish().unwrap();
    black_box(&writer);
    writer.into_parts().0
}

fn decode(input: &[u8], mut decoder: impl MimeDecoder) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    decoder.decode(black_box(input), &mut output);
    output
}

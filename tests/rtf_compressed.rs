// Ported from MimeKit's Tnef and Text RtfCompressedToRtfTests,
// RtfCompressedBuilder, and RtfCompressedToRtfFuzzTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{Read, Write};

use mimecraft::{
    filters::MimeFilter,
    io::{FilteredReader, FilteredWriter},
    tnef::{
        RtfCompressedToRtf as Filter, RtfCompressionMode as Mode, RtfDecompressionError as Error,
    },
};

#[path = "../benches/support/rtf_compressed.rs"]
mod support;
use support::{Builder, crc, document};

// Published [MS-OXRTFCP] examples, also used by MimeKit's Text/Tnef suites.
// https://msdn.microsoft.com/en-us/library/ee217938(v=exchg.80).aspx
const SIMPLE: &[u8] = b"\x2d\0\0\0\x2b\0\0\0LZFu\xf1\xc5\xc7\xa7\x03\x00\x0a\x00rcpg125B2\x0a\xf3 hel\x09\x00 bw\x05\xb0ld}\x0a\x80\x0f\xa0";
const SIMPLE_TEXT: &[u8] = b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n";
// https://msdn.microsoft.com/en-us/library/ee158471(v=exchg.80).aspx
const CROSSING: &[u8] =
    b"\x1a\0\0\0\x1c\0\0\0LZFu\xe2\xd4\x4b\x51\x41\x00\x04 WXYZ\x0d\x6e}\x01\x0e\xb0";

fn decode_with(filter: &mut Filter, input: &[u8], chunk: usize) -> Vec<u8> {
    let mut output = Vec::new();
    for bytes in input.chunks(chunk) {
        filter.filter(bytes, &mut output);
    }
    filter.finish(&mut output);
    output
}

fn check(input: &[u8], expected: &[u8], mode: Mode) {
    for chunk in [1, 2, 3, 7, 4096, usize::MAX] {
        let mut filter = Filter::new();
        assert_eq!(decode_with(&mut filter, input, chunk), expected);
        assert_eq!(filter.compression_mode(), mode);
        assert!(filter.is_complete());
        assert!(filter.is_valid_crc32());
        assert_eq!(filter.validate(), Ok(()));
    }
}

#[test]
fn published_examples_and_uncompressed_payload() {
    assert_eq!(crc(&SIMPLE[16..]), 0xa7c7c5f1);
    assert_eq!(crc(&CROSSING[16..]), 0x514bd4e2);
    check(SIMPLE, SIMPLE_TEXT, Mode::Compressed);
    check(CROSSING, b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}", Mode::Compressed);
    let raw = b"{\\rtf1\\ansi\\ansicpg1252\\pard test}";
    check(
        &document(raw, raw.len() as i32, Mode::Uncompressed, 0),
        raw,
        Mode::Uncompressed,
    );
}

#[test]
fn every_header_and_token_split_preserves_output_and_crc() {
    // Every header field and two-byte reference must resume across chunks.
    for input in [SIMPLE, CROSSING] {
        for split in 0..=input.len() {
            let mut filter = Filter::new();
            let mut output = b"prefix".to_vec();
            filter.filter(&input[..split], &mut output);
            filter.finish(&mut output);
            filter.filter(&[], &mut output);
            filter.filter(&input[split..], &mut output);
            filter.finish(&mut output);
            filter.finish(&mut output);
            filter.validate().unwrap();
            assert_eq!(&output[..6], b"prefix");
            let expected = if input == SIMPLE {
                SIMPLE_TEXT
            } else {
                b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}"
            };
            assert_eq!(&output[6..], expected);
        }
    }
}

#[test]
fn mode_is_unknown_until_twelve_bytes_and_reset_clears_it() {
    for raw in [0, 1, 0x44434241, u32::MAX, 0x414c454d, 0x75465a4c] {
        assert_eq!(u32::from(Mode::from(raw)), raw);
        let input = document(&[], 0, Mode::from(raw), 0);
        let mut filter = Filter::default();
        for (index, byte) in input.iter().enumerate() {
            filter.filter(&[*byte], &mut Vec::new());
            assert_eq!(
                filter.compression_mode(),
                if index < 11 {
                    Mode::Unknown(0)
                } else {
                    Mode::from(raw)
                }
            );
        }
        filter.reset();
        assert_eq!(filter.compression_mode(), Mode::Unknown(0));
        assert!(!filter.is_complete());
        assert!(!filter.is_valid_crc32());
    }
}

#[test]
fn unknown_modes_discard_content_and_report_explicit_errors() {
    for mode in [0, 0x44434241, u32::MAX] {
        for chunk in [1, 3, 4096] {
            let mut input = SIMPLE.to_vec();
            input[8..12].copy_from_slice(&mode.to_le_bytes());
            let mut filter = Filter::new();
            assert!(decode_with(&mut filter, &input, chunk).is_empty());
            assert_eq!(filter.compression_mode(), Mode::Unknown(mode));
            assert_eq!(
                filter.validate(),
                Err(Error::UnsupportedCompressionMode(mode))
            );
            assert!(!filter.is_valid_crc32());
        }
    }
}

#[test]
fn incomplete_headers_and_content_never_report_valid_crc() {
    let raw = document(b"abcd", 4, Mode::Uncompressed, 0);
    for input in [SIMPLE, raw.as_slice()] {
        for length in 0..input.len() {
            let mut filter = Filter::new();
            decode_with(&mut filter, &input[..length], 1);
            assert!(!filter.is_valid_crc32(), "{length}");
            assert_eq!(
                filter.validate(),
                Err(if length < 16 {
                    Error::IncompleteHeader
                } else {
                    Error::IncompleteContent
                })
            );
        }
    }
    // The checksum is unknown until the entire header has been read. Even
    // a zero CRC in a complete header is not validation of a partial body.
    let mut filter = Filter::new();
    let mut output = decode_with(&mut filter, &raw[..16], 3);
    assert!(!filter.is_valid_crc32());
    filter.filter(&raw[16..], &mut output);
    assert_eq!(filter.validate(), Ok(()));
    assert_eq!(output, b"abcd");
}

#[test]
fn invalid_checksums_do_not_hide_decoded_bytes() {
    for mode in [Mode::Compressed, Mode::Uncompressed] {
        let mut input = if mode == Mode::Compressed {
            SIMPLE.to_vec()
        } else {
            document(SIMPLE_TEXT, 43, mode, 0)
        };
        input[12..16].copy_from_slice(&0x12345678u32.to_le_bytes());
        let mut filter = Filter::new();
        assert_eq!(decode_with(&mut filter, &input, 1), SIMPLE_TEXT);
        assert!(!filter.is_valid_crc32());
        // [MS-OXRTFCP] requires a zero CRC for UNCOMPRESSED, not a
        // computed checksum over its content.
        assert_eq!(
            filter.validate(),
            Err(Error::InvalidCrc32 {
                expected: 0x12345678,
                actual: if mode == Mode::Compressed {
                    0xa7c7c5f1
                } else {
                    0
                },
            })
        );
    }
}

#[test]
fn hostile_size_fields_never_drive_allocation_or_underflow() {
    // COMPSIZE is CONTENTS length plus twelve. RAWSIZE and COMPSIZE are
    // untrusted: no arithmetic difference or reservation should overflow,
    // and a claimed 1 GiB output must not allocate 1 GiB from a bare header.
    for size in [i32::MIN, -1, 0, 3, 11, 12, 0x40000000, i32::MAX] {
        for mode in [Mode::Compressed, Mode::Uncompressed] {
            let mut input = document(&[], size, mode, 0);
            let mut filter = Filter::new();
            let mut output = Vec::new();
            filter.filter(&input, &mut output);
            assert_eq!(output.capacity(), 0);
            assert_eq!(filter.validate(), Ok(()));
            input[..4].copy_from_slice(&size.to_le_bytes());
            filter.reset();
            filter.filter(&input, &mut output);
            assert_eq!(output.capacity(), 0);
            assert_eq!(
                filter.validate(),
                if size < 12 {
                    Err(Error::InvalidCompressedSize(size))
                } else if size == 12 {
                    Ok(())
                } else {
                    Err(Error::IncompleteContent)
                }
            );
        }
    }
}

#[test]
fn contents_boundaries_ignore_trailing_data_and_checksum_it_only_once() {
    // Remove the terminating reference, so only COMPSIZE marks the end.
    // Extra data must neither decompress nor be folded into the CRC.
    let contents = &SIMPLE[16..SIMPLE.len() - 2];
    let input = document(contents, 43, Mode::Compressed, crc(contents));
    let mut trailing = input.clone();
    trailing.extend(0..64);
    for bytes in [input, trailing] {
        check(&bytes, SIMPLE_TEXT, Mode::Compressed);
    }
    let mut early = SIMPLE.to_vec();
    early[..4].copy_from_slice(&14i32.to_le_bytes());
    early[12..16].copy_from_slice(&crc(&SIMPLE[16..18]).to_le_bytes());
    check(&early, &[], Mode::Compressed);
    let mut raw = document(b"bodyTRAILING", 99, Mode::Uncompressed, 0);
    raw[..4].copy_from_slice(&16i32.to_le_bytes());
    check(&raw, b"body", Mode::Uncompressed);
}

#[test]
fn end_marker_stops_before_declared_boundary_and_discards_later_input() {
    let mut input = SIMPLE.to_vec();
    input[..4].copy_from_slice(&1000i32.to_le_bytes());
    input.extend_from_slice(b"extra bytes");
    let mut filter = Filter::new();
    assert_eq!(decode_with(&mut filter, &input, 1), SIMPLE_TEXT);
    filter.validate().unwrap();
    let mut output = Vec::new();
    filter.filter(SIMPLE, &mut output);
    assert!(output.is_empty());
    filter.validate().unwrap();
}

#[test]
fn initializer_references_literals_and_overlapping_copies() {
    // A reference is valid before any literal: the dictionary starts with
    // the 207-byte initializer.
    let mut builder = Builder::new();
    builder.reference(0, 6);
    builder.end();
    check(&builder.build(), b"{\\rtf1", Mode::Compressed);
    let mut builder = Builder::new();
    builder.literals(b"{\\rtf1\\ansi hello, world}");
    builder.end();
    check(&builder.build(), &builder.expected, Mode::Compressed);
    let mut builder = Builder::new();
    let offset = builder.write_offset;
    builder.literals(b"ab");
    // Copy one byte at a time and write it back as we go, so this run may
    // legitimately overlap bytes it is producing.
    builder.reference(offset, 10);
    builder.end();
    assert_eq!(builder.expected, b"abababababab");
    check(&builder.build(), b"abababababab", Mode::Compressed);
}

#[test]
fn dictionary_reads_and_writes_wrap_at_four_kilobytes() {
    let mut builder = Builder::new();
    let data: Vec<_> = (0..10000).map(|i| b' ' + (i % 95) as u8).collect();
    builder.literals(&data);
    // More than 4096 writes must wrap, not run beyond the dictionary.
    assert_eq!(builder.write_offset, (207 + data.len()) % 4096);
    builder.reference(207, 17);
    builder.end();
    check(&builder.build(), &builder.expected, Mode::Compressed);
    let mut builder = Builder::new();
    builder.literals(
        &(0..4096 - 207)
            .map(|i| b'A' + (i % 26) as u8)
            .collect::<Vec<_>>(),
    );
    // The dictionary is exactly full and the write offset has wrapped to zero.
    assert_eq!(builder.write_offset, 0);
    // A reference near its end must wrap rather than read past it.
    builder.reference(4090, 17);
    builder.end();
    check(&builder.build(), &builder.expected, Mode::Compressed);
}

#[test]
fn reset_matches_a_fresh_dictionary_including_uninitialized_slots() {
    let mut filter = Filter::new();
    let mut first = Builder::new();
    first.literals(&[b'x'; 10000]);
    first.end();
    assert_eq!(decode_with(&mut filter, &first.build(), 7), first.expected);
    filter.reset();
    let mut second = Builder::new();
    second.reference(512, 17);
    second.end();
    // Reset must not let malformed references disclose a previous document.
    assert_eq!(decode_with(&mut filter, &second.build(), 1), [0; 17]);
    filter.validate().unwrap();
    filter.reset();
    assert_eq!(
        decode_with(
            &mut filter,
            &document(b"second", 6, Mode::Uncompressed, 0),
            1
        ),
        b"second"
    );
    filter.validate().unwrap();
}

#[test]
fn clone_snapshots_partial_headers_tokens_and_dictionary_without_aliasing() {
    for split in 0..SIMPLE.len() {
        let mut first = Filter::new();
        let mut expected = Vec::new();
        first.filter(&SIMPLE[..split], &mut expected);
        let mut second = first.clone();
        let mut actual = expected.clone();
        first.filter(&SIMPLE[split..], &mut expected);
        for byte in &SIMPLE[split..] {
            second.filter(&[*byte], &mut actual);
        }
        assert_eq!(actual, SIMPLE_TEXT);
        assert_eq!(actual, expected);
        first.reset();
        assert_eq!(second.validate(), Ok(()));
    }
}

#[test]
fn trait_objects_and_filtered_io_preserve_output_and_expose_validation() {
    let mut filter: Box<dyn MimeFilter> = Box::new(Filter::new());
    let mut output = Vec::new();
    filter.filter(SIMPLE, &mut output);
    filter.finish(&mut output);
    assert_eq!(output, SIMPLE_TEXT);
    filter.reset();
    let mut reader = FilteredReader::new(SIMPLE, Filter::new());
    let mut output = Vec::new();
    reader.read_to_end(&mut output).unwrap();
    reader.filter().validate().unwrap();
    assert_eq!(output, SIMPLE_TEXT);
    let mut writer = FilteredWriter::new(Vec::new(), Filter::new());
    for byte in SIMPLE {
        writer.write_all(&[*byte]).unwrap();
        writer.flush().unwrap();
    }
    writer.finish().unwrap();
    writer.filter().validate().unwrap();
    assert_eq!(writer.get_ref(), SIMPLE_TEXT);
    let mut reader = FilteredReader::new(&SIMPLE[..20], Filter::new());
    reader.read_to_end(&mut Vec::new()).unwrap();
    assert_eq!(reader.filter().validate(), Err(Error::IncompleteContent));
}

#[test]
fn validation_errors_have_diagnostics_and_no_hidden_sources() {
    use std::error::Error as _;
    for error in [
        Error::IncompleteHeader,
        Error::InvalidCompressedSize(-1),
        Error::UnsupportedCompressionMode(0x44434241),
        Error::IncompleteContent,
        Error::InvalidCrc32 {
            expected: 1,
            actual: 2,
        },
    ] {
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
}

// A deterministic PRNG replaces System.Random; mutation coverage and its
// productivity thresholds are preserved, not .NET's precise random sequence.
fn random(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn well_formed(seed: u32, length: usize) -> Vec<u8> {
    let mut state = seed;
    let data: Vec<_> = (0..length)
        .map(|_| 0x20 + (random(&mut state) % 95) as u8)
        .collect();
    let mut builder = Builder::new();
    builder.literals(&data);
    builder.reference(0, 17);
    builder.reference(207, 9);
    builder.end();
    builder.build()
}

fn invariants(input: &[u8], chunk: usize) -> Vec<u8> {
    let start = std::time::Instant::now();
    let mut filter = Filter::new();
    let output = decode_with(&mut filter, input, chunk);
    // The densest full run is a flag and eight two-byte references: seventeen
    // input bytes for 136 output bytes. Leave room for partial runs/dictionary.
    assert!(output.len() <= input.len() * 9 + 4096);
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    output
}

#[test]
fn fuzz_truncation_at_every_offset_and_hostile_headers() {
    // Header fields and tokens must tolerate a stream stopping part way through.
    let input = well_formed(1, 512);
    for length in 0..=input.len() {
        let expected = invariants(&input[..length], usize::MAX);
        assert_eq!(invariants(&input[..length], 1), expected);
    }
    for value in [0i32, i32::MIN, i32::MAX, -1, 11, 0x40000000] {
        for field in 0..4 {
            let mut mutated = input.clone();
            mutated[field * 4..field * 4 + 4].copy_from_slice(&value.to_le_bytes());
            invariants(&mutated, usize::MAX);
        }
    }
}

#[test]
fn fuzz_random_bit_flips_exercise_productive_decompression() {
    // Bit flips change literals into references and alter offsets and lengths.
    let input = well_formed(3, 1024);
    let mut state = 20260101;
    let mut productive = 0;
    for _ in 0..5000 {
        let mut mutated = input.clone();
        for _ in 0..1 + random(&mut state) % 16 {
            let index = random(&mut state) as usize % mutated.len();
            mutated[index] ^= 1 << (random(&mut state) % 8);
        }
        productive += usize::from(!invariants(&mutated, usize::MAX).is_empty());
    }
    // Rejecting everything would leave the test passing without exercising
    // dictionary copies, so retain MimeKit's productive-iteration threshold.
    assert!(productive > 2500, "{productive}");
}

#[test]
fn fuzz_random_bodies_small_chunks_and_unknown_modes() {
    let mut state = 20260102;
    let mut productive = 0;
    for _ in 0..2000 {
        let length = 1 + random(&mut state) as usize % 2048;
        let bytes: Vec<_> = (0..length).map(|_| random(&mut state) as u8).collect();
        let input = document(
            &bytes,
            random(&mut state) as i32,
            Mode::Compressed,
            random(&mut state),
        );
        productive += usize::from(!invariants(&input, usize::MAX).is_empty());
    }
    assert!(productive > 1000, "{productive}");
    let input = well_formed(4, 300);
    state = 20260103;
    for chunk in [1, 2, 5] {
        for _ in 0..250 {
            let mut mutated = input.clone();
            for _ in 0..1 + random(&mut state) % 8 {
                let index = random(&mut state) as usize % mutated.len();
                mutated[index] ^= 1 << (random(&mut state) % 8);
            }
            assert_eq!(
                invariants(&mutated, chunk),
                invariants(&mutated, usize::MAX)
            );
        }
    }
    state = 20260104;
    for _ in 0..500 {
        let mode = random(&mut state);
        let mut mutated = input.clone();
        mutated[8..12].copy_from_slice(&mode.to_le_bytes());
        assert!(invariants(&mutated, usize::MAX).is_empty());
    }
}

#[test]
fn benchmark_workloads_check_all_bytes_against_reference_encoder() {
    for name in support::CASES {
        let case = support::workload(name);
        case.check();
        assert!(!case.input.is_empty());
        assert_eq!(case.name, name);
    }
}

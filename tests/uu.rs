// Includes cases adapted from MimeKit's UUEncoderTests and UUDecoderTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{EncoderOptionsError, NewLineFormat, UuDecoder, UuEncoder};

fn newline(format: NewLineFormat) -> &'static [u8] {
    match format {
        NewLineFormat::Dos => b"\r\n",
        NewLineFormat::Unix => b"\n",
        NewLineFormat::Mixed => panic!("not an encoder format"),
    }
}

fn random_bytes(length: usize) -> Vec<u8> {
    let mut state = 0xf135_7ae1u32;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

// An independent, whole-body encoder using a packed 24-bit value. Allow all
// possible line lengths and either zero spelling to exercise tolerant decoding.
fn reference_encode(input: &[u8], width: usize, format: NewLineFormat, backtick: bool) -> Vec<u8> {
    let alphabet = |value: u8| {
        if value == 0 && backtick {
            b'`'
        } else {
            value + b' '
        }
    };
    let mut output = Vec::new();
    for line in input.chunks(width) {
        output.push(alphabet(line.len() as u8));
        for chunk in line.chunks(3) {
            let mut word = 0u32;
            for index in 0..3 {
                word = (word << 8) | u32::from(chunk.get(index).copied().unwrap_or(0));
            }
            for shift in [18, 12, 6, 0] {
                output.push(alphabet(((word >> shift) & 63) as u8));
            }
        }
        output.extend_from_slice(newline(format));
    }
    output.push(alphabet(0));
    output.extend_from_slice(newline(format));
    output
}

// Whole-input reference adapted from MimeKit's scalar decoder, deliberately
// retaining its packed saved bytes and arithmetic ranks rather than our table.
fn reference_decode(mut input: &[u8], payload_only: bool) -> Vec<u8> {
    if !payload_only {
        let mut offset = 0;
        let mut found = false;
        for line in input.split_inclusive(|&byte| byte == b'\n') {
            offset += line.len();
            if line.starts_with(b"begin ") && line.ends_with(b"\n") {
                found = true;
                break;
            }
        }
        if !found {
            return Vec::new();
        }
        input = &input[offset..];
    }
    let rank = |byte: u8| byte.wrapping_sub(b' ') & 63;
    let mut output = Vec::new();
    let mut remaining = 0;
    let mut eol = false;
    let mut saved = 0u32;
    let mut count = 0;
    for &byte in input {
        if byte == b'\r' {
            continue;
        }
        if byte == b'\n' {
            eol = true;
            continue;
        }
        if remaining == 0 || eol {
            remaining = rank(byte);
            eol = false;
            if remaining == 0 {
                break;
            }
            continue;
        }
        saved = (saved << 8) | u32::from(byte);
        count += 1;
        if count == 4 {
            let mut word = 0u32;
            for byte in saved.to_be_bytes() {
                word = (word << 6) | u32::from(rank(byte));
            }
            for shift in [16, 8, 0].into_iter().take(usize::from(remaining.min(3))) {
                output.push((word >> shift) as u8);
            }
            remaining = remaining.saturating_sub(3);
            saved = 0;
            count = 0;
        }
    }
    output
}

fn encode(input: &[u8], format: NewLineFormat, chunks: &[usize]) -> Vec<u8> {
    let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
    let mut output = Vec::new();
    let mut offset = 0;
    for &size in chunks.iter().cycle() {
        let end = offset + size.min(input.len() - offset);
        encoder.encode(&input[offset..end], &mut output);
        offset = end;
        if offset == input.len() {
            break;
        }
    }
    encoder.finish(&mut output);
    output
}

fn decoder(payload_only: bool) -> UuDecoder {
    if payload_only {
        UuDecoder::for_payload()
    } else {
        UuDecoder::new()
    }
}

fn decode(input: &[u8], payload_only: bool, size: usize) -> Vec<u8> {
    let mut decoder = decoder(payload_only);
    let mut output = Vec::new();
    for chunk in input.chunks(size) {
        decoder.decode(chunk, &mut output);
        decoder.decode(b"", &mut output);
    }
    output
}

#[test]
fn known_vectors_and_default_format() {
    assert_eq!(UuEncoder::new().new_line_format(), NewLineFormat::Dos);
    assert_eq!(UuEncoder::default().new_line_format(), NewLineFormat::Dos);
    let cases: &[(&[u8], &[u8])] = &[
        (b"", b"`\r\n"),
        (b"\0", b"!````\r\n`\r\n"),
        (b"\0\0", b"\"````\r\n`\r\n"),
        (b"\0\0\0", b"#````\r\n`\r\n"),
        (b"\xff", b"!_P``\r\n`\r\n"),
        (b"Cat", b"#0V%T\r\n`\r\n"),
        (b"abc", b"#86)C\r\n`\r\n"),
    ];
    for &(input, expected) in cases {
        assert_eq!(
            reference_encode(input, 45, NewLineFormat::Dos, true),
            expected
        );
        for chunk in 1..=8 {
            assert_eq!(encode(input, NewLineFormat::Dos, &[chunk]), expected);
            assert_eq!(decode(expected, true, chunk), input);
        }
    }
    assert_eq!(
        UuEncoder::with_new_line_format(NewLineFormat::Mixed).unwrap_err(),
        EncoderOptionsError::InvalidNewLineFormat(NewLineFormat::Mixed)
    );
}

#[test]
fn encoding_matches_reference_at_all_line_and_triplet_boundaries() {
    let input = random_bytes(10007);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in (0..=300).chain([449, 450, 451, 4499, 4500, 4501, 10007]) {
            let data = &input[..length];
            let expected = reference_encode(data, 45, format, true);
            assert_eq!(reference_decode(&expected, true), data);
            for size in [
                1,
                2,
                3,
                4,
                11,
                12,
                13,
                44,
                45,
                46,
                89,
                90,
                91,
                135,
                4096,
                usize::MAX,
            ] {
                assert_eq!(
                    encode(data, format, &[size]),
                    expected,
                    "length={length}, chunk={size}"
                );
            }
            // Saved one-byte and two-byte chunks must not reorder the payload.
            assert_eq!(encode(data, format, &[0, 1, 0, 2, 3, 45]), expected);
        }
    }
}

#[test]
fn full_lines_emit_immediately_but_partial_lines_wait_for_finish() {
    let mut encoder = UuEncoder::new();
    let mut output = Vec::new();
    encoder.encode(&[0; 44], &mut output);
    assert!(output.is_empty());
    encoder.encode(b"", &mut output);
    assert!(output.is_empty());
    encoder.encode(&[0], &mut output);
    let expected = [b"M".as_slice(), &[b'`'; 60], b"\r\n"].concat();
    assert_eq!(output, expected);
    encoder.finish(&mut output);
    assert_eq!(&output[expected.len()..], b"`\r\n");
}

#[test]
fn encoder_clone_reset_finish_and_output_reuse() {
    let input = random_bytes(137);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = reference_encode(&input, 45, format, true);
        let empty = [b"`".as_slice(), newline(format)].concat();
        for split in 0..=input.len() {
            let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
            let mut prefix = b"prefix".to_vec();
            encoder.encode(&input[..split], &mut prefix);
            let mut cloned = encoder.clone();
            for encoder in [&mut encoder, &mut cloned] {
                let mut output = prefix.clone();
                encoder.encode(&input[split..], &mut output);
                encoder.finish(&mut output);
                assert_eq!(&output[..6], b"prefix");
                assert_eq!(&output[6..], expected);
                output.clear();
                encoder.finish(&mut output);
                encoder.finish(&mut output);
                assert_eq!(output, empty.repeat(2));
                encoder.encode(b"discard", &mut output);
                encoder.reset();
                output.clear();
                encoder.encode(&input, &mut output);
                encoder.finish(&mut output);
                assert_eq!(output, expected);
                assert_eq!(encoder.new_line_format(), format);
            }
        }
        let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
        let mut output = Vec::new();
        let mut collected = Vec::new();
        for chunk in input.chunks(7) {
            encoder.encode(chunk, &mut output);
            collected.append(&mut output);
        }
        encoder.finish(&mut output);
        collected.append(&mut output);
        assert_eq!(collected, expected);
    }
}

#[test]
fn decoder_accepts_all_lengths_newlines_and_zero_spellings() {
    let data = random_bytes(253);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for backtick in [false, true] {
            for width in 1..=63 {
                for length in [width - 1, width, width * 3 + 1] {
                    let input = &data[..length];
                    let payload = reference_encode(input, width, format, backtick);
                    let wrapped = [
                        b"begin 644 file.bin".as_slice(),
                        newline(format),
                        &payload,
                        b"end",
                        newline(format),
                    ]
                    .concat();
                    for size in [1, 3, 13, 16, 32, 61, 64, 4096] {
                        assert_eq!(decode(&payload, true, size), input);
                        assert_eq!(decode(&wrapped, false, size), input);
                    }
                }
            }
        }
    }
}

#[test]
fn begin_marker_scanning_and_every_split() {
    let payload = b"#0V%T\r\n`\r\nend\r\n";
    let headers: &[&[u8]] = &[
        b"begin 644 cat\n",
        b"\n\nbegin ignored metadata\r\n",
        b"b\nbe\nbeg\nbegi\nbegin\nbegin\tbad\nbegin valid\n",
        b"Begin wrong\nxbegin wrong\nbegin wrong\rnot yet\n",
        b"x\0begin ignored\r\nbegin valid\n",
        b"begin \n",
    ];
    for &header in headers {
        let input = [header, payload].concat();
        for split in 0..=input.len() {
            let mut decoder = UuDecoder::new();
            let mut output = Vec::new();
            decoder.decode(&input[..split], &mut output);
            decoder.decode(b"", &mut output);
            decoder.decode(&input[split..], &mut output);
            assert_eq!(output, b"Cat");
        }
    }
    for header in [
        b"x\0begin 644 cat\r\n".as_slice(),
        b"\0begin 644 cat\r\n",
        b" begin 644 cat\n",
        b"Begin 644 cat\n",
        b"begin\t644 cat\n",
        b"begin",
        b"begin no newline",
    ] {
        let input = [header, payload].concat();
        for split in 0..=input.len() {
            let mut decoder = UuDecoder::default();
            let mut output = Vec::new();
            decoder.decode(&input[..split], &mut output);
            decoder.decode(&input[split..], &mut output);
            assert!(output.is_empty());
        }
    }
}

#[test]
fn malformed_lines_preserve_mimekit_state_across_every_split() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"$86)C\r\n#86)C\r\n`\r\n", b"abcabc"),
        (b"#8\r6)\rC\n`\n", b"abc"),
        (b"#86\n#)C\n`\n", b"abc"),
        (b"!86)C\n`\n", b"a"),
        (b"\"86)C\n`\n", b"ab"),
        (b"#86)C#86)C`\n", b"abcabc"),
        (b"#86)C \n#0V%T\n", b"abc"),
        (b"#86)\n`\n", b""),
    ];
    for &(input, expected) in cases {
        assert_eq!(reference_decode(input, true), expected);
        for split in 0..=input.len() {
            let mut decoder = UuDecoder::for_payload();
            let mut output = Vec::new();
            decoder.decode(&input[..split], &mut output);
            decoder.decode(&input[split..], &mut output);
            assert_eq!(output, expected, "input={input:?}, split={split}");
        }
    }
}

#[test]
fn every_byte_mutation_matches_mimekit_scalar_reference() {
    let original = reference_encode(&random_bytes(17), 7, NewLineFormat::Dos, true);
    for byte in 0..=255 {
        for position in 0..original.len() {
            for insert in [false, true] {
                let mut input = original.clone();
                if insert {
                    input.insert(position, byte);
                } else {
                    input[position] = byte;
                }
                let expected = reference_decode(&input, true);
                for size in [1, 7, 33, 4096] {
                    assert_eq!(
                        decode(&input, true, size),
                        expected,
                        "byte={byte}, position={position}, insert={insert}, chunk={size}"
                    );
                }
            }
        }
    }
}

#[test]
fn equivalent_noncanonical_sextets_decode_identically() {
    let input: Vec<u8> = (0..=255).collect();
    let original = reference_encode(&input, 45, NewLineFormat::Unix, true);
    for offset in [0u8, 64, 128, 192] {
        let encoded: Vec<u8> = original
            .iter()
            .map(|&byte| {
                let alternative = byte.wrapping_add(offset);
                if byte == b'\n' || matches!(alternative, b'\r' | b'\n') {
                    byte
                } else {
                    alternative
                }
            })
            .collect();
        for size in [1, 7, 61, 4096] {
            assert_eq!(decode(&encoded, true, size), input);
        }
    }
}

#[test]
fn decoder_clone_reset_termination_and_output_reuse() {
    let data = random_bytes(137);
    let payload = reference_encode(&data, 45, NewLineFormat::Dos, true);
    for payload_only in [false, true] {
        let input = if payload_only {
            payload.clone()
        } else {
            [
                b"not a header\nbeg\nbegin 644 data\r\n".as_slice(),
                &payload,
                b"end\n",
            ]
            .concat()
        };
        for split in 0..=input.len() {
            let mut decoder = decoder(payload_only);
            let mut prefix = b"prefix".to_vec();
            decoder.decode(&input[..split], &mut prefix);
            let mut cloned = decoder.clone();
            for decoder in [&mut decoder, &mut cloned] {
                let mut output = prefix.clone();
                decoder.decode(&input[split..], &mut output);
                assert_eq!(&output[..6], b"prefix");
                assert_eq!(&output[6..], data);
                let length = output.len();
                decoder.decode(&input, &mut output);
                assert_eq!(output.len(), length);
                decoder.reset();
                output.clear();
                decoder.decode(&input[..split], &mut output);
                decoder.reset();
                output.clear();
                decoder.decode(&input, &mut output);
                assert_eq!(output, data);
            }
        }
        let mut decoder = decoder(payload_only);
        let mut output = Vec::new();
        let mut collected = Vec::new();
        for byte in &input {
            decoder.decode(std::slice::from_ref(byte), &mut output);
            collected.append(&mut output);
        }
        assert_eq!(collected, data);
    }
}

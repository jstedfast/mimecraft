// Includes cases adapted from MimeKit's YEncoderTests, YDecoderTests, and
// YEncodingTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::encodings::{EncoderOptionsError, NewLineFormat, YDecoder, YEncoder};

const SPECIAL: &[u8] = &[214, 224, 227, 19, 4, 223, 23];
const SPECIAL_ENCODED: &[u8] = b"=@=J=M=}=n=IA";

fn newline(format: NewLineFormat) -> &'static [u8] {
    match format {
        NewLineFormat::Dos => b"\r\n",
        NewLineFormat::Unix => b"\n",
        NewLineFormat::Mixed => panic!("invalid encoder format"),
    }
}

// Independent bit-at-a-time CRC, not the production table.
fn update_crc(mut crc: u32, input: &[u8]) -> u32 {
    for &byte in input {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 * (crc & 1));
        }
    }
    crc
}

fn checksum(input: &[u8]) -> u32 {
    update_crc(u32::MAX, input)
}

fn reference_encode(input: &[u8], width: usize, format: NewLineFormat) -> Vec<u8> {
    let mut output = Vec::new();
    let mut line = Vec::new();
    for &byte in input {
        // Select escapes from original input values, not the shifted alphabet.
        match byte {
            214 => line.extend_from_slice(b"=@"),
            223 => line.extend_from_slice(b"=I"),
            224 => line.extend_from_slice(b"=J"),
            227 => line.extend_from_slice(b"=M"),
            19 => line.extend_from_slice(b"=}"),
            4 => line.extend_from_slice(b"=n"),
            _ => line.push(((u16::from(byte) + 42) % 256) as u8),
        }
        if line.len() >= width {
            output.append(&mut line);
            output.extend_from_slice(newline(format));
        }
    }
    if !line.is_empty() {
        output.append(&mut line);
        output.extend_from_slice(newline(format));
    }
    output
}

// Whole-input line parsing, independently of the incremental marker states.
// Retain MimeKit's optional-part scanner's handling of malformed candidates.
fn reference_decode(mut input: &[u8], payload_only: bool) -> Vec<u8> {
    if !payload_only {
        let mut offset = 0;
        let mut found = false;
        for line in input.split_inclusive(|&byte| byte == b'\n') {
            offset += line.len();
            if line.starts_with(b"=ybegin ") && line.ends_with(b"\n") {
                found = true;
                break;
            }
        }
        if !found {
            return Vec::new();
        }
        input = &input[offset..];
        while input.first() == Some(&b'=') {
            if input.len() < 2 || input.starts_with(b"=ye") {
                return Vec::new();
            }
            if input[1] != b'y' {
                break;
            }
            let part = input.starts_with(b"=ypart ");
            let Some(end) = input.iter().position(|&byte| byte == b'\n') else {
                return Vec::new();
            };
            input = &input[end + 1..];
            if part {
                break;
            }
        }
    }

    let mut output = Vec::new();
    let mut escaped = false;
    let mut start_of_line = true;
    for &byte in input {
        match byte {
            b'\r' | b'\n' => {
                escaped = false;
                start_of_line |= byte == b'\n';
            }
            b'=' if !escaped => escaped = true,
            b'y' if escaped && start_of_line => break,
            _ => {
                let offset = if escaped { 106 } else { 42 };
                output.push(((u16::from(byte) + 256 - offset) % 256) as u8);
                escaped = false;
                start_of_line = false;
            }
        }
    }
    output
}

fn decoder(payload_only: bool) -> YDecoder {
    if payload_only {
        YDecoder::for_payload()
    } else {
        YDecoder::new()
    }
}

fn encode(input: &[u8], width: usize, format: NewLineFormat, chunk_size: usize) -> Vec<u8> {
    let mut encoder = YEncoder::with_options(width, format).unwrap();
    let mut output = Vec::new();
    let mut crc = u32::MAX;
    for chunk in input.chunks(chunk_size) {
        encoder.encode(chunk, &mut output);
        encoder.encode(b"", &mut output);
        crc = update_crc(crc, chunk);
        assert_eq!(encoder.checksum(), crc);
    }
    encoder.finish(&mut output);
    assert_eq!(encoder.checksum(), crc);
    let length = output.len();
    encoder.finish(&mut output);
    assert_eq!(output.len(), length);
    output
}

fn decode(input: &[u8], payload_only: bool, chunk_size: usize) -> Vec<u8> {
    let mut decoder = decoder(payload_only);
    let mut output = Vec::new();
    let mut crc = u32::MAX;
    for chunk in input.chunks(chunk_size) {
        let start = output.len();
        decoder.decode(chunk, &mut output);
        decoder.decode(b"", &mut output);
        crc = update_crc(crc, &output[start..]);
        assert_eq!(decoder.checksum(), crc);
    }
    output
}

#[test]
fn defaults_options_and_validation_order() {
    for encoder in [YEncoder::new(), YEncoder::default()] {
        assert_eq!(encoder.new_line_format(), NewLineFormat::Dos);
        assert_eq!(encoder.line_length(), 128);
        assert_eq!(encoder.checksum(), u32::MAX);
    }
    assert_eq!(YDecoder::default().checksum(), u32::MAX);
    assert_eq!(YDecoder::for_payload().checksum(), u32::MAX);
    for width in 60..=998 {
        let encoder = YEncoder::with_line_length(width).unwrap();
        assert_eq!(encoder.line_length(), width);
        assert_eq!(encoder.new_line_format(), NewLineFormat::Dos);
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let encoder = YEncoder::with_options(width, format).unwrap();
            assert_eq!(encoder.line_length(), width);
            assert_eq!(encoder.new_line_format(), format);
        }
    }
    for width in [0, 1, 59, 999, usize::MAX] {
        assert_eq!(
            YEncoder::with_line_length(width).unwrap_err().requested(),
            width
        );
        assert!(matches!(
            YEncoder::with_options(width, NewLineFormat::Unix),
            Err(EncoderOptionsError::InvalidLineLength(error)) if error.requested() == width
        ));
        assert!(matches!(
            YEncoder::with_options(width, NewLineFormat::Mixed),
            Err(EncoderOptionsError::InvalidNewLineFormat(
                NewLineFormat::Mixed
            ))
        ));
    }
    assert!(YEncoder::with_options(128, NewLineFormat::Mixed).is_err());
    assert!(YEncoder::with_new_line_format(NewLineFormat::Mixed).is_err());
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let encoder = YEncoder::with_new_line_format(format).unwrap();
        assert_eq!(encoder.new_line_format(), format);
        assert_eq!(encoder.line_length(), 128);
    }
}

#[test]
fn source_vectors_and_finalized_crc() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = [SPECIAL_ENCODED, newline(format)].concat();
        for size in 1..=20 {
            assert_eq!(encode(SPECIAL, 128, format, size), expected);
            assert_eq!(decode(&expected, true, size), SPECIAL);
        }
        let input = vec![214; 93];
        let full_line = [b"=@".repeat(30), newline(format).to_vec()].concat();
        let expected = [
            full_line.repeat(3),
            b"=@=@=@".to_vec(),
            newline(format).to_vec(),
        ]
        .concat();
        assert_eq!(encode(&input, 60, format, 7), expected);
        assert!(encode(b"", 128, format, 1).is_empty());
    }
    let mut encoder = YEncoder::new();
    let mut encoded = Vec::new();
    encoder.encode(b"123456789", &mut encoded);
    encoder.finish(&mut encoded);
    assert_eq!(!encoder.checksum(), 0xcbf4_3926);
    assert_eq!(encoded, b"[\\]^_`abc\r\n");
    let mut decoder = YDecoder::for_payload();
    let mut output = Vec::new();
    decoder.decode(&encoded, &mut output);
    assert_eq!(output, b"123456789");
    assert_eq!(!decoder.checksum(), 0xcbf4_3926);
}

#[test]
fn all_bytes_wrapping_and_chunk_boundaries() {
    let input: Vec<u8> = (0..=255).cycle().take(4097).collect();
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for width in [60, 61, 76, 127, 128, 129, 253, 254, 255, 256, 998] {
            let expected = reference_encode(&input, width, format);
            for chunk in [1, 2, 3, 7, 59, 60, 127, 128, 255, 998, 4096, 8192] {
                assert_eq!(encode(&input, width, format, chunk), expected);
                assert_eq!(decode(&expected, true, chunk), input);
            }
            for line in expected.split_inclusive(|&byte| byte == b'\n') {
                let payload = &line[..line.len() - newline(format).len()];
                assert!(payload.len() <= width + 1);
                assert!(!payload.ends_with(b"="));
            }
        }
    }
    // This known CRC exercises every entry in the table over a full byte range.
    let all_bytes: Vec<u8> = (0..=255).collect();
    assert_eq!(!checksum(&all_bytes), 0x2905_8c73);
    let mut encoder = YEncoder::new();
    encoder.encode(&all_bytes, &mut Vec::new());
    assert_eq!(!encoder.checksum(), 0x2905_8c73);
}

#[test]
fn escape_pairs_can_extend_every_configured_width_by_one() {
    for width in 60..=998 {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let mut input = vec![23; width - 1];
            input.extend_from_slice(&[214, 23]);
            let expected = [
                vec![b'A'; width - 1],
                b"=@".to_vec(),
                newline(format).to_vec(),
                b"A".to_vec(),
                newline(format).to_vec(),
            ]
            .concat();
            assert_eq!(encode(&input, width, format, width - 1), expected);
        }
    }
}

#[test]
fn finish_retains_checksum_and_reset_starts_independent_body() {
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let mut encoder = YEncoder::with_options(60, format).unwrap();
        let mut output = b"prefix".to_vec();
        encoder.finish(&mut output);
        assert_eq!(output, b"prefix");
        assert_eq!(encoder.checksum(), u32::MAX);
        encoder.encode(&[23; 60], &mut output);
        assert_eq!(
            &output[6..],
            [vec![b'A'; 60], newline(format).to_vec()].concat()
        );
        let before_finish = output.clone();
        encoder.finish(&mut output);
        assert_eq!(output, before_finish);
        assert_eq!(encoder.checksum(), checksum(&[23; 60]));
        output.clear();
        encoder.encode(SPECIAL, &mut output);
        encoder.finish(&mut output);
        assert_eq!(encoder.checksum(), checksum(&[&[23; 60], SPECIAL].concat()));
        assert_eq!(output, [SPECIAL_ENCODED, newline(format)].concat());
        encoder.reset();
        assert_eq!(encoder.checksum(), u32::MAX);
        assert_eq!(encoder.line_length(), 60);
        assert_eq!(encoder.new_line_format(), format);
        output.clear();
        encoder.encode(SPECIAL, &mut output);
        encoder.reset(); // Do not emit the abandoned line's final newline.
        encoder.finish(&mut output);
        assert_eq!(output, SPECIAL_ENCODED);
        assert_eq!(encoder.checksum(), u32::MAX);
    }
}

#[test]
fn encoder_clone_and_drained_output_at_every_split() {
    let input: Vec<u8> = (0..=255).cycle().take(389).collect();
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = reference_encode(&input, 61, format);
        for split in 0..=input.len() {
            let mut encoder = YEncoder::with_options(61, format).unwrap();
            let mut prefix = Vec::new();
            encoder.encode(&input[..split], &mut prefix);
            let mut clone = encoder.clone();
            let mut suffix = Vec::new();
            let mut cloned_suffix = Vec::new();
            encoder.encode(&input[split..], &mut suffix);
            encoder.finish(&mut suffix);
            for chunk in input[split..].chunks(7) {
                clone.encode(chunk, &mut cloned_suffix);
            }
            clone.finish(&mut cloned_suffix);
            assert_eq!(suffix, cloned_suffix);
            assert_eq!([prefix, suffix].concat(), expected);
            assert_eq!(clone.checksum(), checksum(&input));
            assert_eq!(encoder.checksum(), clone.checksum());
            clone.reset();
            assert_eq!(clone.checksum(), u32::MAX);
            assert_eq!(clone.new_line_format(), format);
            assert_eq!(clone.line_length(), 61);
            assert_eq!(encoder.checksum(), checksum(&input));
        }
    }
}

#[test]
fn begin_and_part_headers_at_every_split() {
    let source = b"ignored preface\n=ybegin part=1 line=128 size=7 name=test.bin\n=ypart begin=1 end=7\n=@=J=M=}=n=IA\n=yend size=7\ntrailing";
    for input in [
        source.to_vec(),
        source
            .split(|&byte| byte == b'\n')
            .collect::<Vec<_>>()
            .join(&b"\r\n"[..]),
    ] {
        assert_eq!(reference_decode(&input, false), SPECIAL);
        for split in 0..=input.len() {
            let mut decoder = YDecoder::new();
            let mut output = Vec::new();
            decoder.decode(&input[..split], &mut output);
            decoder.decode(b"", &mut output);
            decoder.decode(&input[split..], &mut output);
            assert_eq!(output, SPECIAL, "split {split}");
            assert_eq!(decoder.checksum(), checksum(SPECIAL));
        }
        for size in 1..=input.len() {
            assert_eq!(decode(&input, false, size), SPECIAL, "chunk {size}");
        }
    }
}

#[test]
fn malformed_header_transitions_match_mimekit() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"=xcontent", b"8content"),
        (b"=yxcontent", b""),
        (b"=ypxcontent", b""),
        (b"=ypaxcontent", b""),
        (b"=yparxcontent", b""),
        (b"=ypartxcontent", b""),
        (b"=ypart begin=1 end=7\ncontent", b"content"),
        (b"=ypartx\ncontent", b"content"),
        (b"=yX\n=ybad\n=ypart \ncontent", b"content"),
        (b"=ye", b""),
        (b"=yeanything\ncontent", b""),
        (b"=@", b"=@"),
        (b"==@", b"==@"),
        (b"=ypart \n=@", b"=@"),
    ];
    for &(suffix, payload) in cases {
        let input = [
            b"=ybegin part=1 line=128 size=7 name=test.bin\n".as_slice(),
            suffix,
        ]
        .concat();
        let expected = reference_decode(payload, true);
        assert_eq!(reference_decode(&input, false), expected);
        for chunk in [1, 2, 7, input.len()] {
            assert_eq!(decode(&input, false, chunk), expected, "{suffix:?}");
        }
    }
    for preface in [
        b"=YBEGIN \n".as_slice(),
        b"x=ybegin \n",
        b"=ybeginX\n",
        b"=yb\0=ybegin \n",
        b"=ybegin\t\n",
        b"=ybegin\r\n",
        b"=ybegin \r",
    ] {
        let input = [preface, b"=ybegin \n=ypart \nABC"].concat();
        let expected = reference_decode(&input, false);
        for chunk in [1, 3, 7, input.len()] {
            assert_eq!(decode(&input, false, chunk), expected, "{preface:?}");
        }
    }
    for input in [
        b"".as_slice(),
        b"=",
        b"=ybegin ",
        b"=ybegin \n=",
        b"=ybegin \n=y",
        b"=ybegin \n=ypart ",
    ] {
        assert!(decode(input, false, 1).is_empty());
    }
}

#[test]
fn payload_escapes_line_breaks_and_early_termination() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"=@", &[214]),
        (b"==", &[211]),
        (b"=\r@", &[22]),
        (b"=\n@", &[22]),
        (b"=\r\n@", &[22]),
        (b"A=yB", &[23, 15, 24]),
        (b"A\r=yB", &[23, 15, 24]),
        (b"A\n=yB", &[23]),
        (b"=y", b""),
        (b"=\ryA", &[79, 23]),
        (b"=\nyA", &[79, 23]),
        (b"ABC=", &[23, 24, 25]),
        (b"\0\xff", &[214, 213]),
        (b"ABC\n=ygarbage\nmore", &[23, 24, 25]),
    ];
    for &(input, expected) in cases {
        assert_eq!(reference_decode(input, true), expected);
        for size in 1..=input.len() + 1 {
            assert_eq!(decode(input, true, size), expected);
        }
    }
}

#[test]
fn framed_round_trip_every_initial_byte_and_split() {
    for byte in 0..=255 {
        let encoded = [
            b"=ybegin line=128 size=1 name=test.bin\r\n".to_vec(),
            reference_encode(&[byte], 128, NewLineFormat::Dos),
            b"=yend size=1\r\n".to_vec(),
        ]
        .concat();
        for split in 0..=encoded.len() {
            let mut decoder = YDecoder::new();
            let mut output = Vec::new();
            decoder.decode(&encoded[..split], &mut output);
            let mut clone = decoder.clone();
            let mut cloned = output.clone();
            decoder.decode(&encoded[split..], &mut output);
            clone.decode(&encoded[split..], &mut cloned);
            assert_eq!(output, [byte], "byte {byte}, split {split}");
            assert_eq!(cloned, output);
            assert_eq!(decoder.checksum(), checksum(&[byte]));
            assert_eq!(clone.checksum(), decoder.checksum());
        }
    }
}

#[test]
fn decoder_clone_reset_checksum_and_output_reuse() {
    for payload_only in [false, true] {
        let input = if payload_only {
            b"=@=J=M=}=n=IA\n=yend size=7\ntrailing".to_vec()
        } else {
            b"preface\n=ybegin \n=ypart \n=@=J=M=}=n=IA\n=yend size=7\ntrailing".to_vec()
        };
        for split in 0..=input.len() {
            let mut decoder = decoder(payload_only);
            let mut prefix = Vec::new();
            decoder.decode(&input[..split], &mut prefix);
            let mut clone = decoder.clone();
            let mut suffix = b"existing".to_vec();
            decoder.decode(&input[split..], &mut suffix);
            let mut cloned_suffix = Vec::new();
            for chunk in input[split..].chunks(3) {
                clone.decode(chunk, &mut cloned_suffix);
            }
            assert_eq!(&suffix[8..], cloned_suffix);
            prefix.extend_from_slice(&suffix[8..]);
            assert_eq!(prefix, SPECIAL);
            assert_eq!(decoder.checksum(), checksum(SPECIAL));
            assert_eq!(clone.checksum(), decoder.checksum());
            suffix.clear();
            decoder.decode(&input, &mut suffix);
            assert!(suffix.is_empty()); // Ended streams do not restart implicitly.
            decoder.reset();
            assert_eq!(decoder.checksum(), u32::MAX);
            decoder.decode(&input, &mut suffix);
            assert_eq!(suffix, SPECIAL);
            clone.reset();
            cloned_suffix.clear();
            clone.decode(SPECIAL_ENCODED, &mut cloned_suffix);
            assert_eq!(cloned_suffix, if payload_only { SPECIAL } else { b"" });
        }
    }
}

#[test]
fn every_two_byte_payload_and_pending_state() {
    for a in 0..=255 {
        for b in 0..=255 {
            let input = [a, b];
            let mut decoder = YDecoder::for_payload();
            let mut output = Vec::new();
            decoder.decode(&input[..1], &mut output);
            decoder.decode(&input[1..], &mut output);
            assert_eq!(output, reference_decode(&input, true));
            let continuation = b"AB\r\n=yend\nignored";
            decoder.decode(continuation, &mut output);
            let expected = reference_decode(&[input.as_slice(), continuation].concat(), true);
            assert_eq!(output, expected, "{input:?}");
            assert_eq!(decoder.checksum(), checksum(&expected));
        }
    }
}

#[test]
fn every_byte_mutation_across_header_and_payload_boundaries() {
    let original = b"=ybegin x\n=ypart x\n=@ABC=J\n=yend\n";
    let continuation = b"\n=ybegin \n=ypart \nABC\n=yend\n";
    for position in 0..=original.len() {
        for byte in 0..=255 {
            for insert in [false, true] {
                let mut input = original.to_vec();
                if insert || position == input.len() {
                    input.insert(position, byte);
                } else {
                    input[position] = byte;
                }
                let expected = reference_decode(&input, false);
                for size in [1, 7, input.len()] {
                    assert_eq!(
                        decode(&input, false, size),
                        expected,
                        "{position}/{byte}/{insert}"
                    );
                }
                // A continuation exposes hidden partial-header and escape state.
                let mut decoder = YDecoder::new();
                let mut output = Vec::new();
                decoder.decode(&input, &mut output);
                decoder.decode(continuation, &mut output);
                input.extend_from_slice(continuation);
                assert_eq!(output, reference_decode(&input, false));
                assert_eq!(decoder.checksum(), checksum(&output));
            }
        }
    }
}

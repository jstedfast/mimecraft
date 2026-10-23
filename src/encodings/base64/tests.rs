// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;

// Exercise every supported code path directly, regardless of which backend
// the public encode/decode methods would choose on this machine.
fn backends() -> Vec<Backend> {
    let result = vec![Backend::Scalar];
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    let result = {
        let mut result = result;
        result.extend(
            simd::Accelerator::available()
                .into_iter()
                .map(Backend::Simd),
        );
        result
    };
    result
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

fn encode(input: &[u8], width: usize, chunks: &[usize], backend: Backend) -> Vec<u8> {
    encode_with_new_line_format(input, width, chunks, backend, NewLineFormat::Dos)
}

fn encode_with_new_line_format(
    input: &[u8],
    width: usize,
    chunks: &[usize],
    backend: Backend,
    new_line_format: NewLineFormat,
) -> Vec<u8> {
    let mut encoder = Base64Encoder::with_options(width, new_line_format).unwrap();
    let mut output = b"prefix".to_vec();
    let mut index = 0;
    for &size in chunks.iter().cycle() {
        let end = (index + size).min(input.len());
        encoder.encode_with_backend(&input[index..end], &mut output, backend);
        index = end;
        if index == input.len() {
            break;
        }
    }
    encoder.finish(&mut output);
    let length = output.len();
    encoder.finish(&mut output);
    assert_eq!(output.len(), length);
    output
}

fn decode(input: &[u8], chunk: usize, backend: Backend) -> (Vec<u8>, (u32, u8, u8)) {
    let mut decoder = Base64Decoder::new();
    let mut output = b"prefix".to_vec();
    for bytes in input.chunks(chunk) {
        decoder.decode_with_backend(bytes, &mut output, backend);
        decoder.decode_with_backend(b"", &mut output, backend);
    }
    (output, (decoder.value, decoder.padding, decoder.count))
}

#[test]
fn unwrapped_encoder_backends_match_scalar_and_mime_payloads() {
    let data = random_bytes(10040);
    let backends = backends();
    eprintln!("Unwrapped Base64 backends: {backends:?}");
    for length in (0..=200).chain([511, 512, 513, 4095, 4096, 4097, 10007]) {
        for offset in 0..32 {
            // Exact-sized allocations also cover vector lookahead at the end.
            let input = data[offset..offset + length].to_vec().into_boxed_slice();
            let mut expected = b"prefix".to_vec();
            encode_unwrapped_with_backend(&input, &mut expected, Backend::Scalar);
            let mut mime = encode(&input, 76, &[usize::MAX], Backend::Scalar);
            mime.retain(|byte| !matches!(byte, b'\r' | b'\n'));
            assert_eq!(expected, mime);
            for &backend in &backends {
                let mut actual = b"prefix".to_vec();
                encode_unwrapped_with_backend(&input, &mut actual, backend);
                encode_unwrapped_with_backend(b"", &mut actual, backend);
                assert_eq!(
                    actual, expected,
                    "{backend:?}, length {length}, offset {offset}"
                );

                // Exercise genuinely unaligned pointers separately from the
                // exact-sized allocations above.
                actual.truncate(6);
                encode_unwrapped_with_backend(&data[offset..offset + length], &mut actual, backend);
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn rank_table_matches_original_classification_for_every_byte() {
    for byte in 0..=255u8 {
        let expected = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 0,
            _ => 0xff,
        };
        assert_eq!(RANK[byte as usize], expected);
    }
}

#[test]
fn every_encoder_backend_matches_scalar_at_block_and_line_boundaries() {
    let data = random_bytes(10007);
    let lengths = (0..=200).chain([569, 570, 571, 4559, 4560, 4561, 10007]);
    let backends = backends();
    eprintln!("Testing backends: {backends:?}");
    for length in lengths {
        for width in [60, 63, 64, 72, 75, 76, 998] {
            let input = &data[..length];
            let expected = encode(input, width, &[usize::MAX], Backend::Scalar);
            let expected_lf: Vec<u8> = expected.iter().copied().filter(|&b| b != b'\r').collect();
            for &backend in &backends {
                for chunk in [
                    1, 2, 3, 11, 12, 13, 15, 16, 17, 23, 24, 25, 27, 28, 31, 32, 33, 48, 55, 56,
                    57, 58, 4096,
                ] {
                    assert_eq!(
                        encode(input, width, &[chunk], backend),
                        expected,
                        "{backend:?}, length={length}, width={width}, chunk={chunk}"
                    );
                    assert_eq!(
                        encode_with_new_line_format(
                            input,
                            width,
                            &[chunk],
                            backend,
                            NewLineFormat::Unix
                        ),
                        expected_lf,
                        "{backend:?}, LF, length={length}, width={width}, chunk={chunk}"
                    );
                }
                assert_eq!(
                    encode(input, width, &[0, 1, 0, 28, 3, 57, 4096], backend),
                    expected
                );
            }
        }
    }
}

#[test]
fn every_decoder_backend_handles_every_byte_at_every_position() {
    let data = random_bytes(72);
    let encoded: Vec<u8> = encode(&data, 76, &[72], Backend::Scalar)[6..]
        .iter()
        .copied()
        .filter(|&byte| !matches!(byte, b'\r' | b'\n'))
        .collect();
    for byte in 0..=255u8 {
        for position in 0..encoded.len() {
            for insert in [false, true] {
                let mut input = encoded.clone();
                if insert {
                    input.insert(position, byte);
                } else {
                    input[position] = byte;
                }
                let expected = decode(&input, input.len(), Backend::Scalar);
                for backend in backends() {
                    for chunk in [1, 17, 33, 4096] {
                        assert_eq!(
                            decode(&input, chunk, backend),
                            expected,
                            "{backend:?}, byte={byte}, position={position}, insert={insert}, chunk={chunk}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn every_decoder_backend_handles_random_noise_and_concatenated_blocks() {
    let random = random_bytes(8192);
    let mut input = Vec::new();
    for (index, chunk) in random.chunks(71).enumerate() {
        let encoded = encode(chunk, 76, &[71], Backend::Scalar);
        input.extend_from_slice(&encoded[6..]);
        input.extend_from_slice(&random[index..index + 13]);
        input.extend_from_slice(b"\r\n!====\xff\0\t");
    }
    for data in [&random, &input] {
        let expected = decode(data, data.len(), Backend::Scalar);
        for backend in backends() {
            for chunk in [1, 2, 3, 4, 15, 16, 17, 31, 32, 33, 77, 4096] {
                assert_eq!(
                    decode(data, chunk, backend),
                    expected,
                    "{backend:?}, chunk={chunk}"
                );
            }
        }
    }
}

#[test]
fn unaligned_input_slices_and_output_prefixes() {
    let bytes = random_bytes(512);
    for offset in 0..32 {
        for length in 0..=160 {
            let input = &bytes[offset..offset + length];
            let expected = encode(input, 76, &[512], Backend::Scalar);
            for backend in backends() {
                assert_eq!(encode(input, 76, &[512], backend), expected);
                let mut padded = vec![b'!'; offset];
                padded.extend_from_slice(&expected[6..]);
                let encoded = &padded[offset..];
                assert_eq!(
                    decode(encoded, 512, backend),
                    decode(encoded, 512, Backend::Scalar)
                );
            }
        }
    }
}

#[test]
fn backend_switching_clone_and_reset_preserve_streaming_state() {
    let input = random_bytes(201);
    let encoded = encode(&input, 60, &[512], Backend::Scalar);
    for backend in backends() {
        for split in 0..=input.len() {
            let mut encoder = Base64Encoder::with_line_length(60).unwrap();
            let mut output = b"prefix".to_vec();
            encoder.encode_with_backend(&input[..split], &mut output, Backend::Scalar);
            let mut cloned = encoder.clone();
            let mut other = output.clone();
            encoder.encode_with_backend(&input[split..], &mut output, backend);
            cloned.encode_with_backend(&input[split..], &mut other, Backend::Scalar);
            encoder.finish(&mut output);
            cloned.finish(&mut other);
            assert_eq!(output, encoded);
            assert_eq!(output, other);
            encoder.encode_with_backend(b"xx", &mut output, backend);
            encoder.reset();
            output.clear();
            encoder.encode_with_backend(&input, &mut output, backend);
            encoder.finish(&mut output);
            assert_eq!(output, &encoded[6..]);
        }
        for split in 0..=encoded.len() - 6 {
            let bytes = &encoded[6..];
            let mut decoder = Base64Decoder::new();
            let mut output = Vec::new();
            decoder.decode_with_backend(&bytes[..split], &mut output, backend);
            let mut cloned = decoder.clone();
            let mut other = output.clone();
            decoder.decode_with_backend(&bytes[split..], &mut output, Backend::Scalar);
            cloned.decode_with_backend(&bytes[split..], &mut other, backend);
            assert_eq!(output, input);
            assert_eq!(output, other);
            decoder.decode_with_backend(b"Zg=", &mut output, backend);
            decoder.reset();
            output.clear();
            decoder.decode_with_backend(bytes, &mut output, backend);
            assert_eq!(output, input);
        }
    }
}

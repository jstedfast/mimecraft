// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;

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
    let mut state = 0x1234_5678u32;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

fn encoded(input: &[u8], format: NewLineFormat) -> Vec<u8> {
    let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
    let mut output = Vec::new();
    encoder.encode_with_backend(input, &mut output, Backend::Scalar);
    encoder.finish(&mut output);
    output
}

fn assert_decoder_state(actual: &UuDecoder, expected: &UuDecoder) {
    assert_eq!(actual.initial, expected.initial);
    assert_eq!(actual.state, expected.state);
    assert_eq!(actual.remaining, expected.remaining);
    assert_eq!(actual.end_of_line, expected.end_of_line);
    assert_eq!(actual.pending_len, expected.pending_len);
    // Only the occupied prefix is live state; vectorized quartets need not
    // overwrite stale bytes from previously completed scalar quartets.
    assert_eq!(
        actual.pending[..actual.pending_len],
        expected.pending[..expected.pending_len]
    );
}

fn compare_decode(input: &[u8], chunks: &[usize], payload: bool, backend: Backend) {
    let mut scalar = if payload {
        UuDecoder::for_payload()
    } else {
        UuDecoder::new()
    };
    let mut actual = scalar.clone();
    let mut expected_output = b"prefix".to_vec();
    let mut actual_output = expected_output.clone();
    let mut offset = 0;
    for &size in chunks.iter().cycle() {
        let end = offset + size.min(input.len() - offset);
        scalar.decode_with_backend(&input[offset..end], &mut expected_output, Backend::Scalar);
        actual.decode_with_backend(&input[offset..end], &mut actual_output, backend);
        assert_eq!(
            actual_output, expected_output,
            "{backend:?}, offset={offset}, size={size}"
        );
        assert_decoder_state(&actual, &scalar);
        offset = end;
        if offset == input.len() {
            break;
        }
    }
    // Continuations expose any hidden state divergence after malformed input,
    // partial quartets, incomplete headers, or a terminator.
    let continuation =
        b"\nbegin 644 next\nM````````````````````````````````````````````````````````````\n`\n";
    scalar.decode_with_backend(continuation, &mut expected_output, Backend::Scalar);
    actual.decode_with_backend(continuation, &mut actual_output, backend);
    assert_eq!(actual_output, expected_output);
    assert_decoder_state(&actual, &scalar);
}

#[test]
fn encoder_backends_match_output_and_pending_state() {
    let data = random_bytes(10007);
    let backends = backends();
    eprintln!("UU backends: {backends:?}");
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in (0..=180).chain([449, 450, 451, 4499, 4500, 4501, 10007]) {
            for backend in &backends {
                for size in [1, 2, 3, 12, 16, 24, 32, 44, 45, 46, 89, 90, 91, 4096] {
                    let mut scalar = UuEncoder::with_new_line_format(format).unwrap();
                    let mut actual = scalar.clone();
                    let mut expected = b"prefix".to_vec();
                    let mut output = expected.clone();
                    for input in data[..length].chunks(size) {
                        scalar.encode_with_backend(input, &mut expected, Backend::Scalar);
                        actual.encode_with_backend(input, &mut output, *backend);
                        assert_eq!(output, expected);
                        assert_eq!(actual.pending_len, scalar.pending_len);
                        assert_eq!(
                            actual.pending[..actual.pending_len],
                            scalar.pending[..scalar.pending_len]
                        );
                        actual.encode_with_backend(b"", &mut output, *backend);
                        assert_eq!(output, expected);
                    }
                    scalar.finish(&mut expected);
                    actual.finish(&mut output);
                    assert_eq!(
                        output, expected,
                        "{backend:?}, length={length}, chunk={size}"
                    );
                    assert_eq!(actual.new_line_format(), format);
                }
            }
        }
    }
}

#[test]
fn decoder_backends_cover_all_line_lengths_and_partial_vectors() {
    let data = random_bytes(63);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        for length in 1..=63 {
            // The private scalar line writer also lets tests form legal lines
            // longer than the encoder's normal 45-byte output limit.
            let mut input = b"junk\nbeg\nbegin 644 file.bin\n".to_vec();
            UuEncoder::with_new_line_format(format)
                .unwrap()
                .write_line(&data[..length], &mut input);
            input.extend_from_slice(b"`\nend\n");
            for backend in backends() {
                for size in [1, 3, 15, 16, 17, 31, 32, 33, 60, 61, 62, 63, 64, 4096] {
                    compare_decode(&input, &[size], false, backend);
                }
                for split in 0..=input.len() {
                    compare_decode(&input, &[split, usize::MAX], false, backend);
                }
            }
        }
    }
}

#[test]
fn decoder_backends_match_every_byte_at_every_position() {
    let original = encoded(&random_bytes(90), NewLineFormat::Dos);
    let backends = backends();
    for byte in 0..=255u8 {
        for position in 0..original.len() {
            for insert in [false, true] {
                let mut input = original.clone();
                if insert {
                    input.insert(position, byte);
                } else {
                    input[position] = byte;
                }
                for &backend in &backends {
                    for size in [7, 33, 4096] {
                        compare_decode(&input, &[size], true, backend);
                    }
                }
            }
        }
    }
}

#[test]
fn decoder_backends_match_random_malformed_lines() {
    let data = random_bytes(10007);
    for iteration in 0..300 {
        let length = iteration * 31 % data.len();
        let mut input = encoded(&data[..length], NewLineFormat::Unix);
        // Insert, delete, or replace line content (including length octets and
        // delimiters). This exercises rollback after speculative line decoding.
        for index in (0..input.len()).step_by(37).rev() {
            match (iteration + index) % 4 {
                0 => {
                    input.remove(index);
                }
                1 => input[index] = b'\r',
                2 => input.insert(index, b'\n'),
                _ => input[index] = data[(index + iteration) % data.len()],
            }
        }
        for backend in backends() {
            compare_decode(&input, &[0, 1, 16, 32, 0, 61, 4096], true, backend);
            compare_decode(&input, &[usize::MAX], true, backend);
        }
    }
}

#[test]
fn backends_preserve_cloning_reset_and_drained_output() {
    let data = random_bytes(181);
    for backend in backends() {
        for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
            let expected = encoded(&data, format);
            for split in 0..=data.len() {
                let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
                let mut prefix = Vec::new();
                encoder.encode_with_backend(&data[..split], &mut prefix, backend);
                let mut clone = encoder.clone();
                for encoder in [&mut encoder, &mut clone] {
                    let mut suffix = Vec::new();
                    encoder.encode_with_backend(&data[split..], &mut suffix, backend);
                    encoder.finish(&mut suffix);
                    assert_eq!([prefix.as_slice(), suffix.as_slice()].concat(), expected);
                    encoder.encode_with_backend(b"discard", &mut suffix, backend);
                    encoder.reset();
                    suffix.clear();
                    encoder.encode_with_backend(&data, &mut suffix, backend);
                    encoder.finish(&mut suffix);
                    assert_eq!(suffix, expected);
                }
            }
            for split in 0..=expected.len() {
                let mut decoder = UuDecoder::for_payload();
                let mut prefix = Vec::new();
                decoder.decode_with_backend(&expected[..split], &mut prefix, backend);
                let mut clone = decoder.clone();
                for decoder in [&mut decoder, &mut clone] {
                    let mut suffix = Vec::new();
                    decoder.decode_with_backend(&expected[split..], &mut suffix, backend);
                    assert_eq!([prefix.as_slice(), suffix.as_slice()].concat(), data);
                    decoder.reset();
                    suffix.clear();
                    decoder.decode_with_backend(&expected, &mut suffix, backend);
                    assert_eq!(suffix, data);
                }
            }
        }
    }
}

#[test]
fn unaligned_and_exact_sized_buffers_match_scalar() {
    for offset in 0..32 {
        let data = random_bytes(offset + 135).into_boxed_slice();
        for length in [0, 1, 15, 16, 31, 32, 44, 45, 46, 89, 90, 135] {
            let input = &data[offset..offset + length];
            for backend in backends() {
                let mut encoder = UuEncoder::new();
                let mut output = Vec::new();
                encoder.encode_with_backend(input, &mut output, backend);
                encoder.finish(&mut output);
                assert_eq!(output, encoded(input, NewLineFormat::Dos));
                let mut padded = vec![0xff; offset];
                padded.extend_from_slice(&output);
                let padded = padded.into_boxed_slice();
                compare_decode(&padded[offset..], &[usize::MAX], true, backend);
            }
        }
    }
}

#[test]
#[ignore = "manual release-mode throughput comparison"]
fn backend_throughput() {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };

    fn measure(bytes: usize, mut run: impl FnMut()) -> f64 {
        for _ in 0..3 {
            run();
        }
        let mut samples = [0.0f64; 5];
        for sample in &mut samples {
            let start = Instant::now();
            let mut iterations = 0;
            while start.elapsed() < Duration::from_millis(100) {
                run();
                iterations += 1;
            }
            *sample = bytes as f64 * iterations as f64 / start.elapsed().as_secs_f64() / 1048576.0;
        }
        samples.sort_by(f64::total_cmp);
        samples[2]
    }

    let data = random_bytes(1024 * 1024);
    for format in [NewLineFormat::Dos, NewLineFormat::Unix] {
        let expected = encoded(&data, format);
        for backend in backends() {
            let mut output = Vec::with_capacity(expected.len());
            let encode_rate = measure(data.len(), || {
                output.clear();
                let mut encoder = UuEncoder::with_new_line_format(format).unwrap();
                for chunk in black_box(&data).chunks(4096) {
                    encoder.encode_with_backend(chunk, &mut output, backend);
                }
                encoder.finish(&mut output);
                black_box(&output);
            });
            assert_eq!(output, expected);
            let decode_rate = measure(expected.len(), || {
                output.clear();
                let mut decoder = UuDecoder::for_payload();
                for chunk in black_box(&expected).chunks(4096) {
                    decoder.decode_with_backend(chunk, &mut output, backend);
                }
                black_box(&output);
            });
            assert_eq!(output, data);
            eprintln!(
                "{backend:?}/{format:?}: encode {encode_rate:.1}, decode {decode_rate:.1} MiB/s"
            );
        }
    }
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn kernels_obey_load_sizes_quartet_limits_and_line_breaks() {
    for accelerator in simd::Accelerator::available() {
        for length in 0..=96 {
            for limit in 0..=24 {
                for stop in [0, 1, 3, 4, 15, 16, 17, 31, 32, 33, 60, 63, 96] {
                    for newline in *b"\r\n" {
                        let mut data = vec![b'!'; length];
                        if stop < length {
                            data[stop] = newline;
                        }
                        let mut expected_groups = 0;
                        while expected_groups < limit && length - expected_groups * 4 >= 16 {
                            let slice = &data[expected_groups * 4..expected_groups * 4 + 16];
                            let first = slice
                                .iter()
                                .position(|b| matches!(b, b'\r' | b'\n'))
                                .unwrap_or(16);
                            let groups = (first / 4).min(limit - expected_groups);
                            expected_groups += groups;
                            if groups < 4 {
                                break;
                            }
                        }
                        let mut output = b"prefix".to_vec();
                        let groups = accelerator.decode_quartets(&data, limit, &mut output);
                        assert_eq!(
                            groups, expected_groups,
                            "{accelerator:?}, length={length}, limit={limit}, stop={stop}"
                        );
                        assert_eq!(&output[..6], b"prefix");
                        assert_eq!(output[6..], [4, 16, 65].repeat(groups));
                    }
                }
            }
        }
        // Explicitly exercise both whole-line overlap kernels independently of
        // driver dispatch, including all partial final decoded triplets.
        for length in 10..=63 {
            let data = random_bytes(length);
            let mut encoded = Vec::new();
            UuEncoder::new().write_line(&data, &mut encoded);
            let payload = &encoded[1..encoded.len() - 2];
            let mut output = [0xa5; 72];
            assert!(accelerator.decode_line(payload, &mut output));
            assert_eq!(&output[..length], data);
            for position in 0..payload.len() {
                for newline in *b"\r\n" {
                    let mut invalid = payload.to_vec();
                    invalid[position] = newline;
                    assert!(!accelerator.decode_line(&invalid, &mut output));
                }
            }
        }
    }
}

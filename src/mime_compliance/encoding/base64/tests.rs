// Adapted from MimeKit's UnitTests/Encodings/Base64ValidatorTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use crate::{MimeComplianceContext, MimeComplianceIssue};

#[derive(Clone, Copy, Debug)]
enum Path {
    Scalar,
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    Simd(simd::Accelerator),
}

impl Path {
    fn validator(self) -> Base64 {
        #[allow(unused_mut)]
        let mut validator = Base64::default();
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        {
            validator.scanner.0 = match self {
                Self::Scalar => None,
                Self::Simd(accelerator) => Some(accelerator),
            };
        }
        validator
    }
}

fn paths() -> Vec<Path> {
    #[allow(unused_mut)]
    let mut paths = vec![Path::Scalar];
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    paths.extend(simd::Accelerator::available().into_iter().map(Path::Simd));
    paths
}

fn position() -> Position {
    Position {
        context: MimeComplianceContext::Storage,
        offset: 100,
        line_start: 100,
        line: 7,
    }
}

fn validate(path: Path, input: &[u8], chunk: usize, alignment: usize) -> Vec<MimeComplianceIssue> {
    let mut validator = path.validator();
    let mut position = position();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    for bytes in input.chunks(chunk) {
        // Guard the bounded slice with special bytes. Neither SIMD loads nor
        // backward pad-bit recovery may include the surrounding allocation.
        let mut buffer = vec![b'?'; alignment + bytes.len() + 33];
        buffer[alignment..alignment + bytes.len()].copy_from_slice(bytes);
        validator
            .write(
                &buffer[alignment..alignment + bytes.len()],
                &mut position,
                &mut logger,
            )
            .unwrap();
        validator.write(&[], &mut position, &mut logger).unwrap();
    }
    validator.finish(&position, &mut logger).unwrap();
    issues
}

fn matches_scalar(input: &[u8], chunks: &[usize]) {
    // The scalar validator is the reference implementation.
    let expected = validate(Path::Scalar, input, usize::MAX, 0);
    for path in paths() {
        for &chunk in chunks {
            assert_eq!(
                validate(path, input, chunk, chunk % 33),
                expected,
                "{path:?}, chunk={chunk}, input={input:?}"
            );
        }
    }
}

fn wrap(input: &[u8], line_length: usize, newline: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for line in input.chunks(line_length) {
        output.extend_from_slice(line);
        output.extend_from_slice(newline);
    }
    output
}

fn random(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}

fn encoded(length: usize, seed: &mut u32) -> Vec<u8> {
    let data: Vec<_> = (0..length).map(|_| (random(seed) >> 24) as u8).collect();
    let mut output = Vec::new();
    crate::encodings::encode_base64_unwrapped(&data, &mut output);
    output
}

#[test]
fn every_byte_at_every_position() {
    // Range comparisons must classify all 256 values identically in every
    // lane, including both sides of vector and line boundaries.
    let original = wrap(&encoded(72, &mut 72), 40, b"\r\n");
    for value in 0..=255 {
        for index in 0..original.len() {
            for replace in [true, false] {
                let mut input = original.clone();
                if replace {
                    input[index] = value;
                } else {
                    input.insert(index, value);
                }
                matches_scalar(&input, &[16, 33, usize::MAX]);
            }
        }
    }
}

#[test]
fn mime_line_tails() {
    for newline in [b"\n".as_slice(), b"\r\n"] {
        let mut original = wrap(&encoded(228, &mut 76), 76, newline);
        original.extend_from_slice(b"?*A");
        for value in 0..=255 {
            for index in 64..76 + newline.len() {
                let mut input = original.clone();
                input[index] = value;
                matches_scalar(&input, &[75, 76, 77, 78, 79, 100, usize::MAX]);
            }
        }
    }
}

#[test]
fn all_line_lengths_and_latches() {
    let chunks = [1, 15, 16, 17, 32, 33, 77, usize::MAX];
    let mut seed = 1234;
    for newline in [b"\n".as_slice(), b"\r\n", b" \t\r\n"] {
        for line_length in 1..=100 {
            for length in [1, 2, 3, 47, 48, 49, 301] {
                let mut data = encoded(length, &mut seed);
                let input = wrap(&data, line_length, newline);
                for path in paths() {
                    for chunk in chunks {
                        assert!(validate(path, &input, chunk, 1).is_empty());
                    }
                }
                while data.last() == Some(&b'=') {
                    data.pop();
                }
                let mut input = Vec::new();
                // Every line has invalid and comment bytes so both per-line
                // latches, line numbers, and columns are checked.
                for line in data.chunks(line_length) {
                    let mut line = line.to_vec();
                    for special in *b"?*?*" {
                        let index = random(&mut seed) as usize % (line.len() + 1);
                        line.insert(index, special);
                    }
                    input.extend(line);
                    input.extend_from_slice(newline);
                }
                matches_scalar(&input, &chunks);
            }
        }
    }
}

#[test]
fn random_bytes_and_bounded_slices() {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for density in [0, 500, 900, 990, 999, 1000] {
        let mut seed = density + 1;
        for _ in 0..300 {
            let length = random(&mut seed) as usize % 800;
            let input: Vec<_> = (0..length)
                .map(|_| {
                    if random(&mut seed) % 1000 < density {
                        ALPHABET[(random(&mut seed) >> 24) as usize % 64]
                    } else {
                        (random(&mut seed) >> 24) as u8
                    }
                })
                .collect();
            matches_scalar(
                &input,
                &[1, 7, 15, 16, 17, 31, 32, 33, 47, 48, 49, 64, 100, 4096],
            );
        }
    }
}

#[test]
fn pad_bits_and_whitespace_only_writes() {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for padding in [1, 2] {
        for &last in ALPHABET {
            for prefix in [0, 12, 28, 60] {
                for whitespace in [Vec::new(), b" \t\r\n".repeat(32)] {
                    let mut input = vec![b'A'; prefix + 3 - padding];
                    input.push(last);
                    input.extend(whitespace);
                    input.extend(std::iter::repeat_n(b'=', padding));
                    matches_scalar(&input, &[1, 2, 3, 15, 16, 17, 31, 32, 33, 4096]);
                }
            }
        }
    }
    for path in paths() {
        // A whole quantum changes the last byte without changing quantum % 4.
        // Empty and whitespace-only writes must preserve that recovered byte.
        for last in *b"Ah" {
            let mut validator = path.validator();
            let mut position = position();
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            let mut alphabet = vec![b'A'; 64];
            alphabet[63] = last;
            for input in [
                b"Zh".as_slice(),
                &alphabet,
                &b" \t\r\n".repeat(32),
                b"",
                b"==",
            ] {
                validator.write(input, &mut position, &mut logger).unwrap();
            }
            validator.finish(&position, &mut logger).unwrap();
            assert_eq!(issues.len(), usize::from(last == b'h'));
            if last == b'h' {
                assert_eq!(issues[0].violation, Violation::InvalidBase64Padding);
                assert_eq!(
                    (
                        issues[0].stream_offset,
                        issues[0].line_number,
                        issues[0].column_number
                    ),
                    (294, 39, 1)
                );
            }
        }
    }
}

#[test]
fn logger_errors_preserve_scalar_state() {
    for suffix in [b"?*".as_slice(), b"*?", b"Zh=", b"Zh==!", b"Zh===", b"A="] {
        let mut input = b"AAAA \r\n".repeat(32);
        input.extend_from_slice(suffix);
        input.extend_from_slice(&b"\nAAAA".repeat(32));
        for fail_at in [0, 1] {
            let run = |path: Path| {
                let mut validator = path.validator();
                let mut position = position();
                let mut issues = Vec::new();
                let result = validator.write(&input, &mut position, &mut |issue| {
                    issues.push(issue);
                    if issues.len() == fail_at + 1 {
                        Err(io::Error::other("stop"))
                    } else {
                        Ok(())
                    }
                });
                (
                    result.is_err(),
                    issues,
                    (position.offset, position.line, position.line_start),
                    (
                        validator.quantum,
                        validator.padding,
                        validator.last_alphabet,
                    ),
                    (
                        validator.invalid,
                        validator.reported_invalid_character,
                        validator.reported_comment,
                    ),
                )
            };
            let expected = run(Path::Scalar);
            for path in paths() {
                assert_eq!(run(path), expected, "{path:?}, suffix={suffix:?}");
            }
        }
    }
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn kernels_stop_before_special_bytes_and_ignore_later_line_feeds() {
    for accelerator in simd::Accelerator::available() {
        for length in 0..=129 {
            for alignment in 0..32 {
                for special in [b'=', b'*', b'?', 0, 128, 255] {
                    let mut buffer = vec![special; alignment + length + 32];
                    let input = &mut buffer[alignment..alignment + length];
                    for (i, byte) in input.iter_mut().enumerate() {
                        *byte = if i % 7 == 0 { b'\n' } else { b'A' };
                    }
                    if length > 0 {
                        input[length / 2] = special;
                    }
                    let prefix = accelerator.scan(input);
                    assert!(prefix.len <= length);
                    assert!(
                        input[..prefix.len]
                            .iter()
                            .all(|&b| CLASS[b as usize] <= WHITESPACE && b != b'=')
                    );
                    if length >= 32 {
                        assert_eq!(prefix.len, length / 2);
                    }
                    assert_eq!(
                        prefix.alphabet,
                        input[..prefix.len]
                            .iter()
                            .filter(|&&b| CLASS[b as usize] == ALPHABET)
                            .count()
                    );
                    assert_eq!(
                        prefix.lines,
                        input[..prefix.len].iter().filter(|&&b| b == b'\n').count() as u64
                    );
                    assert_eq!(
                        prefix.line_start,
                        input[..prefix.len]
                            .iter()
                            .rposition(|&b| b == b'\n')
                            .map_or(0, |i| i + 1)
                    );
                }
            }
        }
    }
}

// Adapted from MimeKit's MimeReaderScanContentTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{Scan, Scanner, scalar};

fn reference(input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
    let mut len = input
        .windows(3)
        .position(|bytes| (multipart && bytes == b"\n--") || (mbox && bytes == b"\nFr"))
        .map_or(input.len(), |index| index + 1);
    if input.ends_with(b"\n-") && multipart || input.ends_with(b"\nF") && mbox {
        len = len.min(input.len() - 1);
    }
    let bytes = &input[..len];
    Scan {
        len,
        lines: bytes.iter().filter(|&&b| b == b'\n').count(),
        dos: bytes.windows(2).filter(|&pair| pair == b"\r\n").count()
            + usize::from(cr && bytes.first() == Some(&b'\n')),
    }
}

fn check(input: &[u8]) {
    for multipart in [false, true] {
        for mbox in [false, true] {
            for cr in [false, true] {
                let expected = reference(input, multipart, mbox, cr);
                assert_eq!(scalar(input, multipart, mbox, cr), expected);
                assert_eq!(
                    Scanner::default().scan(input, multipart, mbox, cr),
                    expected
                );
                #[cfg(all(
                    feature = "simd",
                    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
                ))]
                for accelerator in super::simd::Accelerator::available() {
                    assert_eq!(
                        accelerator.scan(input, multipart, mbox, cr),
                        expected,
                        "{accelerator:?}, len={}, multipart={multipart}, mbox={mbox}, cr={cr}",
                        input.len()
                    );
                }
            }
        }
    }
}

#[test]
fn short_unaligned_slices_and_vector_tails_match_reference() {
    for len in (0..=130).chain([255, 256, 257, 4095, 4096, 4097]) {
        for alignment in 0..32 {
            let mut input = vec![b'\r'; alignment];
            input.extend(b"x\r\nY\nZ\r\0\xff-".iter().copied().cycle().take(len));
            check(&input[alignment..]);
        }
    }
}

#[test]
fn candidates_at_every_lane_and_partial_prefixes_match_reference() {
    for offset in 0..96 {
        for prefix in [
            b"\n--boundary\r\n".as_slice(),
            b"\r\n--boundary\r\n",
            b"\nFrom sender\n",
            b"\r\nFrom sender\n",
            b"\n-False\n",
            b"\nFwrong\n",
            b"\nFr\n--",
            b"\n--\nFr",
            b"\n",
            b"\n-",
            b"\nF",
        ] {
            for fill in [b'x', b'\n', b'\r', 0, 255] {
                let mut input = vec![fill; offset];
                input.extend_from_slice(prefix);
                check(&input);
                // Ensure the candidate also takes the vector path instead of
                // only testing the scalar tail near the end of a short slice.
                input.extend_from_slice(b"\n\r\n".repeat(32).as_slice());
                check(&input);
            }
        }
    }
}

#[test]
fn randomized_binary_and_dense_newlines_match_reference() {
    let mut random = 0x45bf_04ab_176c_293du64;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random
    };
    for iteration in 0usize..512 {
        let len = next() as usize % 8193;
        let input: Vec<_> = (0..len)
            .map(|_| {
                let byte = next();
                if iteration.is_multiple_of(2) {
                    byte as u8
                } else {
                    let alphabet = b"\n\n\r--Fro \0\xffx";
                    alphabet[byte as usize % alphabet.len()]
                }
            })
            .collect();
        check(&input);
    }
}

#[test]
fn upstream_skip_non_boundary_lines_fuzz() {
    let mut random = 1337u64;
    let mut next = |limit| {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random as usize % limit
    };
    let alphabet = b"\r\n\n--Fraaaaaaaaaaxyz";
    for iteration in 0..50_000 {
        let prefix = next(4);
        let length = next(300);
        let end = prefix + length;
        let mut buffer: Vec<_> = (0..end + 4)
            .map(|_| alphabet[next(alphabet.len())])
            .collect();
        // Make lines longer in some iterations so multiple vectors are
        // scanned between linefeeds.
        if iteration % 2 == 0 {
            for byte in &mut buffer[prefix..end] {
                if *byte == b'\n' && next(4) != 0 {
                    *byte = b'b';
                }
            }
        }
        buffer[end] = b'\n';
        // Unlike MimeKit, the Rust kernels cannot inspect the sentinel. Their
        // reference also handles partial marker prefixes and incomplete lines.
        check(&buffer[prefix..end]);
    }
}

#[test]
fn upstream_skip_non_boundary_lines_sweep() {
    // Place both newline formats and candidate prefixes at every pair of
    // offsets, covering every lane and crossing of vector boundaries.
    const LENGTH: usize = 80;
    for next in [b"--".as_slice(), b"Fr", b"-x", b"F-", b"-"] {
        for crlf in 0..4 {
            for first in 0..LENGTH {
                for second in first + 1..=LENGTH {
                    let mut buffer = [b'a'; LENGTH + 4];
                    let end = if second == LENGTH { LENGTH } else { LENGTH - 1 };
                    buffer[first] = b'\n';
                    if crlf & 1 != 0 && first > 0 {
                        buffer[first - 1] = b'\r';
                    }
                    if second < end {
                        buffer[second] = b'\n';
                        if crlf & 2 != 0 && second - 1 > first {
                            buffer[second - 1] = b'\r';
                        }
                        for (index, byte) in next.iter().enumerate() {
                            if second + 1 + index < end {
                                buffer[second + 1 + index] = *byte;
                            }
                        }
                    }
                    buffer[end] = b'\n';
                    check(&buffer[..end]);
                }
            }
        }
    }
}

#[test]
fn upstream_skip_non_boundary_lines_edge_cases() {
    let mut inputs: Vec<String> = [
        "",
        "\n",
        "\r\n",
        "abc",
        "abc\n",
        "abc\r\n--",
        "abc\r\n-",
        "abc\r\n--boundary\r\n",
        "\n--",
        "\n\n\n\n",
        "\r\r\r\n",
        "\r",
        "abc\r",
        "line\r\nFrom someone\r\nmore\r\n",
        "line\nFrom someone\nmore\n",
        "line\n---\n",
        "line\n-x\nline\n",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for length in [31, 32, 15] {
        inputs.push(format!("{}\n--{}", "a".repeat(length), "a".repeat(40)));
    }
    for length in [30, 31, 15] {
        inputs.push(format!("{}\r\n{}\n", "a".repeat(length), "a".repeat(40)));
    }
    for suffix in ["\n", "\n-", "\n--"] {
        inputs.push(format!("{}{suffix}", "a".repeat(100)));
    }
    inputs.push("a".repeat(1000));
    for input in inputs {
        for prefix in 0..3 {
            let mut buffer = vec![b'\r'; prefix];
            buffer.extend_from_slice(input.as_bytes());
            buffer.extend_from_slice(b"\n---");
            // Bytes outside the slice are not part of its first line ending
            // or a marker prefix after its final newline.
            check(&buffer[prefix..prefix + input.len()]);
        }
    }
}

#[test]
fn crlf_carry_matches_whole_input_across_all_chunk_sizes() {
    let input = b"\nabc\r\n\0\xff\r\r\n\n\r".repeat(100);
    let expected = reference(&input, false, false, true);
    for size in 1..=129 {
        let mut actual = Scan::default();
        let mut cr = true;
        for bytes in input.chunks(size) {
            let scanned = Scanner::default().scan(bytes, false, false, cr);
            actual.len += scanned.len;
            actual.lines += scanned.lines;
            actual.dos += scanned.dos;
            cr = bytes.last() == Some(&b'\r');
        }
        assert_eq!(actual, expected, "chunk size {size}");
    }
}

// Adapted from MimeKit's UnitTests/Utils/MemoryTests.cs and Utf8Tests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;

#[derive(Debug)]
enum Kernel {
    Dispatch,
    Scalar,
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    Simd(simd::Accelerator),
}

impl Kernel {
    fn line(&self, input: &[u8], null: bool, eight_bit: bool) -> Line {
        match self {
            Self::Dispatch => Scanner::default().line(input, null, eight_bit),
            Self::Scalar => line_scalar(input, null, eight_bit),
            #[cfg(all(
                feature = "simd",
                any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
            ))]
            Self::Simd(kernel) => kernel.line(input, null, eight_bit),
        }
    }

    fn utf8(&self, input: &[u8]) -> Option<usize> {
        match self {
            Self::Dispatch => Scanner::default().invalid_utf8(input),
            Self::Scalar => invalid_utf8_scalar(input),
            #[cfg(all(
                feature = "simd",
                any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
            ))]
            Self::Simd(kernel) => kernel.invalid_utf8(input),
        }
    }
}

fn kernels() -> Vec<Kernel> {
    let kernels = vec![Kernel::Dispatch, Kernel::Scalar];
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    let kernels = kernels
        .into_iter()
        .chain(simd::Accelerator::available().into_iter().map(Kernel::Simd))
        .collect();
    kernels
}

struct Random(u64);

impl Random {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        for byte in bytes {
            *byte = self.next() as u8;
        }
    }
}

fn check_line(kernels: &[Kernel], input: &[u8], flags: usize) {
    let mut expected = Line::default();
    for (index, &byte) in input.iter().enumerate() {
        if byte == b'\n' {
            expected.lf = Some(index);
            break;
        }
        if flags & 1 != 0 && byte == 0 && expected.null.is_none() {
            expected.null = Some(index);
        }
        if flags & 2 != 0 && byte >= 128 && expected.eight_bit.is_none() {
            expected.eight_bit = Some(index);
        }
    }
    for kernel in kernels {
        assert_eq!(
            kernel.line(input, flags & 1 != 0, flags & 2 != 0),
            expected,
            "{kernel:?}, length={}, flags={flags}",
            input.len()
        );
    }
}

#[test]
fn header_line_scanner_byte_positions() {
    let kernels = kernels();
    let mut data = [0xff; 160];
    for start in 0..32 {
        for lf in [
            None,
            Some(0),
            Some(1),
            Some(15),
            Some(16),
            Some(31),
            Some(32),
            Some(63),
            Some(64),
            Some(95),
        ] {
            for special in 0..96 {
                data.fill(0xff);
                data[start..start + 96].fill(b'a');
                data[start + special] = 0;
                data[start + (special + 37) % 96] = 0x80;
                data[start + (special + 7) % 96] = 0;
                data[start + (special + 53) % 96] = 0xff;
                data[start + 96] = 0;
                data[start + 97] = b'\n';
                if let Some(lf) = lf {
                    data[start + lf] = b'\n';
                }
                for flags in 0..4 {
                    check_line(&kernels, &data[start..start + 96], flags);
                }
            }
        }
    }
}

#[test]
fn header_line_scanner_bounds_and_random_data() {
    let kernels = kernels();
    let mut random = Random(7319);
    let mut data = [0; 576];
    for iteration in 0..20_000 {
        let start = random.next() % 32;
        let length = if iteration < 513 {
            iteration
        } else {
            random.next() % 513
        };
        random.fill(&mut data);
        if iteration & 1 == 0 {
            data[start..start + length].fill(b'a');
        }
        // Poison bytes outside the bounded search, including an LF that must never match.
        data[start + length..start + length + 3].copy_from_slice(&[0, 0xff, b'\n']);
        for flags in 0..4 {
            check_line(&kernels, &data[start..start + length], flags);
        }
    }
}

fn check_utf8(kernels: &[Kernel], input: &[u8], expected: Option<usize>) {
    for kernel in kernels {
        assert_eq!(
            kernel.utf8(input),
            expected,
            "{kernel:?}, length={}",
            input.len()
        );
    }
}

fn reference_utf8(input: &[u8]) -> Option<usize> {
    std::str::from_utf8(input)
        .err()
        .map(|error| error.valid_up_to())
}

#[test]
fn required_arm64_simd_support() {
    if std::env::var("MIMECRAFT_REQUIRE_ARM64_SIMD").as_deref() != Ok("1") {
        return;
    }
    #[cfg(not(all(feature = "simd", target_arch = "aarch64", target_endian = "little")))]
    panic!("the ARM64 job must compile the little-endian NEON implementations");
    #[cfg(all(feature = "simd", target_arch = "aarch64", target_endian = "little"))]
    assert!(
        simd::Accelerator::detect().is_some(),
        "the ARM64 job must execute NEON"
    );
}

#[test]
fn invalid_utf8_vector_boundaries() {
    let kernels = kernels();
    for text in [
        "a",
        "\u{e9}",
        "\u{65e5}",
        "\u{1f600}",
        "ascii \u{e9} \u{65e5}\u{1f600}",
    ] {
        let source = text.repeat(1024).into_bytes();
        let mut data = [0xff; 1056];
        for alignment in 0..32 {
            data.fill(0xff);
            data[alignment..alignment + 1024].copy_from_slice(&source[..1024]);
            for length in 0..=1024 {
                let input = &data[alignment..alignment + length];
                check_utf8(&kernels, input, reference_utf8(input));
            }
        }
        for length in [
            15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512,
        ] {
            data[1..length + 1].copy_from_slice(&source[..length]);
            for index in 0..length {
                let saved = data[index + 1];
                for byte in [
                    0, 0x7f, 0x80, 0xbf, 0xc0, 0xc2, 0xdf, 0xe0, 0xed, 0xef, 0xf0, 0xf4, 0xf5, 0xff,
                ] {
                    data[index + 1] = byte;
                    let input = &data[1..length + 1];
                    check_utf8(&kernels, input, reference_utf8(input));
                }
                data[index + 1] = saved;
            }
        }
    }
}

#[test]
fn invalid_utf8_after_valid_prefix() {
    let kernels = kernels();
    for malformed in [
        &b"\x80"[..],
        b"\xff",
        b"\xc0\xaf",
        b"\xe0\x80\x80",
        b"\xed\xa0\x80",
        b"\xf0\x80\x80\x80",
        b"\xf4\x90\x80\x80",
        b"\xc2!",
        b"\xe2!\x80",
        b"\xe2\x82!",
        b"\xf0!\x80\x80",
        b"\xf0\x90!\x80",
        b"\xf0\x90\x80!",
        b"\xc2",
        b"\xe2\x82",
        b"\xf0\x90\x80",
    ] {
        check_utf8(&kernels, malformed, Some(0));
        for padding in [0, 15, 16, 31, 32, 127, 4096] {
            let mut prefix = vec![b'a'; padding];
            prefix.extend_from_slice("\0\u{e9}\r\n \u{65e5}\u{1f600}".as_bytes());
            check_utf8(&kernels, &prefix, None);
            let mut data = vec![0xff];
            data.extend_from_slice(&prefix);
            data.extend_from_slice(malformed);
            data.push(0xff);
            check_utf8(&kernels, &data[1..data.len() - 1], Some(prefix.len()));
            // Keep errors inside full vectors, not just in the scalar tail.
            data.pop();
            data.extend_from_slice(&[b'a'; 128]);
            data.push(0xff);
            check_utf8(&kernels, &data[1..data.len() - 1], Some(prefix.len()));
        }
    }
}

#[test]
fn invalid_utf8_random_input() {
    let kernels = kernels();
    let mut random = Random(2083);
    let prefix = "ascii\0\u{e9}\r\n \u{65e5}\u{1f600}".as_bytes();
    let mut data = [0; 256];
    for _ in 0..4096 {
        random.fill(&mut data);
        let offset = random.next() % 32;
        data[offset..offset + prefix.len()].copy_from_slice(prefix);
        let length = random.next() % (data.len() - offset + 1);
        let input = &data[offset..offset + length];
        check_utf8(&kernels, input, reference_utf8(input));
    }
}

// Adapted from MimeKit's Utils/Memory.cs and Utils/Utf8.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
#[path = "scan/x86.rs"]
mod simd;
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[path = "scan/neon.rs"]
mod simd;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Line {
    pub lf: Option<usize>,
    pub null: Option<usize>,
    pub eight_bit: Option<usize>,
}

impl Line {
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn masks(&mut self, offset: usize, lf: u32, mut null: u32, mut eight_bit: u32) -> bool {
        if lf != 0 {
            // Include only lanes preceding the first LF, which cannot itself
            // be NUL or 8-bit. Wrapping subtraction also handles lane zero.
            let before = lf ^ lf.wrapping_sub(1);
            null &= before;
            eight_bit &= before;
            self.lf = Some(offset + lf.trailing_zeros() as usize);
        }
        if self.null.is_none() && null != 0 {
            self.null = Some(offset + null.trailing_zeros() as usize);
        }
        if self.eight_bit.is_none() && eight_bit != 0 {
            self.eight_bit = Some(offset + eight_bit.trailing_zeros() as usize);
        }
        lf != 0
    }

    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn finish(mut self, input: &[u8], offset: usize, null: bool, eight_bit: bool) -> Self {
        let tail = line_scalar(
            &input[offset..],
            null && self.null.is_none(),
            eight_bit && self.eight_bit.is_none(),
        );
        self.lf = tail.lf.map(|i| offset + i);
        self.null = self.null.or(tail.null.map(|i| offset + i));
        self.eight_bit = self.eight_bit.or(tail.eight_bit.map(|i| offset + i));
        self
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    not(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    )),
    derive(Default)
)]
pub(super) struct Scanner {
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    accelerator: Option<simd::Accelerator>,
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
impl Default for Scanner {
    fn default() -> Self {
        Self {
            accelerator: simd::Accelerator::detect(),
        }
    }
}

impl Scanner {
    // First LF and requested special bytes preceding it, relative to this
    // bounded slice. Unlike MimeKit's callers, no writable sentinel is needed.
    pub fn line(self, input: &[u8], null: bool, eight_bit: bool) -> Line {
        if !null && !eight_bit {
            return Line {
                lf: memchr::memchr(b'\n', input),
                ..Line::default()
            };
        }
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Some(accelerator) = self.accelerator {
            return accelerator.line(input, null, eight_bit);
        }
        line_scalar(input, null, eight_bit)
    }

    pub fn invalid_utf8(self, input: &[u8]) -> Option<usize> {
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if input.len() >= 128
            && let Some(accelerator) = self.accelerator
        {
            return accelerator.invalid_utf8(input);
        }
        invalid_utf8_scalar(input)
    }
}

fn line_scalar(input: &[u8], null: bool, eight_bit: bool) -> Line {
    let mut line = Line::default();
    let mut offset = 0;
    const ONES: u64 = u64::MAX / 255;
    const HIGH: u64 = ONES * 128;
    let zeros = |word: u64| word.wrapping_sub(ONES) & !word & HIGH;
    // Safe unaligned word loads need no pointer alignment prologue. Bytewise
    // finishing gives exact positions regardless of native endianness.
    while input.len() - offset >= 8 {
        let block = &input[offset..offset + 8];
        let word = u64::from_ne_bytes(block.try_into().expect("eight bytes"));
        if zeros(word ^ (ONES * 10)) != 0 {
            break;
        }
        if null && line.null.is_none() && zeros(word) != 0 {
            line.null = block.iter().position(|&b| b == 0).map(|i| offset + i);
        }
        if eight_bit && line.eight_bit.is_none() && word & HIGH != 0 {
            line.eight_bit = block.iter().position(|&b| b >= 128).map(|i| offset + i);
        }
        offset += 8;
    }
    for (index, &byte) in input.iter().enumerate().skip(offset) {
        if byte == b'\n' {
            line.lf = Some(index);
            break;
        }
        if null && byte == 0 && line.null.is_none() {
            line.null = Some(index);
        }
        if eight_bit && byte >= 128 && line.eight_bit.is_none() {
            line.eight_bit = Some(index);
        }
    }
    line
}

fn invalid_utf8_scalar(input: &[u8]) -> Option<usize> {
    // Rust's checked UTF-8 validator already reports the leading byte for
    // overlong, surrogate, out-of-range, missing and invalid continuations.
    std::str::from_utf8(input)
        .err()
        .map(|error| error.valid_up_to())
}

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
fn invalid_utf8_tail(input: &[u8], mut offset: usize, incomplete: bool) -> Option<usize> {
    if incomplete {
        // The first error or scalar tail may belong to a sequence starting
        // in the preceding block. Rewind to its leading byte, not a continuation.
        offset -= 3;
        while offset > 0 && input[offset] & 0xc0 == 0x80 {
            offset -= 1;
        }
    }
    invalid_utf8_scalar(&input[offset..]).map(|i| offset + i)
}

// Lookup algorithm from simdutf/SimdUnicode (MIT), revision
// d20b63710e8893dede79890f29fd5ae6745d3e2d, via MimeKit's Utf8.cs.
// Source copyright (c) 2023 SimdUnicode authors; see LICENSE-SimdUnicode.
// Intersect the previous byte's nibble masks with the current high-nibble mask.
// Bits 0-6 flag invalid pairs; bit 7 marks adjacent continuations and is checked
// against the requirements of the preceding three-/four-byte sequence.
#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
const HIGH: [u8; 16] = [2, 2, 2, 2, 2, 2, 2, 2, 128, 128, 128, 128, 33, 1, 21, 73];
#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
const LOW: [u8; 16] = [
    0xe7, 0xa3, 0x83, 0x83, 0x8b, 0xcb, 0xcb, 0xcb, 0xcb, 0xcb, 0xcb, 0xcb, 0xcb, 0xdb, 0xcb, 0xcb,
];
#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
const CURRENT: [u8; 16] = [1, 1, 1, 1, 1, 1, 1, 1, 0xe6, 0xae, 0xba, 0xba, 1, 1, 1, 1];

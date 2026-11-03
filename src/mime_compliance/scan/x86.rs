// Adapted from MimeKit's Utils/Memory.cs and Utils/Utf8.cs.
// UTF-8 lookup algorithm attributed to simdutf/SimdUnicode (MIT); see scan.rs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use super::{CURRENT, HIGH, LOW, Line, invalid_utf8_scalar, invalid_utf8_tail};

#[derive(Clone, Copy, Debug)]
enum Cpu {
    Sse2,
    Sse41,
    Avx2,
}

// Only this module constructs CPU tokens, after checking every required feature.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(Cpu);

impl Accelerator {
    pub fn detect() -> Option<Self> {
        if is_x86_feature_detected!("avx2") {
            Some(Self(Cpu::Avx2))
        } else if is_x86_feature_detected!("ssse3") && is_x86_feature_detected!("sse4.1") {
            Some(Self(Cpu::Sse41))
        } else if is_x86_feature_detected!("sse2") {
            Some(Self(Cpu::Sse2))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn available() -> Vec<Self> {
        let mut result = Vec::new();
        if is_x86_feature_detected!("sse2") {
            result.push(Self(Cpu::Sse2));
        }
        if is_x86_feature_detected!("ssse3") && is_x86_feature_detected!("sse4.1") {
            result.push(Self(Cpu::Sse41));
        }
        if is_x86_feature_detected!("avx2") {
            result.push(Self(Cpu::Avx2));
        }
        result
    }

    pub fn line(self, input: &[u8], null: bool, eight_bit: bool) -> Line {
        // SAFETY: the token guarantees features; kernels bound all vector loads.
        unsafe {
            match self.0 {
                Cpu::Avx2 => line_avx2(input, null, eight_bit),
                _ => line_sse2(input, null, eight_bit),
            }
        }
    }

    pub fn invalid_utf8(self, input: &[u8]) -> Option<usize> {
        // SAFETY: the token guarantees features; kernels bound all vector loads.
        unsafe {
            match self.0 {
                Cpu::Avx2 => utf8_avx2(input),
                Cpu::Sse41 => utf8_sse(input),
                Cpu::Sse2 => invalid_utf8_scalar(input),
            }
        }
    }
}

#[target_feature(enable = "sse2")]
fn line_sse2(input: &[u8], null: bool, eight_bit: bool) -> Line {
    let mut line = Line::default();
    let mut offset = 0;
    while input.len() - offset >= 16 {
        // SAFETY: the complete 16-byte unaligned load fits the slice.
        let block = unsafe { _mm_loadu_si128(input.as_ptr().add(offset).cast()) };
        let lf = _mm_movemask_epi8(_mm_cmpeq_epi8(block, _mm_set1_epi8(10))) as u32;
        let nulls = if null && line.null.is_none() {
            _mm_movemask_epi8(_mm_cmpeq_epi8(block, _mm_setzero_si128())) as u32
        } else {
            0
        };
        let eight = if eight_bit && line.eight_bit.is_none() {
            _mm_movemask_epi8(block) as u32
        } else {
            0
        };
        if line.masks(offset, lf, nulls, eight) {
            return line;
        }
        offset += 16;
    }
    line.finish(input, offset, null, eight_bit)
}

#[target_feature(enable = "avx2")]
fn line_avx2(input: &[u8], null: bool, eight_bit: bool) -> Line {
    let mut line = Line::default();
    let mut offset = 0;
    while input.len() - offset >= 32 {
        // SAFETY: the complete 32-byte unaligned load fits the slice.
        let block = unsafe { _mm256_loadu_si256(input.as_ptr().add(offset).cast()) };
        let lf = _mm256_movemask_epi8(_mm256_cmpeq_epi8(block, _mm256_set1_epi8(10))) as u32;
        let nulls = if null && line.null.is_none() {
            _mm256_movemask_epi8(_mm256_cmpeq_epi8(block, _mm256_setzero_si256())) as u32
        } else {
            0
        };
        let eight = if eight_bit && line.eight_bit.is_none() {
            _mm256_movemask_epi8(block) as u32
        } else {
            0
        };
        if line.masks(offset, lf, nulls, eight) {
            return line;
        }
        offset += 32;
    }
    line.finish(input, offset, null, eight_bit)
}

#[target_feature(enable = "ssse3,sse4.1")]
fn utf8_sse(input: &[u8]) -> Option<usize> {
    // SAFETY: each lookup table contains exactly 16 bytes.
    let (high, low, next) = unsafe {
        (
            _mm_loadu_si128(HIGH.as_ptr().cast()),
            _mm_loadu_si128(LOW.as_ptr().cast()),
            _mm_loadu_si128(CURRENT.as_ptr().cast()),
        )
    };
    // Only the last three lanes can carry an unfinished sequence to the next block.
    let max = _mm_setr_epi8(
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        0xefu8 as i8,
        0xdfu8 as i8,
        0xbfu8 as i8,
    );
    let nibble = _mm_set1_epi8(15);
    let high_bit = _mm_set1_epi8(-128);
    let mut previous = _mm_setzero_si128();
    let mut incomplete = _mm_setzero_si128();
    let mut offset = 0;
    while input.len() - offset >= 16 {
        // SAFETY: all 16 bytes fit the checked slice window.
        let current = unsafe { _mm_loadu_si128(input.as_ptr().add(offset).cast()) };
        if _mm_movemask_epi8(current) == 0 {
            if _mm_testz_si128(incomplete, incomplete) == 0 {
                break;
            }
            incomplete = _mm_setzero_si128();
        } else {
            let p1 = _mm_alignr_epi8::<15>(current, previous);
            let p2 = _mm_alignr_epi8::<14>(current, previous);
            let p3 = _mm_alignr_epi8::<13>(current, previous);
            let h = _mm_shuffle_epi8(high, _mm_and_si128(_mm_srli_epi16::<4>(p1), nibble));
            let l = _mm_shuffle_epi8(low, _mm_and_si128(p1, nibble));
            let n = _mm_shuffle_epi8(next, _mm_and_si128(_mm_srli_epi16::<4>(current), nibble));
            let required = _mm_and_si128(
                _mm_or_si128(
                    _mm_subs_epu8(p2, _mm_set1_epi8(0x60)),
                    _mm_subs_epu8(p3, _mm_set1_epi8(0x70)),
                ),
                high_bit,
            );
            let errors = _mm_xor_si128(_mm_and_si128(_mm_and_si128(h, l), n), required);
            if _mm_testz_si128(errors, errors) == 0 {
                break;
            }
            incomplete = _mm_subs_epu8(current, max);
        }
        previous = current;
        offset += 16;
    }
    invalid_utf8_tail(input, offset, _mm_testz_si128(incomplete, incomplete) == 0)
}

#[target_feature(enable = "avx2")]
fn utf8_avx2(input: &[u8]) -> Option<usize> {
    // SAFETY: each lookup table contains exactly 16 bytes.
    let (high, low, next) = unsafe {
        (
            _mm256_broadcastsi128_si256(_mm_loadu_si128(HIGH.as_ptr().cast())),
            _mm256_broadcastsi128_si256(_mm_loadu_si128(LOW.as_ptr().cast())),
            _mm256_broadcastsi128_si256(_mm_loadu_si128(CURRENT.as_ptr().cast())),
        )
    };
    let max = _mm256_setr_epi8(
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        -1,
        0xefu8 as i8,
        0xdfu8 as i8,
        0xbfu8 as i8,
    );
    let nibble = _mm256_set1_epi8(15);
    let high_bit = _mm256_set1_epi8(-128);
    let mut previous = _mm256_setzero_si256();
    let mut incomplete = _mm256_setzero_si256();
    let mut offset = 0;
    while input.len() - offset >= 32 {
        // SAFETY: all 32 bytes fit the checked slice window.
        let current = unsafe { _mm256_loadu_si256(input.as_ptr().add(offset).cast()) };
        if _mm256_movemask_epi8(current) == 0 {
            if _mm256_testz_si256(incomplete, incomplete) == 0 {
                break;
            }
            incomplete = _mm256_setzero_si256();
        } else {
            // Align across 128-bit lanes as well as across successive blocks.
            let bridge = _mm256_permute2x128_si256::<0x21>(previous, current);
            let p1 = _mm256_alignr_epi8::<15>(current, bridge);
            let p2 = _mm256_alignr_epi8::<14>(current, bridge);
            let p3 = _mm256_alignr_epi8::<13>(current, bridge);
            let h = _mm256_shuffle_epi8(high, _mm256_and_si256(_mm256_srli_epi16::<4>(p1), nibble));
            let l = _mm256_shuffle_epi8(low, _mm256_and_si256(p1, nibble));
            let n = _mm256_shuffle_epi8(
                next,
                _mm256_and_si256(_mm256_srli_epi16::<4>(current), nibble),
            );
            let required = _mm256_and_si256(
                _mm256_or_si256(
                    _mm256_subs_epu8(p2, _mm256_set1_epi8(0x60)),
                    _mm256_subs_epu8(p3, _mm256_set1_epi8(0x70)),
                ),
                high_bit,
            );
            let errors = _mm256_xor_si256(_mm256_and_si256(_mm256_and_si256(h, l), n), required);
            if _mm256_testz_si256(errors, errors) == 0 {
                break;
            }
            incomplete = _mm256_subs_epu8(current, max);
        }
        previous = current;
        offset += 32;
    }
    invalid_utf8_tail(
        input,
        offset,
        _mm256_testz_si256(incomplete, incomplete) == 0,
    )
}

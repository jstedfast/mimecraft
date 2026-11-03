// Adapted from MimeKit's Encodings/Base64Validator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use super::Prefix;

#[derive(Clone, Copy, Debug)]
enum Cpu {
    Sse2,
    Avx2,
}

// Only this module constructs CPU tokens, after checking the required features.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(Cpu);

impl Accelerator {
    pub fn detect() -> Option<Self> {
        if is_x86_feature_detected!("avx2") {
            Some(Self(Cpu::Avx2))
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
        if is_x86_feature_detected!("avx2") {
            result.push(Self(Cpu::Avx2));
        }
        result
    }

    pub fn scan(self, input: &[u8]) -> Prefix {
        let mut prefix = Prefix::default();
        // SAFETY: the token guarantees CPU features; kernels bound all loads.
        unsafe {
            match self.0 {
                Cpu::Avx2 => scan_avx2(input, &mut prefix),
                Cpu::Sse2 => scan_sse2(input, &mut prefix),
            }
        }
        prefix
    }
}

// x86 has no unsigned byte comparisons. Bias the beginning of each range to
// i8::MIN and use one signed comparison against its end. OR'ing with 0x20 maps
// 'A'-'Z' to 'a'-'z' without mapping any other byte into that range.
#[target_feature(enable = "sse2")]
fn is_alphabet_sse2(block: __m128i) -> __m128i {
    let letters = _mm_cmplt_epi8(
        _mm_add_epi8(
            _mm_or_si128(block, _mm_set1_epi8(0x20)),
            _mm_set1_epi8((0x80 - b'a') as i8),
        ),
        _mm_set1_epi8(-128 + 26),
    );
    let digits = _mm_cmplt_epi8(
        _mm_add_epi8(block, _mm_set1_epi8((0x80 - b'0') as i8)),
        _mm_set1_epi8(-128 + 10),
    );
    _mm_or_si128(
        _mm_or_si128(letters, digits),
        _mm_or_si128(
            _mm_cmpeq_epi8(block, _mm_set1_epi8(b'+' as i8)),
            _mm_cmpeq_epi8(block, _mm_set1_epi8(b'/' as i8)),
        ),
    )
}

#[target_feature(enable = "avx2")]
fn is_alphabet_avx2(block: __m256i) -> __m256i {
    // The same biased signed comparisons as the 128-bit kernel.
    let letters = _mm256_cmpgt_epi8(
        _mm256_set1_epi8(-128 + 26),
        _mm256_add_epi8(
            _mm256_or_si256(block, _mm256_set1_epi8(0x20)),
            _mm256_set1_epi8((0x80 - b'a') as i8),
        ),
    );
    let digits = _mm256_cmpgt_epi8(
        _mm256_set1_epi8(-128 + 10),
        _mm256_add_epi8(block, _mm256_set1_epi8((0x80 - b'0') as i8)),
    );
    _mm256_or_si256(
        _mm256_or_si256(letters, digits),
        _mm256_or_si256(
            _mm256_cmpeq_epi8(block, _mm256_set1_epi8(b'+' as i8)),
            _mm256_cmpeq_epi8(block, _mm256_set1_epi8(b'/' as i8)),
        ),
    )
}

#[target_feature(enable = "sse2")]
fn scan_sse2(input: &[u8], prefix: &mut Prefix) {
    // Accumulate locally rather than updating validator state on every block.
    while input.len() - prefix.len >= 16 {
        // SAFETY: the complete 16-byte unaligned load fits the slice.
        let block = unsafe { _mm_loadu_si128(input.as_ptr().add(prefix.len).cast()) };
        let alphabet = is_alphabet_sse2(block);
        let alphabet_mask = _mm_movemask_epi8(alphabet) as u32;
        // Alphabet-only blocks need only be counted.
        if alphabet_mask == 0xffff {
            prefix.alphabet += 16;
            prefix.len += 16;
            continue;
        }
        // Line feeds and whitespace are accepted but not alphabet characters;
        // keep the LF mask separately for line/column tracking.
        let lf = _mm_cmpeq_epi8(block, _mm_set1_epi8(10));
        let whitespace = _mm_or_si128(
            _mm_cmpeq_epi8(block, _mm_set1_epi8(13)),
            _mm_or_si128(
                _mm_cmpeq_epi8(block, _mm_set1_epi8(32)),
                _mm_cmpeq_epi8(block, _mm_set1_epi8(9)),
            ),
        );
        let valid = _mm_movemask_epi8(_mm_or_si128(alphabet, _mm_or_si128(lf, whitespace)));
        if !prefix.masks(
            16,
            alphabet_mask,
            _mm_movemask_epi8(lf) as u32,
            valid as u32,
        ) {
            return;
        }
    }
}

#[target_feature(enable = "avx2")]
fn scan_avx2(input: &[u8], prefix: &mut Prefix) {
    while input.len() - prefix.len >= 32 {
        // SAFETY: the complete 32-byte unaligned load fits the slice.
        let block = unsafe { _mm256_loadu_si256(input.as_ptr().add(prefix.len).cast()) };
        let alphabet = is_alphabet_avx2(block);
        let alphabet_mask = _mm256_movemask_epi8(alphabet) as u32;
        if alphabet_mask == u32::MAX {
            prefix.alphabet += 32;
            prefix.len += 32;
            continue;
        }
        let lf = _mm256_cmpeq_epi8(block, _mm256_set1_epi8(10));
        let whitespace = _mm256_or_si256(
            _mm256_cmpeq_epi8(block, _mm256_set1_epi8(13)),
            _mm256_or_si256(
                _mm256_cmpeq_epi8(block, _mm256_set1_epi8(32)),
                _mm256_cmpeq_epi8(block, _mm256_set1_epi8(9)),
            ),
        );
        let valid =
            _mm256_movemask_epi8(_mm256_or_si256(alphabet, _mm256_or_si256(lf, whitespace)));
        if !prefix.masks(
            32,
            alphabet_mask,
            _mm256_movemask_epi8(lf) as u32,
            valid as u32,
        ) {
            return;
        }
    }
    // Consume a remaining 16-byte block before returning to the scalar tail.
    scan_sse2(input, prefix);
}

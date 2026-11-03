// Adapted from MimeKit's Encodings/Base64Validator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

use super::Prefix;
use std::arch::aarch64::*;

// Only this module constructs CPU tokens after checking the required features.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(());

impl Accelerator {
    pub fn detect() -> Option<Self> {
        (cfg!(target_endian = "little") && std::arch::is_aarch64_feature_detected!("neon"))
            .then_some(Self(()))
    }

    #[cfg(test)]
    pub fn available() -> Vec<Self> {
        Self::detect().into_iter().collect()
    }

    pub fn scan(self, input: &[u8]) -> Prefix {
        // SAFETY: the private token verifies NEON; the kernel bounds all loads.
        unsafe { scan_neon(input) }
    }
}

#[target_feature(enable = "neon")]
fn mask(bytes: uint8x16_t) -> u32 {
    // Comparison lanes are 0 or 255. Weight and sum each half independently
    // to emulate ExtractMostSignificantBits.
    // SAFETY: the weights contain exactly 16 bytes.
    let weights =
        unsafe { vld1q_u8([1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128].as_ptr()) };
    let bits = vandq_u8(bytes, weights);
    u32::from(vaddv_u8(vget_low_u8(bits))) | (u32::from(vaddv_u8(vget_high_u8(bits))) << 8)
}

#[target_feature(enable = "neon")]
fn is_alphabet(block: uint8x16_t) -> uint8x16_t {
    // Bias the start of each range to i8::MIN. OR'ing with 0x20 folds only
    // uppercase letters into the lowercase range.
    let letters = vcltq_s8(
        vreinterpretq_s8_u8(vaddq_u8(
            vorrq_u8(block, vdupq_n_u8(0x20)),
            vdupq_n_u8(0x80 - b'a'),
        )),
        vdupq_n_s8(-128 + 26),
    );
    let digits = vcltq_s8(
        vreinterpretq_s8_u8(vaddq_u8(block, vdupq_n_u8(0x80 - b'0'))),
        vdupq_n_s8(-128 + 10),
    );
    vorrq_u8(
        vorrq_u8(letters, digits),
        vorrq_u8(
            vceqq_u8(block, vdupq_n_u8(b'+')),
            vceqq_u8(block, vdupq_n_u8(b'/')),
        ),
    )
}

#[target_feature(enable = "neon")]
fn scan_neon(input: &[u8]) -> Prefix {
    // Accumulate locally rather than updating validator state on every block.
    let mut prefix = Prefix::default();
    while input.len() - prefix.len >= 16 {
        // SAFETY: the complete 16-byte unaligned load fits the slice.
        let block = unsafe { vld1q_u8(input.as_ptr().add(prefix.len)) };
        let alphabet = is_alphabet(block);
        let alphabet_mask = mask(alphabet);
        // Alphabet-only blocks need only be counted.
        if alphabet_mask == 0xffff {
            prefix.alphabet += 16;
            prefix.len += 16;
            continue;
        }
        // Keep line feeds separate from other harmless whitespace so the
        // consumed prefix can update line/column tracking without rescanning.
        let lf = vceqq_u8(block, vdupq_n_u8(10));
        let whitespace = vorrq_u8(
            vceqq_u8(block, vdupq_n_u8(13)),
            vorrq_u8(
                vceqq_u8(block, vdupq_n_u8(32)),
                vceqq_u8(block, vdupq_n_u8(9)),
            ),
        );
        let valid = mask(vorrq_u8(alphabet, vorrq_u8(lf, whitespace)));
        if !prefix.masks(16, alphabet_mask, mask(lf), valid) {
            break;
        }
    }
    prefix
}

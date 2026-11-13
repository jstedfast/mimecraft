// Adapted from MimeKit's Utils/Memory.cs and Utils/Utf8.cs.
// UTF-8 lookup algorithm attributed to simdutf/SimdUnicode (MIT); see scan.rs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

use super::clean::{Clean, Run, Validation, escapes_valid};
use super::{CURRENT, HIGH, LOW, Line, invalid_utf8_tail};
use crate::mime_compliance::encoding::base64::simd::is_alphabet;
use std::arch::aarch64::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(());

impl Accelerator {
    pub fn clean_content(
        self,
        input: &[u8],
        validation: Validation,
        column: u64,
        cr: bool,
        null: bool,
        eight: bool,
    ) -> Clean {
        // SAFETY: the token verifies NEON; all loads fit the real input slice.
        unsafe {
            match validation {
                Validation::Base64 => clean_neon::<1>(input, column, cr, null, eight),
                Validation::QuotedPrintable => clean_neon::<2>(input, column, cr, null, eight),
                _ => clean_neon::<0>(input, column, cr, null, eight),
            }
        }
    }

    pub fn detect() -> Option<Self> {
        (cfg!(target_endian = "little") && std::arch::is_aarch64_feature_detected!("neon"))
            .then_some(Self(()))
    }
    #[cfg(test)]
    pub fn available() -> Vec<Self> {
        Self::detect().into_iter().collect()
    }
    pub fn line(self, input: &[u8], null: bool, eight_bit: bool) -> Line {
        // SAFETY: the private token verifies NEON and the kernel bounds all loads.
        unsafe { line_neon(input, null, eight_bit) }
    }
    pub fn invalid_utf8(self, input: &[u8]) -> Option<usize> {
        // SAFETY: the token verifies little-endian NEON; loads stay in the slice.
        unsafe { utf8_neon(input) }
    }
}

#[target_feature(enable = "neon")]
fn clean_neon<const MODE: u8>(
    input: &[u8],
    column: u64,
    cr: bool,
    null: bool,
    eight: bool,
) -> Clean {
    let mut run = Run::new(column, cr);
    while input.len() - run.clean.examined >= 16 {
        let offset = run.clean.examined;
        // SAFETY: the complete 16-byte load fits the checked window.
        let block = unsafe { vld1q_u8(input.as_ptr().add(offset)) };
        if MODE != 1 && ((eight && vmaxvq_u8(block) >= 128) || (null && vminvq_u8(block) == 0)) {
            break;
        }
        let alphabet = if MODE == 1 {
            mask(is_alphabet(block))
        } else {
            0
        };
        if MODE == 1 && alphabet == 0xffff {
            if !run.push(input, 16, 0, 0, alphabet) {
                break;
            }
            continue;
        }
        let lf = mask(vceqq_u8(block, vdupq_n_u8(10)));
        let equals = if MODE == 2 {
            mask(vceqq_u8(block, vdupq_n_u8(61)))
        } else {
            0
        };
        let cr = if lf != 0 || equals != 0 || MODE == 1 {
            mask(vceqq_u8(block, vdupq_n_u8(13)))
        } else {
            0
        };
        if equals != 0 {
            // Unsigned subtraction folds the desired ASCII range to 0..N.
            let letters = vcltq_u8(
                vsubq_u8(vorrq_u8(block, vdupq_n_u8(32)), vdupq_n_u8(b'a')),
                vdupq_n_u8(6),
            );
            let digits = vcltq_u8(vsubq_u8(block, vdupq_n_u8(b'0')), vdupq_n_u8(10));
            let hex = mask(vorrq_u8(letters, digits));
            if !escapes_valid(input, offset, 16, equals, hex, cr, lf) {
                break;
            }
        }
        if MODE == 1 {
            let whitespace = mask(vorrq_u8(
                vceqq_u8(block, vdupq_n_u8(32)),
                vceqq_u8(block, vdupq_n_u8(9)),
            ));
            if alphabet | lf | cr | whitespace != 0xffff {
                break;
            }
        }
        if !run.push(input, 16, lf, cr, alphabet) {
            break;
        }
    }
    run.clean
}

#[target_feature(enable = "neon")]
fn mask(bytes: uint8x16_t) -> u32 {
    // Comparison lanes are 0 or 255. Weight the bits and sum each half
    // independently to emulate ExtractMostSignificantBits.
    // SAFETY: the weights contain exactly 16 bytes.
    let weights =
        unsafe { vld1q_u8([1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128].as_ptr()) };
    let bits = vandq_u8(bytes, weights);
    u32::from(vaddv_u8(vget_low_u8(bits))) | (u32::from(vaddv_u8(vget_high_u8(bits))) << 8)
}

#[target_feature(enable = "neon")]
fn line_neon(input: &[u8], null: bool, eight_bit: bool) -> Line {
    let mut line = Line::default();
    let mut offset = 0;
    while input.len() - offset >= 16 {
        // SAFETY: all 16 bytes fit the checked window.
        let block = unsafe { vld1q_u8(input.as_ptr().add(offset)) };
        let lf = mask(vceqq_u8(block, vdupq_n_u8(10)));
        let nulls = if null && line.null.is_none() {
            mask(vceqq_u8(block, vdupq_n_u8(0)))
        } else {
            0
        };
        let eight = if eight_bit && line.eight_bit.is_none() {
            mask(vcgeq_u8(block, vdupq_n_u8(128)))
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

#[target_feature(enable = "neon")]
fn utf8_neon(input: &[u8]) -> Option<usize> {
    // SAFETY: the lookup and maximum tables each contain exactly 16 bytes.
    let (high, low, next, max) = unsafe {
        (
            vld1q_u8(HIGH.as_ptr()),
            vld1q_u8(LOW.as_ptr()),
            vld1q_u8(CURRENT.as_ptr()),
            vld1q_u8(
                [
                    255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0xef, 0xdf,
                    0xbf,
                ]
                .as_ptr(),
            ),
        )
    };
    let mut previous = vdupq_n_u8(0);
    let mut incomplete = vdupq_n_u8(0);
    let mut offset = 0;
    while input.len() - offset >= 16 {
        // SAFETY: all 16 bytes fit the checked window.
        let current = unsafe { vld1q_u8(input.as_ptr().add(offset)) };
        if vmaxvq_u8(current) < 128 {
            if vmaxvq_u8(incomplete) != 0 {
                break;
            }
            incomplete = vdupq_n_u8(0);
        } else {
            let p1 = vextq_u8::<15>(previous, current);
            let p2 = vextq_u8::<14>(previous, current);
            let p3 = vextq_u8::<13>(previous, current);
            let h = vqtbl1q_u8(high, vshrq_n_u8::<4>(p1));
            let l = vqtbl1q_u8(low, vandq_u8(p1, vdupq_n_u8(15)));
            let n = vqtbl1q_u8(next, vshrq_n_u8::<4>(current));
            let required = vandq_u8(
                vorrq_u8(
                    vqsubq_u8(p2, vdupq_n_u8(0x60)),
                    vqsubq_u8(p3, vdupq_n_u8(0x70)),
                ),
                vdupq_n_u8(128),
            );
            let errors = veorq_u8(vandq_u8(vandq_u8(h, l), n), required);
            if vmaxvq_u8(errors) != 0 {
                break;
            }
            // Only the last three lanes can carry an unfinished sequence.
            incomplete = vqsubq_u8(current, max);
        }
        previous = current;
        offset += 16;
    }
    invalid_utf8_tail(input, offset, vmaxvq_u8(incomplete) != 0)
}

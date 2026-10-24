// Adapted from MimeKit's MimeReader Vector128 boundary scan.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

use std::arch::aarch64::*;

use super::Scan;

// Only this module can construct the token, after checking NEON support.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(());

impl Accelerator {
    pub(super) fn detect() -> Option<Self> {
        std::arch::is_aarch64_feature_detected!("neon").then_some(Self(()))
    }

    #[cfg(test)]
    pub(super) fn available() -> Vec<Self> {
        Self::detect().into_iter().collect()
    }

    pub(super) fn scan(self, input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
        // SAFETY: the token verifies NEON; the kernel bounds every vector load.
        unsafe { scan_neon(input, multipart, mbox, cr) }
    }
}

#[target_feature(enable = "neon")]
fn mask(bytes: uint8x16_t) -> u32 {
    // Comparison lanes are 0 or 255. Give each bit a distinct weight, then
    // sum each half independently to emulate Vector128.ExtractMostSignificantBits.
    // SAFETY: the table contains exactly 16 bytes.
    let weights =
        unsafe { vld1q_u8([1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128].as_ptr()) };
    let bits = vandq_u8(bytes, weights);
    u32::from(vaddv_u8(vget_low_u8(bits))) | (u32::from(vaddv_u8(vget_high_u8(bits))) << 8)
}

#[target_feature(enable = "neon")]
fn scan_neon(input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
    let vlf = vdupq_n_u8(b'\n');
    let vcr = vdupq_n_u8(b'\r');
    let dash = vdupq_n_u8(b'-');
    let f = vdupq_n_u8(b'F');
    let r = vdupq_n_u8(b'r');
    let mut scan = Scan::default();
    let mut carry = u32::from(cr);
    // Unlike MimeKit's sentinel-backed buffer, all shifted loads must fit
    // the real slice. The scalar tail also preserves partial marker prefixes.
    while input.len() - scan.len >= 18 {
        // SAFETY: all 16 bytes fit the 18-byte window checked above.
        let bytes = unsafe { vld1q_u8(input.as_ptr().add(scan.len)) };
        let lf = mask(vceqq_u8(bytes, vlf));
        let cr = mask(vceqq_u8(bytes, vcr));
        if lf != 0 {
            let mut possible = vdupq_n_u8(0);
            if multipart || mbox {
                // SAFETY: the shifted vectors end at len+17 and len+18.
                let (n1, n2) = unsafe {
                    (
                        vld1q_u8(input.as_ptr().add(scan.len + 1)),
                        vld1q_u8(input.as_ptr().add(scan.len + 2)),
                    )
                };
                if multipart {
                    possible = vandq_u8(vceqq_u8(n1, dash), vceqq_u8(n2, dash));
                }
                if mbox {
                    possible = vorrq_u8(possible, vandq_u8(vceqq_u8(n1, f), vceqq_u8(n2, r)));
                }
            }
            let candidates = lf & mask(possible);
            let crlf = lf & ((cr << 1) | carry);
            if scan.push_masks(lf, crlf, candidates) {
                return scan;
            }
        }
        // A CR in the final lane can pair with LF in the next vector's lane 0.
        carry = cr >> 15;
        scan.len += 16;
    }
    scan.finish(input, multipart, mbox, carry != 0)
}

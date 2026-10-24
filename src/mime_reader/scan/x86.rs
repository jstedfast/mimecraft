// Adapted from MimeKit's MimeReader Vector128/Vector256 boundary scans.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

use super::Scan;

#[derive(Clone, Copy, Debug)]
enum Cpu {
    Sse2,
    Avx2,
}

// Construction is private: safe entry points require a verified CPU token.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(Cpu);

impl Accelerator {
    pub(super) fn detect() -> Option<Self> {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("popcnt") {
            Some(Self(Cpu::Avx2))
        } else if is_x86_feature_detected!("sse2") {
            Some(Self(Cpu::Sse2))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn available() -> Vec<Self> {
        let mut result = Vec::new();
        if is_x86_feature_detected!("sse2") {
            result.push(Self(Cpu::Sse2));
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("popcnt") {
            result.push(Self(Cpu::Avx2));
        }
        result
    }

    pub(super) fn scan(self, input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
        // SAFETY: the token verifies all required features; each kernel bounds
        // its unaligned loads, including the two shifted lookahead vectors.
        unsafe {
            match self.0 {
                Cpu::Sse2 => scan_sse2(input, multipart, mbox, cr),
                Cpu::Avx2 => scan_avx2(input, multipart, mbox, cr),
            }
        }
    }
}

#[target_feature(enable = "sse2")]
fn scan_sse2(input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
    let vlf = _mm_set1_epi8(b'\n' as i8);
    let vcr = _mm_set1_epi8(b'\r' as i8);
    let dash = _mm_set1_epi8(b'-' as i8);
    let f = _mm_set1_epi8(b'F' as i8);
    let r = _mm_set1_epi8(b'r' as i8);
    let mut scan = Scan::default();
    let mut carry = u32::from(cr);
    // MimeKit's shifted loads can include its sentinel. Here all three loads
    // must fit the actual slice, leaving at most 17 bytes for the scalar tail.
    while input.len() - scan.len >= 18 {
        // SAFETY: [len..len+16) lies within the 18 bytes checked above.
        let bytes = unsafe { _mm_loadu_si128(input.as_ptr().add(scan.len).cast()) };
        let lf = _mm_movemask_epi8(_mm_cmpeq_epi8(bytes, vlf)) as u32;
        let cr = _mm_movemask_epi8(_mm_cmpeq_epi8(bytes, vcr)) as u32;
        if lf != 0 {
            let mut possible = _mm_setzero_si128();
            if multipart || mbox {
                // SAFETY: shifted loads end at len+17 and len+18, in bounds.
                let (n1, n2) = unsafe {
                    (
                        _mm_loadu_si128(input.as_ptr().add(scan.len + 1).cast()),
                        _mm_loadu_si128(input.as_ptr().add(scan.len + 2).cast()),
                    )
                };
                if multipart {
                    possible = _mm_and_si128(_mm_cmpeq_epi8(n1, dash), _mm_cmpeq_epi8(n2, dash));
                }
                if mbox {
                    possible = _mm_or_si128(
                        possible,
                        _mm_and_si128(_mm_cmpeq_epi8(n1, f), _mm_cmpeq_epi8(n2, r)),
                    );
                }
            }
            let candidates = lf & _mm_movemask_epi8(possible) as u32;
            let crlf = lf & ((cr << 1) | carry);
            if scan.push_masks(lf, crlf, candidates) {
                return scan;
            }
        }
        // Carry a final CR into lane zero of the next vector, so a split CRLF
        // has exactly the same line count and format as the per-line scanner.
        carry = cr >> 15;
        scan.len += 16;
    }
    scan.finish(input, multipart, mbox, carry != 0)
}

#[target_feature(enable = "avx2,popcnt")]
fn scan_avx2(input: &[u8], multipart: bool, mbox: bool, cr: bool) -> Scan {
    let vlf = _mm256_set1_epi8(b'\n' as i8);
    let vcr = _mm256_set1_epi8(b'\r' as i8);
    let dash = _mm256_set1_epi8(b'-' as i8);
    let f = _mm256_set1_epi8(b'F' as i8);
    let r = _mm256_set1_epi8(b'r' as i8);
    let mut scan = Scan::default();
    let mut carry = u32::from(cr);
    // No sentinel: the p+2 load must end within the borrowed slice.
    while input.len() - scan.len >= 34 {
        // SAFETY: the loop bounds cover all 32 bytes of this unaligned load.
        let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add(scan.len).cast()) };
        let lf = _mm256_movemask_epi8(_mm256_cmpeq_epi8(bytes, vlf)) as u32;
        let cr = _mm256_movemask_epi8(_mm256_cmpeq_epi8(bytes, vcr)) as u32;
        if lf != 0 {
            let mut possible = _mm256_setzero_si256();
            if multipart || mbox {
                // SAFETY: both shifted vectors fit the checked 34-byte window.
                let (n1, n2) = unsafe {
                    (
                        _mm256_loadu_si256(input.as_ptr().add(scan.len + 1).cast()),
                        _mm256_loadu_si256(input.as_ptr().add(scan.len + 2).cast()),
                    )
                };
                if multipart {
                    possible =
                        _mm256_and_si256(_mm256_cmpeq_epi8(n1, dash), _mm256_cmpeq_epi8(n2, dash));
                }
                if mbox {
                    possible = _mm256_or_si256(
                        possible,
                        _mm256_and_si256(_mm256_cmpeq_epi8(n1, f), _mm256_cmpeq_epi8(n2, r)),
                    );
                }
            }
            let candidates = lf & _mm256_movemask_epi8(possible) as u32;
            let crlf = lf & ((cr << 1) | carry);
            if scan.push_masks(lf, crlf, candidates) {
                return scan;
            }
        }
        carry = cr >> 31;
        scan.len += 32;
    }
    scan.finish(input, multipart, mbox, carry != 0)
}

// Adapted from MimeKit's AdvSimd UU kernels.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

use std::arch::aarch64::*;

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

    pub(super) fn encode_lines(self, input: &[u8], newline: &[u8], output: &mut Vec<u8>) -> usize {
        // SAFETY: the token verifies NEON; fixed-size lines bound all loads,
        // and full-vector stores go to a fixed-size local array.
        unsafe { encode_neon(input, newline, output) }
    }

    pub(super) fn decode_line(self, input: &[u8], output: &mut [u8; 72]) -> bool {
        assert!((16..=84).contains(&input.len()) && input.len().is_multiple_of(4));
        // SAFETY: NEON and the line bounds are verified; the largest scratch
        // store ends at byte 67, within the 72-byte output array.
        unsafe { decode_line_neon(input, output) }
    }

    pub(super) fn decode_quartets(self, input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
        // SAFETY: NEON is verified; the kernel bounds every load and uses
        // local stores, appending only complete quartets before CR/LF.
        unsafe { decode_quartets_neon(input, limit, output) }
    }
}

#[target_feature(enable = "neon")]
fn encode_vector(bytes: uint8x16_t, offset: u8) -> uint8x16_t {
    // The same four-triplet expansion as MimeKit's Vector128 kernel, expressed
    // with byte shuffles/shifts (as in our Base64 NEON port) to avoid assumptions
    // about word endianness:
    // left:  [a >> 2, a << 4, b << 2, c]
    // right: [     0, b >> 4, c >> 6, 0]
    // OR and mask give [a >> 2, ((a & 3) << 4) | (b >> 4), ...].
    // The tail uses offset 4 because it is loaded four bytes before its data.
    // SAFETY: all tables contain exactly 16 elements.
    let (left, right, left_shifts, right_shifts) = unsafe {
        (
            vld1q_u8([0, 0, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 9, 10, 11].as_ptr()),
            vld1q_u8([0, 1, 2, 0, 3, 4, 5, 0, 6, 7, 8, 0, 9, 10, 11, 0].as_ptr()),
            vld1q_s8([-2, 4, 2, 0, -2, 4, 2, 0, -2, 4, 2, 0, -2, 4, 2, 0].as_ptr()),
            vld1q_s8(
                [
                    -8, -4, -6, -8, -8, -4, -6, -8, -8, -4, -6, -8, -8, -4, -6, -8,
                ]
                .as_ptr(),
            ),
        )
    };
    let mask = vdupq_n_u8(63);
    let offset = vdupq_n_u8(offset);
    let indices = vandq_u8(
        vorrq_u8(
            vshlq_u8(vqtbl1q_u8(bytes, vaddq_u8(left, offset)), left_shifts),
            vshlq_u8(vqtbl1q_u8(bytes, vaddq_u8(right, offset)), right_shifts),
        ),
        mask,
    );
    // ((v + 63) & 63) + 33 maps 0 to backtick, and 1..63 to ASCII 33..95.
    vaddq_u8(vandq_u8(vaddq_u8(indices, mask), mask), vdupq_n_u8(33))
}

#[target_feature(enable = "neon")]
fn encode_neon(input: &[u8], newline: &[u8], output: &mut Vec<u8>) -> usize {
    let (lines, _) = input.as_chunks::<45>();
    for line in lines {
        let mut encoded = [0u8; 60];
        // The final block overlaps: load [29..45), encode [33..45),
        // store [44..60). No load crosses a line or reads before its start.
        // SAFETY: all loads fit the fixed 45-byte line; all stores fit encoded.
        unsafe {
            for (read, write) in [(0, 0), (12, 16), (24, 32), (29, 44)] {
                let bytes = vld1q_u8(line.as_ptr().add(read));
                let value = encode_vector(bytes, if read == 29 { 4 } else { 0 });
                vst1q_u8(encoded.as_mut_ptr().add(write), value);
            }
        }
        output.push(b'M');
        output.extend_from_slice(&encoded);
        output.extend_from_slice(newline);
    }
    lines.len() * 45
}

#[target_feature(enable = "neon")]
fn decode_vector(bytes: uint8x16_t) -> uint8x16_t {
    // Every byte maps to a sextet: (c - 0x20) & 0x3f. CR/LF are checked
    // separately before committing any output.
    let ranks = vandq_u8(vsubq_u8(bytes, vdupq_n_u8(32)), vdupq_n_u8(63));
    // Pack [a,b,c,d] into [a << 2 | b >> 4, b << 4 | c >> 2, c << 6 | d].
    // Byte-wise shifts are equivalent to MimeKit's word merge and byte shuffle,
    // without word-endianness assumptions. Table indices 16 zero unused lanes.
    // SAFETY: each table has 16 elements.
    let (left, right, left_shifts, right_shifts) = unsafe {
        (
            vld1q_u8([0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 16, 16, 16, 16].as_ptr()),
            vld1q_u8([1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15, 16, 16, 16, 16].as_ptr()),
            vld1q_s8([2, 4, 6, 2, 4, 6, 2, 4, 6, 2, 4, 6, 0, 0, 0, 0].as_ptr()),
            vld1q_s8([-4, -2, 0, -4, -2, 0, -4, -2, 0, -4, -2, 0, 0, 0, 0, 0].as_ptr()),
        )
    };
    vorrq_u8(
        vshlq_u8(vqtbl1q_u8(ranks, left), left_shifts),
        vshlq_u8(vqtbl1q_u8(ranks, right), right_shifts),
    )
}

#[target_feature(enable = "neon")]
fn line_breaks(bytes: uint8x16_t) -> uint8x16_t {
    vorrq_u8(
        vceqq_u8(bytes, vdupq_n_u8(b'\r')),
        vceqq_u8(bytes, vdupq_n_u8(b'\n')),
    )
}

#[target_feature(enable = "neon")]
fn decode_line_neon(input: &[u8], output: &mut [u8; 72]) -> bool {
    let mut index = 0;
    loop {
        let offset = index.min(input.len() - 16);
        // The last four quartets can overlap the previous vector.
        // SAFETY: validated length is a multiple of four in 16..=84,
        // offset+16 <= input.len(), and offset/4*3+16 <= 67.
        unsafe {
            let bytes = vld1q_u8(input.as_ptr().add(offset));
            if vmaxvq_u8(line_breaks(bytes)) != 0 {
                return false;
            }
            vst1q_u8(
                output.as_mut_ptr().add(offset / 4 * 3),
                decode_vector(bytes),
            );
        }
        if offset + 16 == input.len() {
            return true;
        }
        index += 16;
    }
}

#[target_feature(enable = "neon")]
fn decode_quartets_neon(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    // Valid lanes contribute 16; CR/LF lanes contribute their index. The
    // minimum is the first line break, equivalent to x86's movemask/ctz.
    // SAFETY: the table has exactly 16 entries.
    let positions =
        unsafe { vld1q_u8([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].as_ptr()) };
    let mut decoded = 0;
    while decoded < limit && input.len() - decoded * 4 >= 16 {
        let mut bytes = [0u8; 16];
        // SAFETY: loop bounds establish a complete load; the store is local.
        let groups = unsafe {
            let value = vld1q_u8(input.as_ptr().add(decoded * 4));
            let first = vminvq_u8(vbslq_u8(line_breaks(value), positions, vdupq_n_u8(16)));
            let groups = (usize::from(first) / 4).min(limit - decoded);
            if groups == 0 {
                break;
            }
            vst1q_u8(bytes.as_mut_ptr(), decode_vector(value));
            groups
        };
        output.extend_from_slice(&bytes[..groups * 3]);
        decoded += groups;
        if groups < 4 {
            break;
        }
    }
    decoded
}

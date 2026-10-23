// Adapted from MimeKit's AdvSimd Base64 kernels.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.
// MimeKit credits aklomp/base64 for the 48-byte NEON encoder and the .NET
// runtime / Wojciech Mula's base64simd project for the decoder lookup algorithm.

#![allow(unsafe_code)]

use std::arch::aarch64::*;

// The private field prevents constructing a token without checking NEON.
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

    pub(super) fn encode(self, input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
        // SAFETY: the capability token verifies NEON. The kernel checks every
        // input load and uses fixed-size local output arrays.
        unsafe { encode_neon(input, limit, output) }
    }

    // Returns (input bytes consumed as complete quartets, scalar prefix length).
    // Complete quartets before the first non-alphabet character are independent
    // and still decoded. From the updated input position, the scalar decoder
    // must handle the rest through that character, inclusive (including '=').
    // A zero scalar length means there was no invalid character before the tail.
    pub(super) fn decode(self, input: &[u8], output: &mut Vec<u8>) -> (usize, usize) {
        // SAFETY: the capability token verifies NEON; the kernel checks bounds.
        unsafe { decode_neon(input, output) }
    }
}

#[target_feature(enable = "neon")]
fn encode_neon(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    // SAFETY: ALPHABET contains exactly 64 bytes, enough for four vectors.
    let alphabet = unsafe { vld1q_u8_x4(super::ALPHABET.as_ptr()) };
    let mask = vdupq_n_u8(63);
    let limit = limit.min(input.len());
    let mut consumed = 0;
    while limit - consumed >= 48 {
        // Load 48 bytes and deinterleave the triplets: a holds input[0, 3, ...],
        // b holds input[1, 4, ...], and c holds input[2, 5, ...].
        // SAFETY: limit <= input.len(), and this load reads exactly 48 bytes.
        let uint8x16x3_t(a, b, c) = unsafe { vld3q_u8(input.as_ptr().add(consumed)) };
        // Divide each triplet's bits among four output indices:
        // input:  aaaaaabb bbbbcccc ccdddddd
        // output: 00aaaaaa 00bbbbbb 00cccccc 00dddddd
        // Shift-and-insert combines adjacent bytes; mask off the top two bits.
        let indices = uint8x16x4_t(
            vshrq_n_u8::<2>(a),
            vandq_u8(vsliq_n_u8::<4>(vshrq_n_u8::<4>(b), a), mask),
            vandq_u8(vsliq_n_u8::<2>(vshrq_n_u8::<6>(c), b), mask),
            vandq_u8(c, mask),
        );
        // Translate values 0..63 using the full 64-byte alphabet table.
        let encoded = uint8x16x4_t(
            vqtbl4q_u8(alphabet, indices.0),
            vqtbl4q_u8(alphabet, indices.1),
            vqtbl4q_u8(alphabet, indices.2),
            vqtbl4q_u8(alphabet, indices.3),
        );
        let mut bytes = [0u8; 64];
        // Interleave the four result vectors back into consecutive quartets.
        // SAFETY: vst4q writes exactly 64 bytes into the local array.
        unsafe { vst4q_u8(bytes.as_mut_ptr(), encoded) };
        output.extend_from_slice(&bytes);
        consumed += 48;
    }

    // Expand four triplets into 16 six-bit indices. Byte-wise shuffles and
    // shifts avoid word-endianness assumptions in this smaller-block path.
    // For each triplet [a, b, c], the tables select and shift these components:
    // left:  [a >> 2, a << 4, b << 2, c]
    // right: [     0, b >> 4, c >> 6, 0]
    // OR and mask with 63 to obtain the same four indices as the bulk loop.
    // Negative shift counts shift right; shifting a byte by -8 produces zero.
    // SAFETY: every constant below has exactly 16 elements.
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
    while input.len() - consumed >= 16 && limit - consumed >= 12 {
        // SAFETY: the loop checks 16 readable bytes, although only 12 are encoded.
        let bytes = unsafe { vld1q_u8(input.as_ptr().add(consumed)) };
        let indices = vandq_u8(
            vorrq_u8(
                vshlq_u8(vqtbl1q_u8(bytes, left), left_shifts),
                vshlq_u8(vqtbl1q_u8(bytes, right), right_shifts),
            ),
            mask,
        );
        let mut encoded = [0u8; 16];
        // SAFETY: the output vector fits the 16-byte local array.
        unsafe { vst1q_u8(encoded.as_mut_ptr(), vqtbl4q_u8(alphabet, indices)) };
        output.extend_from_slice(&encoded);
        consumed += 12;
    }
    consumed
}

#[target_feature(enable = "neon")]
fn decode_neon(input: &[u8], output: &mut Vec<u8>) -> (usize, usize) {
    // The Base64 alphabet maps back to six-bit ranks using these offsets:
    //   From      To        Add  Characters
    //   43        62        +19  +
    //   47        63        +16  /
    //   48..57    52..61      +4  0..9
    //   65..90     0..25     -65  A..Z
    //   97..122   26..51     -71  a..z
    // Everything else, including '=', must be handled by the scalar decoder.
    //
    // A byte is valid iff lut_hi[byte >> 4] & lut_lo[byte & 15] is zero.
    // The offset hash is (byte >> 4) - (byte == '/'), distinguishing '/' from
    // '+' despite their shared high nibble. Unsigned 191 and 185 represent
    // -65 and -71 in the wrapping byte additions below.
    // SAFETY: each lookup/shift table contains exactly 16 elements.
    let (lut_hi, lut_lo, lut_shift, positions, left, right, left_shifts, right_shifts) = unsafe {
        (
            vld1q_u8(
                [
                    0x10, 0x10, 1, 2, 4, 8, 4, 8, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
                ]
                .as_ptr(),
            ),
            vld1q_u8(
                [
                    0x15, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x13, 0x1a, 0x1b,
                    0x1b, 0x1b, 0x1a,
                ]
                .as_ptr(),
            ),
            vld1q_u8([0, 16, 19, 4, 191, 191, 185, 185, 0, 0, 0, 0, 0, 0, 0, 0].as_ptr()),
            vld1q_u8([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].as_ptr()),
            vld1q_u8([0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 16, 16, 16, 16].as_ptr()),
            vld1q_u8([1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15, 16, 16, 16, 16].as_ptr()),
            vld1q_s8([2, 4, 6, 2, 4, 6, 2, 4, 6, 2, 4, 6, 0, 0, 0, 0].as_ptr()),
            vld1q_s8([-4, -2, 0, -4, -2, 0, -4, -2, 0, -4, -2, 0, 0, 0, 0, 0].as_ptr()),
        )
    };
    let mut consumed = 0;
    while input.len() - consumed >= 16 {
        // SAFETY: the loop guarantees a complete 16-byte input load.
        let bytes = unsafe { vld1q_u8(input.as_ptr().add(consumed)) };
        let hi = vshrq_n_u8::<4>(bytes);
        let lo = vandq_u8(bytes, vdupq_n_u8(15));
        let invalid = vandq_u8(vqtbl1q_u8(lut_hi, hi), vqtbl1q_u8(lut_lo, lo));
        let valid = if vmaxvq_u8(invalid) == 0 {
            16
        } else {
            // Valid lanes contribute the sentinel 16; invalid lanes contribute
            // their position. The minimum is the first invalid byte's offset.
            vminvq_u8(vbslq_u8(
                vceqq_u8(invalid, vdupq_n_u8(0)),
                vdupq_n_u8(16),
                positions,
            )) as usize
        };
        let groups = valid / 4;
        if groups != 0 {
            let shift = vqtbl1q_u8(lut_shift, vaddq_u8(hi, vceqq_u8(bytes, vdupq_n_u8(b'/'))));
            let ranks = vaddq_u8(bytes, shift);
            // Pack each four ranks [a, b, c, d] into three bytes:
            // input:  00aaaaaa 00bbbbbb 00cccccc 00dddddd
            // output: aaaaaabb bbbbcccc ccdddddd
            // left:  [a << 2, b << 4, c << 6]
            // right: [b >> 4, c >> 2,      d]
            // Shifts discard overflowing bits. Out-of-range table indices (16)
            // zero the final four lanes, so no word-endianness conversion is needed.
            let packed = vorrq_u8(
                vshlq_u8(vqtbl1q_u8(ranks, left), left_shifts),
                vshlq_u8(vqtbl1q_u8(ranks, right), right_shifts),
            );
            let mut decoded = [0u8; 16];
            // SAFETY: store the full vector locally, appending only bytes from
            // complete valid quartets (at most 12 bytes).
            unsafe { vst1q_u8(decoded.as_mut_ptr(), packed) };
            output.extend_from_slice(&decoded[..groups * 3]);
            consumed += groups * 4;
        }
        if valid != 16 {
            // Only complete quartets were consumed. Hand the partial quartet
            // and the first invalid byte to scalar decoding before resuming.
            return (consumed, valid % 4 + 1);
        }
    }
    (consumed, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neon_load_sizes_and_line_limits() {
        let Some(accelerator) = Accelerator::detect() else {
            return;
        };
        let input = [0xff; 64];
        for (length, limit, expected) in [
            (15, 12, 0),
            (16, 11, 0),
            (16, 12, 12),
            (47, 48, 36),
            (48, 47, 36),
            (48, 48, 48),
            (60, 60, 48),
            (64, 60, 60),
        ] {
            let mut output = Vec::new();
            let consumed = accelerator.encode(&input[..length], limit, &mut output);
            assert_eq!(consumed, expected, "length={length}, limit={limit}");
            assert_eq!(output, vec![b'/'; consumed / 3 * 4]);
        }
    }

    #[test]
    fn neon_decode_stops_at_first_non_alphabet_byte() {
        let Some(accelerator) = Accelerator::detect() else {
            return;
        };
        for invalid in [b'=', b' ', b'\r', b'\n', 0, 0x80, 0xff] {
            for position in 0..32 {
                let mut input = [b'A'; 32];
                input[position] = invalid;
                let mut output = Vec::new();
                assert_eq!(
                    accelerator.decode(&input, &mut output),
                    (position / 4 * 4, position % 4 + 1)
                );
                assert_eq!(output, vec![0; position / 4 * 3]);
            }
        }
        for length in 0..=33 {
            let mut output = Vec::new();
            assert_eq!(
                accelerator.decode(&[b'A'; 33][..length], &mut output),
                (length / 16 * 16, 0)
            );
            assert_eq!(output, vec![0; length / 16 * 12]);
        }
    }
}

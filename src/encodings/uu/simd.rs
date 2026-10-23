// Adapted from MimeKit's SSSE3/AVX2 UU kernels.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

#![allow(unsafe_code)]

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[derive(Clone, Copy, Debug)]
enum Cpu {
    Ssse3,
    Avx2,
}

// Construction is private: safe entry points require a verified CPU token.
#[derive(Clone, Copy, Debug)]
pub(super) struct Accelerator(Cpu);

impl Accelerator {
    pub(super) fn detect() -> Option<Self> {
        if is_x86_feature_detected!("ssse3") {
            Some(Self(if is_x86_feature_detected!("avx2") {
                Cpu::Avx2
            } else {
                Cpu::Ssse3
            }))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn available() -> Vec<Self> {
        let mut result = Vec::new();
        if is_x86_feature_detected!("ssse3") {
            result.push(Self(Cpu::Ssse3));
            if is_x86_feature_detected!("avx2") {
                result.push(Self(Cpu::Avx2));
            }
        }
        result
    }

    pub(super) fn encode_lines(self, input: &[u8], newline: &[u8], output: &mut Vec<u8>) -> usize {
        // SAFETY: the token verifies CPU features. Kernels process fixed 45-byte
        // slices and store each line into a bounded local array.
        unsafe {
            match self.0 {
                Cpu::Ssse3 => encode_ssse3(input, newline, output),
                Cpu::Avx2 => encode_avx2(input, newline, output),
            }
        }
    }

    pub(super) fn decode_line(self, input: &[u8], output: &mut [u8; 72]) -> bool {
        // A length prefix holds at most 63 bytes, hence at most 21 quartets.
        assert!((16..=84).contains(&input.len()) && input.len().is_multiple_of(4));
        // SAFETY: features are verified, bounds above cover every overlapping
        // load and the maximum 71-byte scratch-store extent.
        unsafe {
            match self.0 {
                Cpu::Ssse3 => decode_line_ssse3(input, output),
                Cpu::Avx2 if input.len() >= 32 => decode_line_avx2(input, output),
                Cpu::Avx2 => decode_line_ssse3(input, output),
            }
        }
    }

    pub(super) fn decode_quartets(self, input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
        // SAFETY: features are verified; kernels bound every load and store
        // into local arrays, appending only complete quartets before CR/LF.
        unsafe {
            match self.0 {
                Cpu::Ssse3 => decode_quartets_ssse3(input, limit, output),
                Cpu::Avx2 => decode_quartets_avx2(input, limit, output),
            }
        }
    }
}

#[target_feature(enable = "ssse3")]
fn encode_vector(bytes: __m128i, shuffle: __m128i) -> __m128i {
    // Reshuffle each input triplet [a,b,c] into [b,a,c,b].
    // In MSB-to-LSB order, each word now holds b c a b:
    // maskAC: 0000bbbb CC000000 AAAAAA00 00000000
    // maskBB: 00000000 00cccccc 000000aa BBBB0000
    let bytes = _mm_shuffle_epi8(bytes, shuffle);
    let ac = _mm_mulhi_epu16(
        _mm_and_si128(bytes, _mm_set1_epi32(0x0fc0_fc00)),
        _mm_set1_epi32(0x0400_0040),
    );
    let bb = _mm_mullo_epi16(
        _mm_and_si128(bytes, _mm_set1_epi32(0x003f_03f0)),
        _mm_set1_epi32(0x0100_0010),
    );
    // Multiply-as-shifts and OR produce, MSB to LSB:
    // 00cccccc 00bbbbCC 00aaBBBB 00AAAAAA
    let indices = _mm_or_si128(ac, bb);
    // ((v + 63) & 63) is (v - 1) mod 64. Adding 33 maps zero
    // to backtick (96) and 1..63 to ASCII 33..95, without a lookup table.
    _mm_add_epi8(
        _mm_and_si128(_mm_add_epi8(indices, _mm_set1_epi8(63)), _mm_set1_epi8(63)),
        _mm_set1_epi8(33),
    )
}

#[target_feature(enable = "ssse3")]
fn encode_ssse3(input: &[u8], newline: &[u8], output: &mut Vec<u8>) -> usize {
    let shuffle = _mm_setr_epi8(1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10);
    let tail = _mm_add_epi8(shuffle, _mm_set1_epi8(4));
    let (lines, _) = input.as_chunks::<45>();
    for line in lines {
        let mut encoded = [0u8; 60];
        // A line is not a multiple of 12. The last block overlaps the previous
        // one: input [33..45) is loaded from [29..45), using a shifted shuffle.
        // SAFETY: loads [0..16), [12..28), [24..40), [29..45) stay in the line;
        // stores at 0,16,32,44 end at or before the 60-byte scratch boundary.
        unsafe {
            for (read, write) in [(0, 0), (12, 16), (24, 32), (29, 44)] {
                let bytes = _mm_loadu_si128(line.as_ptr().add(read).cast());
                let result = encode_vector(bytes, if read == 29 { tail } else { shuffle });
                _mm_storeu_si128(encoded.as_mut_ptr().add(write).cast(), result);
            }
        }
        output.push(b'M');
        output.extend_from_slice(&encoded);
        output.extend_from_slice(newline);
    }
    lines.len() * 45
}

#[target_feature(enable = "avx2")]
fn encode_vector_avx2(bytes: __m256i, shuffle: __m256i) -> __m256i {
    // Same sextet extraction and alphabet mapping, independently in each lane.
    let bytes = _mm256_shuffle_epi8(bytes, shuffle);
    let ac = _mm256_mulhi_epu16(
        _mm256_and_si256(bytes, _mm256_set1_epi32(0x0fc0_fc00)),
        _mm256_set1_epi32(0x0400_0040),
    );
    let bb = _mm256_mullo_epi16(
        _mm256_and_si256(bytes, _mm256_set1_epi32(0x003f_03f0)),
        _mm256_set1_epi32(0x0100_0010),
    );
    let indices = _mm256_or_si256(ac, bb);
    _mm256_add_epi8(
        _mm256_and_si256(
            _mm256_add_epi8(indices, _mm256_set1_epi8(63)),
            _mm256_set1_epi8(63),
        ),
        _mm256_set1_epi8(33),
    )
}

#[target_feature(enable = "avx2")]
fn encode_avx2(input: &[u8], newline: &[u8], output: &mut Vec<u8>) -> usize {
    // Each shuffle can access only its own 128-bit lane. Arrange the low lane's
    // useful bytes at [4..16), and the high lane's at [0..12).
    let shuffle = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let head = _mm256_setr_epi32(0, 0, 1, 2, 3, 4, 5, 6);
    let tail = _mm256_setr_epi32(0, 2, 3, 4, 5, 6, 7, 0);
    let (lines, _) = input.as_chunks::<45>();
    for line in lines {
        let mut encoded = [0u8; 60];
        // Input [0..24) -> output [0..32); input [21..45) -> [28..60).
        // SAFETY: 32-byte loads at 0 and 13 end within the 45-byte line;
        // overlapping 32-byte stores at 0 and 28 fit exactly in encoded.
        unsafe {
            let bytes = _mm256_loadu_si256(line.as_ptr().cast());
            let bytes = _mm256_permutevar8x32_epi32(bytes, head);
            _mm256_storeu_si256(
                encoded.as_mut_ptr().cast(),
                encode_vector_avx2(bytes, shuffle),
            );
            let bytes = _mm256_loadu_si256(line.as_ptr().add(13).cast());
            let bytes = _mm256_permutevar8x32_epi32(bytes, tail);
            _mm256_storeu_si256(
                encoded.as_mut_ptr().add(28).cast(),
                encode_vector_avx2(bytes, shuffle),
            );
        }
        output.push(b'M');
        output.extend_from_slice(&encoded);
        output.extend_from_slice(newline);
    }
    lines.len() * 45
}

#[target_feature(enable = "ssse3")]
fn decode_vector(bytes: __m128i) -> __m128i {
    // Every byte maps to a sextet; CR/LF validation is done by the caller.
    let ranks = _mm_and_si128(_mm_sub_epi8(bytes, _mm_set1_epi8(32)), _mm_set1_epi8(63));
    // Each 16-bit word becomes (a << 6) | b; each 32-bit word then becomes
    // (a << 18) | (b << 12) | (c << 6) | d.
    let pairs = _mm_maddubs_epi16(ranks, _mm_set1_epi32(0x0140_0140));
    let packed = _mm_madd_epi16(pairs, _mm_set1_epi32(0x0001_1000));
    // Reverse the three meaningful bytes of each word and pack them together.
    _mm_shuffle_epi8(
        packed,
        _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1),
    )
}

#[target_feature(enable = "ssse3")]
fn line_breaks(bytes: __m128i) -> u32 {
    _mm_movemask_epi8(_mm_or_si128(
        _mm_cmpeq_epi8(bytes, _mm_set1_epi8(b'\r' as i8)),
        _mm_cmpeq_epi8(bytes, _mm_set1_epi8(b'\n' as i8)),
    )) as u32
}

#[target_feature(enable = "avx2")]
fn decode_vector_avx2(bytes: __m256i) -> __m256i {
    let ranks = _mm256_and_si256(
        _mm256_sub_epi8(bytes, _mm256_set1_epi8(32)),
        _mm256_set1_epi8(63),
    );
    let pairs = _mm256_maddubs_epi16(ranks, _mm256_set1_epi32(0x0140_0140));
    let packed = _mm256_madd_epi16(pairs, _mm256_set1_epi32(0x0001_1000));
    let packed = _mm256_shuffle_epi8(
        packed,
        _mm256_setr_epi8(
            2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 2, 1, 0, 6, 5, 4, 10, 9, 8, 14,
            13, 12, -1, -1, -1, -1,
        ),
    );
    // Compact the two 12-byte results across the 128-bit lane boundary.
    _mm256_permutevar8x32_epi32(packed, _mm256_setr_epi32(0, 1, 2, 4, 5, 6, 3, 7))
}

#[target_feature(enable = "avx2")]
fn line_breaks_avx2(bytes: __m256i) -> u32 {
    _mm256_movemask_epi8(_mm256_or_si256(
        _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\r' as i8)),
        _mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(b'\n' as i8)),
    )) as u32
}

#[target_feature(enable = "ssse3")]
fn decode_line_ssse3(input: &[u8], output: &mut [u8; 72]) -> bool {
    let mut index = 0;
    loop {
        // The last four quartets may overlap the previous vector. Overlapping
        // stores overwrite scratch garbage with the same decoded payload bytes.
        let offset = index.min(input.len() - 16);
        // SAFETY: validated length is a multiple of four in 16..=84.
        // offset + 16 <= input.len(), and offset/4*3 + 16 <= 67.
        unsafe {
            let bytes = _mm_loadu_si128(input.as_ptr().add(offset).cast());
            if line_breaks(bytes) != 0 {
                return false;
            }
            _mm_storeu_si128(
                output.as_mut_ptr().add(offset / 4 * 3).cast(),
                decode_vector(bytes),
            );
        }
        if offset + 16 == input.len() {
            return true;
        }
        index += 16;
    }
}

#[target_feature(enable = "avx2")]
fn decode_line_avx2(input: &[u8], output: &mut [u8; 72]) -> bool {
    let mut index = 0;
    loop {
        let offset = index.min(input.len() - 32);
        // SAFETY: validated length is a multiple of four in 32..=84.
        // offset + 32 <= input.len(), and offset/4*3 + 32 <= 71.
        unsafe {
            let bytes = _mm256_loadu_si256(input.as_ptr().add(offset).cast());
            if line_breaks_avx2(bytes) != 0 {
                return false;
            }
            _mm256_storeu_si256(
                output.as_mut_ptr().add(offset / 4 * 3).cast(),
                decode_vector_avx2(bytes),
            );
        }
        if offset + 32 == input.len() {
            return true;
        }
        index += 32;
    }
}

#[target_feature(enable = "ssse3")]
fn decode_quartets_ssse3(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    let mut decoded = 0;
    while decoded < limit && input.len() - decoded * 4 >= 16 {
        let mut bytes = [0u8; 16];
        // SAFETY: loop checks the entire 16-byte load; the store is local.
        let groups = unsafe {
            let value = _mm_loadu_si128(input.as_ptr().add(decoded * 4).cast());
            let groups = (line_breaks(value).trailing_zeros() as usize / 4)
                .min(4)
                .min(limit - decoded);
            if groups == 0 {
                break;
            }
            _mm_storeu_si128(bytes.as_mut_ptr().cast(), decode_vector(value));
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

#[target_feature(enable = "avx2,ssse3")]
fn decode_quartets_avx2(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    let mut decoded = 0;
    while decoded < limit && input.len() - decoded * 4 >= 32 {
        let mut bytes = [0u8; 32];
        // SAFETY: loop checks the entire 32-byte load; the store is local.
        let groups = unsafe {
            let value = _mm256_loadu_si256(input.as_ptr().add(decoded * 4).cast());
            let groups = (line_breaks_avx2(value).trailing_zeros() as usize / 4)
                .min(8)
                .min(limit - decoded);
            if groups == 0 {
                break;
            }
            _mm256_storeu_si256(bytes.as_mut_ptr().cast(), decode_vector_avx2(value));
            groups
        };
        output.extend_from_slice(&bytes[..groups * 3]);
        decoded += groups;
        if groups < 8 {
            break;
        }
    }
    decoded + decode_quartets_ssse3(&input[decoded * 4..], limit - decoded, output)
}

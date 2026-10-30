// Adapted from MimeKit's SSSE3/AVX2 Base64 kernels.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.
// MimeKit credits the .NET runtime Base64 helpers and Wojciech Mula's
// base64simd project for the shuffle, translation, and packing algorithms.
// Its MIME-line tail optimization was inspired by simdutf/SimdBase64,
// but is specialized for this permissive decoder rather than a direct port.

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

// Only this module can construct the capability token, after checking the CPU.
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

    pub(super) fn encode(self, input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
        // SAFETY: construction verifies the required features. Kernels check
        // input bounds and limit before each load and use local arrays for stores.
        unsafe {
            match self.0 {
                Cpu::Ssse3 => encode_ssse3(input, limit, output),
                Cpu::Avx2 => encode_avx2(input, limit, output),
            }
        }
    }

    // Returns (input bytes consumed, scalar prefix length). Consumed input
    // contains complete quartets and, for AVX2 MIME-line tails, their newline.
    // Quartets before the first non-alphabet character are independent and can
    // still be decoded. From the updated input position, the scalar decoder must
    // handle the remaining characters through that invalid character, inclusive.
    // Padding '=' counts as invalid here, even though the scalar decoder accepts it.
    // A zero scalar length means no invalid character needs scalar processing.
    // A run always advances when input.len() >= 16.
    pub(super) fn decode(self, input: &[u8], output: &mut Vec<u8>) -> (usize, usize) {
        // SAFETY: same feature and bounds invariants as encode().
        unsafe {
            match self.0 {
                Cpu::Ssse3 => decode_ssse3(input, output),
                Cpu::Avx2 => decode_avx2(input, output),
            }
        }
    }
}

#[target_feature(enable = "ssse3")]
fn encode_vector(str: __m128i) -> __m128i {
    // Input bytes, MSB to LSB (the four high bytes are read but not encoded):
    // ? ? ? ? l k j i h g f e d c b a
    let shuffle = _mm_setr_epi8(1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10);
    let lut = _mm_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -19, -16, 0, 0,
    );
    // Reshuffle each triplet into a four-byte group, MSB to LSB:
    // k l j k
    // h i g h
    // e f d e
    // b c a b
    let str = _mm_shuffle_epi8(str, shuffle);
    // Bits: uppercase are the most significant bits of each original byte,
    // lowercase the least significant bits. After masking with 0x0fc0fc00:
    // 0000kkkk LL000000 JJJJJJ00 00000000
    // 0000hhhh II000000 GGGGGG00 00000000
    // 0000eeee FF000000 DDDDDD00 00000000
    // 0000bbbb CC000000 AAAAAA00 00000000
    //
    // Taking the high half of each 16-bit product shifts these fields down:
    // 00000000 00kkkkLL 00000000 00JJJJJJ
    // 00000000 00hhhhII 00000000 00GGGGGG
    // 00000000 00eeeeFF 00000000 00DDDDDD
    // 00000000 00bbbbCC 00000000 00AAAAAA
    let ac = _mm_mulhi_epu16(
        _mm_and_si128(str, _mm_set1_epi32(0x0fc0_fc00)),
        _mm_set1_epi32(0x0400_0040),
    );
    // The complementary mask, 0x003f03f0, selects:
    // 00000000 00llllll 000000jj KKKK0000
    // 00000000 00iiiiii 000000gg HHHH0000
    // 00000000 00ffffff 000000dd EEEE0000
    // 00000000 00cccccc 000000aa BBBB0000
    //
    // Taking the low half of each 16-bit product shifts these fields up:
    // 00llllll 00000000 00jjKKKK 00000000
    // 00iiiiii 00000000 00ggHHHH 00000000
    // 00ffffff 00000000 00ddEEEE 00000000
    // 00cccccc 00000000 00aaBBBB 00000000
    let bb = _mm_mullo_epi16(
        _mm_and_si128(str, _mm_set1_epi32(0x003f_03f0)),
        _mm_set1_epi32(0x0100_0010),
    );
    // Combining the disjoint fields gives sixteen six-bit alphabet indices:
    // 00llllll 00kkkkLL 00jjKKKK 00JJJJJJ
    // 00iiiiii 00hhhhII 00ggHHHH 00GGGGGG
    // 00ffffff 00eeeeFF 00ddEEEE 00DDDDDD
    // 00cccccc 00bbbbCC 00aaBBBB 00AAAAAA
    let indices = _mm_or_si128(ac, bb);
    // Translate values 0..63 to the Base64 alphabet using five offset ranges:
    //   From      To        Add    LUT index  Characters
    //   0..25     65..90     +65       0      A..Z
    //  26..51     97..122    +71       1      a..z
    //  52..61     48..57      -4     2..11    0..9
    //  62         43        -19      12      +
    //  63         47        -16      13      /
    //
    // Saturating subtraction by 51 produces 0 for letters and 1..12 otherwise.
    // The comparison produces -1 for values above 25; subtracting it adds 1,
    // separating lowercase from uppercase and correcting all remaining indices.
    let ranges = _mm_sub_epi8(
        _mm_subs_epu8(indices, _mm_set1_epi8(51)),
        _mm_cmpgt_epi8(indices, _mm_set1_epi8(25)),
    );
    _mm_add_epi8(indices, _mm_shuffle_epi8(lut, ranges))
}

#[target_feature(enable = "ssse3")]
fn encode_ssse3(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    let mut consumed = 0;
    let limit = limit.min(input.len());
    while input.len() - consumed >= 16 && limit - consumed >= 12 {
        let mut encoded = [0u8; 16];
        // SAFETY: the loop guarantees 16 readable bytes; the local output is
        // exactly 16 bytes. Unaligned loads/stores impose no alignment requirement.
        unsafe {
            let str = _mm_loadu_si128(input.as_ptr().add(consumed).cast());
            _mm_storeu_si128(encoded.as_mut_ptr().cast(), encode_vector(str));
        }
        output.extend_from_slice(&encoded);
        consumed += 12;
    }
    consumed
}

#[target_feature(enable = "avx2")]
fn encode_vector_avx2(str: __m256i) -> __m256i {
    // Apply encode_vector's bit splitting and alphabet translation independently
    // in each 128-bit lane. The caller loads bytes 0..15 and 12..27, so both
    // lanes start with 12 useful bytes and can use the same shuffle:
    // high lane, MSB to LSB: ? ? ? ? x w v u t s r q p o n m
    // low lane,  MSB to LSB: ? ? ? ? l k j i h g f e d c b a
    //
    // Unlike MimeKit's shifted 32-byte load, these overlapping loads need no
    // initial four-byte permutation and never read before the input slice.
    let shuffle = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10,
    ));
    let lut = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, -19, -16, 0, 0,
    ));
    // After reshuffling, each lane contains four groups of the form b c a b.
    // The masks and multiply-as-shifts below have the same bit layouts as
    // encode_vector(), repeated across eight triplets instead of four.
    let str = _mm256_shuffle_epi8(str, shuffle);
    let ac = _mm256_mulhi_epu16(
        _mm256_and_si256(str, _mm256_set1_epi32(0x0fc0_fc00)),
        _mm256_set1_epi32(0x0400_0040),
    );
    let bb = _mm256_mullo_epi16(
        _mm256_and_si256(str, _mm256_set1_epi32(0x003f_03f0)),
        _mm256_set1_epi32(0x0100_0010),
    );
    let indices = _mm256_or_si256(ac, bb);
    // Use the same five alphabet ranges as encode_vector(): saturated index
    // minus the comparison mask, followed by adding the selected ASCII offset.
    let ranges = _mm256_sub_epi8(
        _mm256_subs_epu8(indices, _mm256_set1_epi8(51)),
        _mm256_cmpgt_epi8(indices, _mm256_set1_epi8(25)),
    );
    _mm256_add_epi8(indices, _mm256_shuffle_epi8(lut, ranges))
}

#[target_feature(enable = "avx2,ssse3")]
fn encode_avx2(input: &[u8], limit: usize, output: &mut Vec<u8>) -> usize {
    let mut consumed = 0;
    let limit = limit.min(input.len());
    while input.len() - consumed >= 28 && limit - consumed >= 24 {
        let mut encoded = [0u8; 32];
        // SAFETY: overlapping 16-byte loads cover [consumed, consumed + 28),
        // checked above. Only the first 12 bytes of each lane are encoded.
        unsafe {
            let lo = _mm_loadu_si128(input.as_ptr().add(consumed).cast());
            let hi = _mm_loadu_si128(input.as_ptr().add(consumed + 12).cast());
            let str = _mm256_inserti128_si256::<1>(_mm256_castsi128_si256(lo), hi);
            _mm256_storeu_si256(encoded.as_mut_ptr().cast(), encode_vector_avx2(str));
        }
        output.extend_from_slice(&encoded);
        consumed += 24;
    }
    consumed + encode_ssse3(&input[consumed..], limit - consumed, output)
}

#[target_feature(enable = "ssse3")]
fn decode_ssse3(input: &[u8], output: &mut Vec<u8>) -> (usize, usize) {
    // Map the Base64 alphabet back to six-bit values. There are three ranges,
    // two single characters, and then everything else (including '='):
    //   From      To        Add  Characters
    //   43        62        +19  +
    //   47        63        +16  /
    //   48..57    52..61      +4  0..9
    //   65..90     0..25     -65  A..Z
    //   97..122   26..51     -71  a..z
    //
    // A character is valid iff its high- and low-nibble lookup masks AND to 0.
    // The offset table uses the perfect hash (c >> 4) - (c == '/'): '/' shares
    // a high nibble with '+', so its comparison mask (0xff) moves it to slot 1.
    let lut_hi = _mm_setr_epi8(
        0x10, 0x10, 1, 2, 4, 8, 4, 8, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    );
    let lut_lo = _mm_setr_epi8(
        0x15, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x13, 0x1a, 0x1b, 0x1b, 0x1b,
        0x1a,
    );
    let lut_shift = _mm_setr_epi8(0, 16, 19, 4, -65, -65, -71, -71, 0, 0, 0, 0, 0, 0, 0, 0);
    let pack = _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1);
    let mut consumed = 0;
    while input.len() - consumed >= 16 {
        // SAFETY: the loop condition guarantees a complete 16-byte input load.
        let str = unsafe { _mm_loadu_si128(input.as_ptr().add(consumed).cast()) };
        // Shuffle uses the low four index bits; masking also clears its high
        // zero-output flag. Bits carried in from adjacent bytes are ignored.
        let hi = _mm_and_si128(_mm_srli_epi32::<4>(str), _mm_set1_epi8(0x2f));
        let lo = _mm_and_si128(str, _mm_set1_epi8(0x2f));
        let invalid = _mm_and_si128(_mm_shuffle_epi8(lut_hi, hi), _mm_shuffle_epi8(lut_lo, lo));
        let valid_mask = _mm_movemask_epi8(_mm_cmpeq_epi8(invalid, _mm_setzero_si128())) as u32;
        // Find the first non-alphabet character. Only whole quartets preceding
        // it can be decoded here, even if subsequent characters are valid.
        let valid = ((!valid_mask) & 0xffff).trailing_zeros().min(16) as usize;
        let groups = valid / 4;
        if groups != 0 {
            let shift = _mm_shuffle_epi8(
                lut_shift,
                _mm_add_epi8(hi, _mm_cmpeq_epi8(str, _mm_set1_epi8(b'/' as i8))),
            );
            let ranks = _mm_add_epi8(str, shift);
            // Six-bit ranks, MSB to LSB; capitals/lowercase identify the high/
            // low bits of the original decoded bytes:
            // 00llllll 00kkkkLL 00jjKKKK 00JJJJJJ
            // 00iiiiii 00hhhhII 00ggHHHH 00GGGGGG
            // 00ffffff 00eeeeFF 00ddEEEE 00DDDDDD
            // 00cccccc 00bbbbCC 00aaBBBB 00AAAAAA
            //
            // Multiply adjacent byte pairs by [64, 1] and add into 16-bit words:
            // 0000kkkk LLllllll 0000JJJJ JJjjKKKK
            // 0000hhhh IIiiiiii 0000GGGG GGggHHHH
            // 0000eeee FFffffff 0000DDDD DDddEEEE
            // 0000bbbb CCcccccc 0000AAAA AAaaBBBB
            let merged = _mm_maddubs_epi16(ranks, _mm_set1_epi32(0x0140_0140));
            // Multiply adjacent word pairs by [4096, 1] to join each quartet:
            // 00000000 JJJJJJjj KKKKkkkk LLllllll
            // 00000000 GGGGGGgg HHHHhhhh IIiiiiii
            // 00000000 DDDDDDdd EEEEeeee FFffffff
            // 00000000 AAAAAAaa BBBBbbbb CCcccccc
            let packed = _mm_madd_epi16(merged, _mm_set1_epi32(0x0001_1000));
            let mut decoded = [0u8; 16];
            // Shuffle bytes back into stream order, removing each zero byte:
            // 00000000 00000000 00000000 00000000
            // LLllllll KKKKkkkk JJJJJJjj IIiiiiii
            // HHHHhhhh GGGGGGgg FFffffff EEEEeeee
            // DDDDDDdd CCcccccc BBBBbbbb AAAAAAaa
            //
            // SAFETY: decoded has space for the entire vector, even for partial
            // runs. Only complete valid quartets are appended to the caller's Vec.
            unsafe {
                _mm_storeu_si128(decoded.as_mut_ptr().cast(), _mm_shuffle_epi8(packed, pack))
            };
            output.extend_from_slice(&decoded[..groups * 3]);
            consumed += groups * 4;
        }
        if valid != 16 {
            // Include the invalid byte in the scalar handoff, not just the
            // leftover valid characters of its incomplete quartet.
            return (consumed, valid % 4 + 1);
        }
    }
    (consumed, 0)
}

#[target_feature(enable = "avx2,ssse3")]
fn decode_avx2(input: &[u8], output: &mut Vec<u8>) -> (usize, usize) {
    // See decode_ssse3() for the nibble validation, ASCII offsets, and packing
    // bit layouts. AVX2 applies the same operations to two 128-bit lanes.
    // Decode full blocks and the 12-character tails of 76-character MIME lines;
    // leave other whitespace, garbage and padding to the narrower/scalar paths.
    let lut_hi = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0x10, 0x10, 1, 2, 4, 8, 4, 8, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    ));
    let lut_lo = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0x15, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x13, 0x1a, 0x1b, 0x1b, 0x1b,
        0x1a,
    ));
    let lut_shift = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        0, 16, 19, 4, -65, -65, -71, -71, 0, 0, 0, 0, 0, 0, 0, 0,
    ));
    let pack = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1,
    ));
    let mut consumed = 0;
    while input.len() - consumed >= 32 {
        // SAFETY: the loop condition guarantees a complete 32-byte input load.
        let str = unsafe { _mm256_loadu_si256(input.as_ptr().add(consumed).cast()) };
        let hi = _mm256_and_si256(_mm256_srli_epi32::<4>(str), _mm256_set1_epi8(0x2f));
        let lo = _mm256_and_si256(str, _mm256_set1_epi8(0x2f));
        let invalid = _mm256_and_si256(
            _mm256_shuffle_epi8(lut_hi, hi),
            _mm256_shuffle_epi8(lut_lo, lo),
        );
        if _mm256_testz_si256(invalid, invalid) == 0 {
            // Let SSSE3 salvage complete quartets before other invalid bytes.
            let valid = _mm_movemask_epi8(_mm_cmpeq_epi8(
                _mm256_castsi256_si128(invalid),
                _mm_setzero_si128(),
            ));
            if valid & 0xfff != 0xfff {
                break;
            }
            let tail_length = if input[consumed + 12] == b'\n' {
                13
            } else if input[consumed + 12] == b'\r' && input[consumed + 13] == b'\n' {
                14
            } else {
                0
            };
            if tail_length == 0 {
                break;
            }
            // Stay in this loop for the 12 characters left after two
            // 32-character blocks. Only those characters must be valid.
            let tail = _mm256_castsi256_si128(str);
            let shift = _mm_shuffle_epi8(
                _mm256_castsi256_si128(lut_shift),
                _mm_add_epi8(
                    _mm256_castsi256_si128(hi),
                    _mm_cmpeq_epi8(tail, _mm_set1_epi8(b'/' as i8)),
                ),
            );
            let ranks = _mm_add_epi8(tail, shift);
            let merged = _mm_maddubs_epi16(ranks, _mm_set1_epi32(0x0140_0140));
            let packed = _mm_madd_epi16(merged, _mm_set1_epi32(0x0001_1000));
            let result = _mm_shuffle_epi8(packed, _mm256_castsi256_si128(pack));
            let mut decoded = [0u8; 16];
            // SAFETY: the local holds the entire vector. Append exactly
            // nine bytes, not the remaining invalid quartet's output.
            // Unlike MimeKit's in-place API, input and output cannot alias.
            unsafe { _mm_storeu_si128(decoded.as_mut_ptr().cast(), result) };
            output.extend_from_slice(&decoded[..9]);
            consumed += tail_length;
            continue;
        }
        let shift = _mm256_shuffle_epi8(
            lut_shift,
            _mm256_add_epi8(hi, _mm256_cmpeq_epi8(str, _mm256_set1_epi8(b'/' as i8))),
        );
        let ranks = _mm256_add_epi8(str, shift);
        let merged = _mm256_maddubs_epi16(ranks, _mm256_set1_epi32(0x0140_0140));
        let packed = _mm256_madd_epi16(merged, _mm256_set1_epi32(0x0001_1000));
        let lanes = _mm256_shuffle_epi8(packed, pack);
        // Each lane has 12 decoded bytes followed by four zero bytes. Select
        // 32-bit words [0, 1, 2, 4, 5, 6] to close the gap between the lanes.
        // The final two words are outside the 24 meaningful output bytes.
        let compact = _mm256_permutevar8x32_epi32(lanes, _mm256_setr_epi32(0, 1, 2, 4, 5, 6, 7, 7));
        let mut decoded = [0u8; 32];
        // SAFETY: store the full vector into a 32-byte local, then append only
        // the 24 decoded bytes. Caller output capacity is managed by Vec.
        unsafe { _mm256_storeu_si256(decoded.as_mut_ptr().cast(), compact) };
        output.extend_from_slice(&decoded[..24]);
        consumed += 32;
    }
    let (tail, scalar) = decode_ssse3(&input[consumed..], output);
    (consumed + tail, scalar)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avx2_consumes_mime_tails_and_continues_without_scalar_handoff() {
        if !is_x86_feature_detected!("avx2") || !is_x86_feature_detected!("ssse3") {
            return;
        }
        for newline in [b"\n".as_slice(), b"\r\n"] {
            // A standalone tail is also valid at a streaming chunk boundary.
            for prefix in [0, 32, 64] {
                let mut input = vec![b'A'; prefix + 12];
                input.extend_from_slice(newline);
                input.extend_from_slice(&[b'A'; 32]);
                let mut output = b"prefix".to_vec();
                let (consumed, scalar) = Accelerator(Cpu::Avx2).decode(&input, &mut output);
                assert_eq!((consumed, scalar), (input.len(), 0));
                assert_eq!(&output[..6], b"prefix");
                assert_eq!(&output[6..], vec![0; prefix / 4 * 3 + 9 + 24]);
            }
        }
    }
}

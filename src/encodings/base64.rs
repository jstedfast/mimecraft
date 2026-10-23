// Ported from MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{EncoderOptionsError, InvalidLineLength, NewLineFormat};

#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
mod simd;

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
mod neon;
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
use neon as simd;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
enum Backend {
    Scalar,
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    Simd(simd::Accelerator),
}

impl Backend {
    fn detect(length: usize) -> Self {
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if length >= 16
            && let Some(accelerator) = simd::Accelerator::detect()
        {
            return Self::Simd(accelerator);
        }
        let _ = length;
        Self::Scalar
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_triplet(bytes: [u8; 3], count: usize) -> [u8; 4] {
    let [a, b, c] = bytes;
    [
        ALPHABET[(a >> 2) as usize],
        ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize],
        if count > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize]
        } else {
            b'='
        },
        if count > 2 {
            ALPHABET[(c & 63) as usize]
        } else {
            b'='
        },
    ]
}

pub(super) fn encode_unwrapped(input: &[u8], output: &mut Vec<u8>) {
    encode_unwrapped_with_backend(input, output, Backend::detect(input.len()));
}

fn encode_unwrapped_with_backend(input: &[u8], output: &mut Vec<u8>, backend: Backend) {
    #[cfg(all(
        feature = "simd",
        any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
    ))]
    let input = if let Backend::Simd(accelerator) = backend {
        // Encoded-word payloads have no MIME line boundaries. Reuse the same
        // bounded vector loads with the entire slice available for encoding.
        let consumed = accelerator.encode(input, input.len(), output);
        &input[consumed..]
    } else {
        input
    };
    let _ = backend;
    let (triplets, remainder) = input.as_chunks::<3>();
    for &triplet in triplets {
        output.extend_from_slice(&encode_triplet(triplet, 3));
    }
    if !remainder.is_empty() {
        let mut bytes = [0; 3];
        bytes[..remainder.len()].copy_from_slice(remainder);
        output.extend_from_slice(&encode_triplet(bytes, remainder.len()));
    }
}

static RANK: [u8; 256] = {
    let mut table = [0xff; 256];
    let mut index = 0;
    while index < ALPHABET.len() {
        table[ALPHABET[index] as usize] = index as u8;
        index += 1;
    }
    table[b'=' as usize] = 0;
    table
};

/// An incremental Base64 encoder with MIME line wrapping.
///
/// Defaults to CRLF line endings and a 76-character line length, excluding
/// newline bytes. Use [`with_new_line_format`](Self::with_new_line_format) or
/// [`with_options`](Self::with_options) to select LF instead. Input bytes are
/// encoded unchanged, including any input newlines.
///
/// Call [`finish`](Self::finish) to emit padding and terminate the final line.
/// Dropping the encoder does not flush pending input.
#[derive(Clone, Debug)]
pub struct Base64Encoder {
    line_length: usize,
    new_line_format: NewLineFormat,
    column: usize,
    pending: [u8; 3],
    pending_len: usize,
}

impl Default for Base64Encoder {
    fn default() -> Self {
        Self {
            line_length: 76,
            new_line_format: NewLineFormat::default(),
            column: 0,
            pending: [0; 3],
            pending_len: 0,
        }
    }
}

impl Base64Encoder {
    /// Creates an encoder with CRLF endings and a 76-character line length.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder with CRLF endings and a requested maximum line length.
    ///
    /// Preserves MimeKit's public constructor behavior: accepted lengths are
    /// capped at 76 and rounded down to a multiple of four.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLineLength`] if the value is outside 60..=998.
    pub fn with_line_length(max_line_length: usize) -> Result<Self, InvalidLineLength> {
        if !(60..=998).contains(&max_line_length) {
            return Err(InvalidLineLength {
                requested: max_line_length,
            });
        }

        // MIME Base64 still caps lines at 76 characters even when a larger
        // line length is requested; complete four-character groups must fit.
        Ok(Self {
            line_length: max_line_length.min(76) / 4 * 4,
            ..Self::default()
        })
    }

    /// Creates an encoder with the selected endings and a 76-character line length.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`].
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        Self::with_options(76, new_line_format)
    }

    /// Creates an encoder with a requested line length and line ending format.
    ///
    /// As with [`with_line_length`](Self::with_line_length), the length is capped
    /// at 76 and rounded down to a multiple of four, excluding newline bytes.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`], or [`EncoderOptionsError::InvalidLineLength`]
    /// if the length is outside 60..=998. The format is checked first.
    ///
    /// ```
    /// use mimecraft::encodings::{Base64Encoder, NewLineFormat};
    ///
    /// let mut encoder = Base64Encoder::with_options(60, NewLineFormat::Unix).unwrap();
    /// let mut output = Vec::new();
    /// encoder.encode(b"hello", &mut output);
    /// encoder.finish(&mut output);
    /// assert_eq!(output, b"aGVsbG8=\n");
    /// ```
    pub fn with_options(
        max_line_length: usize,
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }

        Ok(Self {
            new_line_format,
            ..Self::with_line_length(max_line_length)
                .map_err(EncoderOptionsError::InvalidLineLength)?
        })
    }

    /// Returns the effective line length, excluding the newline.
    pub fn line_length(&self) -> usize {
        self.line_length
    }

    /// Returns the selected line ending format.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Appends encoded input to `output`, retaining at most two input bytes.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.encode_with_backend(input, output, Backend::detect(input.len()));
    }

    fn encode_with_backend(&mut self, mut input: &[u8], output: &mut Vec<u8>, backend: Backend) {
        // Complete any partially read triplet from the previous call before
        // handing whole triplets to the SIMD or scalar loops.
        if self.pending_len != 0 {
            let count = (3 - self.pending_len).min(input.len());
            self.pending[self.pending_len..self.pending_len + count]
                .copy_from_slice(&input[..count]);
            self.pending_len += count;
            input = &input[count..];

            if self.pending_len != 3 {
                return;
            }

            self.emit(self.pending, 3, output);
            self.pending_len = 0;
        }

        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Backend::Simd(accelerator) = backend {
            while input.len() >= 16 {
                // A 76-character MIME line holds 19 quartets, or 57 input bytes.
                // NEON encodes its first 48 bytes in one pass, AVX2 in two,
                // and SSSE3 in four; the remaining 9 bytes are scalar.
                // Limit consumed input separately from readable input: vector
                // loads may look ahead, but must not encode across a newline.
                let line_input = (self.line_length - self.column) / 4 * 3;
                let consumed = accelerator.encode(input, line_input, output);
                if consumed != 0 {
                    self.column += consumed / 3 * 4;
                    if self.column == self.line_length {
                        output.extend_from_slice(self.new_line_format.as_bytes());
                        self.column = 0;
                    }
                    input = &input[consumed..];
                } else {
                    // Too little room on this line for a vector; advance by
                    // one triplet until a vector fits or a new line begins.
                    self.emit([input[0], input[1], input[2]], 3, output);
                    input = &input[3..];
                }
            }
        }
        let _ = backend;

        let (triplets, remainder) = input.as_chunks::<3>();
        for &triplet in triplets {
            self.emit(triplet, 3, output);
        }

        // Keep up to two remaining input bytes for the next call or finish().
        self.pending[..remainder.len()].copy_from_slice(remainder);
        self.pending_len = remainder.len();
    }

    fn emit(&mut self, bytes: [u8; 3], count: usize, output: &mut Vec<u8>) {
        output.extend_from_slice(&encode_triplet(bytes, count));
        self.column += 4;
        if self.column == self.line_length {
            output.extend_from_slice(self.new_line_format.as_bytes());
            self.column = 0;
        }
    }

    /// Appends pending input with padding and terminates any unfinished line.
    ///
    /// Leaves the encoder ready for a new independent body, retaining its line
    /// length and ending settings. Empty bodies produce no output. Repeated
    /// calls without intervening input have no effect.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.pending_len != 0 {
            let mut bytes = [0; 3];
            bytes[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
            self.emit(bytes, self.pending_len, output);
        }
        if self.column != 0 {
            output.extend_from_slice(self.new_line_format.as_bytes());
        }
        self.reset();
    }

    /// Discards pending input, retaining the line length and ending settings.
    ///
    /// Does not emit padding or a newline; use [`finish`](Self::finish) to
    /// complete the current body instead.
    pub fn reset(&mut self) {
        self.column = 0;
        self.pending = [0; 3];
        self.pending_len = 0;
    }
}

/// An incremental, tolerant MIME Base64 decoder.
///
/// Matches MimeKit's scalar decoder: bytes outside the Base64 alphabet and `=`
/// are ignored, and decoding continues after padded quartets so independently
/// padded blocks can be concatenated. This is **not** a strict Base64 validator.
///
/// Only complete quartets produce output. An incomplete final quartet remains
/// pending until more input arrives or [`reset`](Self::reset) is called; missing
/// padding is not synthesized. Excess padding is processed as additional
/// quartets, following the same positional rules as other input.
#[derive(Clone, Debug, Default)]
pub struct Base64Decoder {
    value: u32,
    // One bit per accepted character, newest in bit 0. Unlike MimeKit's
    // previous-character word, only the positions of '=' need to be saved.
    padding: u8,
    count: u8,
}

impl Base64Decoder {
    /// Creates a tolerant MIME Base64 decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends bytes decoded from complete quartets to `output`.
    ///
    /// Existing output is never cleared. Whitespace and other non-alphabet
    /// bytes are ignored without interrupting a pending quartet.
    #[inline]
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        if input.len() < 16 {
            self.decode_scalar(input, output);
        } else {
            self.decode_with_backend(input, output, Backend::detect(input.len()));
        }
    }

    fn decode_with_backend(&mut self, input: &[u8], output: &mut Vec<u8>, backend: Backend) {
        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Backend::Simd(accelerator) = backend {
            let mut input = input;
            while input.len() >= 16 {
                // Use scalar decoding until we reach a quartet boundary and
                // the next character is a valid, non-padding Base64 character.
                if self.count != 0 || RANK[input[0] as usize] == 0xff || input[0] == b'=' {
                    self.decode_scalar(&input[..1], output);
                    input = &input[1..];
                    continue;
                }
                let (consumed, scalar) = accelerator.decode(input, output);
                input = &input[consumed..];
                // Handle the unfinished quartet through the first invalid
                // character before trying SIMD again. Padding and ignored
                // bytes must retain the scalar decoder's positional behavior.
                self.decode_scalar(&input[..scalar], output);
                input = &input[scalar..];
            }
            self.decode_scalar(input, output);
            return;
        }
        let _ = backend;
        self.decode_scalar(input, output);
    }

    fn decode_scalar(&mut self, input: &[u8], output: &mut Vec<u8>) {
        // Decode every accepted quartet into up to three bytes. Invalid bytes
        // do not advance the quartet; '=' contributes a zero rank and a pad bit.
        for &byte in input {
            let rank = RANK[byte as usize];
            if rank == 0xff {
                continue;
            }
            self.value = (self.value << 6) | u32::from(rank);
            self.padding = (self.padding << 1) | u8::from(byte == b'=');
            self.count += 1;

            if self.count == 4 {
                // MimeKit checks padding in positions 2, 3, and 4, in order.
                if self.padding & 4 == 0 {
                    output.push((self.value >> 16) as u8);
                    if self.padding & 2 == 0 {
                        output.push((self.value >> 8) as u8);
                        if self.padding & 1 == 0 {
                            output.push(self.value as u8);
                        }
                    }
                }
                self.reset();
            }
        }
    }

    /// Discards a pending partial quartet, ready for an independent body.
    pub fn reset(&mut self) {
        self.value = 0;
        self.padding = 0;
        self.count = 0;
    }
}

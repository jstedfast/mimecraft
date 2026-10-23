// Ported from MimeKit.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{EncoderOptionsError, NewLineFormat};

#[cfg(all(feature = "simd", target_arch = "aarch64"))]
mod neon;
#[cfg(all(feature = "simd", any(target_arch = "x86", target_arch = "x86_64")))]
mod simd;
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

const INPUT_PER_LINE: usize = 45;

#[cfg(all(
    feature = "simd",
    any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
))]
fn decode_lines(accelerator: simd::Accelerator, input: &[u8], output: &mut Vec<u8>) -> usize {
    let mut consumed = 0;
    let mut decoded = [0u8; 72];
    while let Some(&prefix) = input.get(consumed) {
        if matches!(prefix, b'\r' | b'\n') {
            break;
        }
        let length = usize::from(RANK[prefix as usize]);
        let count = length.div_ceil(3) * 4;
        // Short lines and incomplete lines are handled by the scalar decoder.
        if count < 16 {
            break;
        }
        let start = consumed + 1;
        let end = start + count;
        let newline = match input.get(end..) {
            Some([b'\n', ..]) => 1,
            Some([b'\r', b'\n', ..]) => 2,
            _ => break,
        };
        // The newline check also establishes bounds for the entire payload.
        // Kernels decode overlapping final vectors into scratch space; publish
        // only the declared bytes after checking the entire line for CR/LF.
        // Unlike MimeKit's pointer API, Vec input/output cannot alias, and no
        // speculative vector stores reach caller-visible output.
        if !accelerator.decode_line(&input[start..end], &mut decoded) {
            break;
        }
        output.extend_from_slice(&decoded[..length]);
        consumed = end + newline;
    }
    consumed
}

fn encode_sextet(value: u8) -> u8 {
    if value == 0 { b'`' } else { value + b' ' }
}

// Unlike Base64, every byte maps to a sextet: (c - 0x20) & 0x3f.
// Space and backtick both represent zero. CR and LF are handled separately.
static RANK: [u8; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        table[index] = (index as u8).wrapping_sub(b' ') & 0x3f;
        index += 1;
    }
    table
};

/// An incremental Unix-to-Unix (UU) payload encoder.
///
/// Encodes at most 45 input bytes per line, using backticks for zero sextets.
/// Defaults to [`NewLineFormat::Dos`] (CRLF); Unix (LF) is also supported.
/// Input bytes are encoded unchanged. No `begin` or `end` wrapper is emitted.
///
/// Call [`finish`](Self::finish) to emit any partial line and the zero-length
/// terminator. Unlike the Base64 and quoted-printable encoders, every call to
/// `finish` emits a terminator, including for an empty body, matching MimeKit.
/// Dropping the encoder does not flush it.
///
/// ```
/// use mimecraft::encodings::{UuDecoder, UuEncoder};
///
/// let mut encoded = Vec::new();
/// let mut encoder = UuEncoder::new();
/// encoder.encode(b"Cat", &mut encoded);
/// encoder.finish(&mut encoded);
/// assert_eq!(encoded, b"#0V%T\r\n`\r\n");
///
/// let mut decoded = Vec::new();
/// UuDecoder::for_payload().decode(&encoded, &mut decoded);
/// assert_eq!(decoded, b"Cat");
/// ```
#[derive(Clone, Debug)]
pub struct UuEncoder {
    new_line_format: NewLineFormat,
    // The line's length must precede its data. Buffer an incomplete input line
    // rather than MimeKit's encoded quartets and separately saved input bytes.
    pending: [u8; INPUT_PER_LINE],
    pending_len: usize,
}

impl Default for UuEncoder {
    fn default() -> Self {
        Self {
            new_line_format: NewLineFormat::default(),
            pending: [0; INPUT_PER_LINE],
            pending_len: 0,
        }
    }
}

impl UuEncoder {
    /// Creates an encoder using CRLF line endings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder using the specified new-line format.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for
    /// [`NewLineFormat::Mixed`].
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }
        Ok(Self {
            new_line_format,
            ..Self::default()
        })
    }

    /// Returns the selected new-line format.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Appends complete encoded lines, retaining at most 44 input bytes.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.encode_with_backend(input, output, Backend::detect(input.len()));
    }

    fn encode_with_backend(&mut self, mut input: &[u8], output: &mut Vec<u8>, backend: Backend) {
        if self.pending_len != 0 {
            let count = (INPUT_PER_LINE - self.pending_len).min(input.len());
            self.pending[self.pending_len..self.pending_len + count]
                .copy_from_slice(&input[..count]);
            self.pending_len += count;
            input = &input[count..];
            if self.pending_len != INPUT_PER_LINE {
                return;
            }
            self.write_line(&self.pending, output);
            self.pending_len = 0;
        }

        #[cfg(all(
            feature = "simd",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        ))]
        if let Backend::Simd(accelerator) = backend {
            // Complete the buffered line scalarly, then vectorize whole lines.
            // The final partial line still waits for more input or finish().
            let consumed = accelerator.encode_lines(input, self.new_line_format.as_bytes(), output);
            input = &input[consumed..];
        }
        let _ = backend;

        let (lines, remainder) = input.as_chunks::<INPUT_PER_LINE>();
        for line in lines {
            self.write_line(line, output);
        }
        self.pending[..remainder.len()].copy_from_slice(remainder);
        self.pending_len = remainder.len();
    }

    fn write_line(&self, input: &[u8], output: &mut Vec<u8>) {
        // The length counts original bytes, not the padding in the last quartet.
        output.push(encode_sextet(input.len() as u8));
        for triplet in input.chunks(3) {
            let a = triplet[0];
            let b = triplet.get(1).copied().unwrap_or(0);
            let c = triplet.get(2).copied().unwrap_or(0);

            // Convert three input bytes to four sextets, just as in Base64.
            // The UU alphabet is offset by space, with backtick replacing zero.
            output.extend_from_slice(&[
                encode_sextet(a >> 2),
                encode_sextet(((a & 0x03) << 4) | (b >> 4)),
                encode_sextet(((b & 0x0f) << 2) | (c >> 6)),
                encode_sextet(c & 0x3f),
            ]);
        }
        output.extend_from_slice(self.new_line_format.as_bytes());
    }

    /// Emits pending input and a zero-length line, retaining the new-line format.
    ///
    /// Every call emits a backtick followed by the selected newline, even with
    /// no intervening input. The encoder is then ready for an independent body.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.pending_len != 0 {
            self.write_line(&self.pending[..self.pending_len], output);
        }
        output.push(b'`');
        output.extend_from_slice(self.new_line_format.as_bytes());
        self.reset();
    }

    /// Discards pending input without emitting a terminator, retaining the format.
    pub fn reset(&mut self) {
        self.pending_len = 0;
        self.pending = [0; INPUT_PER_LINE];
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecoderState {
    SeekingBegin,
    Begin { matched: usize },
    Header,
    Payload,
    Ended,
}

/// An incremental, tolerant Unix-to-Unix (UU) decoder.
///
/// By default, skips input until a case-sensitive `begin ` marker at the start
/// of a line and ignores the rest of that header. It does not validate or return
/// permissions or filenames. Use [`for_payload`](Self::for_payload) to decode
/// unwrapped output from [`UuEncoder`].
///
/// Matches MimeKit's scalar decoder: CR is ignored, LF starts a new line, and
/// every other byte maps to a sextet. Incomplete quartets remain pending, even
/// across malformed line breaks; this is not a strict validator. Only complete
/// quartets produce output, capped by the line's declared byte count.
///
/// A zero-length line (space or backtick in canonical input) ends decoding.
/// Subsequent input is ignored until [`reset`](Self::reset), including `end`
/// trailers and additional bodies.
#[derive(Clone, Debug)]
pub struct UuDecoder {
    initial: DecoderState,
    state: DecoderState,
    pending: [u8; 4],
    pending_len: usize,
    remaining: u8,
    end_of_line: bool,
}

impl Default for UuDecoder {
    fn default() -> Self {
        // Start at a line boundary so a begin marker at offset zero is accepted.
        let initial = DecoderState::Begin { matched: 0 };
        Self {
            initial,
            state: initial,
            pending: [0; 4],
            pending_len: 0,
            remaining: 0,
            end_of_line: false,
        }
    }
}

impl UuDecoder {
    /// Creates a decoder that first searches for a `begin ` line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a decoder for payload without a `begin ` header.
    pub fn for_payload() -> Self {
        Self {
            initial: DecoderState::Payload,
            state: DecoderState::Payload,
            ..Self::default()
        }
    }

    /// Appends decoded bytes, preserving partial headers and quartets.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.decode_with_backend(input, output, Backend::detect(input.len()));
    }

    fn decode_with_backend(&mut self, mut input: &[u8], output: &mut Vec<u8>, backend: Backend) {
        let _ = backend;
        while !input.is_empty() {
            #[cfg(all(
                feature = "simd",
                any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
            ))]
            if self.state == DecoderState::Payload
                && self.pending_len == 0
                && let Backend::Simd(accelerator) = backend
            {
                if self.remaining == 0 || self.end_of_line {
                    let consumed = decode_lines(accelerator, input, output);
                    if consumed != 0 {
                        self.remaining = 0;
                        self.end_of_line = true;
                        input = &input[consumed..];
                        continue;
                    }
                } else if self.remaining >= 3 && input.len() >= 16 {
                    // Only complete three-byte quartets may bypass the scalar
                    // tail. Stop before any quartet containing CR or LF.
                    let quartets =
                        accelerator.decode_quartets(input, usize::from(self.remaining / 3), output);
                    if quartets != 0 {
                        self.remaining -= (quartets * 3) as u8;
                        input = &input[quartets * 4..];
                        continue;
                    }
                }
            }
            let byte = input[0];
            input = &input[1..];
            match self.state {
                DecoderState::SeekingBegin => {
                    if byte == b'\n' {
                        self.state = DecoderState::Begin { matched: 0 };
                    }
                }
                DecoderState::Begin { matched } => {
                    if byte == b"begin "[matched] {
                        self.state = if matched + 1 == b"begin ".len() {
                            DecoderState::Header
                        } else {
                            DecoderState::Begin {
                                matched: matched + 1,
                            }
                        };
                    } else {
                        // A failed prefix consumes the rest of its line. Only LF
                        // permits another marker, even after NUL or a partial match.
                        self.state = if byte == b'\n' {
                            DecoderState::Begin { matched: 0 }
                        } else {
                            DecoderState::SeekingBegin
                        };
                    }
                }
                DecoderState::Header => {
                    if byte == b'\n' {
                        self.state = DecoderState::Payload;
                    }
                }
                DecoderState::Payload => {
                    if byte == b'\r' {
                        continue;
                    }
                    if byte == b'\n' {
                        self.end_of_line = true;
                        continue;
                    }

                    if self.remaining == 0 || self.end_of_line {
                        // The first octet is the line length. Also accept a new
                        // length immediately when the previous count is exhausted.
                        self.remaining = RANK[byte as usize];
                        self.end_of_line = false;
                        if self.remaining == 0 {
                            self.state = DecoderState::Ended;
                            return;
                        }
                        // Keep any incomplete quartet from a malformed prior line,
                        // matching MimeKit rather than silently discarding it.
                        continue;
                    }

                    self.pending[self.pending_len] = RANK[byte as usize];
                    self.pending_len += 1;
                    if self.pending_len == 4 {
                        let [a, b, c, d] = self.pending;
                        let decoded = [a << 2 | b >> 4, b << 4 | c >> 2, c << 6 | d];
                        let count = self.remaining.min(3);
                        output.extend_from_slice(&decoded[..usize::from(count)]);
                        self.remaining -= count;
                        self.pending_len = 0;
                    }
                }
                DecoderState::Ended => return,
            }
        }
    }

    /// Discards stream state, retaining begin-line scanning or payload-only mode.
    pub fn reset(&mut self) {
        self.state = self.initial;
        self.pending = [0; 4];
        self.pending_len = 0;
        self.remaining = 0;
        self.end_of_line = false;
    }
}

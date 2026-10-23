// Ported from MimeKit's Rfc2231Encoder, Rfc2231Decoder, and ByteExtensions.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::hex::{encode_hex_byte, hex_rank};
use crate::parse_utils::is_attribute;

/// A stateless percent encoder for RFC 2231 MIME parameter value payloads.
///
/// Matches MimeKit's `Rfc2231Encoder`: uses `%HH` escapes rather than ordinary
/// hexadecimal encoding. Leaves MIME attribute characters unchanged
/// and escapes other bytes with uppercase hex. Space becomes `%20`, not `+`.
/// This is not a general-purpose URI or form encoder.
///
/// Does not add charset/language prefixes, parameter names, quotes,
/// continuations, or line breaks, and does not convert character sets.
/// Each call appends its complete result; no `finish` or `reset` is needed.
///
/// ```
/// use mimecraft::encodings::{Rfc2231Decoder, Rfc2231Encoder};
///
/// let mut encoded = Vec::new();
/// Rfc2231Encoder::new().encode(b"report 100%.txt", &mut encoded);
/// assert_eq!(encoded, b"report%20100%25.txt");
///
/// let mut decoder = Rfc2231Decoder::new();
/// let mut decoded = Vec::new();
/// for chunk in encoded.chunks(2) {
///     decoder.decode(chunk, &mut decoded);
/// }
/// assert_eq!(decoded, b"report 100%.txt");
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct Rfc2231Encoder;

impl Rfc2231Encoder {
    /// Creates a MIME parameter percent encoder.
    pub fn new() -> Self {
        Self
    }

    /// Appends encoded bytes without retaining state or adding line breaks.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            // Attribute characters exclude RFC 2045 token specials and the
            // additional RFC 2231 specials '*', apostrophe, and '%'.
            if is_attribute(byte) {
                output.push(byte);
            } else {
                encode_hex_byte(b'%', byte, output);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum DecoderState {
    #[default]
    PassThrough,
    Percent,
    FirstByte(u8),
}

/// An incremental, tolerant percent decoder for MIME parameter value payloads.
///
/// Accepts upper- and lowercase hex in `%HH` escapes. Invalid three-byte
/// sequences pass through literally, without reinterpreting any `%` within
/// them. Incomplete escapes (`%` or `%X`, even when X is not a hex digit) remain
/// pending until more input arrives or [`reset`](Self::reset) discards them.
///
/// Matches MimeKit's `Rfc2231Decoder`: `+` stays `+`, whitespace is preserved,
/// and no charset conversion, parameter parsing, or strict validation occurs.
#[derive(Clone, Debug, Default)]
pub struct Rfc2231Decoder {
    state: DecoderState,
}

impl Rfc2231Decoder {
    /// Creates a tolerant MIME parameter percent decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends decoded bytes, preserving incomplete percent escapes.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            self.state = match self.state {
                DecoderState::PassThrough if byte == b'%' => DecoderState::Percent,
                DecoderState::PassThrough => {
                    output.push(byte);
                    DecoderState::PassThrough
                }
                DecoderState::Percent => DecoderState::FirstByte(byte),
                DecoderState::FirstByte(first) => {
                    if let (Some(high), Some(low)) = (hex_rank(first), hex_rank(byte)) {
                        output.push(high << 4 | low);
                    } else {
                        // Invalid encoded sequence: pass it through undecoded.
                        // Wait for both bytes, even when the first is not hex.
                        output.extend_from_slice(&[b'%', first, byte]);
                    }
                    DecoderState::PassThrough
                }
            };
        }
    }

    /// Discards any pending percent escape without emitting it.
    pub fn reset(&mut self) {
        self.state = DecoderState::PassThrough;
    }

    pub(crate) fn has_pending_escape(&self) -> bool {
        !matches!(self.state, DecoderState::PassThrough)
    }
}

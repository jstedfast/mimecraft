// Ported from MimeKit's Rfc2047Base64Encoder, Rfc2047QuotedPrintableEncoder,
// and ByteExtensions.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{base64::encode_unwrapped, hex::encode_hex_byte};

/// A stateless Base64 encoder for the payload of an RFC 2047 encoded-word.
///
/// Each call writes a complete, independently padded B-encoded payload without
/// line breaks. Unlike [`super::Base64Encoder`], this is not incremental:
/// supply all bytes for one payload in a single call.
///
/// Does not write `=?charset?B?...?=` wrappers, convert character sets, split
/// characters or words, enforce the encoded-word length limit, or fold headers.
/// Those operations belong to a higher-level header formatter.
///
/// ```
/// use mimecraft::encodings::Rfc2047Base64Encoder;
///
/// let mut payload = Vec::new();
/// Rfc2047Base64Encoder::new().encode(b"hello world", &mut payload);
/// assert_eq!(payload, b"aGVsbG8gd29ybGQ=");
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct Rfc2047Base64Encoder;

impl Rfc2047Base64Encoder {
    /// Creates an encoded-word B payload encoder.
    pub fn new() -> Self {
        Self
    }

    /// Appends one complete Base64 payload, including any required padding.
    ///
    /// Reuses the Base64 SIMD backends when available. No state is retained,
    /// no newline is written, and no `finish` or `reset` is needed.
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&self, input: &[u8], output: &mut Vec<u8>) {
        encode_unwrapped(input, output);
    }
}

/// The context-specific alphabet for RFC 2047 Q encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QEncodeMode {
    /// Phrase payloads, such as an address display name.
    ///
    /// Only ASCII letters, digits, and `!*+-/` remain literal.
    Phrase,
    /// Text payloads, such as an unstructured header value.
    ///
    /// Uses MimeKit's encoded-word-safe alphabet, also escaping backslashes
    /// for use in quoted strings and comments. Not a body quoted-printable mode.
    Text,
}

/// A stateless Q encoder for the payload of an RFC 2047 encoded-word.
///
/// Spaces become underscores. Literal underscores, `=`, `?`, non-ASCII bytes,
/// and characters unsafe in the selected context are escaped as uppercase
/// `=HH`. No soft breaks, hard line breaks, or trailing-whitespace rules from
/// body quoted-printable encoding apply.
///
/// Each call appends its complete result; no `finish` or `reset` is needed.
/// Does not add encoded-word wrappers, convert character sets, enforce word
/// lengths, or fold headers. Decode payloads using
/// [`super::QuotedPrintableDecoder::for_encoded_words`].
///
/// ```
/// use mimecraft::encodings::{QEncodeMode, Rfc2047QuotedPrintableEncoder};
///
/// let mut payload = Vec::new();
/// Rfc2047QuotedPrintableEncoder::new(QEncodeMode::Text)
///     .encode(b"hello_world = ?", &mut payload);
/// assert_eq!(payload, b"hello=5Fworld_=3D_=3F");
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Rfc2047QuotedPrintableEncoder {
    mode: QEncodeMode,
}

impl Rfc2047QuotedPrintableEncoder {
    /// Creates a Q payload encoder for the specified context.
    pub fn new(mode: QEncodeMode) -> Self {
        Self { mode }
    }

    /// Returns the selected context-specific alphabet.
    pub fn mode(&self) -> QEncodeMode {
        self.mode
    }

    /// Appends Q-encoded bytes without retaining state or adding line breaks.
    ///
    /// Empty input has no effect. Existing output is never cleared.
    pub fn encode(&self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            if byte == b' ' {
                output.push(b'_');
                continue;
            }
            let safe = byte.is_ascii_alphanumeric()
                || match self.mode {
                    QEncodeMode::Phrase => matches!(byte, b'!' | b'*' | b'+' | b'-' | b'/'),
                    // Backslash is not an RFC 2047 especial, but a literal one
                    // would be a quoted-pair in quoted strings and is forbidden
                    // in encoded-words in comments. Always escape it, as MimeKit
                    // does, along with the encoded-word specials and underscore.
                    QEncodeMode::Text => matches!(
                        byte,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'^'
                            | b'`'
                            | b'{'
                            | b'|'
                            | b'}'
                            | b'~'
                    ),
                };
            if safe {
                output.push(byte);
            } else {
                encode_hex_byte(b'=', byte, output);
            }
        }
    }
}

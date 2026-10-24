// Adapted from MimeKit's MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Byte-level masking of raw MIME header values for inspecting syntax and layout.
//!
//! These whole-value helpers return a new buffer of exactly the input length.
//! They do not decode, validate, repair, fold, or mutate headers. Each call starts
//! fresh state; do not split one value across calls.
//!
//! This is not guaranteed removal of sensitive information. Lengths, selected
//! syntax, and potential encoded-word charset/encoding labels remain visible;
//! address masking also preserves escaped bytes. Even malformed labels can pass
//! through verbatim. Output may contain non-ASCII bytes, controls, or invalid
//! UTF-8 and is not necessarily valid MIME or safe display text.
//!
//! Pass values such as [`crate::Header::raw_value`], not complete header records.
//! Header dispatch, preservation policies, Received/content-header masking, and
//! full-message anonymization are not implemented here. Review output before
//! sharing it. Use [`crate::filters::AnonymizeFilter`] for simpler byte masking
//! that preserves only SPACE, TAB, CR, and LF.

use crate::parse_utils::is_whitespace;

const ADDRESS_SPECIALS: &[u8] = b" \t\r\n()<>[]:;@,.";
const WHITESPACE: &[u8] = b" \t\r\n";

/// Masks an unstructured raw header value, retaining potential encoded-word syntax.
///
/// Ordinary bytes become `x`, except SPACE, TAB, CR, LF, `=`, and `?`.
/// A permissive state machine preserves potential charset and encoding labels,
/// masks payload bytes except whitespace and `?`, and retains a closing `?=`.
/// No label validation or encoded-word decoding occurs. Malformed and incomplete
/// sequences follow MimeKit's transitions, not the RFC 2047 decoder's grammar.
///
/// Quotation marks and backslashes have no special treatment outside labels.
/// Returns exactly one byte per input byte; see the [module documentation](self)
/// for scope and limitations.
///
/// ```
/// use mimecraft::anonymize::unstructured_header_value;
///
/// assert_eq!(
///     unstructured_header_value(b" =?utf-8?q?hello_world?=\r\n"),
///     b" =?utf-8?q?xxxxxxxxxxx?=\r\n",
/// );
/// ```
pub fn unstructured_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut word = EncodedWordState::None;
    raw_value
        .iter()
        .map(|&byte| word.mask(byte, WHITESPACE))
        .collect()
}

/// Masks an address-like raw value while retaining punctuation, quotes, and escapes.
///
/// Outside quoted strings, preserves SPACE, TAB, CR, LF, `()<>[]:;@,.`, `=`,
/// and `?`, with the same potential encoded-word handling as
/// [`unstructured_header_value`]. This does not parse mailboxes, comments, groups,
/// or domain literals; malformed address syntax is retained rather than repaired.
///
/// Double quotes and backslashes are retained. An odd run of backslashes escapes
/// the following byte, which is copied verbatim, even inside quotes. Unescaped
/// quotes toggle quoted-string handling: only CR/LF survive masking inside
/// quotes, while spaces and tabs become `x`. Encoded-word state is paused, not
/// reset, while processing quotes, escapes, or quoted text.
///
/// Returns exactly one byte per input byte. Escaped data and potential labels
/// may remain readable; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::address_header_value;
///
/// assert_eq!(
///     address_header_value(b" Alice <alice@example.com>\r\n"),
///     b" xxxxx <xxxxx@xxxxxxx.xxx>\r\n",
/// );
/// ```
pub fn address_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(raw_value.len());
    let mut word = EncodedWordState::None;
    let mut escaped = false;
    let mut quoted = false;

    for &byte in raw_value {
        let masked = match byte {
            b'\\' => {
                escaped = !escaped;
                byte
            }
            b'"' => {
                if escaped {
                    escaped = false;
                } else {
                    quoted = !quoted;
                }
                byte
            }
            _ if escaped => {
                escaped = false;
                byte
            }
            _ if quoted => {
                // Inside quoted strings, preserve CR/LF for inspecting folding,
                // but mask spaces and tabs along with the other quoted text.
                if matches!(byte, b'\r' | b'\n') {
                    byte
                } else {
                    b'x'
                }
            }
            _ => word.mask(byte, ADDRESS_SPECIALS),
        };
        output.push(masked);
    }
    output
}

#[derive(Clone, Copy)]
enum EncodedWordState {
    None,
    Equals,
    EqualsQuestion,
    Charset,
    CharsetQuestion,
    Encoding,
    EncodingQuestion,
    Payload,
    PayloadQuestion,
}

impl EncodedWordState {
    fn mask(&mut self, byte: u8, specials: &[u8]) -> u8 {
        match byte {
            b'=' => match self {
                Self::None => {
                    *self = Self::Equals;
                    byte
                }
                Self::Payload => {
                    // Mask '=' inside the payload, including encoding padding.
                    b'x'
                }
                _ => {
                    // Close ?= or leave an interrupted encoded-word sequence.
                    *self = Self::None;
                    byte
                }
            },
            b'?' => {
                *self = match self {
                    Self::Equals => Self::EqualsQuestion,
                    Self::Charset => Self::CharsetQuestion,
                    Self::Encoding => Self::EncodingQuestion,
                    Self::Payload => Self::PayloadQuestion,
                    // A normal '?' passes through too; unexpected transitions
                    // leave encoded-word mode rather than rejecting the value.
                    _ => Self::None,
                };
                byte
            }
            _ => match self {
                Self::EqualsQuestion | Self::Charset => {
                    // Let the potential charset name pass through unchanged.
                    *self = Self::Charset;
                    byte
                }
                Self::CharsetQuestion | Self::Encoding => {
                    // Likewise preserve the encoding name without validation.
                    *self = Self::Encoding;
                    byte
                }
                Self::EncodingQuestion | Self::Payload => {
                    *self = Self::Payload;
                    // Whitespace inside a payload can affect MIME tokenization,
                    // especially malformed folding, so keep it visible.
                    if is_whitespace(byte) { byte } else { b'x' }
                }
                Self::None | Self::Equals | Self::PayloadQuestion => {
                    // Reprocess unexpected bytes as ordinary text. Returning
                    // their masked value avoids MimeKit's one-byte cursor rewind.
                    *self = Self::None;
                    if specials.contains(&byte) { byte } else { b'x' }
                }
            },
        }
    }
}

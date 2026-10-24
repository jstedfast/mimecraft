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
//! through verbatim. Content-header masking preserves the leading type or
//! disposition, parameter names, and selected parameter values, even if malformed.
//! Received masking preserves an entire suffix if the date parser accepts it,
//! including any unrelated text ignored by that parser.
//! Output may contain non-ASCII bytes, controls, or invalid UTF-8 and is not
//! necessarily valid MIME or safe display text.
//!
//! Pass values such as [`crate::Header::raw_value`], not complete header records.
//! Header dispatch, configurable preservation policies, and
//! full-message anonymization are not implemented here. Review output before
//! sharing it. Use [`crate::filters::AnonymizeFilter`] for simpler byte masking
//! that preserves only SPACE, TAB, CR, and LF.

use crate::{DateTimeOffset, parse_utils::is_whitespace};

const ADDRESS_SPECIALS: &[u8] = b" \t\r\n()<>[]:;@,.";
const RECEIVED_SPECIALS: &[u8] = b"<>[]:@,.";
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

/// Masks a raw Received value, retaining trace syntax and recognized date suffixes.
///
/// Preserves SPACE, TAB, CR, LF, `<>[]:@,.`, and semicolons outside comments.
/// The keywords `from`, `by`, `via`, `with`, `id`, and `for` retain their original
/// spelling when matched ASCII-case-insensitively at a token boundary and
/// followed by MIME whitespace. An exact keyword at EOF is masked. Comments
/// also delimit tokens; punctuation such as `<` does not start a new token.
///
/// Nested comments are scanned iteratively. Parentheses, backslashes, the same
/// punctuation, and whitespace survive inside comments, but keywords do not.
/// An odd backslash run masks the following byte, including whitespace or a
/// parenthesis, instead of interpreting it. Unclosed comments remain comments
/// through EOF; unmatched closing parentheses outside comments are masked.
///
/// After each semicolon outside comments, skips MIME whitespace and tries
/// [`DateTimeOffset::parse`] on the remaining slice. If accepted, the **entire
/// suffix** is copied verbatim, including comments, ignored tokens, and trailing
/// data. Otherwise masking resumes. This is tolerant date recognition, not
/// validation or formatting; missing times and other recovered forms can pass.
///
/// Returns a new buffer of exactly the input length. Each call starts fresh;
/// lookahead means masking a prefix can differ from that prefix of the full
/// result. No encoded-word or charset decoding occurs. Preserved dates and
/// unrelated suffix data may be sensitive; see the [module documentation](self).
///
/// ```
/// use mimecraft::anonymize::received_header_value;
///
/// assert_eq!(
///     received_header_value(b" from mail.example; 15 Nov 2017 14:16:18 -0000\r\n"),
///     b" from xxxx.xxxxxxx; 15 Nov 2017 14:16:18 -0000\r\n",
/// );
/// ```
pub fn received_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let mut index = 0;

    while index < raw_value.len() {
        if is_whitespace(raw_value[index]) {
            index += 1;
        } else if raw_value[index] == b'(' {
            // Consume '(' and scan the comment, including nested comments.
            let mut depth = 1usize;
            let mut escaped = false;
            index += 1;

            while index < raw_value.len() && depth > 0 {
                let byte = raw_value[index];
                match byte {
                    b'\\' => escaped = !escaped,
                    _ if escaped => {
                        // Escaped bytes are masked even when they are whitespace
                        // or would otherwise open or close a comment.
                        output[index] = b'x';
                        escaped = false;
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ if !is_whitespace(byte) && !RECEIVED_SPECIALS.contains(&byte) => {
                        // Mask everything else in the comment.
                        output[index] = b'x';
                    }
                    _ => {}
                }
                index += 1;
            }
        } else if raw_value[index] == b';' {
            index += 1;
            while index < raw_value.len() && is_whitespace(raw_value[index]) {
                index += 1;
            }

            // This might be a date. Like MimeKit, don't mask any of the suffix
            // if the date parser accepts it, even bytes the parser ignores.
            if DateTimeOffset::parse(&raw_value[index..]).is_ok() {
                break;
            }
        } else if let Some(length) = received_keyword_length(&raw_value[index..]) {
            index += length;
        } else {
            while index < raw_value.len() {
                let byte = raw_value[index];
                if is_whitespace(byte) || matches!(byte, b';' | b'(') {
                    break;
                }
                if !RECEIVED_SPECIALS.contains(&byte) {
                    output[index] = b'x';
                }
                index += 1;
            }
        }
    }
    output
}

fn received_keyword_length(input: &[u8]) -> Option<usize> {
    // Look for: from, by, via, with, id, for. All are at most four bytes,
    // and even an exact match must have a following MIME whitespace byte.
    let length = input.iter().take(5).position(|&byte| is_whitespace(byte))?;
    let word = &input[..length];
    if word.eq_ignore_ascii_case(b"from")
        || word.eq_ignore_ascii_case(b"by")
        || word.eq_ignore_ascii_case(b"via")
        || word.eq_ignore_ascii_case(b"with")
        || word.eq_ignore_ascii_case(b"id")
        || word.eq_ignore_ascii_case(b"for")
    {
        Some(length)
    } else {
        None
    }
}

/// Masks parameter values in a raw Content-Type value, preserving MIME structure.
///
/// Copies everything through the first semicolon verbatim, without validating
/// the media type. Parameter names and continuation suffixes also pass through.
/// Values named `boundary`, `charset`, `delsp`, or `format` are preserved using
/// ASCII-case-insensitive comparison of the name before its first `*`.
/// Names are not trimmed or decoded, and continuation suffixes are not validated.
///
/// Other value bytes become `x`. Outside quotes, SPACE, TAB, CR, LF, and
/// separating semicolons survive. Inside quoted values, quotes, backslashes,
/// and CR/LF survive; all other bytes, including spaces, tabs, and escaped text,
/// are masked. No RFC 2231 or encoded-word decoding occurs.
///
/// Follows MimeKit's permissive parameter states, not [`crate::ParameterList`]
/// parsing. In particular, an incomplete starred parameter followed by `;`
/// retains its accumulated base name when reading the next parameter. This can
/// change which subsequent values are preserved. An unclosed quote keeps later
/// semicolons inside that value. Missing semicolons do not start new parameters.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// Leading text, names, and preserved values may contain sensitive data or
/// malformed bytes; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::content_type_value;
///
/// assert_eq!(
///     content_type_value(b" text/plain; charset=utf-8; name=report.txt\r\n"),
///     b" text/plain; charset=utf-8; name=xxxxxxxxxx\r\n",
/// );
/// ```
pub fn content_type_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the MIME type; only parameter values are candidates.
    content_header_value(raw_value)
}

/// Masks parameter values in a raw Content-Disposition value.
///
/// Preserves everything through the first semicolon, including an unrecognized
/// disposition token. Uses exactly the same parameter rules as
/// [`content_type_value`], including preservation of `boundary`, `charset`,
/// `delsp`, and `format` values even on Content-Disposition. Other values,
/// including filenames, sizes, and dates, are masked rather than decoded.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// This does not validate syntax or guarantee removal of sensitive information;
/// see the [module documentation](self) for limitations.
///
/// ```
/// use mimecraft::anonymize::content_disposition_value;
///
/// assert_eq!(
///     content_disposition_value(b" attachment; filename=\"my file.txt\"\r\n"),
///     b" attachment; filename=\"xxxxxxxxxxx\"\r\n",
/// );
/// ```
pub fn content_disposition_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the "attachment" or "inline" part.
    content_header_value(raw_value)
}

#[derive(Clone, Copy)]
enum ParameterState {
    Semicolon,
    Name,
    NameStar,
    Value,
}

fn is_preserved_parameter_name(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"boundary")
        || name.eq_ignore_ascii_case(b"charset")
        || name.eq_ignore_ascii_case(b"delsp")
        || name.eq_ignore_ascii_case(b"format")
}

fn content_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let Some(start) = raw_value.iter().position(|&byte| byte == b';') else {
        return output;
    };
    let mut state = ParameterState::Semicolon;
    let mut name = Vec::with_capacity(16);
    let mut escaped = false;
    let mut quoted = false;
    let mut preserve = false;

    for (&byte, masked) in raw_value[start + 1..].iter().zip(&mut output[start + 1..]) {
        if matches!(state, ParameterState::Semicolon) && byte != b';' && !is_whitespace(byte) {
            state = ParameterState::Name;
        }
        match state {
            ParameterState::Semicolon => {
                // Preserve repeated semicolons and intervening whitespace.
            }
            ParameterState::Name => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // An incomplete parameter name: recover at the semicolon.
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                b'*' => state = ParameterState::NameStar,
                _ => name.push(byte),
            },
            ParameterState::NameStar => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // This starred parameter seems incomplete. Like MimeKit,
                    // resume at the next name without clearing the old base name.
                    state = ParameterState::Semicolon;
                }
                _ => {}
            },
            ParameterState::Value => match byte {
                b'"' => {
                    if escaped {
                        escaped = false;
                    } else {
                        quoted = !quoted;
                    }
                }
                b'\\' if quoted => escaped = !escaped,
                b'\r' | b'\n' if quoted => {
                    // Keep CR/LF for inspecting folding within a quoted value.
                    escaped = false;
                }
                _ if quoted => {
                    if !preserve {
                        *masked = b'x';
                    }
                    escaped = false;
                }
                b';' => {
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                _ if !preserve && !is_whitespace(byte) => *masked = b'x',
                _ => {}
            },
        }
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

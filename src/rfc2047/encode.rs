// Adapted from MimeKit's Utils/Rfc2047.cs (GetRfc822Words, AppendEncodedWord,
// CharsetConvert and Encode), and Parameter.cs (EncodeRfc2047).
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    charsets::{Charset, CharsetError},
    encodings::{QEncodeMode, Rfc2047Base64Encoder, Rfc2047QuotedPrintableEncoder},
    parse_utils::is_ascii_atom,
    utils::quote,
};

/// Options for complete-value RFC 2047 text and phrase encoding.
///
/// The default charset is UTF-8 and the encoded-word limit is 75 bytes,
/// including the wrapper. Automatic B/Q selection uses the shorter payload,
/// preferring Q on ties; ISO-2022-JP always uses B.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodeOptions {
    /// Charset for encoded words. Plain ASCII text does not need conversion.
    ///
    /// Unmappable characters cause an error, never substitution or an implicit
    /// fallback to another charset. Legacy mapping limitations are those of
    /// [`Charset::encode`].
    pub charset: Charset,
    /// Encoded-word size, including `=?charset?B/Q?` and `?=`, in `1..=75`.
    ///
    /// A limit too small to hold a wrapper and one complete character returns
    /// [`EncodeError::WordTooLong`]. This is not a physical-line limit.
    pub max_word_length: usize,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            charset: Charset::UTF8,
            max_word_length: 75,
        }
    }
}

/// A complete header value could not be encoded.
///
/// No partial output is returned on error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// The requested encoded-word limit is outside `1..=75`.
    InvalidWordLength {
        /// The rejected limit.
        requested: usize,
    },
    /// The selected charset cannot represent the supplied text.
    Charset(CharsetError),
    /// A complete character and its wrapper cannot fit in an encoded word.
    WordTooLong {
        /// UTF-8 byte offset of the character in the supplied text.
        offset: usize,
        /// Bytes needed for the encoded character and wrapper.
        required: usize,
        /// The configured encoded-word limit.
        limit: usize,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWordLength { requested } => {
                write!(
                    f,
                    "encoded-word length must be between 1 and 75 bytes, got {requested}"
                )
            }
            Self::Charset(error) => error.fmt(f),
            Self::WordTooLong {
                offset,
                required,
                limit,
            } => write!(
                f,
                "character at byte {offset} needs a {required}-byte encoded word, exceeding {limit}"
            ),
        }
    }
}

impl Error for EncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Charset(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CharsetError> for EncodeError {
    fn from(error: CharsetError) -> Self {
        Self::Charset(error)
    }
}

/// Encodes an unstructured value, such as a Subject, with default options.
///
/// Returns ASCII value syntax only: no field name, folding, or terminating
/// newline. Callers must fold complete fields before writing them as headers.
///
/// ```
/// use mimecraft::rfc2047::{decode_text, encode_text};
///
/// let value = encode_text("Hello Ren\u{e9}e")?;
/// assert_eq!(value, "=?utf-8?q?Hello_Ren=C3=A9e?=");
/// assert_eq!(decode_text(value.as_bytes()).text, "Hello Ren\u{e9}e");
/// # Ok::<(), mimecraft::rfc2047::EncodeError>(())
/// ```
///
/// # Errors
///
/// Returns [`EncodeError`] if the value cannot be encoded.
pub fn encode_text(text: &str) -> Result<String, EncodeError> {
    encode_text_with_options(text, &EncodeOptions::default())
}

/// Encodes an unstructured value with explicit charset and word-length options.
///
/// Short printable ASCII tokens remain literal. Non-ASCII text, controls,
/// literal `=?` sequences, or overlong tokens cause the *entire value* to be
/// encoded. This deliberately avoids MimeKit's mixed plain/encoded-word merging
/// heuristics. All original whitespace is retained inside encoded words.
///
/// Words are separated by a single syntactic space, ignored during decoding.
/// Each word independently contains complete characters and a complete charset
/// encoding, including any state-reset sequences. Output is not folded.
/// Decoded controls, CR/LF, and NUL are preserved, not sanitized.
///
/// # Errors
///
/// Returns [`EncodeError`] for invalid options, unmappable text, or a character
/// that cannot fit within the encoded-word limit.
pub fn encode_text_with_options(
    text: &str,
    options: &EncodeOptions,
) -> Result<String, EncodeError> {
    encode(text, options, QEncodeMode::Text)
}

/// Encodes a display-name phrase with default options.
///
/// Returns ASCII phrase syntax, not a complete address or header. ASCII phrases
/// containing punctuation are quoted and escaped; Unicode and controls use
/// encoded words. Input is a literal display name, not already-quoted syntax.
///
/// ```
/// use mimecraft::rfc2047::encode_phrase;
///
/// assert_eq!(encode_phrase("Doe, Jane")?, "\"Doe, Jane\"");
/// assert_eq!(encode_phrase("John \"Jacob\" Smith")?, "\"John \\\"Jacob\\\" Smith\"");
/// # Ok::<(), mimecraft::rfc2047::EncodeError>(())
/// ```
///
/// # Errors
///
/// Returns [`EncodeError`] if the phrase cannot be encoded.
pub fn encode_phrase(phrase: &str) -> Result<String, EncodeError> {
    encode_phrase_with_options(phrase, &EncodeOptions::default())
}

/// Encodes a literal display-name phrase with explicit options.
///
/// Applies [`encode_text_with_options`]'s charset, word-boundary, and error
/// rules, using the restricted phrase Q alphabet. Short ASCII phrases are
/// atoms separated by single spaces, or one quoted string. An empty phrase is
/// `""`. Longer quoted strings use encoded words rather than splitting the
/// literal text and inserting whitespace.
///
/// [`super::decode_phrase`] does not remove syntactic quotes. Parse/unquote
/// the ASCII phrase syntax *before* decoding encoded words; unquoting afterwards
/// could remove quotes or backslashes that are part of the decoded display name.
/// This function does not fold fields, validate addresses, or encode MIME
/// parameters; use the typed content-header serializers for parameters instead.
///
/// # Errors
///
/// Returns [`EncodeError`] for invalid options, unmappable text, or a character
/// that cannot fit within the encoded-word limit.
pub fn encode_phrase_with_options(
    phrase: &str,
    options: &EncodeOptions,
) -> Result<String, EncodeError> {
    encode(phrase, options, QEncodeMode::Phrase)
}

fn encode(text: &str, options: &EncodeOptions, mode: QEncodeMode) -> Result<String, EncodeError> {
    if !(1..=75).contains(&options.max_word_length) {
        return Err(EncodeError::InvalidWordLength {
            requested: options.max_word_length,
        });
    }

    // A literal encoded-word lookalike must itself be encoded, even inside a
    // quoted phrase: tolerant readers would otherwise reinterpret the value.
    let needs_encoding = text
        .bytes()
        .any(|byte| !byte.is_ascii() || byte.is_ascii_control())
        || text.contains("=?")
        || text
            .split(' ')
            .any(|word| word.len() > options.max_word_length);
    if !needs_encoding {
        if mode == QEncodeMode::Text {
            return Ok(text.to_owned());
        }
        let atoms = !text.is_empty()
            && text
                .split(' ')
                .all(|word| !word.is_empty() && word.bytes().all(is_ascii_atom));
        if atoms {
            return Ok(text.to_owned());
        }
        let quoted = quote(text);
        if quoted.len() <= options.max_word_length || text.is_empty() {
            return Ok(quoted);
        }
    }

    // All fragments of an overlong word must remain encoded so separator
    // whitespace is discarded instead of becoming part of the original text.
    encode_words(text, options, mode, options.max_word_length)
}

pub(crate) fn encode_parameter(
    text: &str,
    options: &EncodeOptions,
    first_word_length: usize,
) -> Result<String, EncodeError> {
    if !(1..=75).contains(&options.max_word_length) {
        return Err(EncodeError::InvalidWordLength {
            requested: options.max_word_length,
        });
    }
    if !(1..=options.max_word_length).contains(&first_word_length) {
        return Err(EncodeError::InvalidWordLength {
            requested: first_word_length,
        });
    }
    // Even long ASCII values must stay entirely encoded: whitespace inserted
    // between plain tokens would otherwise become part of the parameter.
    encode_words(text, options, QEncodeMode::Text, first_word_length)
}

#[derive(Clone, Copy)]
enum PayloadEncoding {
    B,
    Q,
}

fn encode_words(
    text: &str,
    options: &EncodeOptions,
    mode: QEncodeMode,
    first_word_length: usize,
) -> Result<String, EncodeError> {
    let charset = options.charset;
    let wrapper_length = charset.name().len() + 7;
    let quoted = Rfc2047QuotedPrintableEncoder::new(mode);
    let base64 = Rfc2047Base64Encoder::new();
    let mut scratch = Vec::new();
    let mut output = String::new();
    let mut start = 0;

    while start < text.len() {
        let limit = if start == 0 {
            first_word_length
        } else {
            options.max_word_length
        };
        let mut selected = None;
        let mut required = wrapper_length;
        for (offset, ch) in text[start..].char_indices() {
            let end = start + offset + ch.len_utf8();
            // Convert each candidate as a complete string. Cutting a converted
            // byte buffer could split a scalar, surrogate pair, or shift state.
            // Candidates are bounded by the encoded-word size, not input size.
            let bytes = charset.encode(&text[start..end])?;
            scratch.clear();
            quoted.encode(&bytes, &mut scratch);
            let base64_length = bytes.len().div_ceil(3) * 4;
            // MimeKit requires B for ISO-2022-JP (RFC 1468). Other charsets use
            // actual payload sizes instead of its 17-percent estimate.
            let encoding = if charset.name().eq_ignore_ascii_case("iso-2022-jp")
                || base64_length < scratch.len()
            {
                PayloadEncoding::B
            } else {
                PayloadEncoding::Q
            };
            let payload_length = match encoding {
                PayloadEncoding::B => base64_length,
                PayloadEncoding::Q => scratch.len(),
            };
            required = wrapper_length + payload_length;
            if required > limit {
                break;
            }
            selected = Some((end, encoding));
        }
        let Some((end, encoding)) = selected else {
            return Err(EncodeError::WordTooLong {
                offset: start,
                required,
                limit,
            });
        };
        let bytes = charset.encode(&text[start..end])?;
        scratch.clear();
        let marker = match encoding {
            PayloadEncoding::B => {
                base64.encode(&bytes, &mut scratch);
                'b'
            }
            PayloadEncoding::Q => {
                quoted.encode(&bytes, &mut scratch);
                'q'
            }
        };
        // Readers discard whitespace between adjacent encoded words. Original
        // spaces must therefore stay in their payloads, not in this separator.
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str("=?");
        output.push_str(charset.name());
        output.push('?');
        output.push(marker);
        output.push('?');
        output.push_str(std::str::from_utf8(&scratch).expect("encoded-word payload is ASCII"));
        output.push_str("?=");
        start = end;
    }
    Ok(output)
}

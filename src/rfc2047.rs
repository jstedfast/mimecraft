// Adapted from MimeKit's Utils/Rfc2047.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! RFC 2047 encoded-word encoding and decoding for text and address phrases.
//!
//! These are complete-value transforms, not header-block or address parsers.
//! They retain MimeKit's tolerant payload decoding and broken-mailer recovery.
//! Charset fallback and replacement are reported in [`DecodedHeader::issues`].
//! Encoding produces ASCII value syntax with bounded encoded words, not folded
//! fields. Explicit charset conversion failures are returned without substitution.

mod encode;

pub(crate) use encode::encode_parameter;

pub use encode::{
    EncodeError, EncodeOptions, encode_phrase, encode_phrase_with_options, encode_text,
    encode_text_with_options,
};

use std::ops::Range;

use crate::{
    charsets::{Charset, decode_best},
    encodings::{Base64Decoder, QuotedPrintableDecoder},
    parse_utils::{is_atom, is_whitespace},
};

/// How encoded-word boundaries are recognized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ComplianceMode {
    /// Recover words merged with ordinary text and literal spaces in payloads.
    #[default]
    Loose,
    /// Recognize only complete whitespace-delimited text tokens or phrase atoms.
    ///
    /// This controls tokenization, not strict RFC or payload validation.
    Strict,
}

/// Options shared by text and phrase decoding.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DecodeOptions {
    /// Encoded-word boundary recognition; defaults to loose compatibility mode.
    pub compliance_mode: ComplianceMode,
    /// Preferred charset for unlabelled bytes or unsupported encoded-word labels.
    ///
    /// Valid UTF-8 takes priority; Latin-1 is the final fallback. A recognized
    /// encoded-word label always takes precedence over this preference.
    pub fallback_charset: Option<Charset>,
}

/// Decoded text together with any charset recovery diagnostics.
///
/// Malformed wrappers are retained as ordinary text. Base64 and Q payloads use
/// the existing permissive codecs, so an empty issue list does not certify
/// well-formed RFC 2047 input. This module's decoders preserve controls, CR/LF,
/// and NUL; [`crate::Header::decode_value`] additionally applies display unfolding.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct DecodedHeader {
    /// The decoded text. This module's functions do not trim, unfold, or unquote;
    /// [`crate::Header::decode_value`] additionally unfolds the decoded text.
    pub text: String,
    /// Charset fallback or replacement, in source order.
    pub issues: Vec<DecodeIssue>,
}

/// A charset recovery event, with a half-open byte range in the supplied input.
///
/// Encoded-word ranges include wrappers and any whitespace between words in
/// the affected run. Ranges identify source regions, not individual bad bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeIssue {
    /// One or more encoded words had unsupported labels and used charset guessing.
    UnsupportedCharset {
        /// The complete encoded-word run.
        range: Range<usize>,
        /// The charset selected for its transfer-decoded bytes.
        selected: Charset,
    },
    /// Invalid bytes in a recognized charset were replaced with U+FFFD.
    InvalidBytes {
        /// The complete encoded-word run.
        range: Range<usize>,
        /// The explicitly declared charset.
        charset: Charset,
    },
    /// Unlabelled non-UTF-8 text required a fallback charset.
    UnlabelledText {
        /// The affected ordinary-text token.
        range: Range<usize>,
        /// The charset selected for that token.
        selected: Charset,
    },
}

/// Decodes unstructured text such as a Subject value using default options.
///
/// ```
/// use mimecraft::rfc2047::decode_text;
///
/// let decoded = decode_text(b"Hello =?utf-8?q?Ren=C3=A9e?=");
/// assert_eq!(decoded.text, "Hello Ren\u{e9}e");
/// assert!(decoded.issues.is_empty());
/// ```
pub fn decode_text(input: &[u8]) -> DecodedHeader {
    decode_text_with_options(input, &DecodeOptions::default())
}

/// Decodes unstructured text with explicit boundary and fallback options.
///
/// Whitespace between recognized encoded words is discarded; all other
/// whitespace is preserved. Consecutive words with the same resolved charset
/// and B/Q encoding share decoder state and are converted to Unicode together,
/// recovering split escape sequences and split multibyte characters.
///
/// Unsupported labels share one unknown-charset identity, matching MimeKit.
/// A change of charset, B/Q encoding, or intervening ordinary text ends a run.
/// Supported charset mappings and replacement policy are those of [`crate::charsets`].
pub fn decode_text_with_options(input: &[u8], options: &DecodeOptions) -> DecodedHeader {
    decode(input, options, false)
}

/// Decodes a phrase, such as an address display name, using default options.
///
/// Phrase punctuation is tokenized separately, allowing encoded words next to
/// quotes, parentheses, and other delimiters. Those delimiters remain in the
/// output; this does not parse addresses, remove comments, or unquote strings.
///
/// ```
/// use mimecraft::rfc2047::decode_phrase;
///
/// let decoded = decode_phrase(b"\"=?iso-8859-1?q?Andr=E9?=\"");
/// assert_eq!(decoded.text, "\"Andr\u{e9}\"");
/// ```
pub fn decode_phrase(input: &[u8]) -> DecodedHeader {
    decode_phrase_with_options(input, &DecodeOptions::default())
}

/// Decodes a phrase with explicit boundary and fallback options.
///
/// Strict mode recognizes ASCII atoms; loose mode also recovers encoded words
/// embedded in atoms. Neither mode validates address grammar or quoted strings.
pub fn decode_phrase_with_options(input: &[u8], options: &DecodeOptions) -> DecodedHeader {
    decode(input, options, true)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WordEncoding {
    B,
    Q,
}

struct EncodedWord {
    charset: Option<Charset>,
    encoding: WordEncoding,
    payload: Range<usize>,
}

struct Run {
    charset: Option<Charset>,
    encoding: WordEncoding,
    range: Range<usize>,
}

struct Writer<'a> {
    input: &'a [u8],
    preferred: Option<Charset>,
    output: DecodedHeader,
    run: Option<Run>,
    scratch: Vec<u8>,
    base64: Base64Decoder,
    quoted: QuotedPrintableDecoder,
}

impl<'a> Writer<'a> {
    fn new(input: &'a [u8], preferred: Option<Charset>) -> Self {
        Self {
            input,
            preferred,
            output: DecodedHeader {
                text: String::new(),
                issues: Vec::new(),
            },
            run: None,
            scratch: Vec::new(),
            base64: Base64Decoder::new(),
            quoted: QuotedPrintableDecoder::for_encoded_words(),
        }
    }

    fn word(&mut self, range: Range<usize>, word: EncodedWord) {
        if self
            .run
            .as_ref()
            .is_some_and(|run| run.charset != word.charset || run.encoding != word.encoding)
        {
            self.flush();
        }
        if let Some(run) = &mut self.run {
            run.range.end = range.end;
        } else {
            self.run = Some(Run {
                charset: word.charset,
                encoding: word.encoding,
                range,
            });
        }
        // Broken mailers split Q escapes, Base64 quartets, and charset byte
        // sequences across words. Keep payload state until the entire run ends.
        match word.encoding {
            WordEncoding::B => self
                .base64
                .decode(&self.input[word.payload], &mut self.scratch),
            WordEncoding::Q => self
                .quoted
                .decode(&self.input[word.payload], &mut self.scratch),
        }
    }

    fn raw(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        self.flush();
        let bytes = &self.input[range.clone()];
        let (text, charset) = decode_best(bytes, self.preferred);
        self.output.text.push_str(&text);
        if charset != Charset::UTF8 {
            self.output.issues.push(DecodeIssue::UnlabelledText {
                range,
                selected: charset,
            });
        }
    }

    fn flush(&mut self) {
        let Some(run) = self.run.take() else {
            return;
        };
        self.base64.reset();
        self.quoted.reset();
        match run.charset {
            Some(charset) => {
                let (text, had_errors) = charset.decode_lossy(&self.scratch);
                self.output.text.push_str(&text);
                if had_errors {
                    self.output.issues.push(DecodeIssue::InvalidBytes {
                        range: run.range,
                        charset,
                    });
                }
            }
            None => {
                let (text, selected) = decode_best(&self.scratch, self.preferred);
                self.output.text.push_str(&text);
                self.output.issues.push(DecodeIssue::UnsupportedCharset {
                    range: run.range,
                    selected,
                });
            }
        }
        self.scratch.clear();
    }
}

fn ascii_atom(byte: u8) -> bool {
    byte.is_ascii() && is_atom(byte)
}

fn word_encoding(byte: u8) -> Option<WordEncoding> {
    match byte {
        b'b' | b'B' => Some(WordEncoding::B),
        b'q' | b'Q' => Some(WordEncoding::Q),
        _ => None,
    }
}

fn encoded_word(input: &[u8], range: Range<usize>) -> Option<EncodedWord> {
    let word = &input[range.clone()];
    if word.len() < 7 || !word.starts_with(b"=?") || !word.ends_with(b"?=") {
        return None;
    }
    let end = word.len() - 2;
    let mut index = 2;
    while index < end && !matches!(word[index], b'?' | b'*') {
        if !ascii_atom(word[index]) {
            return None;
        }
        index += 1;
    }
    if index == 2 || index == end {
        return None;
    }
    let charset_end = index;
    if word[index] == b'*' {
        // The optional language suffix is syntax, not part of the charset label.
        index += 1;
        while index < end && word[index] != b'?' {
            if !ascii_atom(word[index]) {
                return None;
            }
            index += 1;
        }
    }
    if index + 2 >= end || word[index] != b'?' || word[index + 2] != b'?' {
        return None;
    }
    let encoding = word_encoding(word[index + 1])?;
    let label = std::str::from_utf8(&word[2..charset_end]).expect("charset is an ASCII atom");
    // Unknown charsets are a reported recovery path, not a failed wrapper match.
    let charset = Charset::from_label(label).ok();
    Some(EncodedWord {
        charset,
        encoding,
        payload: range.start + index + 3..range.end - 2,
    })
}

fn encoded_start(input: &[u8], start: usize) -> bool {
    if !input[start..].starts_with(b"=?") {
        return false;
    }
    let mut index = start + 2;
    while index < input.len() && input[index] != b'?' && !is_whitespace(input[index]) {
        index += 1;
    }
    index + 2 < input.len()
        && input[index] == b'?'
        && word_encoding(input[index + 1]).is_some()
        && input[index + 2] == b'?'
}

fn loose_word_end(input: &[u8], start: usize, phrase: bool) -> usize {
    let mut index = start;
    if input.len() - start > 2 && input[start..].starts_with(b"=?") {
        index += 2;
        while index < input.len() && input[index] != b'?' && !is_whitespace(input[index]) {
            index += 1;
        }
        if index + 3 < input.len()
            && input[index] == b'?'
            && word_encoding(input[index + 1]).is_some()
            && input[index + 2] == b'?'
        {
            index += 3;
            while index + 2 < input.len() && !input[index..].starts_with(b"?=") {
                // Do not let an unterminated word consume a later valid word.
                if input[index] == b'=' && encoded_start(input, index) {
                    break;
                }
                index += 1;
            }
            if input[index..].starts_with(b"?=") {
                return index + 2;
            }
        }
        // Rescan after the initial "=?" so malformed candidates make progress.
        index = start + 2;
    }
    // Recover encoded words merged into ordinary words without the separating
    // whitespace required by RFC 2047.
    while index < input.len()
        && if phrase {
            is_atom(input[index])
        } else {
            !is_whitespace(input[index])
        }
    {
        if input.len() - index > 2 && input[index..].starts_with(b"=?") {
            break;
        }
        index += 1;
    }
    index
}

fn decode(input: &[u8], options: &DecodeOptions, phrase: bool) -> DecodedHeader {
    let mut writer = Writer::new(input, options.fallback_charset);
    let mut index = 0;
    let mut previous_encoded = false;
    while index < input.len() {
        let whitespace_start = index;
        while index < input.len() && is_whitespace(input[index]) {
            index += 1;
        }
        let whitespace = whitespace_start..index;
        if index == input.len() {
            writer.raw(whitespace);
            break;
        }
        let start = index;
        if phrase && !ascii_atom(input[index]) {
            while index < input.len() && !is_whitespace(input[index]) && !ascii_atom(input[index]) {
                index += 1;
            }
            writer.raw(whitespace);
            writer.raw(start..index);
            previous_encoded = false;
            continue;
        }
        if options.compliance_mode == ComplianceMode::Loose {
            index = loose_word_end(input, start, phrase);
        } else {
            // Strict mode tests the whole atom/text token rather than finding
            // encoded-word wrappers embedded inside another word.
            while index < input.len()
                && if phrase {
                    ascii_atom(input[index])
                } else {
                    !is_whitespace(input[index])
                }
            {
                index += 1;
            }
        }
        let range = start..index;
        if let Some(word) = encoded_word(input, range.clone()) {
            // RFC 2047 discards whitespace between encoded words, even when
            // the charset or transfer encoding changes and ends a payload run.
            if !previous_encoded {
                writer.raw(whitespace);
            }
            writer.word(range, word);
            previous_encoded = true;
        } else {
            writer.raw(whitespace);
            writer.raw(range);
            previous_encoded = false;
        }
    }
    writer.flush();
    writer.output
}

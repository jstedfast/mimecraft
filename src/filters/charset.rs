// Adapted from MimeKit's IO/Filters/CharsetFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use encoding_rs::{CoderResult, Decoder, Encoder, EncoderResult};

use super::MimeFilter;
use crate::charsets::{Charset, CharsetError, Codec};

const BUFFER_SIZE: usize = 4096;

/// Incrementally converts bytes between two supported MIME character sets.
///
/// Retains incomplete source characters and stateful encoding shifts across
/// chunks. Conversion uses bounded intermediate buffers, not a copy of the
/// complete input. Charset labels and mappings are shared with [`Charset`]:
/// ASCII/Latin-1 retain their MIME meanings, BOMs are preserved rather than
/// sniffed or inserted, and UTF-16 output is actual UTF-16.
///
/// This is a replacement-based filter, not strict validation. Malformed input
/// becomes U+FFFD; characters unavailable in the target become `?` (one per
/// Unicode scalar). Replacement is reported by sticky diagnostic flags.
/// These uniform rules deliberately differ from .NET's configurable fallbacks;
/// HTML numeric character references are never substituted.
///
/// Call [`finish`](Self::finish) to resolve trailing incomplete input and emit
/// final target shift sequences. Repeated finishes are no-ops. Reset before
/// supplying another nonempty input after finishing.
///
/// ```
/// use mimecraft::{charsets::Charset, filters::CharsetFilter};
///
/// let mut filter = CharsetFilter::new(Charset::UTF8, Charset::LATIN1);
/// let mut bytes = Vec::new();
/// filter.filter(b"Ren\xc3", &mut bytes);
/// filter.filter(b"\xa9e", &mut bytes);
/// filter.finish(&mut bytes);
/// assert_eq!(bytes, b"Ren\xe9e");
/// assert!(!filter.had_errors());
/// ```
pub struct CharsetFilter {
    source: Charset,
    target: Charset,
    decoder: Option<Decoder>,
    encoder: Option<Encoder>,
    decode_errors: bool,
    encode_errors: bool,
    finished: bool,
}

impl CharsetFilter {
    /// Creates a fresh converter using already resolved charset values.
    pub fn new(source: Charset, target: Charset) -> Self {
        let decoder = match source.codec() {
            Codec::Encoding(encoding) => Some(encoding.new_decoder_without_bom_handling()),
            Codec::Ascii | Codec::Latin1 => None,
        };
        let encoder = match target.codec() {
            Codec::Encoding(encoding)
                if encoding != encoding_rs::UTF_8
                    && encoding != encoding_rs::UTF_16LE
                    && encoding != encoding_rs::UTF_16BE =>
            {
                Some(encoding.new_encoder())
            }
            _ => None,
        };
        Self {
            source,
            target,
            decoder,
            encoder,
            decode_errors: false,
            encode_errors: false,
            finished: false,
        }
    }

    /// Resolves source and target labels using [`Charset::from_label`].
    ///
    /// Returns [`CharsetError::UnsupportedLabel`] for an unsupported label.
    pub fn from_labels(source: &str, target: &str) -> Result<Self, CharsetError> {
        Ok(Self::new(
            Charset::from_label(source)?,
            Charset::from_label(target)?,
        ))
    }

    /// Returns the resolved source character set.
    pub fn source_charset(&self) -> Charset {
        self.source
    }

    /// Returns the resolved target character set.
    pub fn target_charset(&self) -> Charset {
        self.target
    }

    /// Reports malformed source input replaced since construction or reset.
    ///
    /// A trailing incomplete sequence is not diagnosed until `finish()`.
    pub fn had_decode_errors(&self) -> bool {
        self.decode_errors
    }

    /// Reports target characters replaced with `?` since construction or reset.
    pub fn had_encode_errors(&self) -> bool {
        self.encode_errors
    }

    /// Reports whether either decoding or encoding required replacement.
    pub fn had_errors(&self) -> bool {
        self.decode_errors || self.encode_errors
    }

    /// Consumes input and appends converted bytes, retaining incomplete state.
    ///
    /// Empty input is a no-op, not end-of-input.
    ///
    /// # Panics
    ///
    /// Panics on nonempty input after `finish()` without an intervening reset.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        if input.is_empty() {
            return;
        }
        assert!(!self.finished, "reset CharsetFilter before reusing it");
        self.convert(input, false, output);
    }

    /// Finalizes decoding first, then encoding, appending all remaining bytes.
    ///
    /// This includes replacement for incomplete source characters and any
    /// target shift back to ASCII. Calling it again does nothing.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.finished {
            return;
        }
        // Even an empty final input must reach the decoder to resolve partial
        // characters retained by an earlier call.
        self.convert(&[], true, output);
        if let Some(encoder) = &mut self.encoder {
            self.encode_errors |= encode_legacy(encoder, "", true, output);
        }
        self.finished = true;
    }

    /// Discards pending conversion state and diagnostics, retaining charsets.
    pub fn reset(&mut self) {
        *self = Self::new(self.source, self.target);
    }

    fn convert(&mut self, mut input: &[u8], last: bool, output: &mut Vec<u8>) {
        if self.decoder.is_some() {
            let mut buffer = [0; BUFFER_SIZE];
            loop {
                let (result, read, written, errors) = self
                    .decoder
                    .as_mut()
                    .expect("backend decoder")
                    .decode_to_utf8(input, &mut buffer, last);
                self.decode_errors |= errors;
                input = &input[read..];
                let text = std::str::from_utf8(&buffer[..written])
                    .expect("decoder emits complete UTF-8 characters");
                self.encode(text, output);
                if result == CoderResult::InputEmpty {
                    break;
                }
            }
        } else {
            for chunk in input.chunks(1024) {
                let (text, errors) = self.source.decode_lossy(chunk);
                self.decode_errors |= errors;
                self.encode(&text, output);
            }
        }
    }

    fn encode(&mut self, text: &str, output: &mut Vec<u8>) {
        // Encode *all* input characters into the output buffer.
        if let Some(encoder) = &mut self.encoder {
            self.encode_errors |= encode_legacy(encoder, text, false, output);
        } else {
            match self.target.codec() {
                Codec::Ascii | Codec::Latin1 => {
                    let maximum = if self.target == Charset::ASCII {
                        127
                    } else {
                        255
                    };
                    for ch in text.chars() {
                        if u32::from(ch) <= maximum {
                            output.push(ch as u8);
                        } else {
                            output.push(b'?');
                            self.encode_errors = true;
                        }
                    }
                }
                Codec::Encoding(_) => output.extend_from_slice(
                    &self
                        .target
                        .encode(text)
                        .expect("Unicode target accepts all text"),
                ),
            }
        }
    }
}

impl MimeFilter for CharsetFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.filter(input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        self.finish(output);
    }

    fn reset(&mut self) {
        self.reset();
    }
}

impl std::fmt::Debug for CharsetFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CharsetFilter")
            .field("source", &self.source)
            .field("target", &self.target)
            .field("decode_errors", &self.decode_errors)
            .field("encode_errors", &self.encode_errors)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

fn encode_legacy(encoder: &mut Encoder, mut text: &str, last: bool, output: &mut Vec<u8>) -> bool {
    let mut buffer = [0; BUFFER_SIZE];
    let mut errors = false;
    loop {
        let (result, read, written) =
            encoder.encode_from_utf8_without_replacement(text, &mut buffer, last);
        output.extend_from_slice(&buffer[..written]);
        text = &text[read..];
        match result {
            EncoderResult::InputEmpty => break,
            EncoderResult::OutputFull => {}
            EncoderResult::Unmappable(_) => {
                errors = true;
                // Encode the replacement through the same encoder so a stateful
                // target is in the right shift state, rather than appending '?'.
                let (result, read, written) =
                    encoder.encode_from_utf8_without_replacement("?", &mut buffer, false);
                assert_eq!(result, EncoderResult::InputEmpty);
                assert_eq!(read, 1);
                output.extend_from_slice(&buffer[..written]);
            }
        }
    }
    errors
}

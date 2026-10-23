// Ported from MimeKit's PassThroughEncoder.cs and PassThroughDecoder.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use crate::ContentEncoding;

/// An encoder that appends input bytes unchanged.
///
/// The selected [`ContentEncoding`] is metadata only, just as in MimeKit.
/// Even when labeled `SevenBit`, this does not reject non-ASCII bytes, normalize
/// newlines, wrap lines, or append a final newline.
///
/// ```
/// use mimecraft::{ContentEncoding, encodings::PassThroughEncoder};
///
/// let mut encoder = PassThroughEncoder::new(ContentEncoding::Binary);
/// let mut output = b"prefix: ".to_vec();
/// encoder.encode(b"a\nb\r\n\xff", &mut output);
/// encoder.finish(&mut output);
/// assert_eq!(output, b"prefix: a\nb\r\n\xff");
/// assert_eq!(encoder.encoding(), ContentEncoding::Binary);
/// ```
#[derive(Clone, Debug, Default)]
pub struct PassThroughEncoder {
    encoding: ContentEncoding,
}

impl PassThroughEncoder {
    /// Creates a pass-through encoder with the specified encoding metadata.
    ///
    /// Any variant is accepted, but none changes the byte-copying behavior.
    /// [`Default::default()`] selects [`ContentEncoding::Unspecified`].
    pub fn new(encoding: ContentEncoding) -> Self {
        Self { encoding }
    }

    /// Returns the configured encoding metadata.
    pub fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    /// Appends the input verbatim, preserving any existing output.
    pub fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.extend_from_slice(input);
    }

    /// Finishes without emitting anything; no input is buffered.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {}

    /// Does nothing; there is no pending state, and the metadata is retained.
    pub fn reset(&mut self) {}
}

/// A decoder that appends input bytes unchanged.
///
/// Like [`PassThroughEncoder`], this treats its [`ContentEncoding`] as metadata
/// only and performs no validation or newline conversion.
#[derive(Clone, Debug, Default)]
pub struct PassThroughDecoder {
    encoding: ContentEncoding,
}

impl PassThroughDecoder {
    /// Creates a pass-through decoder with the specified encoding metadata.
    ///
    /// Any variant is accepted, but none changes the byte-copying behavior.
    /// [`Default::default()`] selects [`ContentEncoding::Unspecified`].
    pub fn new(encoding: ContentEncoding) -> Self {
        Self { encoding }
    }

    /// Returns the configured encoding metadata.
    pub fn encoding(&self) -> ContentEncoding {
        self.encoding
    }

    /// Appends the input verbatim, preserving any existing output.
    pub fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.extend_from_slice(input);
    }

    /// Does nothing; there is no pending state, and the metadata is retained.
    pub fn reset(&mut self) {}
}

// Ported from MimeKit's IO/Filters/EncoderFilter.cs and DecoderFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;
use crate::{
    ContentEncoding,
    encodings::{MimeDecoder, MimeEncoder},
};

/// Adapts an incremental encoder to the filter interface.
///
/// Owns its encoder without requiring allocation or dynamic dispatch. Boxed
/// encoders are also supported. Cloning snapshots both encoder and filter state
/// when the encoder implements [`Clone`].
///
/// ```
/// use mimecraft::{encodings::Base64Encoder, filters::EncoderFilter};
///
/// let mut filter = EncoderFilter::new(Base64Encoder::new());
/// let mut output = Vec::new();
/// filter.filter(b"C", &mut output);
/// assert!(!filter.has_output());
/// filter.filter(b"at", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"Q2F0\r\n");
/// assert!(filter.has_output());
/// ```
#[derive(Clone, Debug)]
pub struct EncoderFilter<E> {
    encoder: E,
    has_output: bool,
}

impl<E: MimeEncoder> EncoderFilter<E> {
    /// Wraps an encoder, retaining its existing configuration and pending state.
    pub fn new(encoder: E) -> Self {
        Self {
            encoder,
            has_output: false,
        }
    }

    /// Returns the wrapped encoder.
    pub fn encoder(&self) -> &E {
        &self.encoder
    }

    /// Returns mutable access to the wrapped encoder.
    ///
    /// Output produced directly through this reference is not counted by
    /// [`has_output`](Self::has_output).
    pub fn encoder_mut(&mut self) -> &mut E {
        &mut self.encoder
    }

    /// Returns the encoder without finishing or resetting it.
    pub fn into_inner(self) -> E {
        self.encoder
    }

    /// Returns the encoder's encoding metadata.
    pub fn encoding(&self) -> ContentEncoding {
        self.encoder.encoding()
    }

    /// Reports whether this filter has emitted bytes since construction or reset.
    ///
    /// Existing bytes in the caller's output buffer do not count. Once true,
    /// this remains true even if the caller drains or clears that buffer.
    pub fn has_output(&self) -> bool {
        self.has_output
    }

    /// Encodes input, appending output and retaining the encoder's pending state.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        let previous_length = output.len();
        self.encoder.encode(input, output);
        self.has_output |= output.len() > previous_length;
    }

    /// Finishes the encoder, preserving its codec-specific finish behavior.
    ///
    /// In particular, every UU finish emits a terminator, and yEnc retains its
    /// checksum. This does not perform I/O or reset the filter.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        let previous_length = output.len();
        self.encoder.finish(output);
        self.has_output |= output.len() > previous_length;
    }

    /// Resets the encoder and output flag, preserving encoder configuration.
    pub fn reset(&mut self) {
        self.encoder.reset();
        self.has_output = false;
    }
}

impl<E: MimeEncoder> MimeFilter for EncoderFilter<E> {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        EncoderFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        EncoderFilter::finish(self, output);
    }

    fn reset(&mut self) {
        EncoderFilter::reset(self);
    }
}

/// Adapts an incremental decoder to the filter interface.
///
/// Owns its decoder without requiring allocation or dynamic dispatch; boxed
/// decoders also work. Cloning snapshots decoder state when it supports [`Clone`].
/// Decoder tolerance and header-scanning modes are unchanged.
///
/// ```
/// use mimecraft::{encodings::Rfc2231Decoder, filters::DecoderFilter};
///
/// let mut filter = DecoderFilter::new(Rfc2231Decoder::new());
/// let mut output = Vec::new();
/// filter.filter(b"file%2", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"file");
/// filter.filter(b"0name", &mut output);
/// assert_eq!(output, b"file name");
/// ```
#[derive(Clone, Debug)]
pub struct DecoderFilter<D> {
    decoder: D,
}

impl<D: MimeDecoder> DecoderFilter<D> {
    /// Wraps a decoder, retaining its existing mode and pending input.
    pub fn new(decoder: D) -> Self {
        Self { decoder }
    }

    /// Returns the wrapped decoder.
    pub fn decoder(&self) -> &D {
        &self.decoder
    }

    /// Returns mutable access to the wrapped decoder.
    pub fn decoder_mut(&mut self) -> &mut D {
        &mut self.decoder
    }

    /// Returns the decoder without finishing or resetting it.
    pub fn into_inner(self) -> D {
        self.decoder
    }

    /// Returns the decoder's encoding metadata.
    pub fn encoding(&self) -> ContentEncoding {
        self.decoder.encoding()
    }

    /// Decodes input, appending output and retaining streaming state.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.decoder.decode(input, output);
    }

    /// Passes empty input to the decoder, matching MimeKit's filter flush.
    ///
    /// Built-in decoders emit nothing here: incomplete groups remain pending,
    /// rather than being flushed, rejected, or reset.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        self.decoder.decode(&[], output);
    }

    /// Resets the decoder, preserving configuration and header-scanning mode.
    pub fn reset(&mut self) {
        self.decoder.reset();
    }
}

impl<D: MimeDecoder> MimeFilter for DecoderFilter<D> {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        DecoderFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        DecoderFilter::finish(self, output);
    }

    fn reset(&mut self) {
        DecoderFilter::reset(self);
    }
}

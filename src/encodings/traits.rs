// Adapted from MimeKit's IMimeEncoder.cs and IMimeDecoder.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{
    Base64Decoder, Base64Encoder, PassThroughDecoder, PassThroughEncoder, QuotedPrintableDecoder,
    QuotedPrintableEncoder, Rfc2231Decoder, UuDecoder, UuEncoder, YDecoder, YEncoder,
};
use crate::ContentEncoding;

/// A common interface for incremental content encoders.
///
/// Input is consumed completely on each call; incomplete groups may be buffered
/// until more input arrives or [`finish`](Self::finish) is called. Output is
/// appended to a caller-owned `Vec`, so no output-size estimate is required.
/// The caller may consume and clear that buffer between calls.
///
/// This trait supports both generic code and `dyn MimeEncoder`; boxed encoders
/// also implement it, so generic adapters can own `Box<dyn MimeEncoder>`. Concrete
/// encoders implement [`Clone`] separately: use `MimeEncoder + Clone` when a
/// generic operation needs to copy the current streaming state.
///
/// Stateless header payload encoders do not implement this trait: in particular,
/// an RFC 2047 B encoder pads each call independently rather than buffering across
/// calls.
///
/// ```
/// use mimecraft::encodings::{Base64Encoder, MimeEncoder};
///
/// fn encode_chunks(encoder: &mut dyn MimeEncoder, chunks: &[&[u8]]) -> Vec<u8> {
///     let mut output = Vec::new();
///     for chunk in chunks {
///         encoder.encode(chunk, &mut output);
///     }
///     encoder.finish(&mut output);
///     output
/// }
///
/// let mut encoder = Base64Encoder::new();
/// assert_eq!(encode_chunks(&mut encoder, &[b"C", b"at"]), b"Q2F0\r\n");
/// ```
pub trait MimeEncoder {
    /// Returns the encoding metadata, matching MimeKit.
    ///
    /// yEnc reports [`ContentEncoding::Default`]. Pass-through encoders report
    /// their configured label without transforming or validating the bytes.
    fn encoding(&self) -> ContentEncoding;

    /// Consumes input, appending encoded bytes and retaining incomplete groups.
    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>);

    /// Appends pending bytes and any codec-specific end-of-input output.
    ///
    /// This is not an I/O flush. Finish behavior remains codec-specific: every
    /// [`UuEncoder`] finish emits a terminator, [`YEncoder`] retains its checksum,
    /// and [`PassThroughEncoder`] emits nothing. Call [`reset`](Self::reset)
    /// before reusing an encoder for an independent body.
    fn finish(&mut self, output: &mut Vec<u8>);

    /// Discards pending input and streaming state while retaining configuration.
    ///
    /// This emits nothing and does not modify previously written output.
    fn reset(&mut self);
}

/// A common interface for incremental content decoders.
///
/// Decoded bytes are appended to a caller-owned `Vec`, which may be consumed and
/// cleared between calls. Each implementation retains its existing tolerance for
/// malformed input, incomplete groups, and end markers; this is not a validation
/// interface. Incomplete final groups remain pending until more input arrives
/// or [`reset`](Self::reset) discards them.
///
/// This trait supports generic code and `dyn MimeDecoder`; boxed decoders also
/// implement it, so generic adapters can own `Box<dyn MimeDecoder>`. Concrete decoders
/// implement [`Clone`] separately, preserving their current streaming state.
/// There is no decoder finish operation, matching MimeKit's decoder interface.
///
/// ```
/// use mimecraft::encodings::{Rfc2231Decoder, MimeDecoder};
///
/// let mut decoder: Box<dyn MimeDecoder> = Box::new(Rfc2231Decoder::new());
/// let mut output = Vec::new();
/// decoder.decode(b"file%2", &mut output);
/// decoder.decode(b"0name", &mut output);
/// assert_eq!(output, b"file name");
/// ```
pub trait MimeDecoder {
    /// Returns the encoding metadata, matching MimeKit.
    ///
    /// [`YDecoder`] and [`Rfc2231Decoder`] report [`ContentEncoding::Default`].
    /// Pass-through decoders return their configured label without interpreting it.
    fn encoding(&self) -> ContentEncoding;

    /// Consumes input, appending decoded bytes and retaining streaming state.
    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>);

    /// Discards pending input and streaming state while retaining configuration.
    ///
    /// This emits nothing and does not modify previously written output.
    fn reset(&mut self);
}

impl<E: MimeEncoder + ?Sized> MimeEncoder for Box<E> {
    fn encoding(&self) -> ContentEncoding {
        (**self).encoding()
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        (**self).encode(input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        (**self).finish(output);
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

impl<D: MimeDecoder + ?Sized> MimeDecoder for Box<D> {
    fn encoding(&self) -> ContentEncoding {
        (**self).encoding()
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        (**self).decode(input, output);
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

impl MimeEncoder for Base64Encoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::Base64
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Base64Encoder::encode(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        Base64Encoder::finish(self, output);
    }

    fn reset(&mut self) {
        Base64Encoder::reset(self);
    }
}

impl MimeEncoder for QuotedPrintableEncoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::QuotedPrintable
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        QuotedPrintableEncoder::encode(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        QuotedPrintableEncoder::finish(self, output);
    }

    fn reset(&mut self) {
        QuotedPrintableEncoder::reset(self);
    }
}

impl MimeEncoder for UuEncoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::UuEncode
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        UuEncoder::encode(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        UuEncoder::finish(self, output);
    }

    fn reset(&mut self) {
        UuEncoder::reset(self);
    }
}

impl MimeEncoder for YEncoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::Default
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        YEncoder::encode(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        YEncoder::finish(self, output);
    }

    fn reset(&mut self) {
        YEncoder::reset(self);
    }
}

impl MimeEncoder for PassThroughEncoder {
    fn encoding(&self) -> ContentEncoding {
        PassThroughEncoder::encoding(self)
    }

    fn encode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        PassThroughEncoder::encode(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        PassThroughEncoder::finish(self, output);
    }

    fn reset(&mut self) {
        PassThroughEncoder::reset(self);
    }
}

impl MimeDecoder for Base64Decoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::Base64
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Base64Decoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        Base64Decoder::reset(self);
    }
}

impl MimeDecoder for QuotedPrintableDecoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::QuotedPrintable
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        QuotedPrintableDecoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        QuotedPrintableDecoder::reset(self);
    }
}

impl MimeDecoder for UuDecoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::UuEncode
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        UuDecoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        UuDecoder::reset(self);
    }
}

impl MimeDecoder for YDecoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::Default
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        YDecoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        YDecoder::reset(self);
    }
}

impl MimeDecoder for Rfc2231Decoder {
    fn encoding(&self) -> ContentEncoding {
        ContentEncoding::Default
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Rfc2231Decoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        Rfc2231Decoder::reset(self);
    }
}

impl MimeDecoder for PassThroughDecoder {
    fn encoding(&self) -> ContentEncoding {
        PassThroughDecoder::encoding(self)
    }

    fn decode(&mut self, input: &[u8], output: &mut Vec<u8>) {
        PassThroughDecoder::decode(self, input, output);
    }

    fn reset(&mut self) {
        PassThroughDecoder::reset(self);
    }
}

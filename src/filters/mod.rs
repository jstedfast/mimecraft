//! Incremental, buffer-oriented MIME filters.
//!
//! Filters append to caller-owned buffers, just like the codecs in
//! [`crate::encodings`]. They do not perform I/O. Call [`MimeFilter::finish`]
//! once at the end of input, not after each chunk, and reset before processing
//! an independent stream.
//!
//! When composing filters, pass each filter's final output through all later
//! filters before finishing those filters. For read-side I/O, use
//! [`crate::io::FilteredReader`]; nesting readers handles this ordering
//! automatically. A mutable list-based filter pipeline is not implemented yet.
//! [`CharsetFilter`] retains split characters and charset shift state, with
//! sticky diagnostics for replacement-based conversion.

// Factory and trait APIs adapted from MimeKit's IO/Filters.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

mod anonymize;
mod best_encoding;
mod charset;
mod codec;
mod from;
mod newline;
mod pass_through;
mod trailing_whitespace;

use crate::{
    ContentEncoding,
    encodings::{
        Base64Decoder, Base64Encoder, EncoderOptionsError, NewLineFormat, QuotedPrintableDecoder,
        QuotedPrintableEncoder, UuDecoder, UuEncoder,
    },
};

pub use anonymize::AnonymizeFilter;
pub use best_encoding::BestEncodingFilter;
pub use charset::CharsetFilter;
pub use codec::{DecoderFilter, EncoderFilter};
pub use from::{ArmoredFromFilter, MboxFromFilter};
pub use newline::{Dos2UnixFilter, Unix2DosFilter};
pub use pass_through::PassThroughFilter;
pub use trailing_whitespace::TrailingWhitespaceFilter;

/// An incremental byte filter, adapted from MimeKit's `IMimeFilter`.
///
/// Each call consumes the supplied input, appending results to `output` without
/// modifying existing bytes. Pending state belongs to the filter, so callers
/// may drain or clear the output between calls.
///
/// This trait supports generic code and trait objects. Boxes and mutable
/// references also implement the trait. Concrete filters may implement
/// [`Clone`] to snapshot their current state.
pub trait MimeFilter {
    /// Processes input, retaining any bytes that need more input to interpret.
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>);

    /// Appends pending bytes and any end-of-input output.
    ///
    /// Unlike MimeKit's `Flush`, this takes no final input: call
    /// [`filter`](Self::filter) first if there are bytes left to process.
    /// This is not an I/O flush or a reset, and repeated calls are not
    /// necessarily idempotent (a UU encoder emits another terminator each time).
    fn finish(&mut self, output: &mut Vec<u8>);

    /// Discards pending state without output, preserving filter configuration.
    fn reset(&mut self);
}

impl<F: MimeFilter + ?Sized> MimeFilter for Box<F> {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        (**self).filter(input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        (**self).finish(output);
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

impl<F: MimeFilter + ?Sized> MimeFilter for &mut F {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        (**self).filter(input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        (**self).finish(output);
    }

    fn reset(&mut self) {
        (**self).reset();
    }
}

/// Creates an encoding filter using CRLF and MimeKit's default width of 78.
///
/// Base64 and quoted-printable cap that width at 76. UU uses its fixed line
/// width; `Unspecified`, `SevenBit`, `EightBit`, and `Binary` pass through unchanged.
/// UU output contains payload and a zero-length terminator, not `begin`/`end`
/// wrappers.
pub fn encoder_filter(encoding: ContentEncoding) -> Box<dyn MimeFilter> {
    encoder_filter_with_options(encoding, 78, NewLineFormat::Dos)
        .expect("default encoder filter options are valid")
}

/// Creates an encoding filter with a requested line width and new-line format.
///
/// Matches MimeKit's enum-based encoder filter factory. `Mixed` is rejected
/// for every encoding, before checking the width. Only Base64 and
/// quoted-printable validate the width (60..=998), then cap it at 76; Base64
/// also rounds down to a multiple of four. UU has a fixed width, and
/// pass-through filters ignore both width and the selected valid line ending.
///
/// # Errors
///
/// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`, or
/// [`EncoderOptionsError::InvalidLineLength`] for an invalid Base64/QP width.
pub fn encoder_filter_with_options(
    encoding: ContentEncoding,
    line_length: usize,
    new_line_format: NewLineFormat,
) -> Result<Box<dyn MimeFilter>, EncoderOptionsError> {
    if new_line_format == NewLineFormat::Mixed {
        return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
    }

    let filter: Box<dyn MimeFilter> = match encoding {
        ContentEncoding::Base64 => Box::new(EncoderFilter::new(Base64Encoder::with_options(
            line_length,
            new_line_format,
        )?)),
        ContentEncoding::QuotedPrintable => Box::new(EncoderFilter::new(
            QuotedPrintableEncoder::with_options(line_length, new_line_format)?,
        )),
        ContentEncoding::UuEncode => Box::new(EncoderFilter::new(UuEncoder::with_new_line_format(
            new_line_format,
        )?)),
        ContentEncoding::Unspecified
        | ContentEncoding::SevenBit
        | ContentEncoding::EightBit
        | ContentEncoding::Binary => Box::new(PassThroughFilter::new()),
    };
    Ok(filter)
}

/// Creates a decoding filter for the specified MIME transfer encoding.
///
/// Base64 and quoted-printable use the existing tolerant body decoders. UU
/// scans for a `begin ` header; to decode unwrapped UU payload, construct a
/// [`DecoderFilter`] with [`UuDecoder::for_payload()`] instead. Other variants
/// pass through unchanged.
///
/// To filter yEnc or parameter percent escaping, construct an adapter with the
/// corresponding codec; these have no dedicated [`ContentEncoding`] variants.
pub fn decoder_filter(encoding: ContentEncoding) -> Box<dyn MimeFilter> {
    match encoding {
        ContentEncoding::Base64 => Box::new(DecoderFilter::new(Base64Decoder::new())),
        ContentEncoding::QuotedPrintable => {
            Box::new(DecoderFilter::new(QuotedPrintableDecoder::new()))
        }
        ContentEncoding::UuEncode => Box::new(DecoderFilter::new(UuDecoder::new())),
        ContentEncoding::Unspecified
        | ContentEncoding::SevenBit
        | ContentEncoding::EightBit
        | ContentEncoding::Binary => Box::new(PassThroughFilter::new()),
    }
}

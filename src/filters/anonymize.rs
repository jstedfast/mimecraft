// Ported from MimeKit's IO/Filters/AnonymizeFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;
use crate::parse_utils::is_whitespace;

/// Replaces every byte except SPACE, TAB, CR, and LF with ASCII `x`.
///
/// Preserves byte counts and whitespace layout, including mixed or bare line
/// endings. This is byte masking, not character processing: each byte of a
/// multibyte character becomes an `x`, even for Unicode whitespace. No transfer
/// decoding, charset conversion, MIME parsing, or header handling occurs.
///
/// This filter alone does not anonymize a message: it preserves lengths and
/// layout, and does not touch headers or content outside the supplied input.
/// It is not a substitute for reviewing data before sharing it.
///
/// Appends to the caller's buffer without changing input or existing output.
/// Has no pending state; chunk boundaries do not affect the result.
///
/// ```
/// use mimecraft::filters::AnonymizeFilter;
///
/// let mut filter = AnonymizeFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"Hello\tworld!\r\n", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"xxxxx\txxxxxx\r\n");
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct AnonymizeFilter;

impl AnonymizeFilter {
    /// Creates a byte-masking filter with no buffered state.
    pub fn new() -> Self {
        Self
    }

    /// Appends one output byte per input byte, preserving only MIME whitespace.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        // Preserve MimeKit's byte whitespace, not the wider set recognized by
        // general-purpose whitespace predicates (which also preserve form feed).
        output.extend(
            input
                .iter()
                .map(|&byte| if is_whitespace(byte) { byte } else { b'x' }),
        );
    }

    /// Emits nothing; no input is buffered.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {}

    /// Does nothing; this filter has no state.
    pub fn reset(&mut self) {}
}

impl MimeFilter for AnonymizeFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        AnonymizeFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        AnonymizeFilter::finish(self, output);
    }

    fn reset(&mut self) {
        AnonymizeFilter::reset(self);
    }
}

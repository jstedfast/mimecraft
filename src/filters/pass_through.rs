// Ported from MimeKit's IO/Filters/PassThroughFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;

/// Appends input unchanged, without buffering, validation, or newline conversion.
///
/// Unlike MimeKit's borrowed input-buffer return, this copies into the
/// caller-owned output buffer, following the other Rust filters.
#[derive(Clone, Copy, Debug, Default)]
pub struct PassThroughFilter;

impl PassThroughFilter {
    /// Creates a pass-through filter.
    pub fn new() -> Self {
        Self
    }

    /// Appends input bytes verbatim.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.extend_from_slice(input);
    }

    /// Emits nothing; no input is buffered.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {}

    /// Does nothing; this filter has no state.
    pub fn reset(&mut self) {}
}

impl MimeFilter for PassThroughFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        PassThroughFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        PassThroughFilter::finish(self, output);
    }

    fn reset(&mut self) {
        PassThroughFilter::reset(self);
    }
}

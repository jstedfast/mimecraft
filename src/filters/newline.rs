// Ported from MimeKit's IO/Filters/Dos2UnixFilter.cs and Unix2DosFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;

/// Converts CRLF to LF, preserving bare LF and lone CR bytes.
///
/// A CR is held until the next byte or [`finish`](Self::finish), so a CRLF pair
/// may span input chunks. The default does not append a newline. With
/// [`with_ensure_new_line`](Self::with_ensure_new_line), finishing adds LF when
/// needed, including for empty input; a trailing CR becomes LF.
///
/// ```
/// use mimecraft::filters::Dos2UnixFilter;
///
/// let mut filter = Dos2UnixFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"one\r", &mut output);
/// filter.filter(b"\ntwo\r", &mut output);
/// assert_eq!(output, b"one\ntwo");
/// filter.finish(&mut output);
/// assert_eq!(output, b"one\ntwo\r");
/// ```
#[derive(Clone, Debug, Default)]
pub struct Dos2UnixFilter {
    ensure_new_line: bool,
    previous: u8,
}

impl Dos2UnixFilter {
    /// Creates a converter that does not add a final newline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with optional final-newline insertion.
    pub fn with_ensure_new_line(ensure_new_line: bool) -> Self {
        Self {
            ensure_new_line,
            previous: 0,
        }
    }

    /// Returns whether finishing ensures a final LF.
    pub fn ensure_new_line(&self) -> bool {
        self.ensure_new_line
    }

    /// Converts CRLF pairs to LF, appending output and holding any trailing CR.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            if byte == b'\n' {
                output.push(byte);
            } else {
                if self.previous == b'\r' {
                    output.push(b'\r');
                }
                if byte != b'\r' {
                    output.push(byte);
                }
            }
            self.previous = byte;
        }
    }

    /// Emits any pending CR and optionally ensures a final LF.
    ///
    /// Repeated calls without new input emit nothing further. Configuration
    /// and the final-byte state are retained; use [`reset`](Self::reset) before
    /// starting an independent stream.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.previous == b'\r' {
            // A trailing CR is held until we know whether LF follows. If we
            // ensure a newline, that CR plus the added LF becomes a single LF.
            if self.ensure_new_line {
                output.push(b'\n');
                self.previous = b'\n';
            } else {
                output.push(b'\r');
                self.previous = 0;
            }
        }
        if self.ensure_new_line && self.previous != b'\n' {
            output.push(b'\n');
            self.previous = b'\n';
        }
    }

    /// Discards pending state, retaining the final-newline option.
    pub fn reset(&mut self) {
        self.previous = 0;
    }
}

impl MimeFilter for Dos2UnixFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Dos2UnixFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        Dos2UnixFilter::finish(self, output);
    }

    fn reset(&mut self) {
        Dos2UnixFilter::reset(self);
    }
}

/// Converts bare LF to CRLF, preserving existing CRLF and lone CR bytes.
///
/// The previous input byte is remembered across chunks, so an existing CRLF
/// pair is never doubled. Input is emitted immediately; no bytes are buffered.
/// With [`with_ensure_new_line`](Self::with_ensure_new_line), finishing appends
/// CRLF when needed, including for empty input, or just LF after a trailing CR.
///
/// ```
/// use mimecraft::filters::Unix2DosFilter;
///
/// let mut filter = Unix2DosFilter::with_ensure_new_line(true);
/// let mut output = Vec::new();
/// filter.filter(b"one\n", &mut output);
/// filter.filter(b"two\r", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"one\r\ntwo\r\n");
/// ```
#[derive(Clone, Debug, Default)]
pub struct Unix2DosFilter {
    ensure_new_line: bool,
    previous: u8,
}

impl Unix2DosFilter {
    /// Creates a converter that does not add a final newline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with optional final-newline insertion.
    pub fn with_ensure_new_line(ensure_new_line: bool) -> Self {
        Self {
            ensure_new_line,
            previous: 0,
        }
    }

    /// Returns whether finishing ensures a final CRLF.
    pub fn ensure_new_line(&self) -> bool {
        self.ensure_new_line
    }

    /// Expands bare LF to CRLF, leaving all other bytes unchanged.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            if byte == b'\n' && self.previous != b'\r' {
                output.push(b'\r');
            }
            output.push(byte);
            self.previous = byte;
        }
    }

    /// Optionally ensures a final CRLF, adding only LF after a trailing CR.
    ///
    /// Repeated calls without new input emit nothing further. Configuration
    /// and the final-byte state are retained; use [`reset`](Self::reset) before
    /// starting an independent stream.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        if self.ensure_new_line && self.previous != b'\n' {
            if self.previous != b'\r' {
                output.push(b'\r');
            }
            output.push(b'\n');
            self.previous = b'\n';
        }
    }

    /// Discards previous-byte state, retaining the final-newline option.
    pub fn reset(&mut self) {
        self.previous = 0;
    }
}

impl MimeFilter for Unix2DosFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        Unix2DosFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        Unix2DosFilter::finish(self, output);
    }

    fn reset(&mut self) {
        Unix2DosFilter::reset(self);
    }
}

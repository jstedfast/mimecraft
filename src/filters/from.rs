// Ported from MimeKit's IO/Filters/MboxFromFilter.cs and ArmoredFromFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;

const MARKER: &[u8; 5] = b"From ";

#[derive(Clone, Debug, Default)]
struct FromFilter {
    matched: usize,
    midline: bool,
}

impl FromFilter {
    fn filter(&mut self, mut input: &[u8], replacement: &[u8], output: &mut Vec<u8>) {
        while !input.is_empty() {
            if self.midline {
                // The previous pass ended mid-line, or we have already decided
                // whether this line starts with "From ". Copy through the next LF.
                let length = input
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(input.len(), |index| index + 1);
                output.extend_from_slice(&input[..length]);
                self.midline = input[length - 1] != b'\n';
                input = &input[length..];
                continue;
            }

            let byte = input[0];
            input = &input[1..];
            if byte == MARKER[self.matched] {
                // This line might start with "From ", but a chunk may end before
                // we know. All saved bytes equal the marker prefix, so only its
                // length needs to be retained instead of a separate input buffer.
                self.matched += 1;
                if self.matched == MARKER.len() {
                    output.extend_from_slice(replacement);
                    self.matched = 0;
                    self.midline = true;
                }
            } else {
                output.extend_from_slice(&MARKER[..self.matched]);
                output.push(byte);
                self.matched = 0;
                self.midline = byte != b'\n';
            }
        }
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        if self.matched != 0 {
            // An incomplete marker at EOF is literal text. It must not combine
            // with later input after these bytes have already been emitted.
            output.extend_from_slice(&MARKER[..self.matched]);
            self.matched = 0;
            self.midline = true;
        }
    }

    fn reset(&mut self) {
        self.matched = 0;
        self.midline = false;
    }
}

/// Escapes lines beginning with exact, case-sensitive `From ` as `>From `.
///
/// Only the start of the stream and positions immediately after LF are line
/// starts. Bare CR does not begin a new line. Existing `>From ` lines and all
/// other bytes, including line endings, remain unchanged.
///
/// Matches MimeKit's mbox filter, not recursive escaping of already quoted
/// lines. The transformation is lossy: an originally quoted line cannot be
/// distinguished from one escaped by this filter.
///
/// ```
/// use mimecraft::filters::MboxFromFilter;
///
/// let mut filter = MboxFromFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"Fr", &mut output);
/// assert!(output.is_empty());
/// filter.filter(b"om sender\r\n>From old\nFrom", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b">From sender\r\n>From old\nFrom");
/// ```
#[derive(Clone, Debug, Default)]
pub struct MboxFromFilter {
    inner: FromFilter,
}

impl MboxFromFilter {
    /// Creates an mbox escaping filter positioned at the beginning of a line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Escapes complete markers, retaining an incomplete prefix across chunks.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.inner.filter(input, b">From ", output);
    }

    /// Emits an incomplete final marker literally, without appending a newline.
    ///
    /// Repeated calls without new input emit nothing further. Line position is
    /// retained; call [`reset`](Self::reset) before an independent stream.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        self.inner.finish(output);
    }

    /// Discards any pending prefix and returns to the beginning of a line.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

impl MimeFilter for MboxFromFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        MboxFromFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        MboxFromFilter::finish(self, output);
    }

    fn reset(&mut self) {
        MboxFromFilter::reset(self);
    }
}

/// Armors lines beginning with exact, case-sensitive `From ` as `=46rom `.
///
/// This uses quoted-printable to encode only the initial `F`. The output must
/// be treated as quoted-printable content, but this is not a full
/// quoted-printable encoder: it neither escapes other bytes nor wraps lines.
/// Apply it after body quoted-printable encoding, not before, to avoid escaping
/// the inserted equals sign again. Each matched line grows by two bytes; leave
/// room for them when choosing the preceding encoder's line width.
///
/// Like [`MboxFromFilter`], this recognizes line starts only at the beginning of
/// the stream or immediately after LF, and retains split marker prefixes.
///
/// ```
/// use mimecraft::filters::ArmoredFromFilter;
///
/// let mut filter = ArmoredFromFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"From one\nFr", &mut output);
/// filter.filter(b"om two", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"=46rom one\n=46rom two");
/// ```
#[derive(Clone, Debug, Default)]
pub struct ArmoredFromFilter {
    inner: FromFilter,
}

impl ArmoredFromFilter {
    /// Creates an armoring filter positioned at the beginning of a line.
    pub fn new() -> Self {
        Self::default()
    }

    /// Armors complete markers, retaining an incomplete prefix across chunks.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        self.inner.filter(input, b"=46rom ", output);
    }

    /// Emits an incomplete final marker literally, without appending a newline.
    ///
    /// Repeated calls without new input emit nothing further. Line position is
    /// retained; call [`reset`](Self::reset) before an independent stream.
    pub fn finish(&mut self, output: &mut Vec<u8>) {
        self.inner.finish(output);
    }

    /// Discards any pending prefix and returns to the beginning of a line.
    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

impl MimeFilter for ArmoredFromFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        ArmoredFromFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        ArmoredFromFilter::finish(self, output);
    }

    fn reset(&mut self) {
        ArmoredFromFilter::reset(self);
    }
}

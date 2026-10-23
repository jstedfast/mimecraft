// Ported from MimeKit's IO/Filters/TrailingWhitespaceFilter.cs and
// Utils/PackedByteArray.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;

/// Removes spaces and tabs immediately before CR, LF, or end of input.
///
/// Both CR and LF independently end a line, including lone CR. Newline bytes
/// are preserved, not normalized. Other whitespace bytes are not stripped.
/// Blanks before ordinary content are emitted unchanged, including indentation.
///
/// Pending blanks use MimeKit's run-length packing: up to 255 equal bytes per
/// 16-bit entry. A long run may span any number of input chunks.
///
/// ```
/// use mimecraft::filters::TrailingWhitespaceFilter;
///
/// let mut filter = TrailingWhitespaceFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"  first \t", &mut output);
/// filter.filter(b"\r\nsecond\t ", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"  first\r\nsecond");
/// ```
#[derive(Clone, Debug, Default)]
pub struct TrailingWhitespaceFilter {
    pending: Vec<u16>,
}

impl TrailingWhitespaceFilter {
    /// Creates a trailing-whitespace filter.
    pub fn new() -> Self {
        Self::default()
    }

    fn save_blank(&mut self, byte: u8) {
        // The low byte is the character; the high byte is its run length.
        // Start another entry at 255 rather than letting the count wrap.
        match self.pending.last_mut() {
            Some(run) if (*run & 0xff) == u16::from(byte) && (*run >> 8) < 255 => {
                *run += 0x100;
            }
            _ => self.pending.push(0x100 | u16::from(byte)),
        }
    }

    fn write_blanks(&mut self, output: &mut Vec<u8>) {
        for run in self.pending.drain(..) {
            let byte = (run & 0xff) as u8;
            let count = usize::from(run >> 8);
            output.extend(std::iter::repeat_n(byte, count));
        }
    }

    /// Appends non-trailing content, buffering spaces and tabs until classified.
    ///
    /// Empty input does not discard pending blanks.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            match byte {
                b' ' | b'\t' => self.save_blank(byte),
                b'\r' | b'\n' => {
                    self.pending.clear();
                    output.push(byte);
                }
                _ => {
                    self.write_blanks(output);
                    output.push(byte);
                }
            }
        }
    }

    /// Discards pending trailing blanks without emitting bytes or a newline.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {
        self.pending.clear();
    }

    /// Discards pending blanks without emitting anything.
    pub fn reset(&mut self) {
        self.pending.clear();
    }
}

impl MimeFilter for TrailingWhitespaceFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        TrailingWhitespaceFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        TrailingWhitespaceFilter::finish(self, output);
    }

    fn reset(&mut self) {
        TrailingWhitespaceFilter::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::TrailingWhitespaceFilter;

    #[test]
    fn whitespace_runs_retain_mimekit_packing_and_do_not_wrap() {
        for count in [0usize, 1, 254, 255, 256, 510, 511, 4096, 65536] {
            let mut filter = TrailingWhitespaceFilter::new();
            let mut output = Vec::new();
            filter.filter(&vec![b' '; count], &mut output);
            assert!(output.is_empty());
            assert_eq!(filter.pending.len(), count.div_ceil(255));
            assert!(filter.pending.iter().all(|run| run & 0xff == 0x20));
            assert_eq!(
                filter
                    .pending
                    .iter()
                    .map(|run| usize::from(run >> 8))
                    .sum::<usize>(),
                count,
            );
            filter.filter(b"x", &mut output);
            assert_eq!(output.len(), count + 1);
            assert!(output[..count].iter().all(|&byte| byte == b' '));
            assert_eq!(output[count], b'x');
            assert!(filter.pending.is_empty());
        }

        let mut filter = TrailingWhitespaceFilter::new();
        let mut output = Vec::new();
        filter.filter(&[b' '; 256], &mut output);
        filter.filter(&[b'\t'; 256], &mut output);
        assert_eq!(filter.pending, [0xff20, 0x0120, 0xff09, 0x0109]);
        filter.finish(&mut output);
        assert!(filter.pending.is_empty());
        assert!(output.is_empty());
    }
}

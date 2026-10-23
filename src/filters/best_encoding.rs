// Ported from MimeKit's IO/Filters/BestEncodingFilter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::MimeFilter;
use crate::{ContentEncoding, EncodingConstraint, encodings::InvalidLineLength};

/// Passes bytes through unchanged while choosing a content transfer encoding.
///
/// Tracks NUL and high-bit bytes, line lengths, and LF-terminated lines starting
/// with exact `From `. Queries use MimeKit's heuristic, not a measurement of
/// every encoder's output size. They neither encode the input nor reset state.
///
/// CR immediately before LF is excluded from line width, even across chunks.
/// Bare CR and an unterminated final CR count toward width. Like MimeKit, a
/// `From ` marker is recorded only when its line's LF arrives, not on `finish`.
///
/// Counters use 64 bits rather than MimeKit's signed 32-bit counters. The
/// 17-percent threshold is computed with exact, overflow-safe integer arithmetic.
///
/// ```
/// use mimecraft::{ContentEncoding, EncodingConstraint, filters::BestEncodingFilter};
///
/// let mut filter = BestEncodingFilter::new();
/// let mut output = Vec::new();
/// filter.filter(b"hello\0world", &mut output);
/// filter.finish(&mut output);
/// assert_eq!(output, b"hello\0world");
/// assert_eq!(
///     filter.best_encoding(EncodingConstraint::SevenBit),
///     ContentEncoding::Base64,
/// );
/// assert_eq!(
///     filter.best_encoding(EncodingConstraint::None),
///     ContentEncoding::Binary,
/// );
/// ```
#[derive(Clone, Debug, Default)]
pub struct BestEncodingFilter {
    total: u64,
    nul_count: u64,
    high_bit_count: u64,
    line_length: u64,
    max_line_length: u64,
    prefix: [u8; 5],
    prefix_length: usize,
    has_from_line: bool,
    previous: u8,
}

impl BestEncodingFilter {
    /// Creates an empty analyzer, which recommends seven-bit encoding.
    pub fn new() -> Self {
        Self::default()
    }

    /// Chooses an encoding using MimeKit's default maximum line length of 78.
    pub fn best_encoding(&self, constraint: EncodingConstraint) -> ContentEncoding {
        self.select_encoding(constraint, 78)
    }

    /// Chooses an encoding with a maximum line length excluding CRLF or LF.
    ///
    /// Unlike an encoder's output width, this limit is not capped at 76. The
    /// final unterminated line also counts. A line exactly at the limit fits.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidLineLength`] if the requested limit is outside 60..=998.
    pub fn best_encoding_with_line_length(
        &self,
        constraint: EncodingConstraint,
        max_line_length: usize,
    ) -> Result<ContentEncoding, InvalidLineLength> {
        if !(60..=998).contains(&max_line_length) {
            return Err(InvalidLineLength::new(max_line_length));
        }

        Ok(self.select_encoding(constraint, max_line_length as u64))
    }

    fn select_encoding(
        &self,
        constraint: EncodingConstraint,
        max_line_length: u64,
    ) -> ContentEncoding {
        // The final line may not be terminated by a newline.
        let longest = self.max_line_length.max(self.line_length);
        let needs_encoding = self.has_from_line || longest > max_line_length;
        // Divide first so multiplying the total by 17 cannot overflow.
        let threshold = self.total / 100 * 17 + self.total % 100 * 17 / 100;

        match constraint {
            EncodingConstraint::SevenBit => {
                if self.nul_count != 0 {
                    return ContentEncoding::Base64;
                }
                if self.high_bit_count != 0 {
                    // MimeKit deliberately uses >= here but > for None below.
                    return if self.high_bit_count >= threshold {
                        ContentEncoding::Base64
                    } else {
                        ContentEncoding::QuotedPrintable
                    };
                }
                if needs_encoding {
                    return ContentEncoding::QuotedPrintable;
                }
            }
            EncodingConstraint::EightBit => {
                if self.nul_count != 0 {
                    return ContentEncoding::Base64;
                }
                if needs_encoding {
                    return ContentEncoding::QuotedPrintable;
                }
                if self.high_bit_count != 0 {
                    return ContentEncoding::EightBit;
                }
            }
            EncodingConstraint::None => {
                if needs_encoding {
                    return if self.nul_count != 0 || self.high_bit_count > threshold {
                        ContentEncoding::Base64
                    } else {
                        ContentEncoding::QuotedPrintable
                    };
                }
                if self.nul_count != 0 {
                    return ContentEncoding::Binary;
                }
                if self.high_bit_count != 0 {
                    return ContentEncoding::EightBit;
                }
            }
        }

        ContentEncoding::SevenBit
    }

    /// Examines input and appends it unchanged, retaining statistics across calls.
    ///
    /// All bytes, including CR and LF, count toward the high-bit ratio's total.
    ///
    /// # Panics
    ///
    /// Panics before processing input if the accumulated byte count would exceed
    /// [`u64::MAX`]. Call [`reset`](Self::reset) before an independent body.
    pub fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        let length = u64::try_from(input.len()).expect("input length exceeds u64::MAX");
        self.total = self
            .total
            .checked_add(length)
            .expect("analyzed byte count exceeds u64::MAX");

        for &byte in input {
            if byte == b'\n' {
                let length = self.line_length - u64::from(self.previous == b'\r');
                self.max_line_length = self.max_line_length.max(length);
                self.line_length = 0;

                // Check our saved line prefix for "From " only at the newline.
                if !self.has_from_line && self.prefix_length == 5 && self.prefix == *b"From " {
                    self.has_from_line = true;
                }
                self.prefix_length = 0;
            } else {
                if byte == 0 {
                    self.nul_count += 1;
                } else if byte > 127 {
                    self.high_bit_count += 1;
                }

                // Retain the prefix across chunks so a split "From " marker
                // can still be recognized when this line eventually reaches LF.
                if !self.has_from_line && self.prefix_length < self.prefix.len() {
                    self.prefix[self.prefix_length] = byte;
                    self.prefix_length += 1;
                }
                self.line_length += 1;
            }
            self.previous = byte;
        }

        output.extend_from_slice(input);
    }

    /// Emits nothing and retains all statistics, including the current line.
    ///
    /// This does not recognize a `From ` marker on an unterminated line.
    pub fn finish(&mut self, _output: &mut Vec<u8>) {}

    /// Clears all statistics and returns to the beginning of a line.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl MimeFilter for BestEncodingFilter {
    fn filter(&mut self, input: &[u8], output: &mut Vec<u8>) {
        BestEncodingFilter::filter(self, input, output);
    }

    fn finish(&mut self, output: &mut Vec<u8>) {
        BestEncodingFilter::finish(self, output);
    }

    fn reset(&mut self) {
        BestEncodingFilter::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_remain_exact_beyond_32_bit_counts() {
        for total in [i32::MAX as u64, u32::MAX as u64 + 1, u64::MAX - 1, u64::MAX] {
            let threshold = (u128::from(total) * 17 / 100) as u64;
            let mut filter = BestEncodingFilter {
                total,
                high_bit_count: threshold,
                has_from_line: true,
                ..BestEncodingFilter::new()
            };
            assert_eq!(
                filter.best_encoding(EncodingConstraint::SevenBit),
                ContentEncoding::Base64
            );
            assert_eq!(
                filter.best_encoding(EncodingConstraint::None),
                ContentEncoding::QuotedPrintable
            );
            filter.high_bit_count -= 1;
            assert_eq!(
                filter.best_encoding(EncodingConstraint::SevenBit),
                ContentEncoding::QuotedPrintable
            );
            filter.high_bit_count += 2;
            assert_eq!(
                filter.best_encoding(EncodingConstraint::None),
                ContentEncoding::Base64
            );
        }
    }

    #[test]
    fn counters_can_cross_the_source_integer_limit() {
        let mut filter = BestEncodingFilter {
            total: i32::MAX as u64,
            high_bit_count: i32::MAX as u64,
            line_length: i32::MAX as u64,
            previous: 0xff,
            ..BestEncodingFilter::new()
        };
        let mut output = Vec::new();
        filter.filter(b"\xff\0\r\n", &mut output);
        assert_eq!(output, b"\xff\0\r\n");
        assert_eq!(filter.total, i32::MAX as u64 + 4);
        assert_eq!(filter.high_bit_count, i32::MAX as u64 + 1);
        assert_eq!(filter.nul_count, 1);
        assert_eq!(filter.max_line_length, i32::MAX as u64 + 2);
        assert_eq!(filter.line_length, 0);
    }

    #[test]
    #[should_panic(expected = "analyzed byte count exceeds u64::MAX")]
    fn counter_overflow_is_explicit() {
        let mut filter = BestEncodingFilter {
            total: u64::MAX,
            ..BestEncodingFilter::new()
        };
        filter.filter(b"x", &mut Vec::new());
    }
}

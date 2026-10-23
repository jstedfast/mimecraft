// Adapted from MimeKit's Text/TextToFlowed.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{next_line, output_newline};
use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// Wraps plain text for `text/plain; format=flowed; delsp=yes`.
///
/// Preserves MimeKit's word-boundary preference, quote-prefix normalization,
/// space stuffing, and trimming of ASCII spaces before hard line breaks.
/// Leading `>` markers (optionally separated by one space each) are interpreted
/// as quotation, not literal text. All input lines, even an unterminated last
/// line, produce a terminating newline. Empty input produces empty output.
/// Input CRLF, LF, and bare CR are accepted; output defaults to CRLF.
///
/// Lines normally target 78 bytes including quote markers, stuffing, and the
/// trailing soft-break space, but excluding the newline. Long words may exceed
/// that target up to 998 bytes before splitting. Deep quote prefixes increase
/// both limits to at least twice the prefix length to avoid amplifying output.
/// Unlike MimeKit's UTF-16 character counts, lengths are UTF-8 byte counts.
/// Splits never divide a UTF-8 character; grapheme clusters may still be split.
/// These are pre-transfer-encoding lengths, not limits for every output charset.
///
/// This is not lossless or strict RFC validation: trailing ASCII spaces are
/// stripped even from `-- ` signature separators, and quote-only lines keep
/// MimeKit's trailing stuffing space. No headers are set automatically.
/// The caller must declare `format=flowed; delsp=yes` when using the result as
/// MIME text. No charset conversion, stream I/O, or header/footer insertion occurs.
///
/// ```
/// use mimecraft::text::{FlowedToText, TextToFlowed};
///
/// let flowed = TextToFlowed::new().convert("From someone\n> quoted text");
/// assert_eq!(flowed, " From someone\r\n> quoted text\r\n");
/// let mut decoder = FlowedToText::new();
/// decoder.delete_space = true;
/// assert_eq!(decoder.convert(&flowed), "From someone\r\n> quoted text\r\n");
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextToFlowed {
    new_line_format: NewLineFormat,
}

impl TextToFlowed {
    /// Creates a converter that emits CRLF on every platform.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with explicit output line endings.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`, which
    /// describes input rather than a valid output format.
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }
        Ok(Self { new_line_format })
    }

    /// Returns the configured output line endings.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Wraps a complete Unicode string without changing the converter.
    pub fn convert(&self, text: &str) -> String {
        let newline = output_newline(self.new_line_format);
        let mut output = String::new();
        let mut flowed = String::with_capacity(78);
        let mut remaining = text;
        while let Some(line) = next_line(&mut remaining) {
            // Trim spaces before user-inserted hard line breaks.
            let mut line = line.trim_end_matches(' ');
            let mut depth = 0;
            while let Some(unquoted) = line.strip_prefix('>') {
                depth += 1;
                line = unquoted.strip_prefix(' ').unwrap_or(unquoted);
            }
            let mut index = 0;
            let mut word_end = 0;
            loop {
                // Count stuffing and the soft-break space in the line limits,
                // but not CRLF. Long words may exceed the soft target.
                flow_line(&mut flowed, line, &mut index, &mut word_end, depth);
                output.push_str(&flowed);
                output.push_str(newline);
                if index == line.len() {
                    break;
                }
            }
        }
        output
    }
}

fn flow_line(
    flowed: &mut String,
    line: &str,
    index: &mut usize,
    word_end: &mut usize,
    depth: usize,
) {
    flowed.clear();
    flowed.extend(std::iter::repeat_n('>', depth));

    // Space-stuff lines starting with space, "From ", or ">" (RFC 3676, 4.4).
    // Quote markers have already been removed, so a leading ">" in the text
    // can only appear after a soft break. Quoted lines always get a space.
    let remaining = &line[*index..];
    if depth > 0 || remaining.starts_with([' ', '>']) || remaining.starts_with("From ") {
        flowed.push(' ');
    }

    let prefix_length = flowed.len();
    // Deep quotation can exhaust either line limit. Always leave at least as
    // much room for text as for the prefix; otherwise repeating the prefix for
    // every word would amplify output by quote depth times the number of words.
    let soft_limit = 78.max(prefix_length * 2);
    let hard_limit = 998.max(prefix_length * 2);
    if flowed.len() + remaining.len() <= soft_limit {
        flowed.push_str(remaining);
        *index = line.len();
        return;
    }

    loop {
        // Cache the word boundary across hard splits so a huge unbroken word
        // is scanned once, not once for every emitted line.
        if *index >= *word_end {
            *word_end = line[*index..]
                .find(' ')
                .map_or(line.len(), |offset| *index + offset);
        }
        let word_length = *word_end - *index;
        let soft_break = if *word_end == line.len() { 0 } else { 2 };
        // Two spaces reserve the original separator plus the delsp marker.
        if flowed.len() + word_length + soft_break <= soft_limit {
            // The entire word fits on the remainder of this line.
            flowed.push_str(&line[*index..*word_end]);
            *index = *word_end;
        } else if flowed.len() > prefix_length
            && prefix_length + word_length + soft_break <= soft_limit
        {
            // Move a word that fits on its own line rather than splitting it.
            break;
        } else if flowed.len() + word_length + soft_break <= hard_limit {
            // Exceed the soft target rather than break a long word (RFC 3676,
            // 4.2), provided the word still fits within the hard limit.
            flowed.push_str(&line[*index..*word_end]);
            *index = *word_end;
        } else if flowed.len() > prefix_length
            && prefix_length + word_length + soft_break <= hard_limit
        {
            // The word fits within the hard limit on a line of its own.
            break;
        } else {
            // No choice but to split this word. Reserve a soft-break space
            // and back up to a UTF-8 boundary instead of splitting a character.
            let mut end = *index + hard_limit - (flowed.len() + 1);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            flowed.push_str(&line[*index..end]);
            *index = end;
            break;
        }

        let limit = if flowed.len() < soft_limit {
            soft_limit
        } else {
            hard_limit
        };
        while flowed.len() + 1 < limit && line.as_bytes().get(*index) == Some(&b' ') {
            flowed.push(' ');
            *index += 1;
        }
        if *index == line.len() || flowed.len() + 1 >= soft_limit {
            break;
        }
    }
    if *index < line.len() {
        flowed.push(' ');
    }
}

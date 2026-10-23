// Adapted from MimeKit's Text/PlainTextPreviewer.cs and Text/TextPreviewer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

/// Produces whitespace-normalized previews of already-decoded plain text.
///
/// Collapses Unicode whitespace and U+200B through U+200D to ASCII spaces,
/// omitting leading and trailing spaces. If input remains when the output
/// reaches its limit, replaces the last character with U+2026 (ellipsis).
/// Trailing-space removal happens before replacement, as in MimeKit's string
/// overload, so a truncated preview can be shorter than the limit. Unconsumed
/// whitespace also triggers an ellipsis; this is not word-boundary truncation.
///
/// The default limit is 230; valid limits are 1 through 1024, inclusive.
/// Lengths count Unicode scalar values, not bytes, grapheme clusters, or
/// MimeKit's UTF-16 code units. Characters are never split, but combining
/// sequences may be. Zero-width joiners are treated as whitespace even inside
/// emoji sequences, preserving the source's whitespace classification.
///
/// No HTML interpretation, flowed-text unwrapping, charset decoding, MIME
/// body selection, or stream I/O occurs. Normalize flowed text separately.
///
/// ```
/// use mimecraft::text::PlainTextPreviewer;
///
/// let previewer = PlainTextPreviewer::with_maximum_preview_length(12)?;
/// assert_eq!(previewer.preview(" \tHello\r\nworld!  More"), "Hello world\u{2026}");
/// # Ok::<(), mimecraft::text::InvalidPreviewLength>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlainTextPreviewer {
    maximum_preview_length: usize,
}

impl Default for PlainTextPreviewer {
    fn default() -> Self {
        Self {
            maximum_preview_length: 230,
        }
    }
}

impl PlainTextPreviewer {
    /// Creates a previewer with a maximum length of 230 Unicode scalar values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a previewer with an explicit maximum output length.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPreviewLength`] if the limit is outside 1..=1024.
    pub fn with_maximum_preview_length(
        maximum_preview_length: usize,
    ) -> Result<Self, InvalidPreviewLength> {
        if !(1..=1024).contains(&maximum_preview_length) {
            return Err(InvalidPreviewLength {
                requested: maximum_preview_length,
            });
        }
        Ok(Self {
            maximum_preview_length,
        })
    }

    /// Returns the maximum preview length in Unicode scalar values.
    pub fn maximum_preview_length(&self) -> usize {
        self.maximum_preview_length
    }

    /// Generates a preview without changing the previewer or the input.
    ///
    /// Empty or all-whitespace input yields an empty string. Stops scanning
    /// once the output reaches the configured limit; does not normalize the
    /// entire input first. Output storage is bounded by that limit, although
    /// arbitrarily long whitespace runs may need to be scanned.
    pub fn preview(&self, text: &str) -> String {
        let mut preview = String::with_capacity(self.maximum_preview_length.min(text.len()));
        let mut remaining = text.chars();
        let mut preview_length = 0;
        let mut whitespace = true;

        // Count complete Unicode scalars instead of UTF-16 code units so
        // truncation cannot leave half a surrogate pair or UTF-8 character.
        while preview_length < self.maximum_preview_length {
            let Some(character) = remaining.next() else {
                break;
            };
            if character.is_whitespace() || ('\u{200b}'..='\u{200d}').contains(&character) {
                if !whitespace {
                    preview.push(' ');
                    preview_length += 1;
                    whitespace = true;
                }
            } else {
                preview.push(character);
                preview_length += 1;
                whitespace = false;
            }
        }

        // Match the string overload's ordering: trim first, then replace the
        // last retained character, even when only whitespace remains unread.
        if whitespace && !preview.is_empty() {
            preview.pop();
        }
        if !remaining.as_str().is_empty() {
            preview.pop();
            preview.push('\u{2026}');
        }
        preview
    }
}

/// A preview length outside the supported range of 1..=1024.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPreviewLength {
    requested: usize,
}

impl InvalidPreviewLength {
    /// Returns the rejected preview length.
    pub fn requested(self) -> usize {
        self.requested
    }
}

impl fmt::Display for InvalidPreviewLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "preview length must be between 1 and 1024 characters, got {}",
            self.requested
        )
    }
}

impl Error for InvalidPreviewLength {}

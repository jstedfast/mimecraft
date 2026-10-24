// Adapted from MimeKit's Text/PlainTextPreviewer.cs and Text/TextPreviewer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, BufRead},
};

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
/// Accepts decoded strings or buffered UTF-8 readers. No HTML interpretation,
/// flowed-text unwrapping, legacy charset decoding, or MIME body selection
/// occurs. Normalize flowed text separately.
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
        let mut preview = Preview::new(self.maximum_preview_length.min(text.len()));
        let mut remaining = text.chars();

        // Count complete Unicode scalars instead of UTF-16 code units so
        // truncation cannot leave half a surrogate pair or UTF-8 character.
        while preview.length < self.maximum_preview_length {
            let Some(character) = remaining.next() else {
                break;
            };
            preview.push(character);
        }
        preview.finish(!remaining.as_str().is_empty())
    }

    /// Generates a preview from the current position of a buffered UTF-8 reader.
    ///
    /// Matches [`Self::preview`], including zero-width whitespace and trimming
    /// before ellipsis replacement. Unlike MimeKit's distinct string and reader
    /// algorithms, both entry points use the same normalization rules.
    /// A UTF-8 BOM is retained as U+FEFF, not stripped or used to detect a charset.
    ///
    /// Consumes complete characters until the normalized output reaches the
    /// limit or EOF. At the limit, peeks for an unread byte to decide whether an
    /// ellipsis is needed, but does not consume that byte or validate the suffix.
    /// Whitespace-only input may require scanning to EOF. Preview storage is
    /// bounded by the configured limit; the whole input is never collected.
    ///
    /// The reader is borrowed, not closed, sought, or drained. Its own buffering
    /// may read ahead from its underlying source; continue through this same
    /// reader to retain those bytes. Use [`std::io::BufReader`] for an unbuffered
    /// source. Charset and MIME transfer decoding must be supplied separately.
    ///
    /// # Errors
    ///
    /// Malformed or incomplete UTF-8 encountered before the preview limit
    /// returns [`io::ErrorKind::InvalidData`]. Read failures propagate, including
    /// failures while checking for unread input. Interrupted reads retry. No
    /// partial preview is returned, and consumed bytes are not restored.
    ///
    /// ```
    /// use mimecraft::text::PlainTextPreviewer;
    ///
    /// let previewer = PlainTextPreviewer::with_maximum_preview_length(5)?;
    /// let mut input = b"Hello world".as_slice();
    /// assert_eq!(previewer.preview_reader(&mut input)?, "Hell\u{2026}");
    /// assert_eq!(input, b" world");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn preview_reader<R: BufRead + ?Sized>(&self, reader: &mut R) -> io::Result<String> {
        let mut preview = Preview::new(self.maximum_preview_length);
        while preview.length < self.maximum_preview_length {
            let Some(character) = read_character(reader)? else {
                return Ok(preview.finish(false));
            };
            preview.push(character);
        }
        // Look ahead without consuming the suffix: a full preview alone does
        // not distinguish exact-length input from text requiring an ellipsis.
        Ok(preview.finish(peek_byte(reader)?.is_some()))
    }
}

struct Preview {
    text: String,
    length: usize,
    whitespace: bool,
}

impl Preview {
    fn new(capacity: usize) -> Self {
        Self {
            text: String::with_capacity(capacity),
            length: 0,
            whitespace: true,
        }
    }

    fn push(&mut self, character: char) {
        if character.is_whitespace() || ('\u{200b}'..='\u{200d}').contains(&character) {
            if !self.whitespace {
                self.text.push(' ');
                self.length += 1;
                self.whitespace = true;
            }
        } else {
            self.text.push(character);
            self.length += 1;
            self.whitespace = false;
        }
    }

    fn finish(mut self, truncated: bool) -> String {
        // Match the string overload's ordering: trim first, then replace the
        // last retained character, even when only whitespace remains unread.
        if self.whitespace && !self.text.is_empty() {
            self.text.pop();
        }
        if truncated {
            self.text.pop();
            self.text.push('\u{2026}');
        }
        self.text
    }
}

fn peek_byte<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Option<u8>> {
    loop {
        match reader.fill_buf() {
            Ok(buffer) => return Ok(buffer.first().copied()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn read_character<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Option<char>> {
    let Some(first) = peek_byte(reader)? else {
        return Ok(None);
    };
    let width = match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    };
    let mut bytes = [first, 0, 0, 0];
    let mut length = 1;
    reader.consume(1);
    // A buffer boundary can split a UTF-8 scalar. Collect only that scalar,
    // leaving decoding strict without validating or consuming the whole buffer.
    while length < width {
        let Some(byte) = peek_byte(reader)? else {
            break;
        };
        bytes[length] = byte;
        length += 1;
        reader.consume(1);
    }
    let text = std::str::from_utf8(&bytes[..length])
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(text.chars().next())
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

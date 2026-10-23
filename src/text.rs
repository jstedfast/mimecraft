// Adapted from MimeKit's Text/FlowedToText.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Whole-string text-format conversion, separate from charset/transfer decoding.
//!
//! Currently provides MimeKit-compatible flowed-text unwrapping. HTML rendering,
//! rich-text conversion, flowed-text encoding, and streaming converters are deferred.

use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// Unwraps flowed text using MimeKit's paragraph and quote-depth rules.
///
/// Removes one stuffing space after the quote prefix, joins lines ending in an
/// ASCII space, and starts a new paragraph when quote depth changes. Quoted
/// output uses a run of `>` followed by one space. Tabs and Unicode whitespace
/// are not soft-break markers. Input CRLF, LF, and bare CR are accepted.
///
/// Hard paragraphs end with the configured newline, even if the final input
/// line was unterminated. A pending nonempty flowed paragraph at EOF is emitted
/// without a newline. An empty pending paragraph is omitted. Like MimeKit's
/// converter, `-- ` is not special-cased as a signature separator: its trailing
/// space is treated as a soft break. This is compatibility behavior, not strict
/// RFC 3676 validation.
///
/// Unlike the platform-dependent C# writer, output defaults to CRLF everywhere.
/// No charset decoding, HTML interpretation, header/footer insertion, or stream
/// I/O occurs. Each call starts fresh and materializes a new string.
///
/// ```
/// use mimecraft::text::FlowedToText;
///
/// let mut converter = FlowedToText::new();
/// assert_eq!(converter.convert("> hello \n> world\n"), "> hello world\r\n");
/// converter.delete_space = true;
/// assert_eq!(converter.convert("soft \nbreak"), "softbreak\r\n");
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FlowedToText {
    /// Removes exactly one trailing soft-break space when true.
    ///
    /// Defaults to false. For MIME text, enable only when `delsp=yes`, ignoring
    /// ASCII case. Other trailing spaces and non-ASCII whitespace are preserved.
    pub delete_space: bool,
    new_line_format: NewLineFormat,
}

impl FlowedToText {
    /// Creates a converter that retains soft-break spaces and emits CRLF.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with explicit paragraph line endings.
    ///
    /// Soft-break spaces are retained until [`Self::delete_space`] is set.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`, which
    /// describes observed input rather than a valid output format.
    pub fn with_new_line_format(
        new_line_format: NewLineFormat,
    ) -> Result<Self, EncoderOptionsError> {
        if new_line_format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }
        Ok(Self {
            new_line_format,
            ..Self::default()
        })
    }

    /// Returns the configured paragraph line endings.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Unwraps a complete Unicode string without changing the converter.
    pub fn convert(&self, text: &str) -> String {
        let newline = match self.new_line_format {
            NewLineFormat::Dos => "\r\n",
            NewLineFormat::Unix => "\n",
            NewLineFormat::Mixed => unreachable!("constructors reject mixed output newlines"),
        };
        let mut output = String::new();
        let mut paragraph = String::new();
        let mut paragraph_depth = None;
        let mut remaining = text;
        while !remaining.is_empty() {
            let line;
            if let Some(index) = remaining.find(['\r', '\n']) {
                line = &remaining[..index];
                let length = if remaining[index..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                remaining = &remaining[index + length..];
            } else {
                line = remaining;
                remaining = "";
            }
            let depth = line.bytes().take_while(|&byte| byte == b'>').count();
            let unquoted = &line[depth..];
            // A leading space after the quote markers is stuffing, not text.
            let unquoted = unquoted.strip_prefix(' ').unwrap_or(unquoted);
            if let Some(previous) = paragraph_depth
                && depth != previous
            {
                // Quote-depth changes win over a preceding soft break.
                append_paragraph(&mut output, &paragraph, previous);
                output.push_str(newline);
                paragraph.clear();
            }
            paragraph_depth = Some(depth);
            paragraph.push_str(unquoted);
            if !unquoted.ends_with(' ') {
                // Without a trailing space, this line ends the paragraph.
                append_paragraph(&mut output, &paragraph, depth);
                output.push_str(newline);
                paragraph.clear();
                paragraph_depth = None;
            } else if self.delete_space {
                // A trailing space marks continuation; delsp removes that
                // marker before joining the next line.
                paragraph.pop();
            }
        }
        // A final flowed line has no hard paragraph ending (MimeKit issue #1130).
        if !paragraph.is_empty() {
            append_paragraph(
                &mut output,
                &paragraph,
                paragraph_depth.expect("pending text has a quote depth"),
            );
        }
        output
    }
}

fn append_paragraph(output: &mut String, paragraph: &str, depth: usize) {
    if depth > 0 {
        output.extend(std::iter::repeat_n('>', depth));
        output.push(' ');
    }
    output.push_str(paragraph);
}

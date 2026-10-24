// Adapted from MimeKit's Text/RtfToText.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};
use std::num::NonZeroUsize;

use super::rtf_interpreter::{
    Content, ContentHandler, DEFAULT_LIMIT, GroupState, InterpreterOptions, interpret,
};
use super::{RtfIssue, output_newline};
use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// Converts original RTF bytes to a complete Unicode plain-text string.
///
/// Interprets group-scoped formatting, font code pages, Unicode escapes,
/// paragraphs, tables, and special characters. Non-text destinations and hidden
/// text are suppressed; field results are rendered without their instructions.
/// Encapsulated HTML is not extracted: its RTF rendering is used instead.
///
/// Output defaults to CRLF on every platform. Input must be original bytes,
/// not a UTF-8 re-encoding of an already decoded RTF document. BOMs are not
/// sniffed. Charset mappings and replacement decoding use
/// [`Charset`](crate::charsets::Charset) and
/// [`CharsetFilter`](crate::filters::CharsetFilter); unavailable pages fall back
/// to Windows-1252 with an optional diagnostic, not to a platform encoding.
///
/// Each call starts fresh. Retained interpreter state is bounded, but the
/// returned string and caller-owned diagnostic/placeholder output are not.
/// Colors and hyperlink targets are not retained because they do not affect
/// plain text; color-table and field-instruction validation are not performed.
///
/// ```
/// use mimecraft::text::RtfToText;
///
/// let text = RtfToText::new().convert(&b"{\\rtf1 Hello \\u233?\\par world}"[..])?;
/// assert_eq!(text, "Hello \u{e9}\r\nworld");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtfToText {
    /// Text prepended verbatim, without an inserted separator.
    pub header: Option<String>,
    /// Text appended verbatim, without an inserted separator.
    pub footer: Option<String>,
    /// Maximum distinct saved group states; defaults to 4096.
    ///
    /// Identical consecutive states share a stack entry. Groups that need an
    /// extra entry beyond this limit are discarded, not rendered in wrong state.
    /// Hyperlink targets do not participate in this text-only state.
    pub max_group_depth: NonZeroUsize,
    /// Maximum recorded font numbers; defaults to 4096.
    ///
    /// Once full, existing entries can still be updated. Unknown fonts use the
    /// document code page. Font names are never stored.
    pub max_font_table_entries: NonZeroUsize,
    /// Per-violation callback quota for each conversion; zero means unlimited.
    ///
    /// All diagnostics share one first-suppression marker. This does not affect
    /// rendering or resource limits. At most sixteen times a positive limit
    /// plus one issues are delivered.
    pub max_issues_per_violation: usize,
    new_line_format: NewLineFormat,
}

impl Default for RtfToText {
    fn default() -> Self {
        Self {
            header: None,
            footer: None,
            max_group_depth: DEFAULT_LIMIT,
            max_font_table_entries: DEFAULT_LIMIT,
            max_issues_per_violation: 0,
            new_line_format: NewLineFormat::Dos,
        }
    }
}

impl RtfToText {
    /// Creates a converter with CRLF output and default resource limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with explicit output line endings.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`.
    pub fn with_new_line_format(format: NewLineFormat) -> Result<Self, EncoderOptionsError> {
        if format == NewLineFormat::Mixed {
            return Err(EncoderOptionsError::InvalidNewLineFormat(format));
        }
        Ok(Self {
            new_line_format: format,
            ..Self::default()
        })
    }

    /// Returns the paragraph and line-break output format.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Converts a complete buffered byte source without reporting diagnostics.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors. Unlike the low-level tokenizer, a failed
    /// whole-document conversion cannot be resumed: partial output/state is
    /// discarded, and the source may already have been consumed.
    pub fn convert<R: BufRead>(&self, reader: R) -> io::Result<String> {
        self.convert_core(reader, None::<fn(RtfIssue)>, None::<fn(usize, &mut String)>)
    }

    /// Converts a source with lexical, structural, and text-decoding diagnostics.
    ///
    /// Diagnostics may already have been delivered when a source error occurs.
    /// Structural checks precede interpretation of each token. Decoder issues
    /// use the current token's start (possibly a later flushing token), or EOF.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors; a failed conversion cannot be resumed.
    pub fn convert_with_issues<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
    ) -> io::Result<String> {
        self.convert_core(reader, Some(report), None::<fn(usize, &mut String)>)
    }

    /// Converts a source with diagnostics and attachment-placeholder rendering.
    ///
    /// `placeholder` receives a zero-based, saturating index and the output
    /// string to append to. Only visible placeholders count. The immediately
    /// following literal or hex-escaped space is removed; other content is kept.
    /// Without this callback placeholders themselves produce no text.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors. Callback side effects are not rolled back.
    pub fn convert_with_callbacks<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
        placeholder: impl FnMut(usize, &mut String),
    ) -> io::Result<String> {
        self.convert_core(reader, Some(report), Some(placeholder))
    }

    fn convert_core<R: BufRead, F: FnMut(RtfIssue), G: FnMut(usize, &mut String)>(
        &self,
        reader: R,
        report: Option<F>,
        placeholder: Option<G>,
    ) -> io::Result<String> {
        // Render the RTF portion, including \htmlrtf blocks, rather than the
        // original HTML markup. Decoding and group recovery are shared with HTML.
        let options = InterpreterOptions {
            max_group_depth: self.max_group_depth,
            max_font_table_entries: self.max_font_table_entries,
            max_color_table_entries: None,
            collect_hyperlinks: false,
            max_issues_per_violation: self.max_issues_per_violation,
        };
        let handler = TextHandler {
            output: self.header.clone().unwrap_or_default(),
            newline: output_newline(self.new_line_format),
            pending_cell_separator: false,
            placeholder,
        };
        let mut output = interpret(reader, options, report, handler)?.output;
        if let Some(footer) = &self.footer {
            output.push_str(footer)
        }
        Ok(output)
    }
}

struct TextHandler<G> {
    output: String,
    newline: &'static str,
    pending_cell_separator: bool,
    placeholder: Option<G>,
}

impl<G> TextHandler<G> {
    fn flush_cell_separator(&mut self) {
        if std::mem::take(&mut self.pending_cell_separator) {
            self.output.push('\t');
        }
    }
}

impl<G: FnMut(usize, &mut String)> ContentHandler for TextHandler<G> {
    fn content(&mut self, _: &GroupState, _: &[Option<u32>], content: Content<'_>) {
        match content {
            Content::Text(text) => {
                self.flush_cell_separator();
                self.output.push_str(text);
            }
            Content::Paragraph | Content::LineBreak => {
                self.flush_cell_separator();
                self.output.push_str(self.newline);
            }
            Content::Cell => {
                // Even the last cell ends in \cell. Defer its separator so a
                // following \row does not leave a trailing tab.
                self.flush_cell_separator();
                self.pending_cell_separator = true;
            }
            Content::Row => {
                self.pending_cell_separator = false;
                self.output.push_str(self.newline);
            }
            Content::ObjectPlaceholder(index) => {
                if self.placeholder.is_some() {
                    self.flush_cell_separator();
                    self.placeholder.as_mut().expect("placeholder callback")(
                        index,
                        &mut self.output,
                    );
                }
            }
        }
    }
}

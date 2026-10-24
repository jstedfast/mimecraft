// Adapted from MimeKit's Text/RtfInterpreter.cs and Text/RtfToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};
use std::num::NonZeroUsize;

use super::RtfIssue;
use super::rtf_interpreter::{
    Content, ContentHandler, DEFAULT_LIMIT, GroupState, InterpreterOptions, interpret,
};

/// Extracts original, unfiltered HTML encapsulated in RTF.
///
/// This is separate from [`super::RtfToHtml`]'s RTF rendering. A recognized
/// `\fromhtml` prefix selects extraction; otherwise the result is `None`, not
/// rendered RTF. `Some(String::new())` represents recognized but empty HTML.
/// Recognition follows MimeKit: a bare or nonzero `\fromhtml` enables it, and
/// zero disables it, within the first ten group-start/control-word tokens.
/// Other token kinds or emitted content end recognition immediately.
///
/// HTMLTAG destinations use the document code page, while surrounding text
/// uses the current font's code page. RTF-only `\htmlrtf` content and ordinary
/// hidden text are suppressed. Like MimeKit, HTMLTAG contents are copied even
/// when hidden/HTMLRTF flags are set. `\par` and `\line` become CRLF; `\tab`
/// becomes a tab. Formatting and table boundaries add no markup.
///
/// Input must contain original RTF bytes, not re-encoded Unicode. Charset,
/// Unicode, group recovery, and diagnostic positions follow [`super::RtfToText`].
/// This is tolerant recognition, not proof of a valid RTF document.
///
/// **The output is untrusted HTML, not sanitized or tokenized.** Scripts, event
/// attributes, and arbitrary URLs are retained. No tag callbacks, wrappers,
/// decorations, attachment rendering, or HTML fragment conversion are applied.
/// Apply an appropriate separate sanitization policy before displaying it.
/// Interpreter state is bounded; the returned string is not.
///
/// ```
/// use mimecraft::text::RtfHtmlExtractor;
///
/// let extractor = RtfHtmlExtractor::new();
/// let rtf = br"{\rtf1\fromhtml1 {\*\htmltag <p>}Hello &amp; goodbye{\*\htmltag </p>}}";
/// assert_eq!(extractor.extract(&rtf[..])?.as_deref(), Some("<p>Hello &amp; goodbye</p>"));
/// assert_eq!(extractor.extract(&b"{\\rtf1 ordinary RTF}"[..])?, None);
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtfHtmlExtractor {
    /// Maximum distinct saved group states; defaults to 4096.
    ///
    /// Identical states fold. Content in excess groups is skipped.
    pub max_group_depth: NonZeroUsize,
    /// Maximum retained font numbers; defaults to 4096.
    ///
    /// Existing entries can still be updated after the table fills.
    pub max_font_table_entries: NonZeroUsize,
    /// Per-violation diagnostic quota; zero means unlimited.
    ///
    /// Uses the shared interpreter's combined lexical, structural, and
    /// decoding report, including its single first-suppression marker.
    pub max_issues_per_violation: usize,
}

impl Default for RtfHtmlExtractor {
    fn default() -> Self {
        Self {
            max_group_depth: DEFAULT_LIMIT,
            max_font_table_entries: DEFAULT_LIMIT,
            max_issues_per_violation: 0,
        }
    }
}

impl RtfHtmlExtractor {
    /// Creates an extractor with default interpreter limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extracts a complete buffered source, returning `None` for non-HTML RTF.
    ///
    /// The whole source is consumed even when HTML is not recognized. Each
    /// invocation starts with fresh recognition and interpreter state.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors without returning partial HTML or `None`.
    /// A failed extraction cannot resume; input may already have been consumed.
    pub fn extract<R: BufRead>(&self, reader: R) -> io::Result<Option<String>> {
        self.extract_core(reader, None::<fn(RtfIssue)>)
    }

    /// Extracts HTML with lexical, structural, and decoding diagnostics.
    ///
    /// Reports issues even when the result is `None`. Color and hyperlink-field
    /// collection are disabled: they are not needed for raw HTML extraction.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors; diagnostic callback side effects are not
    /// rolled back and partial HTML is not returned.
    pub fn extract_with_issues<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
    ) -> io::Result<Option<String>> {
        self.extract_core(reader, Some(report))
    }

    fn extract_core<R: BufRead, F: FnMut(RtfIssue)>(
        &self,
        reader: R,
        report: Option<F>,
    ) -> io::Result<Option<String>> {
        let options = InterpreterOptions {
            max_group_depth: self.max_group_depth,
            max_font_table_entries: self.max_font_table_entries,
            max_color_table_entries: None,
            collect_hyperlinks: false,
            extract_html: true,
            max_issues_per_violation: self.max_issues_per_violation,
        };
        Ok(interpret(reader, options, report, HtmlCollector { output: None })?.output)
    }
}

struct HtmlCollector {
    output: Option<String>,
}

impl ContentHandler for HtmlCollector {
    fn begin(&mut self, from_html: bool) {
        if from_html {
            self.output = Some(String::new());
        }
    }

    fn content(&mut self, _: &GroupState, _: &[Option<u32>], content: Content<'_>) {
        let Some(output) = &mut self.output else {
            return;
        };
        match content {
            // Both HTMLTAG destinations and unsuppressed surrounding text are
            // already HTML. Encoding either again would corrupt the original.
            Content::Text(text) => output.push_str(text),
            Content::Paragraph | Content::LineBreak => output.push_str("\r\n"),
            // Extraction does not synthesize table markup or attachments.
            Content::Cell | Content::Row | Content::ObjectPlaceholder(_) => {}
        }
    }
}

// Adapted from MimeKit's Text/RtfToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::collections::HashMap;
use std::fmt::Write;
use std::io::{self, BufRead};
use std::num::NonZeroUsize;

use super::rtf_interpreter::{
    Alignment, BOLD, Content, ContentHandler, DEFAULT_LIMIT, GroupState, Hyperlink, IN_TABLE,
    ITALIC, InterpreterOptions, STRIKE, UNDERLINE, VerticalAlignment, interpret,
};
use super::text_to_html::write_decoration;
use super::{HeaderFooterFormat, HtmlWriter, RtfIssue, output_newline};
use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// Renders original RTF bytes as an HTML document or body fragment.
///
/// Supports character formatting, paragraph alignment, line breaks, tables,
/// font code pages, Unicode, colors, and scheme-filtered hyperlink fields.
/// Hidden text and non-text destinations are suppressed. Text and attribute
/// values are escaped by [`HtmlWriter`]; significant spaces are preserved with
/// alternating nonbreaking spaces, and tabs become four nonbreaking spaces.
///
/// This is the RTF-rendering path, not an encapsulated-HTML extractor:
/// `\fromhtml` does not switch modes, `\htmltag` groups are skipped, and
/// `\htmlrtf` content is rendered. HTML tag callbacks and extraction/filtering
/// of encapsulated HTML are not implemented.
///
/// Input must contain original bytes, not re-encoded Unicode RTF. Code-page
/// decoding, resource recovery, and source-error behavior match [`super::RtfToText`].
/// Generated block separators default to CRLF on every platform.
///
/// Retained interpreter state, field instructions, and CSS caching are bounded;
/// the output string and caller-owned callback data are not. This is not an HTML
/// sanitizer. HTML-format decorations and placeholder markup are trusted caller
/// content. Allowed hyperlink schemes do not establish a target's trustworthiness.
///
/// ```
/// use mimecraft::text::RtfToHtml;
///
/// let mut converter = RtfToHtml::new();
/// converter.output_html_fragment = true;
/// let html = converter.convert(&b"{\\rtf1 Hello {\\b world}!}"[..])?;
/// assert_eq!(html,
///     "<div>Hello <span style=\"font-weight: bold;\">world</span>!</div>\r\n");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtfToHtml {
    /// Omits the outer HTML/body tags when true; defaults to false.
    pub output_html_fragment: bool,
    /// Optional content before the rendered body.
    pub header: Option<String>,
    /// How to interpret the header; defaults to plain text.
    pub header_format: HeaderFooterFormat,
    /// Optional content after the rendered body.
    pub footer: Option<String>,
    /// How to interpret the footer; defaults to plain text.
    pub footer_format: HeaderFooterFormat,
    /// Maximum distinct saved group states; defaults to 4096.
    ///
    /// Identical states fold. Hyperlink identity is group-scoped and participates
    /// in folding using constant-time reference comparison. Excess groups are skipped.
    pub max_group_depth: NonZeroUsize,
    /// Maximum font numbers retained; defaults to 4096.
    ///
    /// Existing entries can still be updated after the table fills.
    pub max_font_table_entries: NonZeroUsize,
    /// Maximum color entries retained, including automatic entries; defaults to 4096.
    ///
    /// Excess entries are dropped and references to them use the default color.
    pub max_color_table_entries: NonZeroUsize,
    /// Per-violation diagnostic quota; zero means unlimited.
    ///
    /// Nineteen violation kinds share one first-suppression marker. At most
    /// nineteen times a positive limit plus one issues are delivered per call.
    pub max_issues_per_violation: usize,
    new_line_format: NewLineFormat,
}

impl Default for RtfToHtml {
    fn default() -> Self {
        Self {
            output_html_fragment: false,
            header: None,
            header_format: HeaderFooterFormat::Text,
            footer: None,
            footer_format: HeaderFooterFormat::Text,
            max_group_depth: DEFAULT_LIMIT,
            max_font_table_entries: DEFAULT_LIMIT,
            max_color_table_entries: DEFAULT_LIMIT,
            max_issues_per_violation: 0,
            new_line_format: NewLineFormat::Dos,
        }
    }
}

impl RtfToHtml {
    /// Creates a document renderer with CRLF separators and default limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a renderer with explicit block-separator line endings.
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

    /// Returns the format of generated block separators.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Converts buffered original bytes without diagnostic or placeholder callbacks.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors without returning partial HTML. A failed
    /// conversion cannot resume; the source may already have been consumed.
    pub fn convert<R: BufRead>(&self, reader: R) -> io::Result<String> {
        self.convert_core(
            reader,
            None::<fn(RtfIssue)>,
            None::<fn(usize, &mut HtmlWriter)>,
        )
    }

    /// Converts bytes with lexical, structural, and interpretation diagnostics.
    ///
    /// Checks and positions follow [`super::RtfToText::convert_with_issues`].
    /// Color-table overflow, undefined colors, and field-instruction truncation
    /// are additionally reported. Field instructions retain at most 2048 UTF-8
    /// bytes, ending at a scalar boundary; later content is discarded.
    /// The retained prefix is still parsed, including unterminated quoted
    /// targets, following [`super::parse_rtf_hyperlink`]'s recovery rules.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors; already-delivered callbacks are not undone.
    pub fn convert_with_issues<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
    ) -> io::Result<String> {
        self.convert_core(reader, Some(report), None::<fn(usize, &mut HtmlWriter)>)
    }

    /// Converts bytes with diagnostics and attachment-placeholder rendering.
    ///
    /// The placeholder callback receives a zero-based, saturating index and the
    /// HTML writer in the current paragraph/inline context. Only visible
    /// placeholders count. Without a callback they produce no markup.
    /// The immediately following literal or hex-escaped space is consumed.
    /// Callback markup is trusted; no tag filtering is applied to it.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors. Callback side effects are not rolled back.
    pub fn convert_with_callbacks<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
        placeholder: impl FnMut(usize, &mut HtmlWriter),
    ) -> io::Result<String> {
        self.convert_core(reader, Some(report), Some(placeholder))
    }

    fn convert_core<R: BufRead, F: FnMut(RtfIssue), G: FnMut(usize, &mut HtmlWriter)>(
        &self,
        reader: R,
        report: Option<F>,
        placeholder: Option<G>,
    ) -> io::Result<String> {
        let options = InterpreterOptions {
            max_group_depth: self.max_group_depth,
            max_font_table_entries: self.max_font_table_entries,
            max_color_table_entries: Some(self.max_color_table_entries),
            collect_hyperlinks: true,
            max_issues_per_violation: self.max_issues_per_violation,
        };
        let handler = HtmlHandler::new(self, placeholder);
        Ok(interpret(reader, options, report, handler)?.finish(self))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SpanFormat {
    flags: u16,
    vertical: VerticalAlignment,
    font_size: i32,
    foreground: Option<u32>,
    background: Option<u32>,
}

impl SpanFormat {
    fn new(state: &GroupState, colors: &[Option<u32>]) -> Self {
        // Index 0 means "auto" even if a writer explicitly defines it. Missing
        // entries also use the default rather than synthesizing a CSS color.
        let color = |index: i32| {
            if index > 0 {
                colors.get(index as usize).copied().flatten()
            } else {
                None
            }
        };
        Self {
            flags: state.flags & (BOLD | ITALIC | UNDERLINE | STRIKE),
            vertical: state.vertical_alignment,
            // A malformed \fs0 must not make the text invisible.
            font_size: if state.font_size > 0 {
                state.font_size
            } else {
                24
            },
            foreground: color(state.foreground),
            background: color(state.background),
        }
    }

    fn is_default(self) -> bool {
        self.flags == 0
            && self.vertical == VerticalAlignment::Baseline
            && self.font_size == 24
            && self.foreground.is_none()
            && self.background.is_none()
    }

    fn css(self) -> String {
        let mut css = String::new();
        if self.flags & BOLD != 0 {
            css.push_str("font-weight: bold; ")
        }
        if self.flags & ITALIC != 0 {
            css.push_str("font-style: italic; ")
        }
        if self.flags & (UNDERLINE | STRIKE) != 0 {
            css.push_str("text-decoration:");
            if self.flags & UNDERLINE != 0 {
                css.push_str(" underline")
            }
            if self.flags & STRIKE != 0 {
                css.push_str(" line-through")
            }
            css.push_str("; ");
        }
        match self.vertical {
            VerticalAlignment::Superscript => css.push_str("vertical-align: super; "),
            VerticalAlignment::Subscript => css.push_str("vertical-align: sub; "),
            VerticalAlignment::Baseline => {}
        }
        if self.font_size != 24 {
            // RTF font sizes are half-points. Use an integer plus optional .5
            // rather than locale-dependent floating-point formatting.
            write!(
                css,
                "font-size: {}{}pt; ",
                self.font_size / 2,
                if self.font_size & 1 != 0 { ".5" } else { "" }
            )
            .expect("string formatting");
        }
        if let Some(rgb) = self.foreground {
            write!(css, "color: #{rgb:06X}; ").expect("string formatting");
        }
        if let Some(rgb) = self.background {
            write!(css, "background-color: #{rgb:06X}; ").expect("string formatting");
        }
        // Remove only the trailing space, retaining the final semicolon.
        if css.ends_with(' ') {
            css.pop();
        }
        css
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tag {
    Table,
    Row,
    Cell,
    Block,
    Link,
    Span,
}

impl Tag {
    fn name(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Row => "tr",
            Self::Cell => "td",
            Self::Block => "div",
            Self::Link => "a",
            Self::Span => "span",
        }
    }
}

const MAX_CSS_CACHE_ENTRIES: usize = 256;

struct HtmlHandler<G> {
    writer: HtmlWriter,
    stack: Vec<Tag>,
    css_cache: HashMap<SpanFormat, String>,
    span_format: Option<SpanFormat>,
    href: Option<Hyperlink>,
    last_was_space: bool,
    newline: &'static str,
    placeholder: Option<G>,
}

impl<G: FnMut(usize, &mut HtmlWriter)> HtmlHandler<G> {
    fn new(options: &RtfToHtml, placeholder: Option<G>) -> Self {
        let mut prefix = String::new();
        if !options.output_html_fragment {
            prefix.push_str("<html><body>")
        }
        write_decoration(
            &mut prefix,
            options.header.as_deref(),
            options.header_format,
        );
        let mut writer = HtmlWriter::new();
        writer.write_markup_text(&prefix);
        Self {
            writer,
            stack: Vec::with_capacity(6),
            css_cache: HashMap::new(),
            span_format: None,
            href: None,
            last_was_space: false,
            newline: output_newline(options.new_line_format),
            placeholder,
        }
    }

    fn index(&self, tag: Tag) -> Option<usize> {
        self.stack.iter().rposition(|&open| open == tag)
    }

    fn open(&mut self, tag: Tag) {
        // There is at most one of each generated element, independent of RTF
        // nesting. Caller-written placeholder markup does not enter this stack.
        debug_assert!(self.index(tag).is_none() && self.stack.len() < 6);
        self.writer
            .write_start_tag(tag.name())
            .expect("known RTF HTML tag");
        self.stack.push(tag);
    }

    fn close_to(&mut self, count: usize) {
        while self.stack.len() > count {
            let tag = self.stack.pop().expect("open element");
            if tag == Tag::Link {
                self.href = None
            }
            self.writer
                .write_end_tag(tag.name())
                .expect("known RTF HTML tag");
            if matches!(tag, Tag::Block | Tag::Row | Tag::Table) {
                self.writer.write_markup_text(self.newline);
            }
        }
    }

    fn close(&mut self, tag: Tag) {
        if let Some(index) = self.index(tag) {
            self.close_to(index)
        }
    }

    fn open_table(&mut self) {
        for tag in [Tag::Table, Tag::Row, Tag::Cell] {
            if self.index(tag).is_none() {
                self.open(tag)
            }
        }
    }

    fn open_block(&mut self, state: &GroupState) {
        // \intbl marks table paragraphs; \pard clears it, so subsequent
        // ordinary content closes the preceding table.
        if state.flags & IN_TABLE != 0 {
            self.open_table()
        } else {
            self.close(Tag::Table)
        }
        if self.index(Tag::Block).is_some() {
            return;
        }
        self.open(Tag::Block);
        // Alignment is a paragraph property: the first content fixes it.
        let alignment = match state.alignment {
            Alignment::Left => None,
            Alignment::Center => Some("text-align: center;"),
            Alignment::Right => Some("text-align: right;"),
            Alignment::Justify => Some("text-align: justify;"),
        };
        if let Some(css) = alignment {
            self.writer
                .write_attribute("style", css)
                .expect("attribute inside start tag");
        }
        self.last_was_space = true;
    }

    fn open_inline(&mut self, state: &GroupState, colors: &[Option<u32>]) {
        let format = SpanFormat::new(state, colors);
        // Close the old link before opening the new one, including for fields
        // nested in a result: HTML anchors must never nest. Spans likewise
        // replace rather than nest as character formatting changes.
        if let Some(index) = self.index(Tag::Link)
            && self.href.as_ref().map(Hyperlink::as_str)
                != state.hyperlink.as_ref().map(Hyperlink::as_str)
        {
            self.close_to(index);
        }
        if let Some(index) = self.index(Tag::Span)
            && self.span_format != Some(format)
        {
            self.close_to(index);
        }
        if let Some(link) = &state.hyperlink
            && self.index(Tag::Link).is_none()
        {
            self.open(Tag::Link);
            self.writer
                .write_attribute("href", link.as_str())
                .expect("attribute inside start tag");
            self.href = Some(link.clone());
        }
        if !format.is_default() && self.index(Tag::Span).is_none() {
            self.open(Tag::Span);
            // Typical documents reuse formats. Bound the cache and clear it
            // when full so distinct malicious styles cannot accumulate forever.
            if !self.css_cache.contains_key(&format) {
                if self.css_cache.len() == MAX_CSS_CACHE_ENTRIES {
                    self.css_cache.clear()
                }
                self.css_cache.insert(format, format.css());
            }
            self.writer
                .write_attribute("style", &self.css_cache[&format])
                .expect("attribute inside start tag");
            self.span_format = Some(format);
        }
    }

    fn write_text(&mut self, text: &str) {
        // HTML collapses spaces; RTF does not. Alternate ordinary and
        // nonbreaking spaces, and approximate a tab by four nonbreaking spaces.
        let mut start = 0;
        for (index, c) in text.char_indices() {
            match c {
                ' ' if self.last_was_space => {
                    self.writer.write_text(&text[start..index]);
                    self.writer.write_text("\u{a0}");
                    start = index + 1;
                    self.last_was_space = false;
                }
                ' ' => self.last_was_space = true,
                '\t' => {
                    self.writer.write_text(&text[start..index]);
                    self.writer.write_text("\u{a0}\u{a0}\u{a0}\u{a0}");
                    start = index + 1;
                    self.last_was_space = false;
                }
                _ => self.last_was_space = false,
            }
        }
        self.writer.write_text(&text[start..]);
    }

    fn line_break(&mut self) {
        self.writer
            .write_empty_element_tag("br")
            .expect("known RTF HTML tag");
        self.last_was_space = true;
    }

    fn finish(mut self, options: &RtfToHtml) -> String {
        self.close_to(0);
        let mut suffix = String::new();
        write_decoration(
            &mut suffix,
            options.footer.as_deref(),
            options.footer_format,
        );
        if !options.output_html_fragment {
            suffix.push_str("</body></html>")
        }
        self.writer.write_markup_text(&suffix);
        self.writer.finish()
    }
}

impl<G: FnMut(usize, &mut HtmlWriter)> ContentHandler for HtmlHandler<G> {
    fn content(&mut self, state: &GroupState, colors: &[Option<u32>], content: Content<'_>) {
        match content {
            Content::Text(text) => {
                self.open_block(state);
                self.open_inline(state, colors);
                self.write_text(text);
            }
            Content::Paragraph => {
                // Browsers collapse empty divs. Give empty RTF paragraphs a
                // line break so they retain their vertical space.
                let empty = self.index(Tag::Block).is_none();
                self.open_block(state);
                if empty {
                    self.line_break()
                }
                self.close(Tag::Block);
            }
            Content::LineBreak => {
                self.open_block(state);
                self.open_inline(state, colors);
                self.line_break();
            }
            Content::Cell => {
                // \cell terminates even an empty cell.
                self.open_table();
                self.close(Tag::Cell);
            }
            Content::Row => {
                for tag in [Tag::Table, Tag::Row] {
                    if self.index(tag).is_none() {
                        self.open(tag)
                    }
                }
                self.close(Tag::Row);
            }
            Content::ObjectPlaceholder(index) => {
                if self.placeholder.is_some() {
                    self.open_block(state);
                    self.open_inline(state, colors);
                    self.placeholder.as_mut().expect("placeholder callback")(
                        index,
                        &mut self.writer,
                    );
                    self.last_was_space = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_nesting_and_css_cache_remain_bounded() {
        let options = RtfToHtml::new();
        let mut handler = HtmlHandler::new(&options, None::<fn(usize, &mut HtmlWriter)>);
        let mut state = GroupState::default();
        for size in 26..1026 {
            state.font_size = size;
            state.flags = if size % 3 == 0 {
                IN_TABLE | BOLD
            } else {
                ITALIC
            };
            handler.content(&state, &[], Content::Text("x"));
            for content in [
                Content::LineBreak,
                Content::Cell,
                Content::Row,
                Content::Paragraph,
            ] {
                handler.content(&state, &[], content);
                assert!(handler.stack.len() <= 6);
                assert!(handler.stack.capacity() <= 6);
                assert!(handler.css_cache.len() <= MAX_CSS_CACHE_ENTRIES);
            }
        }
        let output = handler.finish(&options);
        assert!(output.ends_with("</body></html>"));
    }
}

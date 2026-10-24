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
use super::{
    HeaderFooterFormat, HtmlAttribute, HtmlAttributeId, HtmlTagContext, HtmlTagId, HtmlWriter,
    RtfIssue, output_newline,
};
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
/// `\htmlrtf` content is rendered. Use [`super::RtfHtmlExtractor`] separately
/// for raw, unfiltered extraction. Optional tag callbacks customize generated
/// body tags, not encapsulated markup.
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
            None::<fn(&mut HtmlTagContext, &mut HtmlWriter)>,
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
        self.convert_core(
            reader,
            Some(report),
            None::<fn(usize, &mut HtmlWriter)>,
            None::<fn(&mut HtmlTagContext, &mut HtmlWriter)>,
        )
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
        self.convert_core(
            reader,
            Some(report),
            Some(placeholder),
            None::<fn(&mut HtmlTagContext, &mut HtmlWriter)>,
        )
    }

    /// Converts bytes with a callback that writes, replaces, or omits body tags.
    ///
    /// Invoked for generated `div`, `span`, `a`, `br`, `table`, `tr`, and `td`
    /// tags. Call [`HtmlTagContext::write_tag`] to preserve a tag and its
    /// attributes. End tags are written automatically unless the context
    /// requests an end callback or deletes the end tag. Suppressed descendants
    /// produce neither markup nor callbacks.
    ///
    /// Document wrappers, headers, and footers bypass the callback. Custom
    /// markup is trusted and is not recursively passed through the callback.
    /// Diagnostic and placeholder callbacks are absent in this overload.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors; callback side effects are not rolled back.
    pub fn convert_with_tag_callback<R: BufRead>(
        &self,
        reader: R,
        tag_callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
    ) -> io::Result<String> {
        self.convert_core(
            reader,
            None::<fn(RtfIssue)>,
            None::<fn(usize, &mut HtmlWriter)>,
            Some(tag_callback),
        )
    }

    /// Converts bytes with diagnostics, tag customization, and placeholders.
    ///
    /// Tag callbacks follow [`Self::convert_with_tag_callback`]. Placeholder
    /// markup bypasses tag callbacks, and placeholders inside suppressed
    /// content skip the placeholder callback without skipping their indices.
    /// Diagnostic reporting is unaffected by tag deletion or suppression.
    ///
    /// # Errors
    ///
    /// Propagates source I/O errors; callback side effects are not rolled back.
    pub fn convert_with_all_callbacks<R: BufRead>(
        &self,
        reader: R,
        report: impl FnMut(RtfIssue),
        tag_callback: impl FnMut(&mut HtmlTagContext, &mut HtmlWriter),
        placeholder: impl FnMut(usize, &mut HtmlWriter),
    ) -> io::Result<String> {
        self.convert_core(reader, Some(report), Some(placeholder), Some(tag_callback))
    }

    fn convert_core<
        R: BufRead,
        F: FnMut(RtfIssue),
        G: FnMut(usize, &mut HtmlWriter),
        T: FnMut(&mut HtmlTagContext, &mut HtmlWriter),
    >(
        &self,
        reader: R,
        report: Option<F>,
        placeholder: Option<G>,
        tag_callback: Option<T>,
    ) -> io::Result<String> {
        let options = InterpreterOptions {
            max_group_depth: self.max_group_depth,
            max_font_table_entries: self.max_font_table_entries,
            max_color_table_entries: Some(self.max_color_table_entries),
            collect_hyperlinks: true,
            extract_html: false,
            max_issues_per_violation: self.max_issues_per_violation,
        };
        let handler = HtmlHandler::new(self, placeholder, tag_callback);
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
    fn id(self) -> HtmlTagId {
        match self {
            Self::Table => HtmlTagId::Table,
            Self::Row => HtmlTagId::Tr,
            Self::Cell => HtmlTagId::Td,
            Self::Block => HtmlTagId::Div,
            Self::Link => HtmlTagId::A,
            Self::Span => HtmlTagId::Span,
        }
    }
}

const MAX_CSS_CACHE_ENTRIES: usize = 256;

struct HtmlHandler<G, T> {
    writer: HtmlWriter,
    stack: Vec<HtmlTagContext>,
    css_cache: HashMap<SpanFormat, String>,
    span_format: Option<SpanFormat>,
    href: Option<Hyperlink>,
    last_was_space: bool,
    newline: &'static str,
    placeholder: Option<G>,
    tag_callback: Option<T>,
}

impl<G: FnMut(usize, &mut HtmlWriter), T: FnMut(&mut HtmlTagContext, &mut HtmlWriter)>
    HtmlHandler<G, T>
{
    fn new(options: &RtfToHtml, placeholder: Option<G>, tag_callback: Option<T>) -> Self {
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
            tag_callback,
        }
    }

    fn index(&self, tag: Tag) -> Option<usize> {
        self.stack
            .iter()
            .rposition(|open| open.tag_id() == tag.id())
    }

    fn suppress_content(&self) -> bool {
        // Generated nesting has at most six elements, so this walk stays
        // bounded even when the source RTF has arbitrarily deep groups.
        self.stack
            .iter()
            .any(|context| context.suppress_inner_content)
    }

    fn write_tag(&mut self, context: &mut HtmlTagContext) {
        if let Some(callback) = &mut self.tag_callback {
            callback(context, &mut self.writer);
        } else {
            context
                .write_tag(&mut self.writer, true)
                .expect("generated HTML tag");
        }
    }

    fn open(&mut self, tag: Tag, attribute: Option<HtmlAttribute>) {
        // There is at most one of each generated element, independent of RTF
        // nesting. Caller-written tag/placeholder markup does not enter this stack.
        debug_assert!(self.index(tag).is_none() && self.stack.len() < 6);
        let mut context = HtmlTagContext::new(tag.id(), attribute.into_iter().collect());
        if !self.suppress_content() {
            self.write_tag(&mut context);
        }
        // Keep even suppressed or unwritten elements so formatting transitions
        // still close the same contexts and resume output at the right boundary.
        self.stack.push(context);
    }

    fn close_to(&mut self, count: usize) {
        while self.stack.len() > count {
            let mut context = self.stack.pop().expect("open element");
            let id = context.tag_id();
            if id == HtmlTagId::A {
                self.href = None
            }
            // Check suppression after popping: an element can suppress its
            // children without suppressing its own closing tag or callback.
            if self.suppress_content() || context.delete_end_tag {
                continue;
            }
            context.mark_end_tag();
            if context.invoke_callback_for_end_tag {
                self.write_tag(&mut context);
            } else {
                context
                    .write_tag(&mut self.writer, false)
                    .expect("generated end tag");
            }
            if matches!(id, HtmlTagId::Div | HtmlTagId::Tr | HtmlTagId::Table) {
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
                self.open(tag, None)
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
        // Alignment is a paragraph property: the first content fixes it.
        let alignment = match state.alignment {
            Alignment::Left => None,
            Alignment::Center => Some("text-align: center;"),
            Alignment::Right => Some("text-align: right;"),
            Alignment::Justify => Some("text-align: justify;"),
        };
        let attribute = alignment.map(|css| {
            HtmlAttribute::from_id(HtmlAttributeId::Style, Some(css.into()))
                .expect("style attribute")
        });
        self.open(Tag::Block, attribute);
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
            let attribute =
                HtmlAttribute::from_id(HtmlAttributeId::Href, Some(link.as_str().into()))
                    .expect("href attribute");
            self.open(Tag::Link, Some(attribute));
            self.href = Some(link.clone());
        }
        if !format.is_default() && self.index(Tag::Span).is_none() {
            // Typical documents reuse formats. Bound the cache and clear it
            // when full so distinct malicious styles cannot accumulate forever.
            if !self.css_cache.contains_key(&format) {
                if self.css_cache.len() == MAX_CSS_CACHE_ENTRIES {
                    self.css_cache.clear()
                }
                self.css_cache.insert(format, format.css());
            }
            let attribute = HtmlAttribute::from_id(
                HtmlAttributeId::Style,
                Some(self.css_cache[&format].clone()),
            )
            .expect("style attribute");
            self.open(Tag::Span, Some(attribute));
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
        if !self.suppress_content() {
            let mut context = HtmlTagContext::new(HtmlTagId::Br, Default::default());
            self.write_tag(&mut context);
        }
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

impl<G: FnMut(usize, &mut HtmlWriter), T: FnMut(&mut HtmlTagContext, &mut HtmlWriter)>
    ContentHandler for HtmlHandler<G, T>
{
    fn content(&mut self, state: &GroupState, colors: &[Option<u32>], content: Content<'_>) {
        match content {
            Content::Text(text) => {
                self.open_block(state);
                self.open_inline(state, colors);
                if !self.suppress_content() {
                    self.write_text(text);
                }
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
                        self.open(tag, None)
                    }
                }
                self.close(Tag::Row);
            }
            Content::ObjectPlaceholder(index) => {
                if self.placeholder.is_some() {
                    self.open_block(state);
                    self.open_inline(state, colors);
                    // The interpreter advances indices even when suppression
                    // skips the callback, keeping attachment numbering aligned.
                    if !self.suppress_content() {
                        self.placeholder.as_mut().expect("placeholder callback")(
                            index,
                            &mut self.writer,
                        );
                    }
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
        for suppress in [false, true] {
            let callback = |context: &mut HtmlTagContext, writer: &mut HtmlWriter| {
                if suppress {
                    context.suppress_inner_content = true;
                    context.delete_end_tag = true;
                } else {
                    context.write_tag(writer, true).unwrap();
                }
            };
            let mut handler =
                HtmlHandler::new(&options, None::<fn(usize, &mut HtmlWriter)>, Some(callback));
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
                    for (index, context) in handler.stack.iter().enumerate() {
                        assert!(context.attributes().len() <= 1);
                        assert!(
                            handler.stack[..index]
                                .iter()
                                .all(|other| other.tag_id() != context.tag_id())
                        );
                    }
                }
            }
            let output = handler.finish(&options);
            if suppress {
                assert_eq!(output, "<html><body></body></html>");
            } else {
                assert!(output.ends_with("</body></html>"));
            }
        }
    }
}

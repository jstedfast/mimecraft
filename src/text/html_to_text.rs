// Adapted from MimeKit's Text/HtmlToText.cs and Text/HtmlTagContextStack.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    io::{self, BufRead, Read, Write},
    num::NonZeroUsize,
};

use super::{
    HtmlTagDisposition, HtmlTagId, HtmlTagToken, HtmlToken, HtmlTokenizerOptions,
    HtmlTokenizerReader, output_newline,
};
use crate::encodings::{EncoderOptionsError, NewLineFormat};

/// A resource limit exceeded while converting HTML to plain text.
///
/// Returned inside an [`io::Error`] with kind [`io::ErrorKind::InvalidData`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlToTextError {
    /// Original input exceeds the UTF-8 byte limit, including suppressed markup.
    InputLength,
    /// Generated body text exceeds the UTF-8 byte limit.
    OutputLength,
    /// Converter or tokenizer element nesting exceeds the configured limit.
    ElementDepth,
}

impl fmt::Display for HtmlToTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputLength => "HTML input length limit exceeded",
            Self::OutputLength => "HTML output length limit exceeded",
            Self::ElementDepth => "HTML element depth limit exceeded",
        })
    }
}

impl Error for HtmlToTextError {}

impl From<HtmlToTextError> for io::Error {
    fn from(error: HtmlToTextError) -> Self {
        Self::new(io::ErrorKind::InvalidData, error)
    }
}

/// Converts HTML documents and fragments to plain text, not Markdown.
///
/// Preserves paragraphs, explicit line breaks, hash-prefixed headings, lists,
/// blockquotes and preformatted text. Tables are flattened without alignment.
/// Scripts, styles, titles, templates, frames and hidden content are omitted.
/// CSS is not evaluated, noscript fallback is included, and no resources are
/// fetched. Link destinations and image alternative text are optional.
///
/// Output is not word-wrapped. Structural trailing spacing is omitted, while
/// explicit breaks and preformatted whitespace are retained. Generated line
/// endings default to CRLF on every platform. Headers and footers are literal
/// text, without inserted separators, and excluded from both length limits.
///
/// Input must be decoded text or strict UTF-8 bytes; there is no charset/BOM
/// detection. Limits count UTF-8 bytes, not MimeKit's UTF-16 code units.
/// Each conversion starts fresh. This does not produce sanitized HTML: escape
/// the result before embedding it in markup.
///
/// ```
/// use mimecraft::text::HtmlToText;
///
/// let converter = HtmlToText::new();
/// assert_eq!(converter.convert("<h1>Title</h1><p>Hello &amp; goodbye.</p>")?,
///     "# Title\r\n\r\nHello & goodbye.");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlToText {
    /// Append link destinations unless they already equal their text; default true.
    pub output_link_urls: bool,
    /// Include image alternative text; defaults to true.
    pub output_image_alt_text: bool,
    /// Text prepended verbatim, outside the body output limit.
    pub header: Option<String>,
    /// Text appended verbatim, outside the body output limit.
    pub footer: Option<String>,
    /// Maximum open elements, counting repeated elements separately; default 4096.
    pub max_element_depth: NonZeroUsize,
    /// Maximum original UTF-8 bytes, including incomplete tokens; default 16 MiB.
    pub max_input_length: NonZeroUsize,
    /// Maximum generated body bytes, including indentation and URLs; default 64 MiB.
    pub max_output_length: NonZeroUsize,
    new_line_format: NewLineFormat,
}

impl Default for HtmlToText {
    fn default() -> Self {
        Self {
            output_link_urls: true,
            output_image_alt_text: true,
            header: None,
            footer: None,
            max_element_depth: NonZeroUsize::new(4096).unwrap(),
            max_input_length: NonZeroUsize::new(16 * 1024 * 1024).unwrap(),
            max_output_length: NonZeroUsize::new(64 * 1024 * 1024).unwrap(),
            new_line_format: NewLineFormat::Dos,
        }
    }
}

impl HtmlToText {
    /// Creates a converter with CRLF output and default resource limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a converter with explicit generated line endings.
    ///
    /// # Errors
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

    /// Returns the generated line-ending format.
    pub fn new_line_format(&self) -> NewLineFormat {
        self.new_line_format
    }

    /// Converts complete Unicode text without modifying the converter.
    ///
    /// # Errors
    /// Returns [`HtmlToTextError`] inside an [`io::Error`] when a limit is exceeded.
    /// No partial output is returned.
    pub fn convert(&self, html: &str) -> io::Result<String> {
        self.convert_reader(html.as_bytes())
    }

    /// Converts a buffered UTF-8 source to an owned string.
    ///
    /// # Errors
    /// Propagates source errors, invalid UTF-8 and conversion limits. Partial
    /// output is discarded; consumed source bytes cannot be recovered.
    pub fn convert_reader<R: BufRead>(&self, reader: R) -> io::Result<String> {
        let mut output = Vec::new();
        self.convert_to(reader, &mut output)?;
        Ok(String::from_utf8(output).expect("converter writes only UTF-8"))
    }

    /// Converts a buffered UTF-8 source to a UTF-8 writer.
    ///
    /// Does not flush the writer. Pass borrowed readers/writers to retain them.
    /// The tokenizer reads bounded chunks rather than buffering the document.
    ///
    /// # Errors
    /// Propagates source and destination errors, invalid UTF-8 and conversion
    /// limits. `Interrupted` I/O is retried. The writer may contain partial
    /// output and the source may have read ahead. A failed conversion cannot
    /// resume; the footer is only written after successful body conversion.
    pub fn convert_to<R: BufRead, W: Write>(&self, reader: R, writer: &mut W) -> io::Result<()> {
        if let Some(header) = &self.header {
            writer.write_all(header.as_bytes())?;
        }
        let limited = LimitedReader {
            source: reader,
            remaining: self.max_input_length.get(),
        };
        Converter::new(self, writer).convert(limited)?;
        if let Some(footer) = &self.footer {
            writer.write_all(footer.as_bytes())?;
        }
        Ok(())
    }
}

struct LimitedReader<R> {
    source: R,
    remaining: usize,
}

impl<R: BufRead> Read for LimitedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let source = self.fill_buf()?;
        let length = source.len().min(buffer.len());
        buffer[..length].copy_from_slice(&source[..length]);
        self.consume(length);
        Ok(length)
    }
}

impl<R: BufRead> BufRead for LimitedReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let buffer = self.source.fill_buf()?;
        if self.remaining == 0 && !buffer.is_empty() {
            return Err(HtmlToTextError::InputLength.into());
        }
        Ok(&buffer[..buffer.len().min(self.remaining)])
    }

    fn consume(&mut self, amount: usize) {
        self.source.consume(amount);
        self.remaining -= amount;
    }
}

#[derive(Clone, Copy, Default)]
struct Formatting {
    indent: usize,
    quote_depth: usize,
    pre_depth: usize,
    list: Option<usize>,
    item: Option<usize>,
    link: Option<usize>,
    heading: Option<usize>,
}

struct Element {
    id: HtmlTagId,
    name: String,
    previous_index: Option<usize>,
    previous: Formatting,
    visible: bool,
    url: Option<String>,
    link_matches: bool,
    link_length: usize,
    next_number: i64,
}

struct Converter<'a, W> {
    options: &'a HtmlToText,
    writer: &'a mut W,
    elements: Vec<Element>,
    // Maps an ASCII-folded tag name to its innermost open element. Each element
    // links to the next outer element of the same name via previous_index.
    // This avoids O(depth) scans for unmatched end tags in untrusted input.
    contexts: HashMap<String, usize>,
    state: Formatting,
    pending_breaks: usize,
    newlines: usize,
    output_length: usize,
    line_start: bool,
    has_output: bool,
    pending_space: bool,
    skip_pre_newline: bool,
    previous_cr: bool,
    marker: Option<String>,
}

fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c')
}

fn heading_level(id: HtmlTagId) -> Option<usize> {
    use HtmlTagId::*;
    match id {
        H1 => Some(1),
        H2 => Some(2),
        H3 => Some(3),
        H4 => Some(4),
        H5 => Some(5),
        H6 => Some(6),
        _ => None,
    }
}

fn is_list(id: HtmlTagId) -> bool {
    matches!(
        id,
        HtmlTagId::Ol | HtmlTagId::Ul | HtmlTagId::Menu | HtmlTagId::Dir
    )
}

fn spacing(id: HtmlTagId) -> usize {
    use HtmlTagId::*;
    match id {
        P | Pre | Listing | Xmp | PlainText | BlockQuote | Table | Dl | H1 | H2 | H3 | H4 | H5
        | H6 => 2,
        Address | Article | Aside | Center | Details | Dialog | Div | FieldSet | FigCaption
        | Figure | Footer | Form | Header | Main | Nav | Section | Summary | Tr | Caption | Dd
        | Dt | TextArea | Li | Ol | Ul | Menu | Dir => 1,
        _ => 0,
    }
}

fn suppresses_content(tag: &HtmlTagToken) -> bool {
    matches!(
        tag.id(),
        HtmlTagId::Script
            | HtmlTagId::Style
            | HtmlTagId::Title
            | HtmlTagId::IFrame
            | HtmlTagId::NoFrames
    ) || tag.name().eq_ignore_ascii_case("template")
        || tag.attributes().contains("hidden")
}

fn get_number(tag: &HtmlTagToken, name: &str, default: i64) -> i64 {
    tag.attributes()
        .get(name)
        .and_then(|attribute| attribute.value())
        .and_then(|value| {
            value
                .trim_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
                .parse::<i32>()
                .ok()
        })
        .map_or(default, i64::from)
}

impl<'a, W: Write> Converter<'a, W> {
    fn new(options: &'a HtmlToText, writer: &'a mut W) -> Self {
        Self {
            options,
            writer,
            elements: Vec::new(),
            contexts: HashMap::new(),
            state: Formatting::default(),
            pending_breaks: 0,
            newlines: 0,
            output_length: 0,
            line_start: true,
            has_output: false,
            pending_space: false,
            skip_pre_newline: false,
            previous_cr: false,
            marker: None,
        }
    }

    fn suppressed(&self) -> bool {
        // Visibility is snapshotted at opening and includes the parent's
        // suppression, so the top element answers this without a stack scan.
        self.elements.last().is_some_and(|element| !element.visible)
    }

    fn boundary(&mut self, count: usize) {
        if self.marker.is_none() {
            self.pending_breaks = self.pending_breaks.max(count);
        }
        self.pending_space = false;
    }

    fn write_output(&mut self, text: &str) -> io::Result<()> {
        if text.len() > self.options.max_output_length.get() - self.output_length {
            return Err(HtmlToTextError::OutputLength.into());
        }
        self.output_length += text.len();
        self.writer.write_all(text.as_bytes())
    }

    fn write_newline(&mut self) -> io::Result<()> {
        self.write_output(output_newline(self.options.new_line_format))
    }

    fn flush_breaks(&mut self) -> io::Result<()> {
        if self.has_output && self.pending_breaks > self.newlines {
            for _ in self.newlines..self.pending_breaks {
                self.write_newline()?;
            }
            self.line_start = true;
            self.newlines = self.pending_breaks;
            self.pending_space = false;
        }
        self.pending_breaks = 0;
        Ok(())
    }

    fn write_character(&mut self, c: char) -> io::Result<()> {
        self.flush_breaks()?;
        if self.pending_space && !self.line_start {
            self.write_output(" ")?;
            self.track_link_character(' ');
        }
        self.pending_space = false;
        if self.line_start {
            for _ in 0..self.state.quote_depth {
                self.write_output(">")?;
            }
            if self.state.quote_depth > 0 {
                self.write_output(" ")?;
            }
            let padding = self.state.indent - self.marker.as_ref().map_or(0, String::len);
            for _ in 0..padding {
                self.write_output(" ")?;
            }
            if let Some(marker) = self.marker.take() {
                self.write_output(&marker)?;
            }
            self.line_start = false;
        }
        self.write_output(c.encode_utf8(&mut [0; 4]))?;
        self.track_link_character(c);
        self.has_output = true;
        self.newlines = 0;
        Ok(())
    }

    fn track_link_character(&mut self, c: char) {
        let Some(index) = self.state.link else { return };
        let element = &mut self.elements[index];
        let Some(url) = &element.url else { return };
        if element.link_length == 0 && c == ' ' {
            return;
        }
        if url
            .get(element.link_length..)
            .is_some_and(|tail| tail.starts_with(c))
        {
            element.link_length += c.len_utf8();
        } else {
            element.link_matches = false;
            element.link_length = url.len() + 1;
        }
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        for c in text.chars() {
            if is_whitespace(c) {
                self.pending_space = true;
            } else {
                self.write_character(c)?;
            }
        }
        Ok(())
    }

    fn line_break(&mut self) -> io::Result<()> {
        self.flush_breaks()?;
        if self.marker.is_some() {
            let continuation_indent = self.state.indent;
            self.finish_marker()?;
            self.state.indent = continuation_indent;
        }
        self.write_newline()?;
        self.has_output = true;
        self.line_start = true;
        self.newlines = (self.newlines + 1).min(2);
        self.pending_space = false;
        self.track_link_character('\n');
        Ok(())
    }

    fn write_preformatted(&mut self, text: &str) -> io::Result<()> {
        for c in text.chars() {
            if self.previous_cr && c == '\n' {
                self.previous_cr = false;
                continue;
            }
            self.previous_cr = c == '\r';
            if self.skip_pre_newline {
                self.skip_pre_newline = false;
                if matches!(c, '\r' | '\n') {
                    continue;
                }
            }
            if matches!(c, '\r' | '\n') {
                self.line_break()?;
            } else {
                self.write_character(c)?;
            }
        }
        Ok(())
    }

    fn finish_marker(&mut self) -> io::Result<()> {
        if let Some(marker) = self.marker.take() {
            self.state.indent -= marker.len();
            self.write_text(marker.trim_end())?;
        }
        Ok(())
    }

    fn close_at(&mut self, index: usize) -> io::Result<()> {
        while self.elements.len() > index {
            let current = self.elements.len() - 1;
            let id = self.elements[current].id;
            let previous = self.elements[current].previous;
            if self.elements[current].visible {
                if id == HtmlTagId::A
                    && let Some(url) = self.elements[current].url.take()
                {
                    self.state.link = None;
                    let breaks = std::mem::take(&mut self.pending_breaks);
                    if self.elements[current].link_length == 0 {
                        self.write_text(&url)?;
                    } else if !self.elements[current].link_matches
                        || self.elements[current].link_length != url.len()
                    {
                        self.write_text(" (")?;
                        self.write_text(&url)?;
                        self.write_character(')')?;
                    }
                    self.pending_breaks = self.pending_breaks.max(breaks);
                }
                if id == HtmlTagId::Li {
                    self.finish_marker()?;
                    self.pending_breaks = 1;
                }
                let count = if is_list(id) {
                    if previous.list.is_none() { 2 } else { 1 }
                } else {
                    spacing(id)
                };
                if count > 0 {
                    self.boundary(count);
                }
                if matches!(id, HtmlTagId::Td | HtmlTagId::Th) {
                    self.pending_space = true;
                }
            }
            self.state = previous;
            let element = self.elements.pop().expect("open element");
            if let Some(previous_index) = element.previous_index {
                self.contexts.insert(element.name, previous_index);
            } else {
                self.contexts.remove(&element.name);
            }
            self.previous_cr = false;
            self.skip_pre_newline = false;
        }
        Ok(())
    }

    fn close(&mut self, name: &str) -> io::Result<()> {
        if let Some(&index) = self.contexts.get(&name.to_ascii_lowercase()) {
            self.close_at(index)?;
        }
        Ok(())
    }

    fn close_in_scope(&mut self, name: &str, scope: &str) -> io::Result<()> {
        if let Some(&index) = self.contexts.get(name)
            && self.in_template_scope(index)
            && self
                .contexts
                .get(scope)
                .is_none_or(|&boundary| index > boundary)
        {
            self.close_at(index)?;
        }
        Ok(())
    }

    fn in_template_scope(&self, index: usize) -> bool {
        self.contexts
            .get("template")
            .is_none_or(|&template| index > template)
    }

    fn close_paragraph(&mut self) -> io::Result<()> {
        let Some(&index) = self.contexts.get("p") else {
            return Ok(());
        };
        for name in [
            "button", "table", "td", "th", "caption", "applet", "marquee", "object", "template",
        ] {
            if self
                .contexts
                .get(name)
                .is_some_and(|&boundary| boundary > index)
            {
                return Ok(());
            }
        }
        self.close_at(index)
    }

    fn start_tag(&mut self, tag: HtmlTagToken) -> io::Result<()> {
        use HtmlTagId::*;
        let id = tag.id();
        if id == Head
            || (matches!(id, Html | Body)
                && !self.suppressed()
                && !tag.attributes().contains("hidden"))
        {
            return Ok(());
        }
        if spacing(id) > 0 {
            self.close_paragraph()?;
        }
        if id == Li
            && let Some(item) = self.state.item
            && self.elements[item].previous.list == self.state.list
            && self.in_template_scope(item)
        {
            self.close_at(item)?;
        }
        if heading_level(id).is_some()
            && let Some(heading) = self.state.heading
            && self.in_template_scope(heading)
        {
            self.close_at(heading)?;
        }
        if id == A {
            self.close_in_scope("a", "template")?;
        }
        if matches!(id, Dd | Dt) {
            self.close_in_scope("dd", "dl")?;
            self.close_in_scope("dt", "dl")?;
        }
        let suppressed = self.suppressed() || suppresses_content(&tag);
        if id.is_empty_element() {
            if !suppressed {
                match id {
                    Br => self.line_break()?,
                    Hr => {
                        self.boundary(2);
                        self.write_text("---")?;
                        self.boundary(2);
                    }
                    Image if self.options.output_image_alt_text => {
                        if let Some(alt) = tag.attributes().get("alt").and_then(|a| a.value()) {
                            self.write_text(alt)?;
                        }
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        if self.elements.len() >= self.options.max_element_depth.get() {
            return Err(HtmlToTextError::ElementDepth.into());
        }
        let index = self.elements.len();
        let name = tag.name().to_ascii_lowercase();
        let previous_index = self.contexts.insert(name.clone(), index);
        self.elements.push(Element {
            id,
            name,
            previous_index,
            previous: self.state,
            visible: !suppressed,
            url: None,
            link_matches: true,
            link_length: 0,
            next_number: 0,
        });
        if suppressed {
            if is_list(id) {
                self.state.list = Some(index);
            }
            if id == Li {
                self.state.item = Some(index);
            }
            if heading_level(id).is_some() {
                self.state.heading = Some(index);
            }
        } else {
            let count = spacing(id);
            if count > 0 {
                self.boundary(count);
            }
            match id {
                BlockQuote => self.state.quote_depth += 1,
                Pre | Listing | Xmp | TextArea | PlainText => {
                    self.state.pre_depth += 1;
                    self.skip_pre_newline = !matches!(id, Xmp | PlainText);
                    self.previous_cr = false;
                }
                Ol | Ul | Menu | Dir => {
                    self.finish_marker()?;
                    self.boundary(if self.state.list.is_none() { 2 } else { 1 });
                    self.elements[index].next_number = get_number(&tag, "start", 1);
                    self.state.list = Some(index);
                }
                Li => {
                    self.state.item = Some(index);
                    if let Some(list) = self.state.list {
                        self.state.indent = self.elements[list].previous.indent;
                    }
                    let marker = if let Some(list) = self.state.list
                        && self.elements[list].id == Ol
                    {
                        let number = get_number(&tag, "value", self.elements[list].next_number);
                        self.elements[list].next_number = number + 1;
                        format!("{number}. ")
                    } else {
                        "- ".into()
                    };
                    self.state.indent += marker.len();
                    self.marker = Some(marker);
                }
                Dd => self.state.indent += 2,
                A if self.options.output_link_urls => {
                    if let Some(url) = tag.attributes().get("href").and_then(|a| a.value()) {
                        let url = url.trim();
                        if !url.is_empty() {
                            self.state.link = Some(index);
                        }
                        self.elements[index].url = Some(url.to_owned());
                    }
                }
                Td | Th => self.pending_space = true,
                _ => {
                    if let Some(level) = heading_level(id) {
                        self.state.heading = Some(index);
                        self.write_text(&format!("{} ", "#".repeat(level)))?;
                    }
                }
            }
        }

        // HTML ignores self-closing syntax on non-void elements, including scripts and templates.
        if tag.is_empty_element() && id == Unknown && !tag.name().eq_ignore_ascii_case("template") {
            self.close_at(index)?;
        }
        Ok(())
    }

    fn convert<R: BufRead>(&mut self, reader: R) -> io::Result<()> {
        let options = HtmlTokenizerOptions {
            decode_character_references: true,
            scripting_enabled: false,
            ignore_truncated_tags: true,
            max_element_depth: self.options.max_element_depth.get(),
        };
        let mut tokenizer =
            HtmlTokenizerReader::with_options(reader, options).expect("nonzero element depth");
        tokenizer.tokenizer_mut().set_track_closed_elements(true);
        while let Some(token) = tokenizer.read_next_token()? {
            let tree = tokenizer.tokenizer().tree_construction();
            if tree.depth_exceeded() {
                return Err(HtmlToTextError::ElementDepth.into());
            }
            for closed in tree.closed_elements() {
                for _ in 0..closed.count {
                    self.close(&closed.name)?;
                }
            }
            let disposition = tree.last_tag().map(|tag| tag.disposition);
            let in_frameset = tree.is_in_frameset();
            tokenizer.tokenizer_mut().clear_closed_elements();

            if !matches!(token, HtmlToken::Data(_) | HtmlToken::CData(_)) {
                self.previous_cr = false;
                self.skip_pre_newline = false;
            }
            match token {
                HtmlToken::Tag(tag) => {
                    if disposition == Some(HtmlTagDisposition::Ignored) {
                        continue;
                    }
                    if tag.is_end_tag() {
                        if heading_level(tag.id()).is_some()
                            && let Some(heading) = self.state.heading
                        {
                            self.close_at(heading)?;
                        } else if tag.id() == HtmlTagId::Br && !self.suppressed() {
                            self.line_break()?;
                        } else {
                            self.close(tag.name())?;
                        }
                    } else if disposition != Some(HtmlTagDisposition::AttributesMerged) {
                        self.start_tag(tag)?;
                    }
                }
                HtmlToken::Data(data) if !self.suppressed() && !in_frameset => {
                    self.character_data(data.data())?;
                }
                HtmlToken::CData(data) if !self.suppressed() && !in_frameset => {
                    self.character_data(&data)?;
                }
                _ => {}
            }
        }
        self.close_at(0)
    }

    fn character_data(&mut self, data: &str) -> io::Result<()> {
        if self.state.pre_depth > 0 {
            self.write_preformatted(data)
        } else {
            self.write_text(data)
        }
    }
}

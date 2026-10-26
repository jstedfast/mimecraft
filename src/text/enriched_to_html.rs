// Adapted from MimeKit's Text/EnrichedToHtml.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{collections::HashMap, error::Error, fmt, num::NonZeroUsize, rc::Rc};

use super::{HeaderFooterFormat, TextToHtml, html_utils::encode_into};

/// A resource limit exceeded while converting enriched text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnrichedToHtmlError {
    /// Original input exceeds the configured UTF-8 byte limit.
    InputLength,
    /// Generated HTML, including decorations, exceeds the UTF-8 byte limit.
    OutputLength,
    /// Command nesting, including unknown commands and parameters, is too deep.
    ElementDepth,
    /// Parameter content exceeds its normalized UTF-8 byte limit.
    ParameterLength,
}

impl fmt::Display for EnrichedToHtmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputLength => "enriched text input length limit exceeded",
            Self::OutputLength => "enriched text output length limit exceeded",
            Self::ElementDepth => "enriched text command depth limit exceeded",
            Self::ParameterLength => "enriched text parameter length limit exceeded",
        })
    }
}

impl Error for EnrichedToHtmlError {}

type Result<T> = std::result::Result<T, EnrichedToHtmlError>;

/// Converts RFC 1896 text/enriched to HTML, not the older text/richtext format.
///
/// Supports character styles, font families, colors, relative sizes, alignment,
/// indentation, excerpts, and language attributes. Unknown commands are ignored
/// while retaining their text; parameters are never displayed. Invalid command
/// syntax is literal text. Unmatched ends are ignored, mismatched ends close
/// intervening commands, and open commands close at EOF.
///
/// Font sizes change by one sixth of the base size, bounded to 1/6 through 12
/// times that size; indentation changes in four-character steps. Font families
/// accept Unicode letters and decimal digits, spaces, hyphens, underscores,
/// and periods. Unlike MimeKit's UTF-16 character check, supplementary letters
/// and digits are accepted as complete Unicode scalars.
///
/// CRLF, LF, and CR are accepted. Outside `nofill`, one newline becomes a space
/// and N consecutive newlines become N-1 breaks. Inside `nofill`, each newline
/// is retained. Generated paragraphs preserve spaces and tabs.
///
/// Body text is escaped and parameters are validated before use. No body links
/// are generated and no resources are fetched. Plain-text headers/footers use
/// [`TextToHtml`]; HTML decorations are trusted markup, not sanitized input.
///
/// Length limits count UTF-8 bytes rather than MimeKit's UTF-16 code units.
/// Conversion returns owned HTML or an error, never partial output. Decorations
/// are outside the input limit; plain-text decorations are converted using the
/// existing converter before checking their output size, so their intermediate
/// allocations are not constrained by the output limit. No stream I/O, charset
/// conversion, or MIME-body replacement is performed.
///
/// ```
/// use mimecraft::text::EnrichedToHtml;
///
/// let converter = EnrichedToHtml { output_html_fragment: true, ..Default::default() };
/// assert_eq!(converter.convert("<bold>Hello</bold> & goodbye")?,
///     "<div style=\"white-space: pre-wrap;\"><div><span style=\"font-weight: bold;\">Hello</span> &amp; goodbye</div></div>");
/// # Ok::<(), mimecraft::text::EnrichedToHtmlError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnrichedToHtml {
    /// Omit the HTML/body wrapper; defaults to false.
    pub output_html_fragment: bool,
    /// Optional content before the enriched body.
    pub header: Option<String>,
    /// Header interpretation; defaults to plain text.
    pub header_format: HeaderFooterFormat,
    /// Optional content after the enriched body.
    pub footer: Option<String>,
    /// Footer interpretation; defaults to plain text.
    pub footer_format: HeaderFooterFormat,
    /// Maximum open commands, including unknown commands and nested parameters.
    /// Defaults to 4096.
    pub max_element_depth: NonZeroUsize,
    /// Maximum parameter bytes after CRLF/CR normalization to LF. Includes
    /// ignored content, nested command markup, and both bytes of `<<`.
    /// Defaults to 4096.
    pub max_parameter_length: NonZeroUsize,
    /// Maximum original input bytes, including commands and suppressed text.
    /// Defaults to 16 MiB.
    pub max_input_length: NonZeroUsize,
    /// Maximum HTML bytes, including markup, escaping, headers, and footers.
    /// Defaults to 64 MiB.
    pub max_output_length: NonZeroUsize,
}

impl Default for EnrichedToHtml {
    fn default() -> Self {
        Self {
            output_html_fragment: false,
            header: None,
            header_format: HeaderFooterFormat::Text,
            footer: None,
            footer_format: HeaderFooterFormat::Text,
            max_element_depth: NonZeroUsize::new(4096).unwrap(),
            max_parameter_length: NonZeroUsize::new(4096).unwrap(),
            max_input_length: NonZeroUsize::new(16 * 1024 * 1024).unwrap(),
            max_output_length: NonZeroUsize::new(64 * 1024 * 1024).unwrap(),
        }
    }
}

impl EnrichedToHtml {
    /// Creates a document converter with default resource limits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Converts a complete decoded string without changing the converter.
    ///
    /// # Errors
    ///
    /// Returns [`EnrichedToHtmlError`] when a configured limit is exceeded.
    /// The input length is checked before conversion or decoration rendering.
    pub fn convert(&self, text: &str) -> Result<String> {
        if text.len() > self.max_input_length.get() {
            return Err(EnrichedToHtmlError::InputLength);
        }
        let mut converter = Converter {
            options: self,
            output: String::new(),
            text: String::new(),
            state: Formatting::default(),
            last_line: Formatting::default(),
            commands: Vec::new(),
            contexts: HashMap::new(),
            parameter: String::new(),
            parameter_target: None,
            parameter_depth: 0,
            parameter_length: 0,
            invalid_parameter: false,
            newlines: 0,
            paragraph_open: false,
            span_open: false,
            implicit_break: false,
            trailing_newline: false,
        };
        if !self.output_html_fragment {
            converter.write("<html><body>")?;
        }
        converter.decoration(self.header.as_deref(), self.header_format)?;
        converter.write("<div style=\"white-space: pre-wrap;\">")?;
        // Normalize on demand rather than copying the entire input. Rust chars
        // keep supplementary scalars intact when the HTML encoder sees them.
        let mut raw = text.chars().peekable();
        let mut input = std::iter::from_fn(|| {
            let ch = raw.next()?;
            if ch == '\r' {
                if raw.peek() == Some(&'\n') {
                    raw.next();
                }
                Some('\n')
            } else {
                Some(ch)
            }
        });
        while let Some(ch) = input.next() {
            if ch == '<' {
                converter.read_command(&mut input)?;
            } else {
                converter.character(ch)?;
            }
        }
        converter.flush_newlines()?;
        converter.close_commands(0)?;
        converter.close_paragraph()?;
        if converter.trailing_newline {
            converter.state = converter.last_line.clone();
            converter.open_paragraph()?;
            converter.write("<br/>")?;
            converter.close_paragraph()?;
        }
        converter.write("</div>")?;
        converter.decoration(self.footer.as_deref(), self.footer_format)?;
        if !self.output_html_fragment {
            converter.write("</body></html>")?;
        }
        Ok(converter.output)
    }
}

#[derive(Clone, Default)]
struct Formatting {
    bold: bool,
    italic: bool,
    underline: bool,
    nofill: bool,
    size: i64,
    left: i64,
    right: i64,
    first_line: i64,
    excerpt: i64,
    font: Option<Rc<str>>,
    color: Option<Rc<str>>,
    language: Option<Rc<str>>,
    alignment: Option<&'static str>,
}

struct Command {
    name: String,
    previous: Formatting,
    previous_index: Option<usize>,
}

struct Converter<'a> {
    options: &'a EnrichedToHtml,
    output: String,
    text: String,
    state: Formatting,
    last_line: Formatting,
    commands: Vec<Command>,
    // Maps an ASCII-folded name to its innermost open command. Each command's
    // previous_index links to the next outer command with the same name.
    contexts: HashMap<String, usize>,
    parameter: String,
    parameter_target: Option<usize>,
    parameter_depth: usize,
    parameter_length: usize,
    invalid_parameter: bool,
    newlines: usize,
    paragraph_open: bool,
    span_open: bool,
    implicit_break: bool,
    trailing_newline: bool,
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "center" | "flushleft" | "flushright" | "flushboth" | "paraindent" | "nofill" | "excerpt"
    )
}

impl Converter<'_> {
    fn write(&mut self, value: &str) -> Result<()> {
        if value.len() > self.options.max_output_length.get() - self.output.len() {
            return Err(EnrichedToHtmlError::OutputLength);
        }
        self.output.push_str(value);
        Ok(())
    }

    fn decoration(&mut self, value: Option<&str>, format: HeaderFooterFormat) -> Result<()> {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            match format {
                HeaderFooterFormat::Html => self.write(value)?,
                HeaderFooterFormat::Text => {
                    let converter = TextToHtml {
                        output_html_fragment: true,
                        ..Default::default()
                    };
                    self.write(&converter.convert(value))?;
                }
            }
        }
        Ok(())
    }

    fn attribute(&mut self, name: &str, value: &str) -> Result<()> {
        self.write(" ")?;
        self.write(name)?;
        self.write("=")?;
        self.write(&super::html_attribute_encode(
            value,
            super::HtmlAttributeQuote::Double,
        ))
    }

    fn open_paragraph(&mut self) -> Result<()> {
        if self.paragraph_open {
            return Ok(());
        }
        self.write("<div")?;
        let mut css = String::new();
        if self.state.nofill {
            css.push_str("white-space: pre; ");
        }
        if let Some(alignment) = self.state.alignment {
            let alignment = if self.state.nofill && alignment == "justify" {
                "left"
            } else {
                alignment
            };
            css.push_str(&format!("text-align: {alignment}; "));
        }
        for (property, count) in [
            (
                "margin-left",
                self.state.left + self.state.excerpt + (-self.state.first_line).max(0),
            ),
            ("margin-right", self.state.right),
            ("text-indent", self.state.first_line),
        ] {
            if count != 0 {
                css.push_str(&format!("{property}: {}ch; ", count * 4));
            }
        }
        if !css.is_empty() {
            self.attribute("style", css.trim_end())?;
        }
        self.write(">")?;
        self.paragraph_open = true;
        Ok(())
    }

    fn open_span(&mut self) -> Result<()> {
        if self.span_open {
            return Ok(());
        }
        let mut css = String::new();
        for (enabled, style) in [
            (self.state.bold, "font-weight: bold; "),
            (self.state.italic, "font-style: italic; "),
            (self.state.underline, "text-decoration: underline; "),
        ] {
            if enabled {
                css.push_str(style);
            }
        }
        if let Some(font) = &self.state.font {
            css.push_str(&format!("font-family: {font}; "));
        }
        if let Some(color) = &self.state.color {
            css.push_str(&format!("color: {color}; "));
        }
        if self.state.size != 0 {
            let size = (1.0 + self.state.size as f64 / 6.0).clamp(1.0 / 6.0, 12.0);
            let size = format!("{size:.3}");
            css.push_str(&format!(
                "font-size: {}em; ",
                size.trim_end_matches('0').trim_end_matches('.')
            ));
        }
        let language = self.state.language.clone();
        if css.is_empty() && language.is_none() {
            return Ok(());
        }
        self.write("<span")?;
        if !css.is_empty() {
            self.attribute("style", css.trim_end())?;
        }
        if let Some(language) = language {
            self.attribute("lang", &language)?;
        }
        self.write(">")?;
        self.span_open = true;
        Ok(())
    }

    fn close_span(&mut self) -> Result<()> {
        self.flush_text()?;
        if self.span_open {
            self.write("</span>")?;
            self.span_open = false;
        }
        Ok(())
    }

    fn close_paragraph(&mut self) -> Result<()> {
        self.close_span()?;
        if self.paragraph_open {
            self.write("</div>")?;
            self.paragraph_open = false;
        }
        Ok(())
    }

    fn append_text(&mut self, ch: char) -> Result<()> {
        self.implicit_break = false;
        self.trailing_newline = false;
        self.text.push(ch);
        if self.text.len() >= 1024 {
            // Keep each Unicode scalar together when encoding the buffer.
            self.flush_text()?;
        }
        Ok(())
    }

    fn flush_text(&mut self) -> Result<()> {
        if self.text.is_empty() {
            return Ok(());
        }
        self.open_paragraph()?;
        self.open_span()?;
        let mut encoded = String::new();
        encode_into(&mut encoded, &self.text, None);
        self.write(&encoded)?;
        self.text.clear();
        Ok(())
    }

    fn break_paragraph(&mut self) -> Result<()> {
        if self.paragraph_open || !self.text.is_empty() {
            self.implicit_break = true;
        }
        self.close_paragraph()
    }

    fn flush_newlines(&mut self) -> Result<()> {
        if self.newlines == 0 {
            return Ok(());
        }
        if !self.state.nofill && self.newlines == 1 {
            self.append_text(' ')?;
        } else {
            let count = if self.state.nofill {
                self.newlines
            } else {
                self.newlines - 1
            };
            let count = count.saturating_sub(usize::from(self.implicit_break));
            self.implicit_break = false;
            for _ in 0..count {
                self.flush_text()?;
                if !self.paragraph_open {
                    self.open_paragraph()?;
                    self.write("<br/>")?;
                }
                self.close_paragraph()?;
            }
            self.trailing_newline = true;
            self.last_line = self.state.clone();
        }
        self.newlines = 0;
        Ok(())
    }

    fn count_parameter(&mut self, count: usize) -> Result<()> {
        if count > self.options.max_parameter_length.get() - self.parameter_length {
            return Err(EnrichedToHtmlError::ParameterLength);
        }
        self.parameter_length += count;
        Ok(())
    }

    fn character(&mut self, ch: char) -> Result<()> {
        if self.parameter_depth != 0 {
            self.count_parameter(ch.len_utf8())?;
            if self.parameter_target.is_some() && !self.invalid_parameter {
                self.parameter.push(ch);
            }
            return Ok(());
        }
        self.parameter_target = None;
        if ch == '\n' {
            self.newlines += 1;
        } else {
            self.flush_newlines()?;
            self.append_text(ch)?;
        }
        Ok(())
    }

    fn check_depth(&self, extra: usize) -> Result<()> {
        if extra >= self.options.max_element_depth.get()
            || self.commands.len() >= self.options.max_element_depth.get() - extra
        {
            Err(EnrichedToHtmlError::ElementDepth)
        } else {
            Ok(())
        }
    }

    fn start_command(&mut self, name: &str) -> Result<()> {
        self.check_depth(0)?;
        if name == "param" {
            self.parameter_depth = 1;
            self.parameter_length = 0;
            self.invalid_parameter = false;
            self.parameter.clear();
            return Ok(());
        }
        self.parameter_target = None;
        if is_block(name) {
            self.break_paragraph()?;
        } else {
            self.close_span()?;
        }
        let index = self.commands.len();
        // Index the nearest matching start, as MimeKit's context stack does:
        // repeated unmatched ends must not repeatedly scan a deep command stack.
        let previous_index = self.contexts.insert(name.to_owned(), index);
        self.commands.push(Command {
            name: name.into(),
            previous: self.state.clone(),
            previous_index,
        });
        match name {
            "bold" => self.state.bold = true,
            "italic" => self.state.italic = true,
            "underline" => self.state.underline = true,
            "fixed" => self.state.font = Some("monospace".into()),
            "bigger" => self.state.size += 1,
            "smaller" => self.state.size -= 1,
            "nofill" => self.state.nofill = true,
            "center" => self.state.alignment = Some("center"),
            "flushleft" => self.state.alignment = Some("left"),
            "flushright" => self.state.alignment = Some("right"),
            "flushboth" => self.state.alignment = Some("justify"),
            "excerpt" => self.state.excerpt += 1,
            "fontfamily" | "color" | "lang" | "paraindent" => self.parameter_target = Some(index),
            _ => {}
        }
        Ok(())
    }

    fn close_commands(&mut self, index: usize) -> Result<()> {
        while self.commands.len() > index {
            let command = self.commands.pop().expect("open command");
            if is_block(&command.name) {
                self.break_paragraph()?;
            } else {
                self.close_span()?;
            }
            self.state = command.previous;
            if let Some(previous) = command.previous_index {
                self.contexts.insert(command.name, previous);
            } else {
                self.contexts.remove(&command.name);
            }
        }
        Ok(())
    }

    fn apply_parameter(&mut self) {
        let Some(index) = self.parameter_target.filter(|_| !self.invalid_parameter) else {
            return;
        };
        let value = self.parameter.trim();
        match self.commands[index].name.as_str() {
            "color" => {
                if let Some(color) = parse_color(value) {
                    self.state.color = Some(color.into());
                }
            }
            "fontfamily" if is_font_family(value) => {
                self.state.font = Some(format!("'{value}'").into())
            }
            "lang" if is_language(value) => self.state.language = Some(value.into()),
            "paraindent" => {
                let (mut left, mut right, mut inward, mut outward) = (0, 0, 0, 0);
                for item in value.split(',') {
                    match item.trim().to_ascii_lowercase().as_str() {
                        "left" => left += 1,
                        "right" => right += 1,
                        "in" => inward += 1,
                        "out" => outward += 1,
                        _ => return,
                    }
                }
                if inward > 0 && outward > 0 {
                    return;
                }
                self.state.left += left;
                self.state.right += right;
                self.state.first_line += inward - outward;
            }
            _ => {}
        }
    }

    fn process_command(&mut self, name: &str, end: bool) -> Result<()> {
        if self.parameter_depth > 0 {
            if name == "param" && end {
                self.parameter_depth -= 1;
                if self.parameter_depth == 0 {
                    self.apply_parameter();
                    self.parameter_target = None;
                    self.parameter.clear();
                    return Ok(());
                }
            } else if name == "param" {
                self.check_depth(self.parameter_depth)?;
                self.parameter_depth += 1;
            }
            self.invalid_parameter = true;
            return self.count_parameter(name.len() + if end { 3 } else { 2 });
        }
        self.flush_newlines()?;
        if end {
            self.parameter_target = None;
            if let Some(&index) = self.contexts.get(name) {
                self.close_commands(index)?;
            }
            Ok(())
        } else {
            self.start_command(name)
        }
    }

    fn read_command(&mut self, input: &mut impl Iterator<Item = char>) -> Result<()> {
        let mut ch = input.next();
        if ch == Some('<') {
            if self.parameter_depth > 0 {
                self.count_parameter(1)?;
            }
            return self.character('<');
        }
        let end = ch == Some('/');
        if end {
            ch = input.next();
        }
        let mut name = String::new();
        while name.len() < 60 && ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-') {
            name.push(ch.expect("command character"));
            ch = input.next();
        }
        if !name.is_empty() && ch == Some('>') {
            return self.process_command(&name.to_ascii_lowercase(), end);
        }
        self.character('<')?;
        if end {
            self.character('/')?;
        }
        for ch in name.chars() {
            self.character(ch)?;
        }
        if let Some(ch) = ch {
            self.character(ch)?;
        }
        Ok(())
    }
}

fn parse_color(value: &str) -> Option<String> {
    let name = value.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "red" | "blue" | "green" | "yellow" | "cyan" | "magenta" | "black" | "white"
    ) {
        return Some(name);
    }
    let bytes = value.as_bytes();
    if bytes.len() != 14 || bytes[4] != b',' || bytes[9] != b',' {
        return None;
    }
    let mut rgb = 0u32;
    for component in bytes.split(|&byte| byte == b',') {
        if component.len() != 4 || !component.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let component = u32::from_str_radix(std::str::from_utf8(component).ok()?, 16).ok()?;
        rgb = (rgb << 8) | ((component + 128) / 257);
    }
    Some(format!("#{rgb:06x}"))
}

fn is_font_family(value: &str) -> bool {
    use unicode_general_category::{GeneralCategory as G, get_general_category};
    !value.is_empty()
        && value.chars().all(|ch| {
            matches!(ch, ' ' | '-' | '_' | '.')
                || matches!(
                    get_general_category(ch),
                    G::UppercaseLetter
                        | G::LowercaseLetter
                        | G::TitlecaseLetter
                        | G::ModifierLetter
                        | G::OtherLetter
                        | G::DecimalNumber
                )
        })
}

fn is_language(value: &str) -> bool {
    value.split('-').enumerate().all(|(index, part)| {
        (1..=8).contains(&part.len())
            && part
                .bytes()
                .all(|ch| ch.is_ascii_alphabetic() || (index != 0 && ch.is_ascii_digit()))
    })
}

// Adapted from MimeKit's Text/HtmlWriter.cs and Text/HtmlWriterState.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{
    HtmlAttribute, HtmlAttributeQuote, HtmlToken,
    html_utils::{encode_into, is_valid_attribute_name, is_valid_tag_name},
};

/// The state of an [`HtmlWriter`], independent of HTML element nesting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HtmlWriterState {
    /// Outside an unfinished start tag; only tags or content can be written.
    ///
    /// Corresponds to MimeKit's `Default` writer state.
    #[default]
    Content,
    /// Inside a start tag; an attribute name, another tag, or content can follow.
    Tag,
    /// After an attribute name; a value, another name, tag, or content can follow.
    ///
    /// Omitting the value leaves a valueless attribute.
    Attribute,
}

/// A rejected HTML writer operation. Errors leave the output and state unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlWriterError {
    /// The tag name is empty or contains a delimiter rejected by MimeKit.
    InvalidTagName,
    /// The attribute name is empty or contains a delimiter rejected by MimeKit.
    InvalidAttributeName,
    /// An attribute name was supplied outside an unfinished start tag.
    AttributeOutsideTag,
    /// An attribute value was supplied without a pending attribute name.
    ValueWithoutAttribute,
}

impl fmt::Display for HtmlWriterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTagName => "invalid HTML tag name",
            Self::InvalidAttributeName => "invalid HTML attribute name",
            Self::AttributeOutsideTag => "cannot write an HTML attribute outside a start tag",
            Self::ValueWithoutAttribute => "cannot write an HTML attribute value without a name",
        })
    }
}

impl Error for HtmlWriterError {}

/// Builds HTML in an owned string using MimeKit's writer state transitions.
///
/// Start tags remain unfinished so attributes can follow. Another tag, text,
/// raw markup, [`flush`](Self::flush), or [`finish`](Self::finish) closes them
/// with `>` or `/>`. End tags are written immediately. Valueless attributes
/// remain valueless unless [`write_attribute_value`](Self::write_attribute_value)
/// is called before the next name, tag, or content.
///
/// Text uses [`html_encode`](super::html_encode)'s escaping and control handling;
/// attribute values always use double-quoted HTML attribute encoding. Names
/// retain their spelling and case. Validation follows MimeKit's delimiter
/// checks, not the complete HTML name grammar: tag names reject NUL, TAB, CR,
/// LF, form feed, SPACE, `/`, and `>`; attribute names additionally reject
/// `=`, both quotes, and `<`. Other characters are retained verbatim.
///
/// This is not a sanitizer or document validator. Names and raw markup must be
/// trusted; the writer does not check URL schemes, event attributes, nesting,
/// or void-element rules. It adds no whitespace except between attributes,
/// performs no automatic linking, and does not track tags inside raw markup.
///
/// This in-memory API uses `&str` instead of C# buffer/index/count overloads.
/// Known tag/attribute identifiers expose canonical names through `as_str()`;
/// [`write_html_attribute`](Self::write_html_attribute) also accepts attributes
/// with malformed names preserved by the tokenizer.
/// [`write_token`](Self::write_token) appends a complete token without name
/// validation or changing its encoding mode. Charset conversion and stream
/// output are not included.
/// Finishing consumes the writer rather than disposing an external stream;
/// it does not synthesize element end tags.
///
/// ```
/// use mimecraft::text::HtmlWriter;
///
/// let mut html = HtmlWriter::new();
/// html.write_start_tag("a")?;
/// html.write_attribute("href", "https://example.com/?a=1&b=2")?;
/// html.write_text("Visit <example>");
/// html.write_end_tag("a")?;
/// assert_eq!(html.finish(),
///     "<a href=\"https://example.com/?a=1&amp;b=2\">Visit &lt;example&gt;</a>");
/// # Ok::<(), mimecraft::text::HtmlWriterError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HtmlWriter {
    output: String,
    state: HtmlWriterState,
    empty: bool,
}

impl HtmlWriter {
    /// Creates an empty writer in [`HtmlWriterState::Content`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current writer state, not the HTML element nesting depth.
    pub fn state(&self) -> HtmlWriterState {
        self.state
    }

    /// Borrows the output without flushing. A start tag may still be unfinished.
    pub fn as_str(&self) -> &str {
        &self.output
    }

    /// Starts a tag that will close with `>`, without inferring void elements.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::InvalidTagName`] for an invalid name.
    /// A previously pending tag is left untouched on error.
    pub fn write_start_tag(&mut self, name: &str) -> Result<(), HtmlWriterError> {
        self.begin_tag(name, false)
    }

    /// Starts a tag that will close with `/>`, regardless of its element name.
    ///
    /// Attributes can be written before the tag is closed.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::InvalidTagName`] for an invalid name.
    /// A previously pending tag is left untouched on error.
    pub fn write_empty_element_tag(&mut self, name: &str) -> Result<(), HtmlWriterError> {
        self.begin_tag(name, true)
    }

    fn begin_tag(&mut self, name: &str, empty: bool) -> Result<(), HtmlWriterError> {
        if !is_valid_tag_name(name) {
            return Err(HtmlWriterError::InvalidTagName);
        }
        self.flush();
        self.output.push('<');
        self.output.push_str(name);
        self.state = HtmlWriterState::Tag;
        self.empty = empty;
        Ok(())
    }

    /// Closes any pending start tag and writes a complete `</name>` tag.
    ///
    /// No matching start tag is required, and attributes cannot follow.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::InvalidTagName`] for an invalid name.
    /// A previously pending tag is left untouched on error.
    pub fn write_end_tag(&mut self, name: &str) -> Result<(), HtmlWriterError> {
        if !is_valid_tag_name(name) {
            return Err(HtmlWriterError::InvalidTagName);
        }
        self.flush();
        self.output.push_str("</");
        self.output.push_str(name);
        self.output.push('>');
        Ok(())
    }

    /// Writes an attribute name and a double-quoted, escaped value.
    ///
    /// An empty value produces `=""`; use [`write_attribute_name`](Self::write_attribute_name)
    /// without a following value for a valueless attribute.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::InvalidAttributeName`] for an invalid name,
    /// or [`HtmlWriterError::AttributeOutsideTag`] outside a pending start tag.
    /// Name validation takes precedence over state validation.
    pub fn write_attribute(&mut self, name: &str, value: &str) -> Result<(), HtmlWriterError> {
        self.write_attribute_name(name)?;
        self.write_attribute_value(value)
    }

    /// Writes an attribute, preserving its name and optional value.
    ///
    /// `None` leaves the attribute valueless; an empty value writes `=""`.
    /// Values are HTML-escaped, and duplicate names are not removed.
    /// Unlike the name-based API, this also accepts malformed attribute names
    /// preserved by the tokenizer. A leading `=` cannot become the preceding
    /// valueless attribute's value: that attribute receives an explicit `=""`.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::AttributeOutsideTag`] outside a pending start
    /// tag. Rejected operations leave the output and state unchanged.
    pub fn write_html_attribute(
        &mut self,
        attribute: &HtmlAttribute,
    ) -> Result<(), HtmlWriterError> {
        self.append_attribute_name(attribute.name())?;
        if let Some(value) = attribute.value() {
            self.write_attribute_value(value)?;
        }
        Ok(())
    }

    /// Writes a space and an attribute name, leaving its optional value pending.
    ///
    /// Names are neither deduplicated nor normalized. Writing another name
    /// leaves the preceding attribute valueless.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::InvalidAttributeName`] for an invalid name,
    /// or [`HtmlWriterError::AttributeOutsideTag`] outside a pending start tag.
    /// Name validation takes precedence over state validation.
    pub fn write_attribute_name(&mut self, name: &str) -> Result<(), HtmlWriterError> {
        if !is_valid_attribute_name(name) {
            return Err(HtmlWriterError::InvalidAttributeName);
        }
        self.append_attribute_name(name)
    }

    fn append_attribute_name(&mut self, name: &str) -> Result<(), HtmlWriterError> {
        if self.state == HtmlWriterState::Content {
            return Err(HtmlWriterError::AttributeOutsideTag);
        }
        // The tokenizer can produce names beginning with '=' (e.g.
        // "<a href/=javascript:x>"). Give a preceding valueless attribute an
        // explicit empty value so '=' cannot be reparsed as its value.
        if self.state == HtmlWriterState::Attribute && name.starts_with('=') {
            self.output.push_str("=\"\"");
        }
        self.output.push(' ');
        self.output.push_str(name);
        self.state = HtmlWriterState::Attribute;
        Ok(())
    }

    /// Writes `=` and one double-quoted, escaped value for the pending name.
    ///
    /// Values are complete strings, not chunks; another value requires a new name.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlWriterError::ValueWithoutAttribute`] unless the current
    /// state is [`HtmlWriterState::Attribute`].
    pub fn write_attribute_value(&mut self, value: &str) -> Result<(), HtmlWriterError> {
        if self.state != HtmlWriterState::Attribute {
            return Err(HtmlWriterError::ValueWithoutAttribute);
        }
        self.output.push_str("=\"");
        encode_into(&mut self.output, value, Some(HtmlAttributeQuote::Double));
        self.output.push('"');
        self.state = HtmlWriterState::Tag;
        Ok(())
    }

    /// Closes any pending tag and appends HTML-escaped character data.
    ///
    /// Even an empty string closes a pending tag. This is character-data
    /// encoding, not JavaScript or CSS escaping for script/style elements.
    pub fn write_text(&mut self, text: &str) {
        self.flush();
        encode_into(&mut self.output, text, None);
    }

    /// Closes any pending tag and appends trusted markup verbatim.
    ///
    /// The markup is neither parsed nor sanitized, including when it contains
    /// unfinished tags. Even empty markup closes a pending tag.
    pub fn write_markup_text(&mut self, markup: &str) {
        self.flush();
        self.output.push_str(markup);
    }

    /// Closes any pending tag and appends a complete token.
    ///
    /// Even an empty data token closes a pending tag. A tag token is written
    /// completely, so additional attributes cannot follow it. The writer ends
    /// in [`HtmlWriterState::Content`], independent of the token's contents.
    ///
    /// Token names and raw contents are not validated or sanitized. Character
    /// data uses the token's encoding mode, not automatic text escaping.
    pub fn write_token(&mut self, token: &HtmlToken) {
        self.flush();
        token.write_to(&mut self.output);
    }

    /// Completes the pending start tag, if any, and returns to content state.
    ///
    /// This is an in-memory state flush, not stream I/O. Repeated calls add
    /// nothing. It does not write end tags for open HTML elements.
    pub fn flush(&mut self) {
        if self.state != HtmlWriterState::Content {
            self.output.push_str(if self.empty { "/>" } else { ">" });
            self.state = HtmlWriterState::Content;
            self.empty = false;
        }
    }

    /// Flushes the pending start tag and returns the output, consuming the writer.
    ///
    /// Does not synthesize end tags for open HTML elements.
    pub fn finish(mut self) -> String {
        self.flush();
        self.output
    }

    pub(super) fn drain_to<W: Write + ?Sized>(&mut self, writer: &mut W) -> io::Result<()> {
        // Keep the pending tag/attribute state across output-buffer drains.
        // A later callback may still supply attributes for that start tag.
        writer.write_all(self.output.as_bytes())?;
        self.output.clear();
        Ok(())
    }
}

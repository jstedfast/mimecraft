// Adapted from MimeKit's Text/HtmlToken.cs and Text/HtmlTokenKind.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use super::{
    HtmlAttributeCollection, HtmlAttributeQuote, HtmlTagId, html_string::HtmlString,
    html_utils::encode_into,
};

/// The kind of content represented by an [`HtmlToken`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlTokenKind {
    /// A CDATA section.
    CData,
    /// A normal or bogus HTML comment.
    Comment,
    /// Character data.
    Data,
    /// A document type declaration.
    DocType,
    /// Raw script data.
    ScriptData,
    /// A start, end, or self-closing tag.
    Tag,
}

/// An owned HTML token, separate from tokenization and document validation.
///
/// This enum replaces MimeKit's token class hierarchy. CDATA and script data
/// contain strings directly; other variants carry their own metadata.
/// [`Self::write_to`] appends one complete token to a string, and
/// [`Self::to_html`] creates a new string. No stream I/O or parsing is performed.
///
/// Tokens are **not sanitized**. Comments, raw data, script data, CDATA, and tag
/// names are written verbatim, even when they contain markup delimiters. Use
/// [`HtmlDataToken::from_text`] to encode plain character data. Tag attributes
/// created by callers remain checked; tokenizer-originated names may be
/// malformed. Attribute values are escaped.
///
/// ```
/// use mimecraft::text::{HtmlDataToken, HtmlToken, HtmlWriter};
///
/// let token = HtmlToken::Data(HtmlDataToken::from_text("a < b"));
/// assert_eq!(token.to_html(), "a &lt; b");
/// let mut html = HtmlWriter::new();
/// html.write_start_tag("p")?;
/// html.write_token(&token);
/// html.write_end_tag("p")?;
/// assert_eq!(html.finish(), "<p>a &lt; b</p>");
/// # Ok::<(), mimecraft::text::HtmlWriterError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HtmlToken {
    /// Raw CDATA contents, enclosed in `<![CDATA[` and `]]>`.
    CData(String),
    /// A normal or bogus comment.
    Comment(HtmlCommentToken),
    /// Character data with explicit encoding/context metadata.
    Data(HtmlDataToken),
    /// A checked document type declaration.
    DocType(HtmlDocTypeToken),
    /// Raw script contents, without surrounding script tags.
    ScriptData(String),
    /// A complete tag, including its attributes and closing delimiter.
    Tag(HtmlTagToken),
}

impl HtmlToken {
    /// Returns the kind without inspecting or parsing the token's contents.
    pub const fn kind(&self) -> HtmlTokenKind {
        match self {
            Self::CData(_) => HtmlTokenKind::CData,
            Self::Comment(_) => HtmlTokenKind::Comment,
            Self::Data(_) => HtmlTokenKind::Data,
            Self::DocType(_) => HtmlTokenKind::DocType,
            Self::ScriptData(_) => HtmlTokenKind::ScriptData,
            Self::Tag(_) => HtmlTokenKind::Tag,
        }
    }

    /// Appends this token's serialization without changing its contents.
    ///
    /// Does not infer nesting, add separators, or inspect adjacent tokens.
    pub fn write_to(&self, output: &mut String) {
        match self {
            Self::CData(data) => {
                output.push_str("<![CDATA[");
                output.push_str(data);
                output.push_str("]]>");
            }
            Self::Comment(comment) => comment.write_to(output),
            Self::Data(data) => data.write_to(output),
            Self::DocType(doctype) => doctype.write_to(output),
            Self::ScriptData(data) => output.push_str(data),
            Self::Tag(tag) => tag.write_to(output),
        }
    }

    /// Serializes this token to a new owned string.
    pub fn to_html(&self) -> String {
        let mut output = String::new();
        self.write_to(&mut output);
        output
    }
}

/// An HTML comment, including the origin of a bogus comment.
///
/// Contents are neither validated nor escaped. Bogus comments restore their
/// original kind of opening delimiter, not necessarily their exact input bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlCommentToken {
    comment: String,
    bogus: bool,
    bang: bool,
}

impl HtmlCommentToken {
    /// Creates a normal comment enclosed in `<!--` and `-->`.
    pub fn new(comment: impl Into<String>) -> Self {
        Self {
            comment: comment.into(),
            bogus: false,
            bang: false,
        }
    }

    /// Creates a bogus comment originating from `<?` or a malformed end tag.
    ///
    /// A leading `?` must already be part of the contents; it selects `<` as
    /// the opening delimiter. Otherwise the opening delimiter is `</`.
    pub fn bogus(comment: impl Into<String>) -> Self {
        Self {
            bogus: true,
            ..Self::new(comment)
        }
    }

    /// Creates a bogus comment originating from `<!`, without that prefix.
    pub fn bang(comment: impl Into<String>) -> Self {
        Self {
            bang: true,
            ..Self::bogus(comment)
        }
    }

    /// Returns the comment contents without any added delimiters.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// Returns whether this is a bogus rather than a normal HTML comment.
    pub fn is_bogus_comment(&self) -> bool {
        self.bogus
    }

    /// Returns whether a bogus comment originated from `<!`.
    pub fn is_bang_comment(&self) -> bool {
        self.bang
    }

    fn write_to(&self, output: &mut String) {
        if !self.bogus {
            output.push_str("<!--");
            output.push_str(&self.comment);
            output.push_str("-->");
        } else {
            // Bogus comments come from "<!", "<?", or "</" followed by a
            // character that cannot start a tag name. The leading '?' is part
            // of the comment in the "<?" case, but '/' is not in the "</" case.
            // Restore it so the output remains a bogus comment: "</<o" must
            // not be serialized as "<<o>".
            if self.bang {
                output.push_str("<!");
            } else if self.comment.starts_with('?') {
                output.push('<');
            } else {
                output.push_str("</");
            }
            output.push_str(&self.comment);
            output.push('>');
        }
    }
}

/// Character data with explicit encoding and tokenizer-context metadata.
///
/// [`Self::new`] matches MimeKit's public constructor: data is already encoded
/// or intentionally raw. [`Self::from_text`] encodes plain text instead.
/// [`Self::from_data_state`] represents raw data from the HTML tokenizer's
/// ordinary data state and protects a trailing literal `<` during serialization.
/// These constructors do not tokenize or verify the supplied context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlDataToken {
    data: HtmlString,
    encode_entities: bool,
    is_data_state: bool,
}

impl HtmlDataToken {
    pub(super) fn from_tokenizer(data: &mut String, encode: bool, data_state: bool) -> Self {
        Self {
            data: HtmlString::take(data),
            encode_entities: encode,
            is_data_state: !encode && data_state,
        }
    }

    /// Creates already-encoded or intentionally raw character data.
    pub fn new(data: impl Into<String>) -> Self {
        Self {
            data: HtmlString::from(data.into()),
            encode_entities: false,
            is_data_state: false,
        }
    }

    /// Creates plain character data to encode with [`super::html_encode`].
    pub fn from_text(data: impl Into<String>) -> Self {
        Self {
            encode_entities: true,
            ..Self::new(data)
        }
    }

    /// Creates raw data-state contents, encoding a trailing literal `<`.
    ///
    /// This is not general escaping: existing references and all other
    /// characters are preserved. It is not appropriate for script/raw-text
    /// contents, where a trailing `<` must remain literal.
    pub fn from_data_state(data: impl Into<String>) -> Self {
        Self {
            is_data_state: true,
            ..Self::new(data)
        }
    }

    /// Returns the stored contents before any serialization-time encoding.
    pub fn data(&self) -> &str {
        &self.data
    }

    fn write_to(&self, output: &mut String) {
        if self.encode_entities {
            encode_into(output, &self.data, None);
        } else if self.is_data_state && self.data.ends_with('<') {
            // A tokenizer data-state token only ends with literal '<' when
            // the following input begins markup a browser treats as text
            // (e.g. the first '<' in "<</>c>"). If that following markup is
            // dropped, writing '<' verbatim could combine with the next token
            // to form a new tag, so encode it.
            output.push_str(&self.data[..self.data.len() - 1]);
            output.push_str("&lt;");
        } else {
            output.push_str(&self.data);
        }
    }
}

/// A complete HTML tag with an immutable name and ordered attributes.
///
/// Unlike [`super::HtmlWriter`]'s name-based operations, token tag names are
/// **not validated**: MimeKit's token constructors preserve arbitrary names.
/// Names must be trusted. Attributes still use [`super::HtmlAttribute`]'s
/// checked constructors; the tokenizer can also preserve malformed names.
/// Empty-element spelling is explicit, not inferred from the recognized name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlTagToken {
    name: std::borrow::Cow<'static, str>,
    attributes: HtmlAttributeCollection,
    id: HtmlTagId,
    empty: bool,
    end: bool,
}

impl HtmlTagToken {
    pub(super) fn from_tokenizer(
        name: &mut String,
        attributes: Vec<super::HtmlAttribute>,
        empty: bool,
        end: bool,
    ) -> Self {
        let (id, name) = HtmlTagId::take_name(name);
        Self {
            name,
            attributes: attributes.into(),
            id,
            empty,
            end,
        }
    }

    /// Creates a start tag closed with `>`, preserving name and attribute order.
    pub fn start(name: impl Into<String>, attributes: impl Into<HtmlAttributeCollection>) -> Self {
        Self::new(name.into(), attributes.into(), false, false)
    }

    /// Creates a self-closing tag spelled with `/>`, irrespective of its name.
    pub fn empty(name: impl Into<String>, attributes: impl Into<HtmlAttributeCollection>) -> Self {
        Self::new(name.into(), attributes.into(), true, false)
    }

    /// Creates an end tag without attributes or a matching-start-tag check.
    pub fn end(name: impl Into<String>) -> Self {
        Self::new(name.into(), HtmlAttributeCollection::new(), false, true)
    }

    fn new(name: String, attributes: HtmlAttributeCollection, empty: bool, end: bool) -> Self {
        Self {
            id: HtmlTagId::from_name(&name),
            name: name.into(),
            attributes,
            empty,
            end,
        }
    }

    /// Returns the original, unvalidated name without changing its spelling.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the name's recognized identifier.
    pub fn id(&self) -> HtmlTagId {
        self.id
    }

    /// Returns attributes in their original order, including duplicate names.
    pub fn attributes(&self) -> &HtmlAttributeCollection {
        &self.attributes
    }

    /// Returns whether this tag is explicitly spelled with `/>`.
    pub fn is_empty_element(&self) -> bool {
        self.empty
    }

    /// Returns whether this is an end tag.
    pub fn is_end_tag(&self) -> bool {
        self.end
    }

    fn write_to(&self, output: &mut String) {
        output.push('<');
        if self.end {
            output.push('/');
        }
        output.push_str(&self.name);
        for (index, attribute) in self.attributes.iter().enumerate() {
            // Give the preceding valueless attribute an explicit empty value
            // so a tokenizer-originated name beginning with '=' cannot be
            // reparsed as its value.
            if index > 0
                && self.attributes[index - 1].value().is_none()
                && attribute.name().starts_with('=')
            {
                output.push_str("=\"\"");
            }
            output.push(' ');
            output.push_str(attribute.name());
            if let Some(value) = attribute.value() {
                output.push_str("=\"");
                encode_into(output, value, Some(HtmlAttributeQuote::Double));
                output.push('"');
            }
        }
        if self.empty {
            output.push('/');
        }
        output.push('>');
    }
}

/// A rejected DOCTYPE edit. All fields remain unchanged on error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlDocTypeError {
    /// A name was empty or contained NUL, HTML whitespace, or `>`.
    InvalidName,
    /// A public identifier contained NUL, `>`, or both quote characters.
    InvalidPublicIdentifier,
    /// A system identifier contained NUL, `>`, or both quote characters.
    InvalidSystemIdentifier,
}

impl fmt::Display for HtmlDocTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid HTML DOCTYPE name",
            Self::InvalidPublicIdentifier => "invalid HTML DOCTYPE public identifier",
            Self::InvalidSystemIdentifier => "invalid HTML DOCTYPE system identifier",
        })
    }
}

impl Error for HtmlDocTypeError {}

/// An HTML document type declaration with checked, optional identifiers.
///
/// Setters retain MimeKit's PUBLIC/SYSTEM keyword transitions. Missing and
/// empty identifiers are distinct; an absent name suppresses both identifiers
/// in the output without deleting their stored values. Identifiers are quoted
/// without entity encoding, choosing a delimiter not present in their contents.
///
/// ```
/// use mimecraft::text::{HtmlDocTypeToken, HtmlToken};
///
/// let mut doctype = HtmlDocTypeToken::new();
/// doctype.set_name(Some("html".into()))?;
/// doctype.set_system_identifier(Some("about:legacy-compat".into()))?;
/// assert_eq!(HtmlToken::DocType(doctype).to_html(),
///            "<!DOCTYPE html SYSTEM \"about:legacy-compat\">");
/// # Ok::<(), mimecraft::text::HtmlDocTypeError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlDocTypeToken {
    // DOCTYPE is rare. Keep its strings out of the inline HtmlToken payload so
    // every data/tag token does not copy space for six unrelated strings.
    pub(super) metadata: Box<DocTypeMetadata>,
    /// Quirks-mode metadata; changing this flag does not change serialization.
    pub force_quirks_mode: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DocTypeMetadata {
    pub(super) raw_tag_name: String,
    name: Option<String>,
    public_identifier: Option<String>,
    system_identifier: Option<String>,
    pub(super) public_keyword: Option<String>,
    pub(super) system_keyword: Option<String>,
    // Preserve original identifier delimiters when supplied by the tokenizer.
    pub(super) public_identifier_quote: HtmlAttributeQuote,
    pub(super) system_identifier_quote: HtmlAttributeQuote,
}

impl Default for HtmlDocTypeToken {
    fn default() -> Self {
        Self {
            metadata: Box::new(DocTypeMetadata {
                raw_tag_name: "DOCTYPE".into(),
                name: None,
                public_identifier: None,
                system_identifier: None,
                public_keyword: None,
                system_keyword: None,
                public_identifier_quote: HtmlAttributeQuote::Double,
                system_identifier_quote: HtmlAttributeQuote::Double,
            }),
            force_quirks_mode: false,
        }
    }
}

impl HtmlDocTypeToken {
    /// Creates an unnamed declaration that serializes as `<!DOCTYPE>`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the document type name, not the `DOCTYPE` keyword.
    pub fn name(&self) -> Option<&str> {
        self.metadata.name.as_deref()
    }

    /// Returns the unencoded public identifier.
    pub fn public_identifier(&self) -> Option<&str> {
        self.metadata.public_identifier.as_deref()
    }

    /// Returns the unencoded system identifier.
    pub fn system_identifier(&self) -> Option<&str> {
        self.metadata.system_identifier.as_deref()
    }

    /// Returns the PUBLIC keyword, retained even after clearing its identifier.
    pub fn public_keyword(&self) -> Option<&str> {
        self.metadata.public_keyword.as_deref()
    }

    /// Returns the SYSTEM keyword, if established by an identifier edit.
    pub fn system_keyword(&self) -> Option<&str> {
        self.metadata.system_keyword.as_deref()
    }

    /// Sets or clears the name, preserving the supplied spelling.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlDocTypeError::InvalidName`] for an empty name or one
    /// containing NUL, TAB, LF, form feed, CR, SPACE, or `>`. Other characters,
    /// including `<`, `/`, quotes, and non-ASCII whitespace, are permitted.
    pub fn set_name(&mut self, value: Option<String>) -> Result<(), HtmlDocTypeError> {
        if value.as_deref().is_some_and(|name| {
            name.is_empty() || name.bytes().any(|byte| b"\0\t\n\x0c\r >".contains(&byte))
        }) {
            return Err(HtmlDocTypeError::InvalidName);
        }
        self.metadata.name = value;
        Ok(())
    }

    /// Sets or clears the public identifier and updates keyword metadata.
    ///
    /// A present value establishes PUBLIC if absent. Clearing it retains PUBLIC
    /// and establishes SYSTEM when a system identifier remains.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlDocTypeError::InvalidPublicIdentifier`] for NUL, `>`, or
    /// a value containing both single and double quotes.
    pub fn set_public_identifier(&mut self, value: Option<String>) -> Result<(), HtmlDocTypeError> {
        if value.as_deref().is_some_and(|id| !is_valid_identifier(id)) {
            return Err(HtmlDocTypeError::InvalidPublicIdentifier);
        }
        if value.is_some() {
            if self.metadata.public_keyword.is_none() {
                self.metadata.public_keyword = Some("PUBLIC".into());
            }
        } else if self.metadata.system_identifier.is_some() {
            self.metadata.system_keyword = Some("SYSTEM".into());
        }
        self.metadata.public_identifier = value;
        Ok(())
    }

    /// Sets or clears the system identifier and updates keyword metadata.
    ///
    /// A present value establishes SYSTEM only without a public identifier and
    /// without an existing SYSTEM keyword. Clearing it clears that keyword.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlDocTypeError::InvalidSystemIdentifier`] for NUL, `>`, or
    /// a value containing both single and double quotes.
    pub fn set_system_identifier(&mut self, value: Option<String>) -> Result<(), HtmlDocTypeError> {
        if value.as_deref().is_some_and(|id| !is_valid_identifier(id)) {
            return Err(HtmlDocTypeError::InvalidSystemIdentifier);
        }
        if value.is_some() {
            if self.metadata.public_identifier.is_none() && self.metadata.system_keyword.is_none() {
                self.metadata.system_keyword = Some("SYSTEM".into());
            }
        } else {
            self.metadata.system_keyword = None;
        }
        self.metadata.system_identifier = value;
        Ok(())
    }

    fn write_to(&self, output: &mut String) {
        self.metadata.write_to(output);
    }
}

impl DocTypeMetadata {
    fn write_to(&self, output: &mut String) {
        output.push_str("<!");
        output.push_str(&self.raw_tag_name);
        // Without a name, a keyword would be parsed as the name, so the
        // identifiers cannot be written.
        if let Some(name) = &self.name {
            output.push(' ');
            output.push_str(name);
            if let Some(id) = &self.public_identifier {
                output.push(' ');
                if let Some(keyword) = &self.public_keyword {
                    output.push_str(keyword);
                }
                write_identifier(output, id, self.public_identifier_quote);
                if let Some(id) = &self.system_identifier {
                    write_identifier(output, id, self.system_identifier_quote);
                }
            } else if let Some(id) = &self.system_identifier {
                output.push(' ');
                if let Some(keyword) = &self.system_keyword {
                    output.push_str(keyword);
                }
                write_identifier(output, id, self.system_identifier_quote);
            }
        }
        output.push('>');
    }
}

fn is_valid_identifier(value: &str) -> bool {
    !value.contains(['>', '\0']) && !(value.contains('\'') && value.contains('"'))
}

fn write_identifier(output: &mut String, identifier: &str, preferred: HtmlAttributeQuote) {
    // An identifier cannot contain the quote character that delimits it.
    // Prefer the original delimiter, falling back to one it does not contain.
    let quote = if preferred == HtmlAttributeQuote::Single && !identifier.contains('\'')
        || identifier.contains('"')
    {
        '\''
    } else {
        '"'
    };
    output.push(' ');
    output.push(quote);
    output.push_str(identifier);
    output.push(quote);
}

#[cfg(test)]
mod tests {
    use super::{HtmlAttributeQuote, HtmlDocTypeToken, HtmlTagToken, HtmlToken};

    #[test]
    fn rare_doctype_metadata_does_not_inflate_every_token() {
        assert!(
            std::mem::size_of::<HtmlToken>()
                <= std::mem::size_of::<HtmlTagToken>() + std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn doctype_preserves_original_keywords_and_usable_quotes() {
        let mut token = HtmlDocTypeToken::new();
        token.metadata.raw_tag_name = "DoCtYpE".into();
        token.metadata.public_keyword = Some("PuBlIc".into());
        token.metadata.system_keyword = Some("SyStEm".into());
        token.metadata.public_identifier_quote = HtmlAttributeQuote::Single;
        token.metadata.system_identifier_quote = HtmlAttributeQuote::Single;
        token.set_name(Some("HTML".into())).unwrap();
        token.set_public_identifier(Some("plain".into())).unwrap();
        token.set_system_identifier(Some("it's".into())).unwrap();
        assert_eq!(
            HtmlToken::DocType(token.clone()).to_html(),
            "<!DoCtYpE HTML PuBlIc 'plain' \"it's\">"
        );
        token.set_public_identifier(Some("it's".into())).unwrap();
        token
            .set_system_identifier(Some("say \"hi\"".into()))
            .unwrap();
        assert_eq!(
            HtmlToken::DocType(token.clone()).to_html(),
            "<!DoCtYpE HTML PuBlIc \"it's\" 'say \"hi\"'>"
        );
        token.set_public_identifier(None).unwrap();
        assert_eq!(token.system_keyword(), Some("SYSTEM"));
        assert_eq!(
            HtmlToken::DocType(token).to_html(),
            "<!DoCtYpE HTML SYSTEM 'say \"hi\"'>"
        );
    }
}

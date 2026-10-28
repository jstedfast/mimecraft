// Adapted from MimeKit's Text/HtmlAttribute.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, error::Error, fmt};

use super::{HtmlAttributeId, html_string::HtmlString, html_utils::is_valid_attribute_name};

/// An error constructing a checked [`HtmlAttribute`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlAttributeError {
    /// The name is empty or contains an attribute-name delimiter.
    InvalidName,
    /// An unknown identifier was supplied instead of an explicit name.
    UnknownId,
}

impl fmt::Display for HtmlAttributeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid HTML attribute name",
            Self::UnknownId => "an unknown HTML attribute requires an explicit name",
        })
    }
}

impl Error for HtmlAttributeError {}

/// An owned, checked HTML attribute with an optional, unescaped value.
///
/// Names and values are immutable. `None` represents a valueless attribute,
/// distinct from `Some(String::new())`, which represents an empty value.
/// Name construction preserves spelling and case, including unknown names.
///
/// Validation uses [`super::HtmlWriter`]'s delimiter checks, not the full HTML
/// grammar. This type is not a sanitizer: names and attribute meanings must be
/// trusted. Values are escaped only when written through the HTML writer.
///
/// ```
/// use mimecraft::text::{HtmlAttribute, HtmlAttributeId};
///
/// let attribute = HtmlAttribute::new("HREF", Some("a&b".into()))?;
/// assert_eq!(attribute.id(), HtmlAttributeId::Href);
/// assert_eq!(attribute.name(), "HREF");
/// assert_eq!(attribute.value(), Some("a&b"));
/// # Ok::<(), mimecraft::text::HtmlAttributeError>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HtmlAttribute {
    name: Cow<'static, str>,
    value: Option<HtmlString>,
    id: HtmlAttributeId,
}

impl HtmlAttribute {
    // Tokenization preserves malformed names for consumers to inspect. Keep
    // this separate from the checked public constructors.
    pub(super) fn from_tokenizer(name: &mut String, value: Option<String>) -> Self {
        let (id, name) = HtmlAttributeId::take_name(name);
        Self {
            id,
            name,
            value: value.map(Into::into),
        }
    }

    pub(super) fn set_tokenizer_value(&mut self, value: &mut String) {
        self.value = Some(HtmlString::take(value));
    }

    pub(super) fn from_tokenizer_str(name: &str, value: Option<&str>) -> Self {
        let (id, canonical) = HtmlAttributeId::canonical_name(name);
        Self {
            id,
            name: canonical.map_or_else(|| Cow::Owned(name.into()), Cow::Borrowed),
            value: value.map(Into::into),
        }
    }

    /// Creates an attribute, preserving the supplied name and unescaped value.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlAttributeError::InvalidName`] for an empty name or one
    /// containing NUL, TAB, CR, LF, form feed, SPACE, `/`, `=`, `>`, quotes, or `<`.
    pub fn new(name: impl Into<String>, value: Option<String>) -> Result<Self, HtmlAttributeError> {
        let name = name.into();
        if !is_valid_attribute_name(&name) {
            return Err(HtmlAttributeError::InvalidName);
        }
        Ok(Self {
            // Immutable names let us resolve the identifier once instead of
            // maintaining MimeKit's lazy, sentinel-based identifier cache.
            id: HtmlAttributeId::from_name(&name),
            name: name.into(),
            value: value.map(Into::into),
        })
    }

    /// Creates an attribute using its identifier's canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns [`HtmlAttributeError::UnknownId`] for [`HtmlAttributeId::Unknown`].
    /// Use [`Self::new`] to construct an attribute with an unrecognized name.
    pub fn from_id(id: HtmlAttributeId, value: Option<String>) -> Result<Self, HtmlAttributeError> {
        if id == HtmlAttributeId::Unknown {
            return Err(HtmlAttributeError::UnknownId);
        }
        Ok(Self {
            name: Cow::Borrowed(id.as_str()),
            value: value.map(Into::into),
            id,
        })
    }

    /// Returns the recognized identifier without changing the original name.
    pub fn id(&self) -> HtmlAttributeId {
        self.id
    }

    /// Returns the original name, including its spelling and case.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the unescaped value, or `None` for a valueless attribute.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

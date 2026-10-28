// Adapted from MimeKit's Text/HtmlNamespace.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// A recognized HTML, foreign-content, or attribute namespace.
///
/// Namespace identifiers do not validate markup or change a token's name.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum HtmlNamespace {
    /// HTML elements.
    #[default]
    Html,
    /// Mathematical Markup Language elements.
    MathMl,
    /// Scalable Vector Graphics elements.
    Svg,
    /// XLink attributes.
    XLink,
    /// XML attributes.
    Xml,
    /// Namespace declaration attributes.
    XmlNs,
}

impl HtmlNamespace {
    /// Returns the canonical namespace URL.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Html => "http://www.w3.org/1999/xhtml",
            Self::MathMl => "http://www.w3.org/1998/Math/MathML",
            Self::Svg => "http://www.w3.org/2000/svg",
            Self::XLink => "http://www.w3.org/1999/xlink",
            Self::Xml => "http://www.w3.org/XML/1998/namespace",
            Self::XmlNs => "http://www.w3.org/2000/xmlns/",
        }
    }

    /// Recognizes a namespace URL using ASCII case-insensitive comparison.
    ///
    /// Does not trim, resolve, or normalize URLs. Unlike MimeKit's helper,
    /// unknown URLs return `None` rather than silently selecting HTML.
    pub fn from_url(url: &str) -> Option<Self> {
        [
            Self::Html,
            Self::MathMl,
            Self::Svg,
            Self::XLink,
            Self::Xml,
            Self::XmlNs,
        ]
        .into_iter()
        .find(|ns| ns.as_str().eq_ignore_ascii_case(url))
    }
}

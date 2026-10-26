// Adapted from MimeKit's BodyBuilder.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    MimeEntity, Multipart, MultipartAlternative, MultipartAlternativeError, MultipartError,
    MultipartRelated, MultipartRelatedError, TextPart, TextPartError, charsets::Charset,
};

/// Assembles common message bodies from text and existing in-memory entities.
///
/// Plain text precedes HTML in multipart/alternative. Linked resources accompany
/// HTML in multipart/related; attachments follow the body in multipart/mixed.
/// Construction consumes the builder and moves entities rather than cloning
/// their headers or content. File-loading convenience methods are not provided.
///
/// ```
/// use mimecraft::{BodyBuilder, MimeMessage, TextFormat};
///
/// let builder = BodyBuilder {
///     text_body: Some("hello".into()),
///     html_body: Some("<p>hello</p>".into()),
///     ..BodyBuilder::new()
/// };
/// let mut message = MimeMessage::with_body(builder.into_message_body()?);
/// assert_eq!(message.text_body(TextFormat::Plain)?.as_deref(), Some("hello"));
/// assert_eq!(message.text_body(TextFormat::Html)?.as_deref(), Some("<p>hello</p>"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct BodyBuilder {
    /// Plain-text content, or `None` to omit this alternative.
    ///
    /// An empty string still creates a text/plain part.
    pub text_body: Option<String>,
    /// HTML content, or `None` to omit this alternative and all linked resources.
    ///
    /// Markup, newlines, and resource references are retained literally.
    pub html_body: Option<String>,
    /// Charset used for both text bodies, defaulting to UTF-8.
    ///
    /// Encoding is strict: unmappable characters fail instead of being replaced.
    pub body_encoding: Charset,
    /// Existing entities to append after the body, in insertion order.
    ///
    /// The builder does not assign dispositions, filenames, or transfer encodings.
    pub attachments: Vec<MimeEntity>,
    /// Existing resources to append after HTML in a related container.
    ///
    /// Assign Content-Id or Content-Location headers before building and use the
    /// corresponding references in HTML. No IDs or resource headers are generated
    /// or modified. Resources are included only when [`Self::html_body`] is `Some`.
    pub linked_resources: Vec<MimeEntity>,
}

impl Default for BodyBuilder {
    fn default() -> Self {
        Self {
            text_body: None,
            html_body: None,
            body_encoding: Charset::UTF8,
            attachments: Vec::new(),
            linked_resources: Vec::new(),
        }
    }
}

impl BodyBuilder {
    /// Creates an empty builder with UTF-8 text encoding.
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes the builder and constructs an in-memory MIME body.
    ///
    /// Adapts MimeKit's `ToMessageBody` using ownership rather than sharing
    /// mutable attachments. A lone attachment with no text is returned directly.
    /// Without text or attachments, returns empty text/plain in the selected
    /// charset. `Some("")` counts as a supplied body. Linked resources without
    /// HTML are ignored and dropped with the consumed builder.
    ///
    /// All required multipart boundaries are generated. Related HTML is the
    /// first child, with `type=text/html` and no `start` parameter. Supplied
    /// entities retain their headers, encoded bytes, framing, and source
    /// positions; no attachment content is read. No HTML conversion, resource
    /// discovery, file loading, or transport preparation is performed.
    ///
    /// # Errors
    ///
    /// Returns text-encoding, header-formatting, or multipart-construction
    /// errors, including OS-randomness failures. The consumed builder and its
    /// entities are dropped on failure; no partial body is returned.
    pub fn into_message_body(mut self) -> Result<MimeEntity, BodyBuilderError> {
        let plain = self
            .text_body
            .as_deref()
            .map(|text| TextPart::with_charset("plain", text, self.body_encoding))
            .transpose()
            .map_err(BodyBuilderError::Text)?
            .map(MimeEntity::from);

        let html = if let Some(text) = self.html_body.as_deref() {
            let text = TextPart::with_charset("html", text, self.body_encoding)
                .map_err(BodyBuilderError::Text)?;
            if self.linked_resources.is_empty() {
                Some(text.into())
            } else {
                let mut related = MultipartRelated::new().map_err(BodyBuilderError::Related)?;
                related
                    .set_root(text.into())
                    .map_err(BodyBuilderError::Related)?;
                related
                    .multipart_mut()
                    .children_mut()
                    .extend(self.linked_resources);
                Some(related.into())
            }
        } else {
            None
        };

        let mut body = match (plain, html) {
            (Some(plain), Some(html)) => {
                let mut alternative =
                    MultipartAlternative::new().map_err(BodyBuilderError::Alternative)?;
                alternative
                    .multipart_mut()
                    .children_mut()
                    .extend([plain, html]);
                Some(alternative.into())
            }
            (plain, html) => plain.or(html),
        };

        if !self.attachments.is_empty() {
            if body.is_none() && self.attachments.len() == 1 {
                return Ok(self.attachments.remove(0));
            }
            let mut mixed = Multipart::new("mixed").map_err(BodyBuilderError::Multipart)?;
            mixed.children_mut().extend(body);
            mixed.children_mut().extend(self.attachments);
            body = Some(mixed.into());
        }

        match body {
            Some(body) => Ok(body),
            None => TextPart::with_charset("plain", "", self.body_encoding)
                .map(MimeEntity::from)
                .map_err(BodyBuilderError::Text),
        }
    }
}

/// A common message body could not be assembled.
#[derive(Debug)]
pub enum BodyBuilderError {
    /// Text could not be encoded or its headers could not be formatted.
    Text(TextPartError),
    /// A mixed container could not be created.
    Multipart(MultipartError),
    /// An alternative container could not be created.
    Alternative(MultipartAlternativeError),
    /// A related container or its HTML root could not be created.
    Related(MultipartRelatedError),
}

impl fmt::Display for BodyBuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(error) => error.fmt(f),
            Self::Multipart(error) => error.fmt(f),
            Self::Alternative(error) => error.fmt(f),
            Self::Related(error) => error.fmt(f),
        }
    }
}

impl Error for BodyBuilderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Text(error) => Some(error),
            Self::Multipart(error) => Some(error),
            Self::Alternative(error) => Some(error),
            Self::Related(error) => Some(error),
        }
    }
}

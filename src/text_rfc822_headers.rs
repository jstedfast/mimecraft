// Adapted from MimeKit's TextRfc822Headers.cs and ParserOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, io::Cursor};

use crate::{ContentHeaderParseError, ContentType, Header, MessagePart, MimeMessage};

/// A structured text/rfc822-headers or message/global-headers container.
///
/// Composes [`MessagePart`] rather than inheriting from it. Construction creates
/// text/rfc822-headers; [`Self::from_message_part`] also accepts the global type.
/// The contained message is optional and retains ownership of its headers and
/// body. Like MimeKit's MessagePart-based implementation, this model does not
/// strip an existing body or enforce a headers-only payload. Supply a message
/// with no body when constructing a returned-header part.
///
/// Loading and conversion use [`crate::MimeEntity::Message`]. Transparent
/// encodings are parsed structurally by default; encoded wrappers or disabled
/// [`crate::MimeEntityParseOptions::parse_message_parts`] remain opaque leaves.
/// The existing strict message parser requires an inner header/body separator
/// for a present message, even when no body bytes follow.
///
/// Metadata, contained-message edits, traversal, and writing are available
/// through [`Self::message_part`] and [`Self::message_part_mut`]. No source I/O,
/// payload copying, or implicit metadata generation occurs during construction
/// or wrapping. Ownership access does not require readable child sources.
///
/// ```
/// use mimecraft::{Header, MimeEntity, MimeMessage, TextRfc822Headers};
///
/// let mut message: MimeMessage = MimeMessage::new();
/// message.headers_mut().add(Header::parse(b"Subject: Original message")?);
/// let returned = TextRfc822Headers::with_message(message);
/// let mut entity: MimeEntity = returned.into();
/// let mut wire = Vec::new();
/// entity.write_to(&mut wire)?;
/// assert!(wire.starts_with(b"Content-Type: text/rfc822-headers\r\n\r\n"));
/// let MimeEntity::Message(part) = MimeEntity::load(&mut wire.as_slice())? else {
///     unreachable!()
/// };
/// let returned = TextRfc822Headers::from_message_part(part)?;
/// assert!(returned.message_part().message().unwrap().headers().contains("Subject"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct TextRfc822Headers<R = Cursor<Vec<u8>>> {
    part: MessagePart<R>,
}

impl<R> TextRfc822Headers<R> {
    /// Creates text/rfc822-headers with no contained message.
    ///
    /// Adds only Content-Type; no disposition, transfer encoding, or inner
    /// message headers are generated.
    pub fn new() -> Self {
        let header = Header::from_formatted("Content-Type: text/rfc822-headers\r\n".into());
        Self {
            part: MessagePart::from_parts(vec![header].into(), None),
        }
    }

    /// Takes a message without copying, traversing, or removing its body.
    pub fn with_message(message: MimeMessage<R>) -> Self {
        let mut headers = Self::new();
        headers.part.set_message(Some(message));
        headers
    }

    /// Adopts a structured message part after checking its first Content-Type.
    ///
    /// Accepts text/rfc822-headers and message/global-headers ignoring ASCII
    /// case. Does not validate transfer encoding or the contained message,
    /// repair headers, or perform source I/O. Later duplicates cannot substitute
    /// for a missing or invalid first value.
    ///
    /// # Errors
    ///
    /// Returns missing, malformed, or other media-type errors. Failure drops
    /// the owned part; borrowed sources remain open and unrepositioned.
    pub fn from_message_part(part: MessagePart<R>) -> Result<Self, TextRfc822HeadersError> {
        let content_type = part
            .content_type()
            .map_err(TextRfc822HeadersError::ContentType)?
            .ok_or(TextRfc822HeadersError::MissingContentType)?;
        if !is_headers_type(&content_type) {
            return Err(TextRfc822HeadersError::NotHeaders(content_type.mime_type()));
        }
        Ok(Self { part })
    }

    /// Returns the structured message wrapper, including its raw headers.
    pub fn message_part(&self) -> &MessagePart<R> {
        &self.part
    }

    /// Edits the underlying message wrapper without imposing headers-only rules.
    ///
    /// Raw edits remain authoritative. The ordinary MessagePart writer checks
    /// its supported media types and transparent encodings when serializing.
    pub fn message_part_mut(&mut self) -> &mut MessagePart<R> {
        &mut self.part
    }

    /// Extracts the message wrapper without copying or accessing child sources.
    pub fn into_message_part(self) -> MessagePart<R> {
        self.part
    }
}

impl<R> Default for TextRfc822Headers<R> {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn is_headers_type(value: &ContentType) -> bool {
    value.is_mime_type("text", "rfc822-headers") || value.is_mime_type("message", "global-headers")
}

/// A structured returned-header container could not be wrapped.
#[derive(Debug)]
pub enum TextRfc822HeadersError {
    /// The first Content-Type is absent.
    MissingContentType,
    /// Content-Type names another media type.
    NotHeaders(String),
    /// The first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
}

impl fmt::Display for TextRfc822HeadersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("returned-header part has no Content-Type"),
            Self::NotHeaders(value) => {
                write!(
                    f,
                    "expected text/rfc822-headers or message/global-headers, got {value}"
                )
            }
            Self::ContentType(error) => error.fmt(f),
        }
    }
}

impl Error for TextRfc822HeadersError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            _ => None,
        }
    }
}

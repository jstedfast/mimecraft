// Adapted from MimeKit's MessagePart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor},
    slice,
};

use crate::{
    ContentEncoding, ContentHeaderParseError, ContentType, Header, HeaderFormatError,
    HeaderFormatOptions, HeaderList, LeafParts, MimeMessage, ParameterParseOptions,
};

/// A MIME entity containing an optional structured message.
///
/// The wrapper's headers describe the attached message, not its body. Message
/// headers and body Content-* headers remain on the contained [`MimeMessage`].
/// Ownership replaces parent pointers and disposal. An internal box breaks
/// the recursive message/entity layout; public accessors expose ordinary
/// references and owned messages without copying payloads.
///
/// [`Self::new`] constructs message/rfc822. Other message subtypes can be
/// supplied explicitly, but subtype-specific grammars are not validated.
/// [`crate::TextRfc822Headers`] composes this model for text/rfc822-headers and
/// message/global-headers; checked edits and writing accept that text type too.
/// Use ordinary [`crate::MimePart`] leaves for message payloads that are not
/// complete messages, such as delivery-status fields or partial fragments.
///
/// Writing and leaf traversal descend through attached messages iteratively.
/// Only transparent wrapper transfer encodings are supported by the writer;
/// encoded wrappers must remain opaque leaves for now, including encoded
/// message/global. [`crate::MimeEntity::load`] constructs this variant for
/// recognized, transparently encoded message types by default. Set
/// [`crate::MimeEntityParseOptions::parse_message_parts`] to false to retain
/// all message parts as opaque leaves instead.
///
/// Untyped children of multipart/digest are implicitly message/rfc822. Loading
/// retains their absent Content-Type header, and writing through that digest
/// parent needs no synthesized header. Add an explicit message Content-Type
/// before writing such a part independently or under a different container.
///
/// ```
/// use mimecraft::{MessagePart, MimeEntity, MimeMessage, TextPart};
///
/// let message = MimeMessage::with_body(TextPart::new("plain", "attached")?.into());
/// let mut attachment = MessagePart::new(message);
/// assert_eq!(attachment.leaf_parts().count(), 1);
/// let mut output = Vec::new();
/// attachment.write_to(&mut output)?;
/// let MimeEntity::Message(parsed) = MimeEntity::load(&mut output.as_slice())? else {
///     unreachable!()
/// };
/// assert_eq!(parsed.message().unwrap().leaf_parts().count(), 1);
/// assert!(output.starts_with(b"Content-Type: message/rfc822\r\n\r\n"));
/// let entity: MimeEntity = attachment.into();
/// assert!(matches!(entity, MimeEntity::Message(_)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MessagePart<R = Cursor<Vec<u8>>> {
    headers: HeaderList,
    message: Option<Box<MimeMessage<R>>>,
}

impl<R> MessagePart<R> {
    /// Takes a message with a message/rfc822 wrapper and no source I/O.
    ///
    /// Does not generate Content-Disposition, a filename, or transfer encoding.
    pub fn new(message: MimeMessage<R>) -> Self {
        let header = Header::from_formatted("Content-Type: message/rfc822\r\n".into());
        Self::from_parts(vec![header].into(), Some(message))
    }

    /// Takes a message with an explicitly selected message subtype.
    ///
    /// Uses default header formatting and [`ContentType::new`]'s compatibility
    /// component rules, including extra slashes in subtypes. This is not strict
    /// subtype-specific validation or transport preparation.
    ///
    /// # Errors
    ///
    /// Returns subtype or formatting errors. Failure drops the supplied owned
    /// message; borrowed sources are not closed or repositioned.
    pub fn with_subtype(subtype: &str, message: MimeMessage<R>) -> Result<Self, MessagePartError> {
        let value = ContentType::new("message", subtype).map_err(MessagePartError::ContentType)?;
        let header = value.to_header().map_err(MessagePartError::HeaderFormat)?;
        Ok(Self::from_parts(vec![header].into(), Some(message)))
    }

    /// Takes raw headers and an optional message without validation or source I/O.
    ///
    /// Missing, invalid, and duplicate metadata is preserved without defaults.
    /// An absent message is distinct from a present message with no body.
    pub fn from_parts(headers: HeaderList, message: Option<MimeMessage<R>>) -> Self {
        Self {
            headers,
            message: message.map(Box::new),
        }
    }

    /// Returns the wrapper's raw headers, excluding the contained message.
    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    /// Edits wrapper metadata without changing or reinterpreting the message.
    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    /// Returns the attached message, if present.
    pub fn message(&self) -> Option<&MimeMessage<R>> {
        self.message.as_deref()
    }

    /// Allows edits to the attached message without changing wrapper headers.
    pub fn message_mut(&mut self) -> Option<&mut MimeMessage<R>> {
        self.message.as_deref_mut()
    }

    /// Replaces or removes the message, returning the old value without source I/O.
    pub fn set_message(&mut self, message: Option<MimeMessage<R>>) -> Option<MimeMessage<R>> {
        std::mem::replace(&mut self.message, message.map(Box::new)).map(|message| *message)
    }

    /// Extracts headers and the message without copying payloads or source I/O.
    pub fn into_parts(self) -> (HeaderList, Option<MimeMessage<R>>) {
        (self.headers, self.message.map(|message| *message))
    }

    /// Visits body leaves, including those inside further attached messages.
    ///
    /// Absent messages and bodies yield no leaves. Traversal is iterative and
    /// performs no source I/O. Ordinary model drop and derived debugging recurse.
    pub fn leaf_parts(&self) -> LeafParts<'_, R> {
        let body = self.message().and_then(|message| message.body());
        LeafParts::new(body.map_or(&[], slice::from_ref))
    }

    /// Parses the first Content-Type, returning `None` when absent.
    ///
    /// Returns a detached value without asserting that it describes a message.
    /// An implicit multipart/digest child still returns `None`: this accessor
    /// describes the raw header, not a parent-dependent effective content type.
    ///
    /// # Errors
    ///
    /// Returns the first header's parse error; later duplicates do not override it.
    pub fn content_type(&self) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.content_type_with_options(&ParameterParseOptions::default())
    }

    /// Parses the first Content-Type with explicit parameter parsing options.
    ///
    /// # Errors
    ///
    /// Returns syntax errors with offsets relative to the raw header value.
    pub fn content_type_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.headers
            .get("Content-Type")
            .map(|header| header.parse_content_type_with_options(options))
            .transpose()
    }

    /// Replaces Content-Type with a message type or text/rfc822-headers.
    ///
    /// Duplicate collapse retains the first field's slot; unrelated headers
    /// and the contained message are unchanged.
    ///
    /// # Errors
    ///
    /// Rejects unsupported media types and formatting failures before mutation.
    pub fn set_content_type(&mut self, value: &ContentType) -> Result<(), MessagePartError> {
        self.set_content_type_with_options(value, &HeaderFormatOptions::default())
    }

    /// Replaces Content-Type with explicit formatting options.
    ///
    /// # Errors
    ///
    /// Returns validation or formatting errors, leaving the wrapper unchanged.
    pub fn set_content_type_with_options(
        &mut self,
        value: &ContentType,
        options: &HeaderFormatOptions,
    ) -> Result<(), MessagePartError> {
        check_message_type(value)?;
        let header = value
            .to_header_with_options(options)
            .map_err(MessagePartError::HeaderFormat)?;
        self.headers
            .replace_all(header)
            .expect("typed formatters always produce a header with a parsed field name");
        Ok(())
    }
}

pub(crate) fn check_message_part<R>(part: &MessagePart<R>, digest_child: bool) -> io::Result<()> {
    let value = part.content_type().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            MessagePartError::ContentType(error),
        )
    })?;
    if let Some(value) = value {
        check_message_type(&value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    } else if !digest_child {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            MessagePartError::MissingContentType,
        ));
    }
    let encoding = crate::mime_part::transfer_encoding(part.headers())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !matches!(
        encoding,
        ContentEncoding::Default
            | ContentEncoding::SevenBit
            | ContentEncoding::EightBit
            | ContentEncoding::Binary
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "structured message transfer encoding must be 7bit, 8bit, or binary; use an opaque leaf for encoded messages",
        ));
    }
    Ok(())
}

fn check_message_type(value: &ContentType) -> Result<(), MessagePartError> {
    // MimeKit's TextRfc822Headers derives from MessagePart despite its text
    // media type. Accept the same type in metadata edits and tree writing.
    if value.is_mime_type("message", "*") || crate::text_rfc822_headers::is_headers_type(value) {
        Ok(())
    } else {
        Err(MessagePartError::NotMessage(value.mime_type()))
    }
}

/// A message wrapper's Content-Type could not be constructed, read, or edited.
#[derive(Debug)]
pub enum MessagePartError {
    /// The operation requires a Content-Type that is absent.
    MissingContentType,
    /// Content-Type is neither message/* nor text/rfc822-headers.
    NotMessage(String),
    /// Subtype construction or raw Content-Type parsing failed.
    ContentType(ContentHeaderParseError),
    /// Checked Content-Type formatting failed.
    HeaderFormat(HeaderFormatError),
}

impl fmt::Display for MessagePartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("message part has no Content-Type header"),
            Self::NotMessage(media_type) => {
                write!(
                    f,
                    "expected message/* or text/rfc822-headers, got {media_type}"
                )
            }
            Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
        }
    }
}

impl Error for MessagePartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            _ => None,
        }
    }
}

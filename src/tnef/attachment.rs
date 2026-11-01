// Adapted from MimeKit's TnefAttachment.cs, TnefAttachMethod.cs, and TnefAttachFlags.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, io::Cursor, ops};

use super::{
    TnefEmbeddedContent, TnefPropertySet, TnefPropertyTag as Tag, TnefReadError, TnefReader,
    TnefValueConversionError,
};

/// The storage method of a TNEF attachment, retaining unrecognized values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TnefAttachMethod {
    /// No attachment method specified.
    None,
    /// A binary blob, normally stored in the legacy AttachData attribute.
    ByValue,
    /// An embedded TNEF message stored in an AttachData property.
    EmbeddedMessage,
    /// An OLE stream stored in an AttachData property.
    Ole,
    /// An unrecognized method, preserved without interpreting it.
    Unknown(i32),
}

impl From<i32> for TnefAttachMethod {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::ByValue,
            5 => Self::EmbeddedMessage,
            6 => Self::Ole,
            value => Self::Unknown(value),
        }
    }
}

impl From<TnefAttachMethod> for i32 {
    fn from(value: TnefAttachMethod) -> Self {
        match value {
            TnefAttachMethod::None => 0,
            TnefAttachMethod::ByValue => 1,
            TnefAttachMethod::EmbeddedMessage => 5,
            TnefAttachMethod::Ole => 6,
            TnefAttachMethod::Unknown(value) => value,
        }
    }
}

/// Attachment rendering flags. Unknown bits are preserved.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TnefAttachFlags(u32);

impl TnefAttachFlags {
    /// No attachment flags.
    pub const NONE: Self = Self(0);
    /// The attachment is invisible in HTML bodies.
    pub const INVISIBLE_IN_HTML: Self = Self(1);
    /// The attachment is invisible in RTF bodies.
    pub const INVISIBLE_IN_RTF: Self = Self(2);
    /// The attachment is referenced and rendered by the HTML body.
    pub const RENDERED_IN_BODY: Self = Self(4);

    /// All original bits, including unknown high-order bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether all specified flags are set.
    pub const fn contains(self, flags: Self) -> bool {
        self.0 & flags.0 == flags.0
    }

    /// Whether any specified flag is set.
    pub const fn intersects(self, flags: Self) -> bool {
        self.0 & flags.0 != 0
    }
}

impl From<u32> for TnefAttachFlags {
    fn from(bits: u32) -> Self {
        Self(bits)
    }
}

impl From<TnefAttachFlags> for u32 {
    fn from(flags: TnefAttachFlags) -> Self {
        flags.bits()
    }
}

impl ops::BitOr for TnefAttachFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl ops::BitAnd for TnefAttachFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Owned attachment content, distinct from an absent content value.
///
/// Reader collection is bounded by existing owned-byte limits. Directly supplied
/// buffers have no additional limits. Keep the variant when moving content:
/// Object retains its IID internally, while Embedded also retains reader context.
#[derive(Clone, Debug)]
pub enum TnefAttachmentContent {
    /// Raw Binary property or legacy AttachData bytes, without any stripping.
    Binary(Vec<u8>),
    /// Raw Object property bytes, including the leading interface IID.
    /// This variant does not infer embedded-message context from the IID.
    Object(Vec<u8>),
    /// A recognized embedded message collected with its original reader context.
    Embedded(TnefEmbeddedContent),
}

impl TnefAttachmentContent {
    /// Attachment bytes without an Object/embedded-message IID.
    ///
    /// A truncated Object shorter than its IID exposes empty content, not padding
    /// or a fabricated message. Binary values retain every byte.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Binary(bytes) => bytes,
            // The 16-byte interface identifier is not attachment content.
            Self::Object(bytes) => &bytes[bytes.len().min(16)..],
            Self::Embedded(content) => content.as_bytes(),
        }
    }
}

/// An owned attachment with checked metadata and repeatably readable content.
///
/// Construction moves the supplied properties and optional content without
/// allocating or validating fields. The properties are retained verbatim:
/// callers assembling an attachment should collect AttachData separately.
/// This model does not partition an entire stream into attachments, synthesize
/// legacy metadata, or choose between duplicate/legacy/MAPI content.
///
/// Metadata getters use the same checked conversions as [`TnefPropertySet`].
/// Only absent properties permit defaults or fallback; empty strings are values,
/// and malformed or incompatible properties return errors.
///
/// ```
/// use mimecraft::tnef::{TnefAttachment, TnefAttachmentContent, TnefPropertySet};
///
/// let attachment = TnefAttachment::new(
///     TnefPropertySet::new(),
///     Some(TnefAttachmentContent::Binary(b"content".to_vec())),
/// );
/// assert!(attachment.has_content());
/// assert_eq!(attachment.as_bytes(), Some(b"content".as_slice()));
/// assert_eq!(attachment.file_name()?, None);
/// # Ok::<(), mimecraft::tnef::TnefValueConversionError>(())
/// ```
#[derive(Clone, Debug)]
pub struct TnefAttachment {
    properties: TnefPropertySet,
    content: Option<TnefAttachmentContent>,
}

impl TnefAttachment {
    /// Moves already-collected properties and content into an attachment.
    /// `None` means absent content; `Some` may hold an empty value.
    /// Collection limits/errors must be handled by the caller, not converted
    /// silently into absent content.
    pub fn new(properties: TnefPropertySet, content: Option<TnefAttachmentContent>) -> Self {
        Self {
            properties,
            content,
        }
    }

    /// Original properties, including duplicates, names, and captured charsets.
    pub fn properties(&self) -> &TnefPropertySet {
        &self.properties
    }

    /// Original content, retaining its wire representation and embedded context.
    pub fn content(&self) -> Option<&TnefAttachmentContent> {
        self.content.as_ref()
    }

    /// Recovers both owned components without cloning or allocating.
    pub fn into_parts(self) -> (TnefPropertySet, Option<TnefAttachmentContent>) {
        (self.properties, self.content)
    }

    /// Gets AttachMethod, defaulting to EmbeddedMessage for recognized embedded
    /// content and ByValue otherwise. A stored method wins even if contradictory.
    ///
    /// # Errors
    ///
    /// Returns a present property's integer-conversion error instead of defaulting.
    pub fn method(&self) -> Result<TnefAttachMethod, TnefValueConversionError> {
        Ok(match self.properties.get_i32(Tag::ATTACH_METHOD)? {
            Some(value) => value.into(),
            None if self.is_embedded_message() => TnefAttachMethod::EmbeddedMessage,
            None => TnefAttachMethod::ByValue,
        })
    }

    /// Whether recognized embedded content can open a detached TNEF reader.
    /// The stored method alone does not establish that context.
    pub fn is_embedded_message(&self) -> bool {
        matches!(self.content, Some(TnefAttachmentContent::Embedded(_)))
    }

    /// Gets AttachLongFilename, falling back to AttachFilename only if absent.
    /// The name is untrusted and must be sanitized before creating a file.
    ///
    /// # Errors
    ///
    /// Returns the first present property's text-conversion error.
    pub fn file_name(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        if let Some(name) = self.properties.get_text(Tag::ATTACH_LONG_FILENAME_W)? {
            return Ok(Some(name));
        }
        self.properties.get_text(Tag::ATTACH_FILENAME_W)
    }

    /// Gets the unparsed AttachMimeTag, if present.
    ///
    /// # Errors
    ///
    /// Returns the property's text-conversion error.
    pub fn mime_type(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.properties.get_text(Tag::ATTACH_MIME_TAG_W)
    }

    /// Gets AttachContentId exactly as stored, without normalization or parsing.
    ///
    /// # Errors
    ///
    /// Returns the property's text-conversion error.
    pub fn content_id(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.properties.get_text(Tag::ATTACH_CONTENT_ID_W)
    }

    /// Gets AttachFlags, preserving unknown bits and defaulting to NONE only
    /// when absent.
    ///
    /// # Errors
    ///
    /// Returns a present property's integer-conversion error.
    pub fn flags(&self) -> Result<TnefAttachFlags, TnefValueConversionError> {
        Ok(TnefAttachFlags::from(
            self.properties.get_i32(Tag::ATTACH_FLAGS)?.unwrap_or(0) as u32,
        ))
    }

    /// Whether a content value exists, even if empty.
    pub fn has_content(&self) -> bool {
        self.content.is_some()
    }

    /// Content bytes, excluding interface IIDs; `None` distinguishes absence.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.content.as_ref().map(TnefAttachmentContent::as_bytes)
    }

    /// Content length excluding interface IIDs; zero for absent content.
    pub fn len(&self) -> usize {
        self.as_bytes().map_or(0, <[u8]>::len)
    }

    /// Whether content is absent or has zero exposed bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Opens an independent seekable cursor; absent content yields an empty one.
    pub fn open_read(&self) -> Cursor<&[u8]> {
        Cursor::new(self.as_bytes().unwrap_or_default())
    }

    /// Opens a repeatable detached reader with captured offsets, depth, charset,
    /// and options, but fresh budgets and fault state. Supply diagnostics through
    /// the reader's `*_with_logger` operations; no parent logger is retained.
    /// Signature and checksums are validated when their readers advance.
    ///
    /// Use [`load_embedded_message`](Self::load_embedded_message) to load its model.
    ///
    /// # Errors
    ///
    /// Returns [`TnefReadError::NotEmbeddedMessage`] without captured embedded
    /// context, or an error from [`TnefEmbeddedContent::open_reader`].
    pub fn open_embedded_reader(&self) -> Result<TnefReader<&[u8]>, TnefReadError> {
        match &self.content {
            Some(TnefAttachmentContent::Embedded(content)) => content.open_reader(),
            _ => Err(TnefReadError::NotEmbeddedMessage),
        }
    }

    /// Loads the captured embedded message with fresh inherited reader budgets.
    /// Repeated calls are independent. Diagnostics are discarded; use the logger
    /// variant to retain them. Nested attachments are not recursively loaded.
    ///
    /// # Errors
    ///
    /// Returns selection, header, I/O, conversion, or resource-limit errors.
    pub fn load_embedded_message(&self) -> Result<super::TnefMessage, TnefReadError> {
        self.open_embedded_reader()?.load_message()
    }

    /// Loads the embedded message, reporting diagnostics to the supplied logger.
    /// No former parent logger is retained.
    ///
    /// # Errors
    ///
    /// Shares [`load_embedded_message`](Self::load_embedded_message) errors and
    /// propagates logger errors.
    pub fn load_embedded_message_with_logger(
        &self,
        logger: &mut dyn super::TnefComplianceLogger,
    ) -> Result<super::TnefMessage, TnefReadError> {
        self.open_embedded_reader()?
            .load_message_with_logger(logger)
    }
}

// Adapted from MimeKit's TnefMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, io::Read};

use super::{
    TnefAttachment, TnefMessageBody, TnefPropertySet, TnefPropertyTag as Tag, TnefReadError,
    TnefReader, TnefRecipient, TnefValueConversionError,
};

/// An owned TNEF message with properties, recipients, bodies, and attachments.
///
/// Legacy attributes are mapped to MAPI properties; valued MAPI properties take
/// precedence. Supported bodies and attachment data are held separately from
/// metadata. Recipients and attachments retain stream order.
///
/// Loading buffers content under the reader's byte, depth, and attachment
/// limits. Exceeding a limit returns an error, not a partially loaded message.
/// Malformed framing otherwise follows the reader's logged recovery rules.
/// Bodies and embedded messages are not decoded or recursively loaded until
/// requested. Rust ownership releases buffers without an explicit dispose call.
///
/// ```
/// use mimecraft::tnef::TnefMessage;
/// let message = TnefMessage::load(b"\x78\x9f\x3e\x22\x34\x12".as_slice())?;
/// assert_eq!(message.legacy_key(), 0x1234);
/// assert!(message.attachments().is_empty());
/// # Ok::<(), mimecraft::tnef::TnefReadError>(())
/// ```
#[derive(Clone, Debug)]
pub struct TnefMessage {
    pub(super) codepage: u32,
    pub(super) legacy_key: u16,
    pub(super) properties: TnefPropertySet,
    pub(super) recipients: Vec<TnefRecipient>,
    pub(super) attachments: Vec<TnefAttachment>,
    pub(super) text_body: Option<TnefMessageBody>,
    pub(super) html_body: Option<TnefMessageBody>,
    pub(super) rtf_body: Option<TnefMessageBody>,
}

impl TnefMessage {
    /// Loads a stream using default reader options and discarding diagnostics.
    /// Pass `&mut source` to retain ownership of the source. For custom limits
    /// or diagnostics, use [`TnefReader::load_message`] or its logger variant.
    ///
    /// # Errors
    ///
    /// Returns header, I/O, assembly conversion, and resource-limit errors.
    pub fn load<R: Read>(source: R) -> Result<Self, TnefReadError> {
        TnefReader::new(source).load_message()
    }

    /// Final message codepage. Earlier metadata keeps its captured decoding;
    /// eight-bit bodies use this final codepage even if declared after the body.
    pub fn codepage(&self) -> u32 {
        self.codepage
    }

    /// Header key used to match attachments to legacy RTF placeholders.
    pub fn legacy_key(&self) -> u16 {
        self.legacy_key
    }

    /// MAPI properties and synthesized legacy properties. Supported bodies
    /// with values are excluded; unsupported/valueless body properties remain.
    pub fn properties(&self) -> &TnefPropertySet {
        &self.properties
    }

    /// Recipients in original row order.
    pub fn recipients(&self) -> &[TnefRecipient] {
        &self.recipients
    }

    /// Attachments in stream order, with lazily loaded embedded messages.
    pub fn attachments(&self) -> &[TnefAttachment] {
        &self.attachments
    }

    /// First MAPI text body, or the first legacy body if no MAPI body exists.
    pub fn text_body(&self) -> Option<&TnefMessageBody> {
        self.text_body.as_ref()
    }

    /// First MAPI HTML body.
    pub fn html_body(&self) -> Option<&TnefMessageBody> {
        self.html_body.as_ref()
    }

    /// First MAPI compressed RTF body, not yet decompressed or validated.
    pub fn rtf_body(&self) -> Option<&TnefMessageBody> {
        self.rtf_body.as_ref()
    }

    /// Trimmed message class, or `None` for absent/whitespace-only text.
    /// The underlying property is unchanged.
    ///
    /// # Errors
    ///
    /// Returns errors for present, incompatible or multivalued properties.
    pub fn message_class(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        Ok(self
            .properties
            .get_text(Tag::MESSAGE_CLASS_W)?
            .and_then(|value| {
                if value.trim().is_empty() {
                    return None;
                }
                Some(match value {
                    Cow::Borrowed(text) => Cow::Borrowed(text.trim()),
                    Cow::Owned(mut text) => {
                        let end = text.trim_end().len();
                        text.truncate(end);
                        let start = text.len() - text.trim_start().len();
                        text.drain(..start);
                        Cow::Owned(text)
                    }
                })
            }))
    }

    /// Subject exactly as stored, including an empty string.
    ///
    /// # Errors
    ///
    /// Returns a present property's text-conversion error.
    pub fn subject(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.properties.get_text(Tag::SUBJECT_W)
    }

    /// Internet message identifier exactly as stored, without normalization.
    ///
    /// # Errors
    ///
    /// Returns a present property's text-conversion error.
    pub fn internet_message_id(&self) -> Result<Option<Cow<'_, str>>, TnefValueConversionError> {
        self.properties.get_text(Tag::INTERNET_MESSAGE_ID_W)
    }
}

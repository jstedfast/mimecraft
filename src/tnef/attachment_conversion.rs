// Adapted from MimeKit's Tnef/TnefMessageConverter.cs attachment conversion.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, io::Cursor};

use fluent_uri::{Uri, UriRef};

use super::{TnefAttachMethod, TnefAttachment, TnefPropertyTag as Tag, TnefValueConversionError};
use crate::{
    ContentDisposition, ContentEncoding, ContentType, DateTimeOffset, EncodingConstraint, Header,
    HeaderFormatError, MessageId, MimeContent, MimePart, filters::BestEncodingFilter,
};

/// Information lost or substituted while converting TNEF content to MIME.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefConversionLossKind {
    /// A malformed or unsuitable header value was omitted or replaced.
    InvalidHeader,
    /// A message identifier could not be safely authored as a msg-id.
    InvalidMessageId,
    /// An attachment refers to external content and was omitted.
    UnsupportedAttachMethod,
    /// An attachment has no content value and was omitted.
    AttachmentWithoutContent,
    /// An embedded-message method lacks recognized embedded content.
    InvalidEmbeddedMessage,
    /// An OLE object was retained rather than rendered as an image.
    OleObjectNotRendered,
    /// An originator or recipient has no usable Internet mailbox.
    UnparsableRecipient,
    /// Legacy unsupported-RTF loss; the property converter now renders RTF.
    UnsupportedRtfBody,
    /// RTF with an unknown compression mode was dropped.
    InvalidRtfBody,
    /// Complete compressed RTF was retained despite a checksum mismatch.
    RtfChecksumMismatch,
    /// RTF interpretation or decoding recovered from malformed/limited content.
    RtfRenderingIssue,
    /// HTML decoding replaced malformed bytes while matching inline references.
    InvalidHtmlBody,
    /// Conversion used MAPI properties instead of restoring the MIME skeleton.
    MimeSkeletonNotRestored,
    /// Reserved for unsupported inline placement; current default conversion
    /// supports HTML references and RTF images and does not emit this loss.
    InlinePlacementNotApplied,
    /// Missing, malformed, or undecodable calendar data.
    InvalidCalendarData,
    /// An unsupported calendar pattern or calendar system.
    UnsupportedCalendarData,
    /// The calendar exception count or byte limit was reached.
    CalendarExceptionLimitExceeded,
}

/// A recoverable conversion loss, in encounter order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TnefConversionLoss {
    /// Machine-readable category.
    pub kind: TnefConversionLossKind,
    /// Explanation of the omitted, substituted, or uninterpreted information.
    pub description: String,
    /// Original attachment index in whole-message conversion, when applicable.
    /// Standalone attachment conversion leaves this unset.
    pub attachment_index: Option<usize>,
}

/// An independently owned attachment part and its conversion losses.
#[derive(Debug)]
#[must_use = "inspect attachment conversion losses before using the output"]
pub struct TnefAttachmentConversionResult {
    /// `None` only when no content value exists. Empty values produce a part.
    pub part: Option<MimePart<Cursor<Vec<u8>>>>,
    /// Recoveries in encounter order; the source attachment is unchanged.
    pub losses: Vec<TnefConversionLoss>,
}

/// A conversion failure that does not produce a partial MIME part.
#[derive(Debug)]
pub enum TnefAttachmentConversionError {
    /// Present metadata has an incompatible, multivalued, or invalid value.
    Property(TnefValueConversionError),
    /// A typed MIME header could not be formatted.
    HeaderFormat(HeaderFormatError),
}

impl fmt::Display for TnefAttachmentConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Property(error) => write!(f, "invalid TNEF attachment property: {error}"),
            Self::HeaderFormat(error) => error.fmt(f),
        }
    }
}

impl Error for TnefAttachmentConversionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Property(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
        }
    }
}

impl From<TnefValueConversionError> for TnefAttachmentConversionError {
    fn from(error: TnefValueConversionError) -> Self {
        Self::Property(error)
    }
}

impl From<HeaderFormatError> for TnefAttachmentConversionError {
    fn from(error: HeaderFormatError) -> Self {
        Self::HeaderFormat(error)
    }
}

impl TnefAttachment {
    /// Converts an attachment to an independently owned MIME leaf.
    ///
    /// Ports MS-OXCMAIL 2.1.3.4's ordinary attachment mapping: sanitized media
    /// type, filename/display name, disposition, size/dates, Content-Id and
    /// location/base. Text gets a seven-bit-safe encoding selected from its
    /// bytes; other content uses Base64. No filename-extension lookup is used.
    ///
    /// Recognized embedded content is retained as `application/ms-tnef`, not
    /// recursively converted. OLE content is retained with an explicit loss.
    /// Object interface IIDs are excluded, just as in [`Self::open_read`].
    /// Missing content is reported and omitted; present empty content survives.
    ///
    /// This standalone operation preserves a stored disposition but does not
    /// infer inline placement from flags, HTML references, or RTF placeholders:
    /// that requires the containing message's best body. It does not assemble
    /// a message, restore MIME skeletons, render OLE, or load external files.
    /// Content is copied once; source buffers and reader context are unchanged.
    ///
    /// Unlike MimeKit's silent header fallbacks, malformed URI/disposition and
    /// replaced media-type values are recorded as `InvalidHeader` losses.
    /// URI syntax follows this crate's RFC 3986 parser, not .NET URI coercion.
    ///
    /// # Errors
    ///
    /// Checked property conversions and header formatting failures are returned,
    /// not mistaken for absent metadata or recovered with silent defaults.
    ///
    /// ```
    /// use mimecraft::tnef::{TnefAttachment, TnefAttachmentContent, TnefPropertySet};
    ///
    /// let attachment = TnefAttachment::new(
    ///     TnefPropertySet::new(),
    ///     Some(TnefAttachmentContent::Binary(vec![1, 2, 3])),
    /// );
    /// let converted = attachment.to_mime_part()?;
    /// assert!(converted.losses.is_empty());
    /// let mut output = Vec::new();
    /// converted.part.unwrap().write_to(&mut output)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn to_mime_part(
        &self,
    ) -> Result<TnefAttachmentConversionResult, TnefAttachmentConversionError> {
        let properties = self.properties();
        let file_name = self.file_name()?;
        let display_name = properties.get_text(Tag::DISPLAY_NAME_W)?;
        let label = file_name
            .as_deref()
            .or(display_name.as_deref())
            .filter(|name| !name.is_empty())
            .map_or_else(|| "(unnamed)".to_owned(), |name| format!("\"{name}\""));
        let method = self.method()?;
        let mut losses = Vec::new();
        let mut loss = |kind, description| {
            losses.push(TnefConversionLoss {
                kind,
                description,
                attachment_index: None,
            })
        };

        // afByReference (2), afByReferenceResolve (3), and afByReferenceOnly (4)
        // refer to files outside the message. An empty part would misrepresent
        // missing content, so omit it and report the loss instead.
        let Some(content) = self.as_bytes() else {
            let kind = if matches!(method, TnefAttachMethod::Unknown(2..=4)) {
                TnefConversionLossKind::UnsupportedAttachMethod
            } else {
                TnefConversionLossKind::AttachmentWithoutContent
            };
            loss(
                kind,
                format!("Attachment {label} has no stored content and was dropped."),
            );
            return Ok(TnefAttachmentConversionResult { part: None, losses });
        };

        let mut content_type = if self.is_embedded_message() {
            // The nested TNEF stream is passed through so the caller can process
            // it later. Do not fabricate winmail.dat or a disposition.
            ContentType::new("application", "ms-tnef").expect("constant media type")
        } else {
            if method == TnefAttachMethod::EmbeddedMessage {
                loss(
                    TnefConversionLossKind::InvalidEmbeddedMessage,
                    format!(
                        "Attachment {label} lacks recognized embedded content and was converted to an ordinary attachment."
                    ),
                );
            } else if method == TnefAttachMethod::Ole {
                // Rendering an OLE object needs the application that created it.
                // Preserve the object rather than claiming it is a rendered image.
                loss(
                    TnefConversionLossKind::OleObjectNotRendered,
                    format!("OLE object attachment {label} was attached without rendering."),
                );
            }
            let mime_type = self.mime_type()?;
            match mime_type
                .as_deref()
                .map(|value| ContentType::parse(value.as_bytes()))
            {
                Some(Ok(value)) if !forbidden_type(&value) => value,
                value => {
                    if value.is_some() {
                        loss(
                            TnefConversionLossKind::InvalidHeader,
                            format!(
                                "The Content-Type of attachment {label} was replaced with application/octet-stream."
                            ),
                        );
                    }
                    ContentType::new("application", "octet-stream").expect("constant media type")
                }
            }
        };

        // MS-OXCMAIL 2.1.3.4.2.2: retain PidTagAttachmentDisposition from MIME.
        // Without a containing body, do not override it based on rendering flags.
        let mut disposition = match properties.get_text(Tag::ATTACH_DISPOSITION_W)? {
            Some(text) => match ContentDisposition::parse(text.as_bytes()) {
                Ok(value) => Some(value),
                Err(error) => {
                    loss(
                        TnefConversionLossKind::InvalidHeader,
                        format!("The disposition of attachment {label} was ignored: {error}"),
                    );
                    None
                }
            },
            None => None,
        };

        // MS-OXCMAIL 2.1.3.4.2.1-2: prefer the long filename, then the short
        // filename, and write it into both parameter collections.
        if let Some(name) = file_name.as_deref().filter(|name| !name.is_empty()) {
            content_type.set_name(Some(name));
            disposition
                .get_or_insert_with(ContentDisposition::default)
                .set_file_name(Some(name));
        }
        // DisplayName overrides only Content-Type's name: MimeKit's historical
        // behavior, rather than a general MS-OXCMAIL rule.
        if let Some(name) = display_name.as_deref().filter(|name| !name.is_empty()) {
            content_type.set_name(Some(name));
        }
        // These RFC 2183 parameters come from the earlier TnefPart converter,
        // not from MS-OXCMAIL's attachment mapping.
        if let Some(size) = properties
            .get_i32(Tag::ATTACH_SIZE)?
            .filter(|size| *size >= 0)
        {
            disposition
                .get_or_insert_with(ContentDisposition::default)
                .set_size(Some(size as u64))
                .expect("nonnegative i32 fits the disposition size");
        }
        for (tag, created) in [
            (Tag::CREATION_TIME, true),
            (Tag::LAST_MODIFICATION_TIME, false),
        ] {
            if let Some(date) = properties.get_datetime(tag)? {
                // UTC and unspecified MAPI coordinates both become UTC. Rust's
                // TNEF dates never carry the machine-local DateTime kind.
                let date = DateTimeOffset::new(
                    date.year(),
                    date.month(),
                    date.day(),
                    date.hour(),
                    date.minute(),
                    date.second(),
                    0,
                )
                .expect("validated TNEF coordinates fit a UTC header date");
                let disposition = disposition.get_or_insert_with(ContentDisposition::default);
                if created {
                    disposition.set_creation_date(Some(date));
                } else {
                    disposition.set_modification_date(Some(date));
                }
            }
        }

        // MS-OXCMAIL 2.1.3.4.2.4 recommends Base64 for ordinary attachments.
        // Text uses the most readable seven-bit-safe encoding instead. Analyze
        // while making the owned copy, avoiding a second content buffer.
        let mut part = create_part(&content_type, content)?;
        if let Some(disposition) = disposition {
            part.set_content_disposition(Some(&disposition))?;
        }

        // MS-OXCMAIL 2.1.3.4.2.3: normalize Content-Id, then validate it using
        // setter rules. The loose parser alone accepts values such as NULs
        // that the authoring API must reject.
        if let Some(text) = self.content_id()? {
            match MessageId::parse_prefix(text.as_bytes())
                .and_then(|(id, _)| MessageId::from_value(id.as_str()))
            {
                Ok(id) => part
                    .set_content_id(Some(id.as_str()))
                    .expect("identifier was checked with the same setter rules"),
                Err(error) => loss(
                    TnefConversionLossKind::InvalidMessageId,
                    format!("The Content-Id of attachment {label} was ignored: {error}"),
                ),
            }
        }
        // Content-Location may be relative; Content-Base must be absolute.
        // RFC 3986 parsing also excludes raw header delimiters and controls.
        for (tag, field, absolute) in [
            (Tag::ATTACH_CONTENT_LOCATION_W, "Content-Location", false),
            (Tag::ATTACH_CONTENT_BASE_W, "Content-Base", true),
        ] {
            if let Some(text) = properties.get_text(tag)? {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let valid = if absolute {
                    Uri::parse(text).is_ok()
                } else {
                    UriRef::parse(text).is_ok()
                };
                if valid {
                    part.headers_mut()
                        .add(Header::from_formatted(format!("{field}: {text}\r\n")));
                } else {
                    loss(
                        TnefConversionLossKind::InvalidHeader,
                        format!(
                            "The {field} of attachment {label} was ignored because it is not a valid URI."
                        ),
                    );
                }
            }
        }
        Ok(TnefAttachmentConversionResult {
            part: Some(part),
            losses,
        })
    }
}

pub(super) fn create_part(
    content_type: &ContentType,
    content: &[u8],
) -> Result<MimePart<Cursor<Vec<u8>>>, HeaderFormatError> {
    let mut bytes = Vec::with_capacity(content.len());
    let encoding = if content_type.is_mime_type("text", "*") {
        let mut filter = BestEncodingFilter::new();
        filter.filter(content, &mut bytes);
        filter.finish(&mut bytes);
        filter.best_encoding(EncodingConstraint::SevenBit)
    } else {
        bytes.extend_from_slice(content);
        ContentEncoding::Base64
    };
    let mut part = MimePart::with_content_type(content_type, MimeContent::new(Cursor::new(bytes)))?;
    part.set_content_transfer_encoding(encoding);
    Ok(part)
}

fn forbidden_type(content_type: &ContentType) -> bool {
    // MS-OXCMAIL 2.1.3.4.2.2 forbids composite and Macintosh wrapper types.
    // Widen message/rfc822 to message/*: Base64 is forbidden for those types,
    // and an attacker-controlled composite type would be reparsed as structure.
    // Do not perform the optional filename-extension lookup.
    content_type.is_mime_type("multipart", "*")
        || content_type.is_mime_type("message", "*")
        || content_type.is_mime_type("application", "applefile")
        || content_type.is_mime_type("application", "mac-binhex40")
}

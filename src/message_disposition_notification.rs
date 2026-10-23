// Adapted from MimeKit's MessageDispositionNotification.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor, Read, Seek},
    num::NonZeroUsize,
};

use crate::{
    ContentHeaderParseError, Header, HeaderList, MimeContent, MimePart, encodings::NewLineFormat,
    report_fields,
};

/// Resource limits for reading a disposition-notification field block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispositionNotificationParseOptions {
    /// Maximum bytes in the entire decoded body, including any trailing data.
    ///
    /// Defaults to 64 MiB. Does not bound encoded input consumed by tolerant
    /// decoders, allocation overhead, or the number of fields independently.
    pub max_decoded_bytes: usize,
    /// Maximum raw bytes per folded header record. Defaults to 16 KiB.
    pub max_header_length: usize,
}

impl Default for DispositionNotificationParseOptions {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 64 * 1024 * 1024,
            max_header_length: 16 * 1024,
        }
    }
}

/// A message/disposition-notification or global-disposition-notification leaf.
///
/// Composes [`MimePart`] with detached [`HeaderList`] access. Fields describe
/// what happened to a delivered message; no required-field, disposition,
/// recipient, or message-ID validation is performed. Raw values, including
/// non-UTF-8 bytes, are retained without charset conversion.
///
/// MIME loading keeps these parts as ordinary leaves. Use [`Self::from_part`]
/// to wrap an extracted leaf, and [`Self::part_mut`] for metadata or serialization.
/// No cache or change events are used: editing returned fields does not change
/// the body until [`Self::set_fields`] explicitly replaces owned memory content.
///
/// ```
/// use mimecraft::{Header, HeaderList, MessageDispositionNotification};
///
/// let mut notification = MessageDispositionNotification::new();
/// let fields: HeaderList = vec![
///     Header::parse(b"Final-Recipient: rfc822; user@example.com")?,
///     Header::parse(b"Disposition: manual-action/MDN-sent-manually; displayed")?,
/// ].into();
/// notification.set_fields(&fields)?;
/// assert_eq!(notification.fields()?, fields);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Field replacement is unavailable for arbitrary borrowed streams:
///
/// ```compile_fail
/// use std::io::Cursor;
/// use mimecraft::{HeaderList, MessageDispositionNotification, MimeContent, MimePart};
///
/// let mut source = Cursor::new(b"X: original".to_vec());
/// let headers = MessageDispositionNotification::new().part().headers().clone();
/// let part = MimePart::from_parts(headers, MimeContent::new(&mut source));
/// let mut notification = MessageDispositionNotification::from_part(part).unwrap();
/// notification.set_fields(&HeaderList::new()).unwrap();
/// ```
#[derive(Debug)]
pub struct MessageDispositionNotification<R = Cursor<Vec<u8>>> {
    part: MimePart<R>,
}

impl MessageDispositionNotification {
    /// Creates message/disposition-notification with empty, unencoded content.
    pub fn new() -> Self {
        let header =
            Header::from_formatted("Content-Type: message/disposition-notification\r\n".into());
        Self {
            part: MimePart::from_parts(
                vec![header].into(),
                MimeContent::new(Cursor::new(Vec::new())),
            ),
        }
    }

    /// Atomically replaces owned content with fields serialized using CRLF.
    ///
    /// See [`Self::set_fields_with_new_line_format`] for serialization rules.
    ///
    /// # Errors
    ///
    /// Invalid media metadata or serialization errors leave the body unchanged.
    pub fn set_fields(&mut self, fields: &HeaderList) -> Result<(), DispositionNotificationError> {
        self.set_fields_with_new_line_format(fields, NewLineFormat::Dos)
    }

    /// Atomically serializes fields using CRLF or LF and replaces owned content.
    ///
    /// Terminates each record and appends one blank separator, including for an
    /// empty field list. Raw spelling, order, duplicates, folds, malformed
    /// records, and values follow the header writer's rules. This is not
    /// authoring validation. Unlike delivery-status groups, field-level
    /// Content-Transfer-Encoding is ordinary data and never encodes a remainder.
    ///
    /// Replaces the entire old body, including any bytes after its first header
    /// block. Outer headers remain unchanged, including requested transfer
    /// encoding and any stale Content-Length. Callers must update descriptive
    /// metadata themselves. Stored encoding becomes `Default`, position is zero,
    /// and the newline hint is cleared. No old source I/O occurs.
    ///
    /// # Errors
    ///
    /// Invalid media metadata, `Mixed` newlines, or serialization errors return
    /// without changing the old body, source position, or headers.
    pub fn set_fields_with_new_line_format(
        &mut self,
        fields: &HeaderList,
        format: NewLineFormat,
    ) -> Result<(), DispositionNotificationError> {
        self.check_type()?;
        let content =
            report_fields::write(fields, format).map_err(DispositionNotificationError::Io)?;
        self.part.set_content(content);
        Ok(())
    }
}

impl Default for MessageDispositionNotification {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Read + Seek> MessageDispositionNotification<R> {
    /// Wraps a leaf after checking its first Content-Type without source I/O.
    ///
    /// Accepts message/disposition-notification and
    /// message/global-disposition-notification, ignoring ASCII case.
    ///
    /// # Errors
    ///
    /// Missing, malformed, and other media types return errors. Failure drops
    /// the owned part; borrowed streams remain open at their original position.
    pub fn from_part(part: MimePart<R>) -> Result<Self, DispositionNotificationError> {
        let notification = Self { part };
        notification.check_type()?;
        Ok(notification)
    }

    /// Returns the underlying leaf.
    pub fn part(&self) -> &MimePart<R> {
        &self.part
    }

    /// Edits the leaf; later field operations recheck its first Content-Type.
    pub fn part_mut(&mut self) -> &mut MimePart<R> {
        &mut self.part
    }

    /// Extracts the leaf without copying or accessing its source.
    pub fn into_part(self) -> MimePart<R> {
        self.part
    }

    /// Reads detached fields using default resource limits.
    ///
    /// See [`Self::fields_with_options`] for parsing and source-position rules.
    ///
    /// # Errors
    ///
    /// Returns metadata, resource-limit, and I/O errors without partial fields.
    pub fn fields(&mut self) -> Result<HeaderList, DispositionNotificationError> {
        self.fields_with_options(&DispositionNotificationParseOptions::default())
    }

    /// Reads the first field block from the stored transfer-decoded body.
    ///
    /// Uses [`HeaderList::load_with_max_header_length`]: preserves raw records,
    /// duplicates, folds, and malformed fields, accepting EOF without a final
    /// newline or blank separator. Empty input or an initial blank line yields
    /// an empty list. Does not skip leading blanks or combine multiple groups.
    /// Bytes after the first separator are not returned, but remain untouched
    /// in the original content. Field-level transfer encodings are not applied.
    ///
    /// Decodes the whole body before parsing, so trailing bytes count toward
    /// the byte limit and trailing I/O failures propagate. Successful decoding
    /// rewinds the source to zero, including before any subsequent parse error.
    /// Parsing uses stored encoding, not a newly edited transfer header; it
    /// changes neither metadata nor bytes and runs afresh on every call.
    ///
    /// # Errors
    ///
    /// Zero limits and invalid media metadata fail before source I/O. Byte and
    /// record limit breaches fail rather than returning truncated success.
    /// Decoder tolerance and failed-copy source positions follow
    /// [`MimeContent::decode_to`]; seek/read failures are never swallowed.
    pub fn fields_with_options(
        &mut self,
        options: &DispositionNotificationParseOptions,
    ) -> Result<HeaderList, DispositionNotificationError> {
        let limit = NonZeroUsize::new(options.max_header_length)
            .filter(|_| options.max_decoded_bytes > 0)
            .ok_or_else(|| {
                DispositionNotificationError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "disposition-notification limits must be positive",
                ))
            })?;
        self.check_type()?;
        report_fields::read(self.part.content_mut(), options.max_decoded_bytes, limit)
            .map_err(DispositionNotificationError::Io)
    }

    fn check_type(&self) -> Result<(), DispositionNotificationError> {
        let content_type = self
            .part
            .content_type()
            .map_err(DispositionNotificationError::ContentType)?
            .ok_or(DispositionNotificationError::MissingContentType)?;
        if content_type.is_mime_type("message", "disposition-notification")
            || content_type.is_mime_type("message", "global-disposition-notification")
        {
            Ok(())
        } else {
            Err(DispositionNotificationError::NotDispositionNotification(
                content_type.mime_type(),
            ))
        }
    }
}

/// A disposition-notification leaf could not be wrapped, read, or edited.
#[derive(Debug)]
pub enum DispositionNotificationError {
    /// The first Content-Type is absent.
    MissingContentType,
    /// Content-Type names another media type.
    NotDispositionNotification(String),
    /// The first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
    /// Invalid options, a resource-limit breach, or I/O failure.
    Io(io::Error),
}

impl fmt::Display for DispositionNotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => {
                f.write_str("disposition-notification part has no Content-Type")
            }
            Self::NotDispositionNotification(value) => {
                write!(
                    f,
                    "expected a disposition-notification media type, got {value}"
                )
            }
            Self::ContentType(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for DispositionNotificationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType | Self::NotDispositionNotification(_) => None,
            Self::ContentType(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

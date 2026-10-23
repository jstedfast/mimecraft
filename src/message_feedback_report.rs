// Adapted from MimeKit's MessageFeedbackReport.cs.
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

/// Resource limits for reading a feedback-report field block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedbackReportParseOptions {
    /// Maximum bytes in the entire decoded body, including any trailing data.
    ///
    /// Defaults to 64 MiB. Does not bound encoded input consumed by tolerant
    /// decoders, allocation overhead, or the number of fields independently.
    pub max_decoded_bytes: usize,
    /// Maximum raw bytes per folded header record. Defaults to 16 KiB.
    pub max_header_length: usize,
}

impl Default for FeedbackReportParseOptions {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 64 * 1024 * 1024,
            max_header_length: 16 * 1024,
        }
    }
}

/// A message/feedback-report leaf containing machine-readable feedback fields.
///
/// Composes [`MimePart`] with detached [`HeaderList`] access, typically as the
/// second child of a [`crate::MultipartReport`]. Required fields, feedback types,
/// versions, addresses, URIs, and field multiplicities are not validated.
/// Raw values, including non-UTF-8 bytes, remain unconverted.
///
/// Loading and conversion use the ordinary [`crate::MimeEntity::Part`] variant.
/// Wrap a loaded leaf with [`Self::from_part`]; access metadata and serialization
/// through [`Self::part_mut`]. Fields are not cached and edits to returned lists
/// do not change content until [`Self::set_fields`] explicitly replaces it.
///
/// ```
/// use mimecraft::{Header, HeaderList, MessageFeedbackReport};
///
/// let mut report = MessageFeedbackReport::new();
/// let fields: HeaderList = vec![
///     Header::parse(b"Feedback-Type: abuse")?,
///     Header::parse(b"User-Agent: SomeGenerator/1.0")?,
///     Header::parse(b"Version: 1")?,
/// ].into();
/// report.set_fields(&fields)?;
/// assert_eq!(report.fields()?, fields);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Field replacement is unavailable for arbitrary borrowed streams:
///
/// ```compile_fail
/// use std::io::Cursor;
/// use mimecraft::{HeaderList, MessageFeedbackReport, MimeContent, MimePart};
///
/// let mut source = Cursor::new(b"Feedback-Type: abuse".to_vec());
/// let headers = MessageFeedbackReport::new().part().headers().clone();
/// let part = MimePart::from_parts(headers, MimeContent::new(&mut source));
/// let mut report = MessageFeedbackReport::from_part(part).unwrap();
/// report.set_fields(&HeaderList::new()).unwrap();
/// ```
#[derive(Debug)]
pub struct MessageFeedbackReport<R = Cursor<Vec<u8>>> {
    part: MimePart<R>,
}

impl MessageFeedbackReport {
    /// Creates message/feedback-report with empty, unencoded content.
    ///
    /// Does not synthesize mandatory fields, disposition, or transfer encoding.
    pub fn new() -> Self {
        let header = Header::from_formatted("Content-Type: message/feedback-report\r\n".into());
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
    pub fn set_fields(&mut self, fields: &HeaderList) -> Result<(), FeedbackReportError> {
        self.set_fields_with_new_line_format(fields, NewLineFormat::Dos)
    }

    /// Serializes fields using CRLF or LF before replacing the entire owned body.
    ///
    /// Terminates each record and appends one blank separator, even for an empty
    /// list. Spelling, order, duplicates, folds, malformed records, and raw values
    /// follow the header writer's rules; this is not authoring validation.
    /// Field-level Content-Transfer-Encoding is ordinary data, not an instruction
    /// to encode fields. Bytes after the old body's first block are discarded.
    ///
    /// Outer headers remain unchanged, including requested transfer encoding
    /// and any stale Content-Length. Stored encoding becomes `Default`, source
    /// position is zero, and the newline hint is cleared. No old source I/O occurs.
    ///
    /// # Errors
    ///
    /// Invalid media metadata, `Mixed` newlines, or serialization errors leave
    /// old bytes, headers, stored encoding, and source position unchanged.
    pub fn set_fields_with_new_line_format(
        &mut self,
        fields: &HeaderList,
        format: NewLineFormat,
    ) -> Result<(), FeedbackReportError> {
        self.check_type()?;
        // MimeKit rewrites content on field-change events. With detached lists,
        // finish serialization before replacing content so failures are atomic.
        let content = report_fields::write(fields, format).map_err(FeedbackReportError::Io)?;
        self.part.set_content(content);
        Ok(())
    }
}

impl Default for MessageFeedbackReport {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Read + Seek> MessageFeedbackReport<R> {
    /// Adopts a leaf after checking its first Content-Type without source I/O.
    ///
    /// Accepts message/feedback-report ignoring ASCII case.
    ///
    /// # Errors
    ///
    /// Missing, malformed, and other media types return errors. Failure drops
    /// the owned part; borrowed sources remain open at their original position.
    pub fn from_part(part: MimePart<R>) -> Result<Self, FeedbackReportError> {
        let report = Self { part };
        report.check_type()?;
        Ok(report)
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
    pub fn fields(&mut self) -> Result<HeaderList, FeedbackReportError> {
        self.fields_with_options(&FeedbackReportParseOptions::default())
    }

    /// Reads the first header block from the stored transfer-decoded body.
    ///
    /// Preserves raw records, duplicates, folds, and malformed fields using
    /// [`HeaderList::load_with_max_header_length`]. EOF without a final newline
    /// or separator is accepted. Empty input or an initial blank line produces
    /// an empty list; leading blanks are not skipped and groups are not combined.
    /// Field-level transfer-encoding declarations are never applied.
    ///
    /// Decodes the whole body before parsing, so trailing bytes count toward
    /// the byte limit and trailing I/O failures propagate. Successful decoding
    /// rewinds the source to zero, even if parsing subsequently fails. Reads
    /// use stored encoding rather than an edited transfer header, run afresh
    /// each time, and leave all stored bytes and metadata unchanged.
    ///
    /// # Errors
    ///
    /// Zero limits and invalid media metadata fail before source I/O. Byte or
    /// folded-record limit breaches fail rather than returning truncated fields.
    /// Decoder tolerance and failed-copy positions follow [`MimeContent::decode_to`];
    /// seek/read failures are never swallowed.
    pub fn fields_with_options(
        &mut self,
        options: &FeedbackReportParseOptions,
    ) -> Result<HeaderList, FeedbackReportError> {
        let limit = NonZeroUsize::new(options.max_header_length)
            .filter(|_| options.max_decoded_bytes > 0)
            .ok_or_else(|| {
                FeedbackReportError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "feedback-report limits must be positive",
                ))
            })?;
        self.check_type()?;
        report_fields::read(self.part.content_mut(), options.max_decoded_bytes, limit)
            .map_err(FeedbackReportError::Io)
    }

    fn check_type(&self) -> Result<(), FeedbackReportError> {
        let content_type = self
            .part
            .content_type()
            .map_err(FeedbackReportError::ContentType)?
            .ok_or(FeedbackReportError::MissingContentType)?;
        if content_type.is_mime_type("message", "feedback-report") {
            Ok(())
        } else {
            Err(FeedbackReportError::NotFeedbackReport(
                content_type.mime_type(),
            ))
        }
    }
}

/// A feedback-report leaf could not be wrapped, read, or edited.
#[derive(Debug)]
pub enum FeedbackReportError {
    /// The first Content-Type is absent.
    MissingContentType,
    /// Content-Type names another media type.
    NotFeedbackReport(String),
    /// The first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
    /// Invalid options, a resource-limit breach, or I/O failure.
    Io(io::Error),
}

impl fmt::Display for FeedbackReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("feedback-report part has no Content-Type"),
            Self::NotFeedbackReport(value) => {
                write!(f, "expected message/feedback-report, got {value}")
            }
            Self::ContentType(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for FeedbackReportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType | Self::NotFeedbackReport(_) => None,
            Self::ContentType(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

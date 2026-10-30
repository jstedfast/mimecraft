// Adapted from MimeKit's MultipartReport.cs and Parameter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, io::Cursor};

use crate::{ContentHeaderParseError, ContentType, HeaderFormatOptions, Multipart, MultipartError};

/// A multipart/report container with checked report-type parameter access.
///
/// Composes [`Multipart`] without imposing a particular child count or layout.
/// The report type should name the second child's subtype, but, like MimeKit,
/// this model neither checks that relationship nor creates report children.
/// Empty and unknown report types are accepted; this is not report validation.
///
/// Construction requires an explicit boundary rather than generating one.
/// Loading and conversion to [`crate::MimeEntity`] use the ordinary multipart
/// variant. Wrap an extracted container with [`Self::from_multipart`] to regain
/// report-specific access. Raw headers remain authoritative; no cache, change
/// events, source I/O, or child traversal is used for report-type operations.
///
/// ```
/// use mimecraft::{MessageDeliveryStatus, MimeEntity, MultipartReport, TextPart};
///
/// let mut report: MultipartReport =
///     MultipartReport::new("delivery-status", "report-boundary")?;
/// report.multipart_mut().children_mut().push(
///     TextPart::new("plain", "Delivery failed.")?.into(),
/// );
/// report.multipart_mut().children_mut().push(MessageDeliveryStatus::new().into());
/// assert_eq!(report.report_type()?.as_deref(), Some("delivery-status"));
/// let entity: MimeEntity = report.into();
/// assert_eq!(entity.leaf_parts().count(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MultipartReport<R = Cursor<Vec<u8>>> {
    multipart: Multipart<R>,
}

impl<R> MultipartReport<R> {
    /// Creates an empty multipart/report with an explicit boundary and report type.
    ///
    /// Trims leading/trailing ASCII space/tab/CR/LF from `report_type`, retaining
    /// case and internal whitespace. Empty strings are retained, not removed.
    /// No child entities, disposition, or transfer encoding are generated.
    ///
    /// # Errors
    ///
    /// Returns boundary validation or Content-Type formatting errors.
    pub fn new(report_type: &str, boundary: &str) -> Result<Self, MultipartReportError> {
        let multipart = Multipart::with_boundary("report", boundary)
            .map_err(MultipartReportError::Multipart)?;
        let mut report = Self { multipart };
        report.set_report_type(report_type)?;
        Ok(report)
    }

    /// Adopts a container after checking its first Content-Type without source I/O.
    ///
    /// Accepts multipart/report ignoring ASCII case. Does not require a boundary,
    /// report-type parameter, or particular children, and does not repair them.
    /// Headers, child ownership, preamble, and epilogue remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error for missing, malformed, or other media types. Failure
    /// drops the owned container; borrowed child sources remain untouched.
    pub fn from_multipart(multipart: Multipart<R>) -> Result<Self, MultipartReportError> {
        let report = Self { multipart };
        report.checked_content_type()?;
        Ok(report)
    }

    /// Returns the underlying multipart container.
    pub fn multipart(&self) -> &Multipart<R> {
        &self.multipart
    }

    /// Edits the container; later report-type operations recheck its media type.
    pub fn multipart_mut(&mut self) -> &mut Multipart<R> {
        &mut self.multipart
    }

    /// Extracts the container without copying or accessing any child source.
    pub fn into_multipart(self) -> Multipart<R> {
        self.multipart
    }

    /// Returns the decoded report-type parameter without trimming or case changes.
    ///
    /// Returns `None` only when the parameter is absent. An empty value returns
    /// `Some("")`. Reads the first Content-Type using default parameter options;
    /// a later duplicate never substitutes for an invalid first record.
    ///
    /// # Errors
    ///
    /// Returns missing, malformed, or non-report Content-Type errors.
    pub fn report_type(&self) -> Result<Option<String>, MultipartReportError> {
        Ok(self
            .checked_content_type()?
            .parameters()
            .get("report-type")
            .map(str::to_owned))
    }

    /// Sets the report type using default Content-Type formatting.
    ///
    /// See [`Self::set_report_type_with_options`] for trimming and no-op rules.
    ///
    /// # Errors
    ///
    /// Metadata, boundary, and formatting errors leave the container unchanged.
    pub fn set_report_type(&mut self, value: &str) -> Result<(), MultipartReportError> {
        self.set_report_type_with_options(value, &HeaderFormatOptions::default())
    }

    /// Sets the report type, trimming outer ASCII space/tab/CR/LF and retaining case.
    ///
    /// Empty and unknown values are allowed, including values that do not match
    /// any child. Does not change other parameters, children, preamble, epilogue,
    /// or unrelated headers. No child source is read or repositioned.
    ///
    /// An exact match to either the supplied or trimmed value is a no-op after
    /// checking the media type. In that case raw headers, folds, and duplicates
    /// survive, and boundary/format options are not validated. Otherwise formats
    /// the complete Content-Type before replacing duplicates at the first slot.
    ///
    /// # Errors
    ///
    /// Returns missing, malformed, or non-report Content-Type errors. A changed
    /// value also requires a valid existing boundary and valid formatting
    /// options. All failures leave stored state unchanged.
    pub fn set_report_type_with_options(
        &mut self,
        value: &str,
        options: &HeaderFormatOptions,
    ) -> Result<(), MultipartReportError> {
        let mut content_type = self.checked_content_type()?;
        let current = content_type.parameters().get("report-type");
        // MimeKit checks before trimming, and Parameter.Value suppresses a
        // change when the trimmed value is equal too. Preserve raw headers in
        // both cases instead of unnecessarily reserializing them.
        let trimmed = crate::parse_utils::trim_whitespace(value);
        if current == Some(value) || current == Some(trimmed) {
            return Ok(());
        }
        // The report type describes the second child, but MimeKit does not
        // enforce that relationship or rewrite the children when it changes.
        content_type
            .parameters_mut()
            .set_optional("report-type", Some(trimmed.to_owned()));
        self.multipart
            .set_content_type_with_options(&content_type, options)
            .map_err(MultipartReportError::Multipart)
    }

    fn checked_content_type(&self) -> Result<ContentType, MultipartReportError> {
        let content_type = self
            .multipart
            .content_type()
            .map_err(MultipartReportError::ContentType)?
            .ok_or(MultipartReportError::MissingContentType)?;
        if !content_type.is_mime_type("multipart", "report") {
            return Err(MultipartReportError::NotReport(content_type.mime_type()));
        }
        Ok(content_type)
    }
}

/// A multipart/report could not be constructed, wrapped, read, or edited.
#[derive(Debug)]
pub enum MultipartReportError {
    /// The first Content-Type is absent.
    MissingContentType,
    /// Content-Type names another media type.
    NotReport(String),
    /// The first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
    /// Multipart construction, boundary validation, or header formatting failed.
    Multipart(MultipartError),
}

impl fmt::Display for MultipartReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("multipart report has no Content-Type header"),
            Self::NotReport(value) => write!(f, "expected multipart/report, got {value}"),
            Self::ContentType(error) => error.fmt(f),
            Self::Multipart(error) => error.fmt(f),
        }
    }
}

impl Error for MultipartReportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType | Self::NotReport(_) => None,
            Self::ContentType(error) => Some(error),
            Self::Multipart(error) => Some(error),
        }
    }
}

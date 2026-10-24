// Adapted from MimeKit's MimePart.cs and MimeEntity.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{Read, Seek},
};

use crate::{
    ContentDisposition, ContentEncoding, ContentHeaderParseError, ContentType, Header,
    HeaderFormatError, HeaderFormatOptions, HeaderList, MimeContent, ParameterParseOptions,
    UnknownContentEncoding,
};

/// A single MIME body stream and its ordered raw headers.
///
/// Headers are the source of truth: typed getters parse the first matching
/// record on each call, returning independent values rather than mutable
/// caches. Typed setters encode a replacement before editing the list,
/// preserving unrelated headers and leaving the list intact on failure.
/// Direct [`Self::headers_mut`] edits are immediately visible to getters.
///
/// The body's [`MimeContent::encoding`] describes its stored bytes; the
/// Content-Transfer-Encoding header independently describes the intended
/// transfer representation. Changing either does not transcode the body.
/// Constructors and metadata edits perform no source I/O or validation of
/// body bytes, MIME structure, or suitability for a particular transport.
///
/// The source may be owned or borrowed, following [`MimeContent`]'s ownership
/// rules. An empty body is represented by an empty stream, not a missing source.
/// [`Self::write_to`] serializes headers and body with streaming transfer
/// conversion. [`crate::TextPart`] adds charset-aware text access, and
/// [`crate::MimeEntity`] embeds leaves in multipart and attached-message trees.
/// Complete entity loading and tree serialization reuse this model.
/// Copying raw body bytes is not a
/// substitute for serialization when the stored encoding and header differ.
///
/// ```
/// use std::io::Cursor;
/// use mimecraft::{ContentEncoding, ContentType, MimeContent, MimePart};
///
/// let content_type = ContentType::new("text", "plain")?;
/// let mut part = MimePart::with_content_type(
///     &content_type, MimeContent::new(Cursor::new(b"hello"))
/// )?;
/// part.set_content_transfer_encoding(ContentEncoding::Base64);
/// assert_eq!(part.content_transfer_encoding()?, ContentEncoding::Base64);
/// assert_eq!(part.content().encoding(), ContentEncoding::Unspecified);
/// let mut decoded = Vec::new();
/// part.content_mut().decode_to(&mut decoded)?;
/// assert_eq!(decoded, b"hello");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MimePart<R> {
    headers: HeaderList,
    content: MimeContent<R>,
}

/// A Content-Duration value is not a nonnegative integer fitting in `i32`.
///
/// Returned for invalid decoded header text or a negative setter argument.
/// Missing headers are not errors. Parsing does not report a raw byte offset
/// because the value is decoded and unfolded before interpreting the number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidContentDuration;

impl fmt::Display for InvalidContentDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("content duration must be an integer from 0 through 2147483647 seconds")
    }
}

impl Error for InvalidContentDuration {}

impl<R: Read + Seek> MimePart<R> {
    /// Creates a part with `Content-Type: application/octet-stream`.
    ///
    /// No transfer-encoding header is added, regardless of the stored content's
    /// encoding. No I/O occurs, and the source position is unchanged.
    pub fn new(content: MimeContent<R>) -> Self {
        let header = Header::from_formatted("Content-Type: application/octet-stream\r\n".into());
        Self::from_parts(vec![header].into(), content)
    }

    /// Creates a part with a checked Content-Type header and the supplied body.
    ///
    /// Uses default [`HeaderFormatOptions`]. The content type is serialized,
    /// not shared with the part. No source I/O or encoding conversion occurs.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] if the Content-Type cannot be formatted.
    /// As with other ownership-taking constructors, failure drops owned content;
    /// a borrowed source is not closed or repositioned.
    pub fn with_content_type(
        content_type: &ContentType,
        content: MimeContent<R>,
    ) -> Result<Self, HeaderFormatError> {
        let header = content_type.to_header()?;
        Ok(Self::from_parts(vec![header].into(), content))
    }

    /// Combines existing raw headers and content without validation or I/O.
    ///
    /// Preserves all header bytes, order, duplicates, and malformed records.
    /// Does not insert a missing Content-Type or infer the stored body's
    /// encoding from its headers. The caller supplies the correct
    /// [`MimeContent::encoding`]. This is not an entity parser.
    pub fn from_parts(headers: HeaderList, content: MimeContent<R>) -> Self {
        Self { headers, content }
    }
}

impl<R> MimePart<R> {
    /// Returns all raw headers, including duplicates and malformed records.
    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    /// Allows raw header edits without changing the stored body or its encoding.
    ///
    /// Typed getters observe edits immediately, including missing/invalid
    /// fields. No cached typed value or invariant is silently repaired.
    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    /// Reads this part's first Content-Id without reading its content.
    ///
    /// Uses the tolerant raw-value parser; see [`crate::MimeEntity::content_id`].
    ///
    /// # Errors
    ///
    /// Returns a first-header parse error with offsets in the raw value.
    pub fn content_id(&self) -> Result<Option<crate::MessageId>, crate::MessageIdParseError> {
        crate::mime_entity::content_id(&self.headers)
    }

    /// Assigns Content-Id, or removes every occurrence for `None`.
    ///
    /// See [`crate::MimeEntity::set_content_id`] for normalization, duplicate
    /// replacement, and validation. Does not read or change the content source.
    ///
    /// # Errors
    ///
    /// Invalid assignments leave all headers unchanged.
    pub fn set_content_id(
        &mut self,
        value: Option<&str>,
    ) -> Result<(), crate::MessageIdParseError> {
        crate::mime_entity::set_content_id(&mut self.headers, value)
    }

    /// Returns whether this part's first disposition is `attachment`.
    ///
    /// Missing disposition is false; filenames and content do not imply a flag.
    /// See [`crate::MimeEntity::is_attachment`] for raw-header semantics.
    ///
    /// # Errors
    ///
    /// Returns the first matching header's parse error.
    pub fn is_attachment(&self) -> Result<bool, ContentHeaderParseError> {
        crate::mime_entity::is_attachment(&self.headers)
    }

    /// Changes the attachment flag without discarding disposition parameters.
    ///
    /// See [`crate::MimeEntity::set_is_attachment`] for no-op and duplicate
    /// behavior. Does not inspect or reposition the source.
    ///
    /// # Errors
    ///
    /// Parse or formatting errors leave all headers unchanged.
    pub fn set_is_attachment(&mut self, value: bool) -> Result<(), crate::AttachmentEditError> {
        self.set_is_attachment_with_options(value, &HeaderFormatOptions::default())
    }

    /// Changes the attachment flag with explicit header formatting options.
    ///
    /// Options are ignored for no-op assignments, as described by
    /// [`crate::MimeEntity::set_is_attachment_with_options`].
    ///
    /// # Errors
    ///
    /// Parse or formatting errors leave all headers unchanged.
    pub fn set_is_attachment_with_options(
        &mut self,
        value: bool,
        options: &HeaderFormatOptions,
    ) -> Result<(), crate::AttachmentEditError> {
        crate::mime_entity::set_is_attachment(&mut self.headers, value, options)
    }

    /// Decodes the first Content-Description header, or returns `None` if absent.
    ///
    /// Uses tolerant RFC 2047 decoding followed by unfolding and Unicode
    /// trimming, as in MimeKit. An empty header returns present empty text.
    /// The detached result retains charset-recovery diagnostics; their offsets
    /// refer to the original raw value. Does not read content or mutate headers.
    pub fn content_description(&self) -> Option<crate::rfc2047::DecodedHeader> {
        self.content_description_with_options(&crate::rfc2047::DecodeOptions::default())
    }

    /// Decodes Content-Description with explicit charset and compliance options.
    ///
    /// See [`Self::content_description`] and [`Header::decode_value_with_options`].
    /// Later duplicate values do not override the first matching record.
    pub fn content_description_with_options(
        &self,
        options: &crate::rfc2047::DecodeOptions,
    ) -> Option<crate::rfc2047::DecodedHeader> {
        self.headers
            .get("Content-Description")
            .map(|header| header.decode_value_with_options(options))
    }

    /// Assigns Content-Description, or removes every occurrence for `None`.
    ///
    /// Uses default unstructured-header formatting. See
    /// [`Self::set_content_description_with_options`] for replacement rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing headers or content.
    pub fn set_content_description(
        &mut self,
        value: Option<&str>,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        self.set_content_description_with_options(
            value,
            &crate::UnstructuredHeaderFormatOptions::default(),
        )
    }

    /// Assigns Content-Description using checked unstructured-header formatting.
    ///
    /// Trims Unicode whitespace and removes CR/LF before encoding. Other
    /// controls and literal encoded-word lookalikes are encoded, not discarded
    /// or interpreted. See [`Header::from_unstructured_with_options`].
    ///
    /// Replaces duplicates at the first matching slot with a canonical
    /// `Content-Description` record, or appends when absent. Like other raw-header
    /// part setters, always reformats even for equal decoded text, rather than
    /// reproducing MimeKit's cache-dependent no-op. Empty text leaves an empty
    /// header; `None` removes all occurrences without validating options.
    ///
    /// # Errors
    ///
    /// Invalid options or encoding failures leave all headers and content
    /// unchanged. Formatting succeeds before any original records are removed.
    pub fn set_content_description_with_options(
        &mut self,
        value: Option<&str>,
        options: &crate::UnstructuredHeaderFormatOptions,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        if let Some(value) = value {
            let header =
                Header::from_unstructured_with_options("Content-Description", value, options)?;
            self.replace_formatted(header);
        } else {
            self.headers.remove_all("Content-Description");
        }
        Ok(())
    }

    /// Reads the first Content-Duration as nonnegative whole seconds.
    ///
    /// Decodes and unfolds the header first, as MimeKit does. Accepts ASCII
    /// decimal digits (including leading zeros) through `i32::MAX`. Surrounding
    /// Unicode whitespace is trimmed by header decoding; numeric parsing also
    /// accepts trailing NULs, optionally preceded by ASCII whitespace, matching
    /// the source's integer parser. Signs, fractions, comments, interior spaces,
    /// non-ASCII digits, and larger numbers are invalid.
    ///
    /// Does not inspect the media, cache a value, or change raw headers. Charset
    /// diagnostics remain available by decoding the raw header directly.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContentDuration`] for an invalid first header, rather
    /// than treating it as absent or using a later valid duplicate. This does
    /// not prevent tolerant entity/message loading; only this accessor checks
    /// the duration. Absence returns `Ok(None)`.
    pub fn content_duration(&self) -> Result<Option<i32>, InvalidContentDuration> {
        let Some(header) = self.headers.get("Content-Duration") else {
            return Ok(None);
        };
        let decoded = header.decode_value();
        // MimeKit uses integer parsing with only leading/trailing whitespace
        // enabled: a leading '+' is not allowed. Its runtime also permits NUL
        // terminators after the number and any trailing numeric whitespace.
        let value = decoded
            .text
            .trim_end_matches('\0')
            .trim_end_matches([' ', '\t', '\r', '\n', '\x0b', '\x0c']);
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(InvalidContentDuration);
        }
        value.parse().map(Some).map_err(|_| InvalidContentDuration)
    }

    /// Assigns a duration in whole seconds, or removes all occurrences for `None`.
    ///
    /// Writes canonical ASCII decimal with CRLF, replacing duplicates at the
    /// first matching slot or appending when absent. Zero is valid. Like other
    /// raw-header part setters, equal values are reformatted. Does not derive
    /// a duration from the body, access sources, or change other metadata.
    ///
    /// # Errors
    ///
    /// Negative values return [`InvalidContentDuration`] before changing headers.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContent, MimePart};
    ///
    /// let mut part = MimePart::new(MimeContent::new(Cursor::new(Vec::<u8>::new())));
    /// part.set_content_description(Some("A short recording"))?;
    /// part.set_content_duration(Some(30))?;
    /// assert_eq!(part.content_duration()?, Some(30));
    /// part.set_content_duration(None)?;
    /// assert_eq!(part.content_duration()?, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_content_duration(
        &mut self,
        value: Option<i32>,
    ) -> Result<(), InvalidContentDuration> {
        if let Some(value) = value {
            if value < 0 {
                return Err(InvalidContentDuration);
            }
            self.replace_formatted(Header::from_formatted(format!(
                "Content-Duration: {value}\r\n"
            )));
        } else {
            self.headers.remove_all("Content-Duration");
        }
        Ok(())
    }

    fn replace_formatted(&mut self, header: Header) {
        self.headers
            .replace_all(header)
            .expect("typed formatters always produce a header with a parsed field name");
    }
}

impl<R: Read + Seek> MimePart<R> {
    /// Returns the stored content and its independent encoding metadata.
    pub fn content(&self) -> &MimeContent<R> {
        &self.content
    }

    /// Allows body access, decoding, or source edits without changing headers.
    ///
    /// Any byte edits must continue to match the stored content's encoding.
    pub fn content_mut(&mut self) -> &mut MimeContent<R> {
        &mut self.content
    }

    /// Replaces the body without I/O or header edits and returns the old content.
    ///
    /// Both streams retain their current positions and encoding metadata.
    pub fn set_content(&mut self, content: MimeContent<R>) -> MimeContent<R> {
        std::mem::replace(&mut self.content, content)
    }

    /// Returns the raw headers and content without copying or repositioning.
    pub fn into_parts(self) -> (HeaderList, MimeContent<R>) {
        (self.headers, self.content)
    }

    /// Parses the first Content-Type header, or returns `None` if absent.
    ///
    /// No implicit media-type default is substituted after raw edits or
    /// [`Self::from_parts`]. Returned values are detached snapshots; use
    /// [`Self::set_content_type`] to commit an edited value to the headers.
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] for an invalid first value, even if
    /// a later duplicate is valid. Offsets refer to that header's raw value.
    pub fn content_type(&self) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.content_type_with_options(&ParameterParseOptions::default())
    }

    /// Parses Content-Type with explicit parameter options.
    ///
    /// # Errors
    ///
    /// Returns the first matching header's parse error, as in [`Self::content_type`].
    pub fn content_type_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.headers
            .get("Content-Type")
            .map(|header| header.parse_content_type_with_options(options))
            .transpose()
    }

    /// Atomically replaces Content-Type using default formatting options.
    ///
    /// Retains the first occurrence's position, removes later duplicates, or
    /// appends if absent. Adopts canonical field spelling and formatted bytes.
    /// Does not inspect the body or validate its media type against its contents.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] without changing headers or content.
    pub fn set_content_type(&mut self, value: &ContentType) -> Result<(), HeaderFormatError> {
        self.set_content_type_with_options(value, &HeaderFormatOptions::default())
    }

    /// Atomically replaces Content-Type with explicit formatting options.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] without changing the part.
    pub fn set_content_type_with_options(
        &mut self,
        value: &ContentType,
        options: &HeaderFormatOptions,
    ) -> Result<(), HeaderFormatError> {
        let header = value.to_header_with_options(options)?;
        self.replace_formatted(header);
        Ok(())
    }

    /// Parses the first Content-Disposition, or returns `None` when absent.
    ///
    /// The returned value is independent; edits require a subsequent setter
    /// call. Invalid values do not silently become `attachment`.
    ///
    /// # Errors
    ///
    /// Returns [`ContentHeaderParseError`] for an invalid first value. Offsets
    /// and parameter diagnostics refer to that header's raw value.
    pub fn content_disposition(
        &self,
    ) -> Result<Option<ContentDisposition>, ContentHeaderParseError> {
        self.content_disposition_with_options(&ParameterParseOptions::default())
    }

    /// Parses Content-Disposition with explicit parameter options.
    ///
    /// # Errors
    ///
    /// Returns the first matching header's parse error.
    pub fn content_disposition_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<ContentDisposition>, ContentHeaderParseError> {
        self.headers
            .get("Content-Disposition")
            .map(|header| header.parse_content_disposition_with_options(options))
            .transpose()
    }

    /// Replaces Content-Disposition, or removes all occurrences for `None`.
    ///
    /// Uses default formatting options and the same first-position/duplicate
    /// behavior as [`Self::set_content_type`]. The body is unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] without changing the part.
    pub fn set_content_disposition(
        &mut self,
        value: Option<&ContentDisposition>,
    ) -> Result<(), HeaderFormatError> {
        self.set_content_disposition_with_options(value, &HeaderFormatOptions::default())
    }

    /// Replaces Content-Disposition with explicit formatting options.
    ///
    /// `None` removes all occurrences without formatting or checking options.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderFormatError`] without changing the part when `value` is present.
    pub fn set_content_disposition_with_options(
        &mut self,
        value: Option<&ContentDisposition>,
        options: &HeaderFormatOptions,
    ) -> Result<(), HeaderFormatError> {
        if let Some(value) = value {
            let header = value.to_header_with_options(options)?;
            self.replace_formatted(header);
        } else {
            self.headers.remove_all("Content-Disposition");
        }
        Ok(())
    }

    /// Reads the requested transfer encoding, or `Unspecified` if its header is absent.
    ///
    /// Matches MimeKit by applying [`Header::decode_value`] to the first
    /// matching header before [`ContentEncoding::parse`]. Charset-recovery
    /// diagnostics remain available by decoding that raw header directly.
    /// Unknown/empty values return errors rather than silently becoming Default.
    /// This does not inspect or change [`MimeContent::encoding`].
    ///
    /// # Errors
    ///
    /// Returns [`UnknownContentEncoding`] for an empty or unrecognized value.
    pub fn content_transfer_encoding(&self) -> Result<ContentEncoding, UnknownContentEncoding> {
        transfer_encoding(&self.headers)
    }

    /// Sets the requested transfer-encoding header without transcoding the body.
    ///
    /// `Unspecified` removes all occurrences. Other encodings replace duplicates
    /// with one canonical token and a CRLF-terminated record at the first slot,
    /// or append if absent. No I/O or automatic encoding analysis occurs.
    pub fn set_content_transfer_encoding(&mut self, encoding: ContentEncoding) {
        if let Some(token) = encoding.as_str() {
            self.replace_formatted(Header::from_formatted(format!(
                "Content-Transfer-Encoding: {token}\r\n"
            )));
        } else {
            self.headers.remove_all("Content-Transfer-Encoding");
        }
    }
}

pub(crate) fn transfer_encoding(
    headers: &HeaderList,
) -> Result<ContentEncoding, UnknownContentEncoding> {
    match headers.get("Content-Transfer-Encoding") {
        Some(header) => ContentEncoding::parse(&header.decode_value().text),
        None => Ok(ContentEncoding::Unspecified),
    }
}

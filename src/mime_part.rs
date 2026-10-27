// Adapted from MimeKit's MimePart.cs and MimeEntity.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Read, Seek},
};

use crate::{
    ContentDisposition, ContentEncoding, ContentHeaderParseError, ContentType, EncodingConstraint,
    Header, HeaderFormatError, HeaderFormatOptions, HeaderList, MimeContent, ParameterParseOptions,
    UnknownContentEncoding, encodings::InvalidLineLength, filters::BestEncodingFilter,
    io::FilteredWriter,
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
    pub(crate) content: MimeContent<R>,
    pub(crate) parsed: Option<crate::mime_entity::ParsedEntity>,
    pub(crate) recovered_encoding: Option<Vec<u8>>,
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

/// A filename could not be assigned to both MIME content headers atomically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileNameEditError {
    /// Assigning a name requires an explicit Content-Type.
    MissingContentType,
    /// The first Content-Disposition could not be parsed.
    ContentDisposition(ContentHeaderParseError),
    /// The first Content-Type could not be parsed.
    ContentType(ContentHeaderParseError),
    /// One of the replacement headers could not be formatted.
    HeaderFormat(HeaderFormatError),
}

impl fmt::Display for FileNameEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => {
                f.write_str("setting a filename requires a Content-Type header")
            }
            Self::ContentDisposition(error) | Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
        }
    }
}

impl Error for FileNameEditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType => None,
            Self::ContentDisposition(error) | Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
        }
    }
}

/// A part's content transfer encoding could not be recommended.
#[derive(Debug)]
pub enum BestEncodingError {
    /// Selection requires an explicit Content-Type.
    MissingContentType,
    /// The first Content-Type could not be parsed.
    ContentType(ContentHeaderParseError),
    /// The requested maximum line length is outside `60..=998`.
    InvalidLineLength(InvalidLineLength),
    /// Reading or rewinding the decoded content failed.
    Io(io::Error),
}

impl fmt::Display for BestEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => {
                f.write_str("encoding selection requires a Content-Type header")
            }
            Self::ContentType(error) => error.fmt(f),
            Self::InvalidLineLength(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for BestEncodingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingContentType => None,
            Self::ContentType(error) => Some(error),
            Self::InvalidLineLength(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

/// A leaf part could not be prepared for the requested transport constraint.
#[derive(Debug)]
pub enum PartPrepareError {
    /// The serialization options have an invalid newline format or line limit.
    InvalidOptions(io::Error),
    /// The first Content-Transfer-Encoding is empty or unrecognized.
    ContentTransferEncoding(UnknownContentEncoding),
    /// Required metadata or decoded content could not be analyzed.
    BestEncoding(BestEncodingError),
}

impl fmt::Display for PartPrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(error) => error.fmt(f),
            Self::ContentTransferEncoding(error) => error.fmt(f),
            Self::BestEncoding(error) => error.fmt(f),
        }
    }
}

impl Error for PartPrepareError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidOptions(error) => Some(error),
            Self::ContentTransferEncoding(error) => Some(error),
            Self::BestEncoding(error) => Some(error),
        }
    }
}

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
        Self {
            headers,
            content,
            parsed: None,
            recovered_encoding: None,
        }
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
        self.set_unstructured_header("Content-Description", value, options)
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

    /// Decodes the first Content-MD5 header, or returns `None` when absent.
    ///
    /// Adapts MimeKit's `ContentMd5` property: tolerant RFC 2047 decoding,
    /// unfolding, and Unicode trimming produce a detached text value with
    /// charset-recovery diagnostics. Empty or malformed checksum text remains
    /// present; later duplicate headers do not override the first.
    ///
    /// This is metadata access, not Base64 decoding, digest-length validation,
    /// checksum computation, or verification against the content. No source I/O
    /// occurs, and raw header edits are visible on the next call.
    pub fn content_md5(&self) -> Option<crate::rfc2047::DecodedHeader> {
        self.content_md5_with_options(&crate::rfc2047::DecodeOptions::default())
    }

    /// Decodes Content-MD5 using explicit charset and compliance options.
    ///
    /// See [`Self::content_md5`] and [`Header::decode_value_with_options`].
    /// Diagnostic offsets refer to the original raw header value.
    pub fn content_md5_with_options(
        &self,
        options: &crate::rfc2047::DecodeOptions,
    ) -> Option<crate::rfc2047::DecodedHeader> {
        // MimeKit exposes Header.Value here, not decoded checksum bytes; retain
        // arbitrary text rather than rejecting malformed or non-Base64 values.
        self.headers
            .get("Content-Md5")
            .map(|header| header.decode_value_with_options(options))
    }

    /// Assigns Content-MD5 text, or removes all occurrences for `None`.
    ///
    /// Uses default checked unstructured-header formatting, without validating
    /// the checksum. See [`Self::set_content_md5_with_options`].
    ///
    /// # Errors
    ///
    /// Formatting failures leave headers and stored content unchanged.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContent, MimePart};
    ///
    /// let mut part = MimePart::new(MimeContent::new(Cursor::new(b"body")));
    /// // Assign metadata supplied by the caller; no checksum is computed.
    /// part.set_content_md5(Some(" XYZ "))?;
    /// assert_eq!(part.content_md5().unwrap().text, "XYZ");
    /// part.set_content_md5(None)?;
    /// assert!(part.content_md5().is_none());
    /// # Ok::<(), mimecraft::UnstructuredHeaderFormatError>(())
    /// ```
    pub fn set_content_md5(
        &mut self,
        value: Option<&str>,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        self.set_content_md5_with_options(value, &crate::UnstructuredHeaderFormatOptions::default())
    }

    /// Assigns Content-MD5 text using explicit unstructured-header formatting.
    ///
    /// Like MimeKit's generic header formatter, trims Unicode whitespace and
    /// unfolds input before encoding. Controls and literal encoded-word
    /// lookalikes are safely encoded, not interpreted. This does not enforce
    /// Base64 syntax, digest length, or consistency with the body.
    ///
    /// Writes canonical `Content-Md5` spelling, replacing duplicates at the
    /// first slot or appending if absent. Equal assignments are reformatted,
    /// as with Content-Description, rather than using MimeKit's cache-dependent
    /// no-op. Empty text leaves an empty header. `None` removes all occurrences
    /// without validating options. Content bytes, encoding, and newline hints
    /// are untouched; later body edits do not refresh or remove this metadata.
    ///
    /// # Errors
    ///
    /// Returns formatting errors before editing any header. No source I/O occurs.
    pub fn set_content_md5_with_options(
        &mut self,
        value: Option<&str>,
        options: &crate::UnstructuredHeaderFormatOptions,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        self.set_unstructured_header("Content-Md5", value, options)
    }

    fn set_unstructured_header(
        &mut self,
        field: &str,
        value: Option<&str>,
        options: &crate::UnstructuredHeaderFormatOptions,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        if let Some(value) = value {
            let header = Header::from_unstructured_with_options(field, value, options)?;
            self.replace_formatted(header);
        } else {
            self.headers.remove_all(field);
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
    /// Taking this borrow conservatively discards parsed no-content/separator
    /// absence, even when only decoding. Subsequent output uses a body separator.
    pub fn content_mut(&mut self) -> &mut MimeContent<R> {
        if let Some(parsed) = &mut self.parsed {
            parsed.no_content = false;
            parsed.body_separator = true;
        }
        &mut self.content
    }

    /// Replaces the body without I/O or header edits and returns the old content.
    ///
    /// Both streams retain their current positions and encoding metadata.
    /// Subsequent output uses a body separator rather than parsed absence.
    pub fn set_content(&mut self, content: MimeContent<R>) -> MimeContent<R> {
        if let Some(parsed) = &mut self.parsed {
            parsed.no_content = false;
            parsed.body_separator = true;
        }
        std::mem::replace(&mut self.content, content)
    }

    /// Returns the raw headers and content without copying or repositioning.
    pub fn into_parts(self) -> (HeaderList, MimeContent<R>) {
        (self.headers, self.content)
    }

    /// Recommends a transfer encoding with a default maximum line length of 78.
    ///
    /// For `text/*` and `message/*`, incrementally analyzes decoded content with
    /// [`BestEncodingFilter`], using the stored [`MimeContent::encoding`], not
    /// the requested Content-Transfer-Encoding header. This is MimeKit's
    /// heuristic, not a comparison of every encoder's output size.
    ///
    /// Other media types return [`ContentEncoding::Binary`] for
    /// [`EncodingConstraint::None`], or [`ContentEncoding::Base64`] otherwise,
    /// without reading or seeking the source, even for an empty body.
    /// Empty text/message content recommends [`ContentEncoding::SevenBit`].
    ///
    /// Does not change headers, stored encoding, or newline metadata, decode
    /// character sets, or normalize newlines. A successful content scan rewinds
    /// to zero; it does not restore the original source position. Each call
    /// starts a fresh scan rather than caching a recommendation.
    ///
    /// # Errors
    ///
    /// Returns [`BestEncodingError`] for missing or invalid first Content-Type
    /// or source I/O failures. Metadata errors occur before source I/O. Failed
    /// reads are not followed by a cleanup seek; a final rewind may fail after
    /// all content has been analyzed. No partial recommendation is returned.
    ///
    /// ```
    /// use mimecraft::{ContentEncoding, EncodingConstraint, TextPart};
    ///
    /// let mut text = TextPart::new("plain", "From someone\n")?;
    /// let encoding = text.part_mut().best_encoding(EncodingConstraint::SevenBit)?;
    /// assert_eq!(encoding, ContentEncoding::QuotedPrintable);
    /// // Selection is separate from changing the requested wire encoding.
    /// text.part_mut().set_content_transfer_encoding(encoding);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn best_encoding(
        &mut self,
        constraint: EncodingConstraint,
    ) -> Result<ContentEncoding, BestEncodingError> {
        self.best_encoding_with_line_length(constraint, 78)
    }

    /// Recommends an encoding with an explicit maximum content-line length.
    ///
    /// The limit excludes CRLF/LF, includes an unterminated final line, and is
    /// in `60..=998`, matching [`BestEncodingFilter`]. It is not an encoder width
    /// setting or a transport-preparation operation. A line exactly at the limit
    /// fits; other heuristic rules, such as NUL or `From ` lines, still apply.
    ///
    /// # Errors
    ///
    /// Like [`Self::best_encoding`], plus invalid line-limit errors. The limit is
    /// checked before metadata or I/O, including for non-text/message parts.
    pub fn best_encoding_with_line_length(
        &mut self,
        constraint: EncodingConstraint,
        max_line_length: usize,
    ) -> Result<ContentEncoding, BestEncodingError> {
        let filter = BestEncodingFilter::new();
        // Validate with the analyzer before source I/O, even on the shortcut
        // path where MimeKit itself does not consult the line-length limit.
        filter
            .best_encoding_with_line_length(constraint, max_line_length)
            .map_err(BestEncodingError::InvalidLineLength)?;
        let content_type = self
            .content_type()
            .map_err(BestEncodingError::ContentType)?
            .ok_or(BestEncodingError::MissingContentType)?;
        if !content_type.is_mime_type("text", "*") && !content_type.is_mime_type("message", "*") {
            return Ok(if constraint == EncodingConstraint::None {
                ContentEncoding::Binary
            } else {
                ContentEncoding::Base64
            });
        }

        // MimeKit measures decoded text/message data through a pass-through
        // filter. Discard its output without collecting the body in memory.
        let mut analyzer = FilteredWriter::new(io::sink(), filter);
        self.content
            .decode_to(&mut analyzer)
            .map_err(BestEncodingError::Io)?;
        analyzer.finish().map_err(BestEncodingError::Io)?;
        analyzer
            .filter()
            .best_encoding_with_line_length(constraint, max_line_length)
            .map_err(BestEncodingError::InvalidLineLength)
    }

    /// Prepares this leaf's transfer-encoding header using default write options.
    ///
    /// Adapts MimeKit's `MimePart.Prepare`; see [`Self::prepare_with_options`].
    /// This is encoding selection, not validation of the entire MIME entity or
    /// a guarantee that arbitrary raw headers and stored data are transport-safe.
    ///
    /// # Errors
    ///
    /// Returns [`PartPrepareError`] without changing headers or stored bytes.
    pub fn prepare(&mut self, constraint: EncodingConstraint) -> Result<(), PartPrepareError> {
        self.prepare_with_options(constraint, &crate::MimePartWriteOptions::default())
    }

    /// Prepares this leaf's transfer encoding for serialization with `options`.
    ///
    /// Retains requested Base64, quoted-printable, and UU encodings without
    /// reading the content or Content-Type. Binary is likewise retained for
    /// an unrestricted transport. Otherwise uses [`Self::best_encoding_with_line_length`]
    /// on decoded content and assigns the result only when it changes. An absent
    /// transfer-encoding header stays absent when SevenBit suffices. No-op paths
    /// preserve raw spelling, folding, and duplicates; changes replace all
    /// duplicates at the first slot with a canonical header.
    ///
    /// Unlike MimeKit's private encoder-width setting, line width stays in
    /// explicit [`crate::MimePartWriteOptions`]: pass the same options when
    /// writing. No settings are stored in the part. Existing encoded bodies
    /// are not rewrapped or verified; safe-encoding shortcuts trust the header.
    /// Raw headers are not refolded. This does not prepare multipart children
    /// or enclosed messages, transcode stored bytes, or change newline hints.
    ///
    /// # Errors
    ///
    /// Validates all write options before metadata or I/O, even for no-ops.
    /// Unknown first transfer encodings are errors rather than implicitly
    /// unspecified. Paths needing analysis also report missing/invalid first
    /// Content-Type and source I/O errors. Headers remain unchanged on failure;
    /// source positioning and rewind failures follow [`Self::best_encoding`].
    ///
    /// ```
    /// use mimecraft::{ContentEncoding, EncodingConstraint, MimePartWriteOptions, TextPart};
    ///
    /// let mut text = TextPart::new("plain", &"x".repeat(70))?;
    /// let options = MimePartWriteOptions { max_line_length: 60, ..Default::default() };
    /// text.part_mut().prepare_with_options(EncodingConstraint::SevenBit, &options)?;
    /// assert_eq!(text.part().content_transfer_encoding()?, ContentEncoding::QuotedPrintable);
    /// let mut output = Vec::new();
    /// text.part_mut().write_content_to_with_options(&mut output, &options)?;
    /// assert!(output.split(|&byte| byte == b'\n')
    ///     .all(|line| line.strip_suffix(b"\r").unwrap_or(line).len() <= 60));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn prepare_with_options(
        &mut self,
        constraint: EncodingConstraint,
        options: &crate::MimePartWriteOptions,
    ) -> Result<(), PartPrepareError> {
        options
            .validate()
            .map_err(PartPrepareError::InvalidOptions)?;
        let encoding = self
            .content_transfer_encoding()
            .map_err(PartPrepareError::ContentTransferEncoding)?;
        match encoding {
            ContentEncoding::QuotedPrintable
            | ContentEncoding::UuEncode
            | ContentEncoding::Base64 => {
                // These encodings are safe no matter what the constraints are.
                return Ok(());
            }
            ContentEncoding::Binary if constraint == EncodingConstraint::None => {
                // No need to re-encode anything on an unrestricted transport.
                return Ok(());
            }
            _ => {}
        }

        let best = self
            .best_encoding_with_line_length(constraint, options.max_line_length)
            .map_err(PartPrepareError::BestEncoding)?;
        if encoding == best
            || (encoding == ContentEncoding::Unspecified && best == ContentEncoding::SevenBit)
        {
            // MimeKit's setter preserves equal encodings. Keep that no-op here
            // instead of canonicalizing raw headers through our typed setter.
            return Ok(());
        }
        self.set_content_transfer_encoding(best);
        Ok(())
    }
}

impl<R> MimePart<R> {
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

    /// Returns a decoded, Unicode-trimmed filename, or `None` when absent.
    ///
    /// Checks the first Content-Disposition's `filename` parameter, then the
    /// first Content-Type's `name` only if no disposition filename exists.
    /// An empty filename is present and prevents fallback. Missing headers have
    /// no implicit values; later duplicates do not override the first records.
    /// Raw edits are visible immediately, and no source I/O occurs.
    ///
    /// Filenames are untrusted metadata, not safe filesystem paths. This does
    /// not strip directory components or reject controls, reserved names, or
    /// traversal syntax. Sanitize appropriately before using a returned value.
    ///
    /// # Errors
    ///
    /// Returns a parse error only for a visited header, with offsets relative
    /// to its raw value. Invalid dispositions do not fall back to Content-Type.
    /// A valid disposition filename bypasses even a malformed Content-Type.
    pub fn file_name(&self) -> Result<Option<String>, ContentHeaderParseError> {
        self.file_name_with_options(&ParameterParseOptions::default())
    }

    /// Reads a filename with explicit parameter parsing and decoding options.
    ///
    /// Uses [`Self::file_name`]'s precedence and trimming. The returned string
    /// does not carry recovery diagnostics; inspect the corresponding parsed
    /// header's [`crate::ParameterList::issues`] for those.
    ///
    /// # Errors
    ///
    /// Returns the first visited header's parse error without changing metadata.
    pub fn file_name_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<String>, ContentHeaderParseError> {
        self.file_name_untrimmed_with_options(options)
            .map(|name| name.map(|name| name.trim().to_owned()))
    }

    pub(crate) fn file_name_untrimmed_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<String>, ContentHeaderParseError> {
        // MimeKit prefers filename over name, even when filename is empty.
        // Keep trimming separate so UU output can reject controls first.
        let disposition = self.content_disposition_with_options(options)?;
        match disposition.as_ref().and_then(ContentDisposition::file_name) {
            Some(name) => Ok(Some(name.to_owned())),
            None => Ok(self
                .content_type_with_options(options)?
                .and_then(|value| value.name().map(str::to_owned))),
        }
    }

    /// Assigns both disposition `filename` and content-type `name` parameters.
    ///
    /// `Some` requires an existing valid Content-Type; no media type is guessed.
    /// Creates an `attachment` disposition when absent, but preserves an existing
    /// disposition token. Values are stored literally, not trimmed: the checked
    /// formatter encodes Unicode, controls, and encoded-word lookalikes safely.
    /// Empty text writes empty parameters; only `None` removes them.
    ///
    /// `None` keeps existing header fields and unrelated parameters, without
    /// creating missing headers. Present headers are always reformatted, even
    /// when values are equal or a removed parameter was absent. Both replacements
    /// are prepared before editing, retaining each first position and collapsing
    /// its duplicates. Unrelated raw records and the content source are unchanged.
    ///
    /// Uses default parsing and [`HeaderFormatOptions`]. As with other typed
    /// metadata edits, skipped malformed parameter fragments are not preserved.
    /// This does not rename files, change stored content, infer media types,
    /// or make a filename safe for filesystem use.
    ///
    /// # Errors
    ///
    /// Returns [`FileNameEditError`] without changing either header on failure.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContent, MimePart};
    ///
    /// let mut part = MimePart::new(MimeContent::new(Cursor::new(b"report")));
    /// part.set_file_name(Some("report.txt"))?;
    /// assert_eq!(part.file_name()?.as_deref(), Some("report.txt"));
    /// assert!(part.is_attachment()?);
    /// part.set_file_name(None)?;
    /// assert_eq!(part.file_name()?, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_file_name(&mut self, value: Option<&str>) -> Result<(), FileNameEditError> {
        self.set_file_name_with_options(value, &HeaderFormatOptions::default())
    }

    /// Assigns both filename parameters with explicit header formatting options.
    ///
    /// Uses [`Self::set_file_name`]'s atomic replacement and absence rules.
    /// With `None` and neither header present, no options are checked or headers
    /// created. Otherwise every present or newly created header is formatted.
    ///
    /// # Errors
    ///
    /// Returns [`FileNameEditError`] without changing headers or content.
    pub fn set_file_name_with_options(
        &mut self,
        value: Option<&str>,
        options: &HeaderFormatOptions,
    ) -> Result<(), FileNameEditError> {
        let mut disposition = self
            .content_disposition()
            .map_err(FileNameEditError::ContentDisposition)?;
        let mut content_type = self
            .content_type()
            .map_err(FileNameEditError::ContentType)?;
        if value.is_some() && content_type.is_none() {
            return Err(FileNameEditError::MissingContentType);
        }
        if value.is_some() {
            disposition.get_or_insert_with(ContentDisposition::default);
        }
        if let Some(disposition) = &mut disposition {
            disposition.set_file_name(value);
        }
        if let Some(content_type) = &mut content_type {
            content_type.set_name(value);
        }
        // MimeKit updates both parameter collections. Format both detached
        // replacements first so failure in the second cannot leave them at odds.
        let disposition = disposition
            .map(|value| value.to_header_with_options(options))
            .transpose()
            .map_err(FileNameEditError::HeaderFormat)?;
        let content_type = content_type
            .map(|value| value.to_header_with_options(options))
            .transpose()
            .map_err(FileNameEditError::HeaderFormat)?;
        if let Some(header) = disposition {
            self.replace_formatted(header);
        }
        if let Some(header) = content_type {
            self.replace_formatted(header);
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
    transfer_encoding_from_header(headers.get("content-transfer-encoding"))
}

pub(crate) fn transfer_encoding_from_header(
    header: Option<&Header>,
) -> Result<ContentEncoding, UnknownContentEncoding> {
    match header {
        Some(header) => ContentEncoding::parse(&header.decode_value().text),
        None => Ok(ContentEncoding::Unspecified),
    }
}

pub(crate) fn transfer_encoding_for_write(
    headers: &HeaderList,
    recovered: Option<&[u8]>,
) -> Result<ContentEncoding, UnknownContentEncoding> {
    // Preserve an unchanged unknown encoding using the reader's default,
    // rather than rejecting bytes that were accepted during parsing.
    // Newly authored or edited invalid metadata still fails validation.
    if recovered.is_some_and(|original| {
        headers
            .get("Content-Transfer-Encoding")
            .is_some_and(|header| header.raw_value() == original)
    }) {
        Ok(ContentEncoding::Unspecified)
    } else {
        transfer_encoding(headers)
    }
}

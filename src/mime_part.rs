// Adapted from MimeKit's MimePart.cs and MimeEntity.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{Read, Seek};

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

    fn replace_formatted(&mut self, header: Header) {
        self.headers
            .replace_all(header)
            .expect("typed formatters always produce a header with a parsed field name");
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

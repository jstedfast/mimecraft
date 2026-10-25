// Adapted from MimeKit's Multipart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    io::{self, Cursor},
};

use crate::{
    ContentEncoding, ContentHeaderParseError, ContentType, HeaderFormatError, HeaderFormatOptions,
    HeaderList, LeafParts, MimeEntity, ParameterParseOptions,
    charsets::{Charset, CharsetError},
    encodings::{EncoderOptionsError, NewLineFormat},
};

/// An ordered multipart container with authoritative raw headers.
///
/// Like MimeKit's constructor, [`Self::new`] generates an OS-random boundary;
/// [`Self::with_boundary`] accepts an explicit boundary instead.
/// Checked edits enforce boundary syntax but do not scan bodies or detect
/// collisions with ancestor/descendant boundaries. The caller must choose
/// collision-free values; random generation makes collisions unlikely but
/// does not verify their absence.
///
/// Preamble and epilogue are raw bytes, not decoded strings or delimiter lines.
/// Raw setters store them without newline conversion or folding; text setters
/// encode UTF-8 and fold at word boundaries. An empty vector
/// represents absence. Parsed delimiter spelling and truncation are retained
/// separately while framing metadata and child structure remain unchanged.
/// Children share one source type but may be nested containers, attached
/// messages, or leaf parts.
///
/// There is no source I/O during construction, metadata editing, or traversal.
/// Raw edits and [`Self::from_parts`] can retain missing or invalid metadata.
/// Typed getters observe it directly, without caches or silent repairs.
/// Synchronous writing normalizes text newlines but preserves parsed markers
/// byte-for-byte. Authored/edited metadata is checked without changing stored
/// values. Overlapping ancestor boundaries are preserved only while the entire
/// active multipart ancestry retains parsed framing; otherwise they are rejected.
/// [`MimeEntity::load`] also retains truncated containers and missing header/body
/// separators.
///
/// ```
/// use mimecraft::{MimeEntity, Multipart, TextPart};
///
/// let mut alternative: Multipart = Multipart::with_boundary("alternative", "alt-boundary")?;
/// alternative.children_mut().push(TextPart::new("plain", "hello")?.into());
/// alternative.children_mut().push(TextPart::new("html", "<b>hello</b>")?.into());
/// let root: MimeEntity = alternative.into();
/// assert_eq!(root.leaf_parts().count(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Multipart<R = Cursor<Vec<u8>>> {
    headers: HeaderList,
    children: Vec<MimeEntity<R>>,
    preamble: Vec<u8>,
    epilogue: Vec<u8>,
    pub(crate) parsed: Option<crate::mime_entity::ParsedEntity>,
    pub(crate) framing: Option<ParsedMultipart>,
}

#[derive(Debug, Default)]
pub(crate) struct ParsedMultipart {
    pub content_type: Vec<u8>,
    pub transfer_encoding: Option<Vec<u8>>,
    pub boundary: Option<String>,
    pub boundaries: Vec<Vec<u8>>,
    pub end: Option<Vec<u8>>,
}

impl Multipart {
    /// Generates a multipart boundary without assigning it to a container.
    ///
    /// Ports MimeKit's `Multipart.GenerateBoundary`: 16 OS-random bytes encoded
    /// as unwrapped, padded Base64, prefixed with `=-`. The result is exactly
    /// 26 ASCII bytes and satisfies the checked boundary grammar.
    /// No clock, process identifier, or fallback random source is used.
    ///
    /// No content or existing boundaries are inspected. Collisions remain
    /// possible; this is generation, not collision detection. Use the result
    /// with [`Self::with_boundary`], [`Self::set_boundary`], or the specialized multipart
    /// constructors.
    ///
    /// ```
    /// use mimecraft::Multipart;
    ///
    /// let boundary = Multipart::generate_boundary()?;
    /// let part: Multipart = Multipart::with_boundary("mixed", &boundary)?;
    /// assert_eq!(part.boundary()?.as_deref(), Some(boundary.as_str()));
    /// # Ok::<(), mimecraft::MultipartError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`MultipartError::Random`] if OS randomness is unavailable,
    /// retaining the underlying error without returning a partial boundary.
    pub fn generate_boundary() -> Result<String, MultipartError> {
        Self::generate_boundary_with(getrandom::fill)
    }

    fn generate_boundary_with(
        fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
    ) -> Result<String, MultipartError> {
        let mut random = [0; 16];
        fill(&mut random).map_err(MultipartError::Random)?;
        let mut boundary = Vec::with_capacity(26);
        boundary.extend_from_slice(b"=-");
        crate::encodings::encode_base64_unwrapped(&random, &mut boundary);
        Ok(String::from_utf8(boundary).expect("the boundary prefix and Base64 alphabet are ASCII"))
    }
}

impl<R> Multipart<R> {
    pub(crate) fn original_framing(&self) -> Option<&ParsedMultipart> {
        // If the framing headers change, parsed boundary markers no longer
        // apply; generate them from the edited metadata when writing instead.
        self.framing.as_ref().filter(|framing| {
            self.headers
                .get("Content-Type")
                .is_some_and(|header| header.raw_value() == framing.content_type)
                && self
                    .headers
                    .get("Content-Transfer-Encoding")
                    .map(|header| header.raw_value())
                    == framing.transfer_encoding.as_deref()
        })
    }

    /// Creates an empty `multipart/subtype` with a checked explicit boundary.
    ///
    /// Uses default header formatting. Does not choose a transfer encoding,
    /// insert children, inspect streams, or check boundary collisions.
    ///
    /// # Errors
    ///
    /// Returns invalid subtype/boundary or header-formatting errors.
    pub fn with_boundary(subtype: &str, boundary: &str) -> Result<Self, MultipartError> {
        let content_type =
            ContentType::new("multipart", subtype).map_err(MultipartError::ContentType)?;
        Self::from_type_and_boundary(content_type, boundary)
    }

    /// Creates an empty multipart with a generated random boundary.
    ///
    /// Like [`Self::with_boundary`], creates only the Content-Type header and no children.
    /// Uses [`Multipart::generate_boundary`] after validating subtype syntax.
    /// Parsers, getters, and writers never generate a missing boundary
    /// automatically. No child-source I/O is performed.
    ///
    /// ```
    /// use mimecraft::{Multipart, TextPart};
    ///
    /// let mut part: Multipart = Multipart::new("alternative")?;
    /// part.children_mut().push(TextPart::new("plain", "hello")?.into());
    /// part.children_mut().push(TextPart::new("html", "<b>hello</b>")?.into());
    /// assert_eq!(part.boundary()?.unwrap().len(), 26);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns subtype, header-formatting, or OS-randomness errors.
    pub fn new(subtype: &str) -> Result<Self, MultipartError> {
        let content_type =
            ContentType::new("multipart", subtype).map_err(MultipartError::ContentType)?;
        let boundary = Multipart::generate_boundary()?;
        Self::from_type_and_boundary(content_type, &boundary)
    }

    fn from_type_and_boundary(
        mut content_type: ContentType,
        boundary: &str,
    ) -> Result<Self, MultipartError> {
        content_type.set_boundary(Some(boundary));
        let mut multipart = Self::from_parts(HeaderList::new(), Vec::new(), Vec::new(), Vec::new());
        multipart.set_content_type(&content_type)?;
        Ok(multipart)
    }

    /// Adopts raw components without validation, defaults, copying, or source I/O.
    ///
    /// Preserves all header records, child order, and preamble/epilogue bytes,
    /// even when metadata is missing or invalid. This is not a MIME parser.
    pub fn from_parts(
        headers: HeaderList,
        children: Vec<MimeEntity<R>>,
        preamble: Vec<u8>,
        epilogue: Vec<u8>,
    ) -> Self {
        Self {
            headers,
            children,
            preamble,
            epilogue,
            parsed: None,
            framing: None,
        }
    }

    /// Returns this container's ordered raw headers.
    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    /// Allows raw metadata editing without changing children or other bytes.
    ///
    /// Typed getters observe edits immediately; no invariant is silently repaired.
    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    /// Reads this container's first Content-Id, without inspecting children.
    ///
    /// See [`MimeEntity::content_id`] for tolerant parsing and raw-header behavior.
    ///
    /// # Errors
    ///
    /// Returns a first-header parse error with offsets in the raw value.
    pub fn content_id(&self) -> Result<Option<crate::MessageId>, crate::MessageIdParseError> {
        crate::mime_entity::content_id(&self.headers)
    }

    /// Assigns this container's Content-Id, or removes all occurrences for `None`.
    ///
    /// See [`MimeEntity::set_content_id`] for checked normalization and duplicate
    /// replacement. Children, root selectors, and source positions are unchanged.
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

    /// Returns whether this container's own disposition is `attachment`.
    ///
    /// Missing disposition is false; children are not inspected.
    ///
    /// # Errors
    ///
    /// Returns the first matching header's parse error.
    pub fn is_attachment(&self) -> Result<bool, ContentHeaderParseError> {
        crate::mime_entity::is_attachment(&self.headers)
    }

    /// Changes this container's attachment flag, retaining its parameters.
    ///
    /// See [`MimeEntity::set_is_attachment`] for no-op and duplicate behavior.
    /// Does not edit children or change attachment traversal rules.
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
    /// [`MimeEntity::set_is_attachment_with_options`].
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

    /// Returns immediate children in their stored order.
    pub fn children(&self) -> &[MimeEntity<R>] {
        &self.children
    }

    /// Allows standard vector operations on immediate children.
    ///
    /// Insertion, removal, and reordering do not edit any headers or streams.
    /// Removed children retain ownership of their sources; clearing drops them.
    /// No nesting limit or subtype-specific child constraints are imposed.
    /// Taking this mutable borrow discards this container's parsed framing,
    /// even without edits; subsequent writing generates complete delimiters.
    pub fn children_mut(&mut self) -> &mut Vec<MimeEntity<R>> {
        self.parsed = None;
        self.framing = None;
        &mut self.children
    }

    pub(crate) fn children_and_epilogue(
        &mut self,
    ) -> (&mut [MimeEntity<R>], &[u8], Option<&ParsedMultipart>) {
        (&mut self.children, &self.epilogue, self.framing.as_ref())
    }

    /// Returns raw bytes preceding the first delimiter, excluding that delimiter.
    pub fn preamble(&self) -> &[u8] {
        &self.preamble
    }

    /// Replaces the raw preamble without conversion and returns the old bytes.
    ///
    /// Discards parsed framing so replacement text cannot consume a delimiter
    /// newline previously shared with an enclosing multipart.
    pub fn set_preamble(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.parsed = None;
        self.framing = None;
        std::mem::replace(&mut self.preamble, bytes)
    }

    /// Returns raw bytes after the closing delimiter line and its line ending.
    pub fn epilogue(&self) -> &[u8] {
        &self.epilogue
    }

    /// Replaces the raw epilogue without conversion and returns the old bytes.
    ///
    /// Discards parsed framing so a closing delimiter precedes the new epilogue.
    pub fn set_epilogue(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.parsed = None;
        self.framing = None;
        std::mem::replace(&mut self.epilogue, bytes)
    }

    /// Decodes the preamble as strict UTF-8 without changing its bytes.
    ///
    /// Returns an empty string for absent text. Newlines and BOMs are preserved;
    /// no charset detection or replacement characters are used.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::InvalidBytes`] for malformed UTF-8. Use
    /// [`Self::preamble_text_with_charset`] for a known legacy charset.
    pub fn preamble_text(&self) -> Result<Cow<'_, str>, CharsetError> {
        self.preamble_text_with_charset(Charset::UTF8)
    }

    /// Decodes the preamble strictly with an explicitly selected charset.
    ///
    /// Does not inspect Content-Type, change bytes, or access child streams.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::InvalidBytes`] for malformed input.
    pub fn preamble_text_with_charset(
        &self,
        charset: Charset,
    ) -> Result<Cow<'_, str>, CharsetError> {
        charset.decode(&self.preamble)
    }

    /// Decodes the epilogue as strict UTF-8 without changing its bytes.
    ///
    /// Like [`Self::preamble_text`], preserves newlines and BOMs. An initial
    /// newline is content, not part of the closing delimiter, and is retained.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::InvalidBytes`] for malformed UTF-8.
    pub fn epilogue_text(&self) -> Result<Cow<'_, str>, CharsetError> {
        self.epilogue_text_with_charset(Charset::UTF8)
    }

    /// Decodes the epilogue strictly with an explicitly selected charset.
    ///
    /// Does not inspect Content-Type, change bytes, or access child streams.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::InvalidBytes`] for malformed input.
    pub fn epilogue_text_with_charset(
        &self,
        charset: Charset,
    ) -> Result<Cow<'_, str>, CharsetError> {
        // MimeKit's RawEpilogue includes the closing boundary's newline and its
        // text getter skips it. Our raw epilogue already excludes that framing;
        // stripping another newline would discard actual epilogue content.
        charset.decode(&self.epilogue)
    }

    /// Replaces the preamble with UTF-8 text folded using CRLF line endings.
    ///
    /// Uses [`Self::set_preamble_text_with_new_line_format`]'s folding rules.
    /// Empty text clears the preamble. Byte-identical folded output is a no-op;
    /// other replacements discard parsed framing as [`Self::set_preamble`] does.
    ///
    /// ```
    /// use mimecraft::Multipart;
    ///
    /// let mut part: Multipart = Multipart::with_boundary("mixed", "boundary")?;
    /// part.set_preamble_text("This message has multiple parts.");
    /// assert_eq!(part.preamble_text()?, "This message has multiple parts.\r\n");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_preamble_text(&mut self, text: &str) {
        self.set_preamble_text_with_new_line_format(text, NewLineFormat::Dos)
            .expect("CRLF is a valid output format");
    }

    /// Replaces the preamble with UTF-8 text using explicit line endings.
    ///
    /// Follows MimeKit's word folding with a soft target of 78 UTF-8 bytes,
    /// rather than UTF-16 code units. If adding a whitespace/word span would
    /// reach or exceed the target on a nonempty line, starts a new line and
    /// drops that word's preceding whitespace. Long words are never split.
    /// Unicode whitespace separates words; whitespace immediately before LF
    /// is discarded and LF becomes the selected newline. Bare CR is otherwise
    /// preserved as whitespace, not treated as a line break.
    ///
    /// Nonempty final lines receive a newline; empty input stays empty.
    /// No headers or child content are changed. Byte-identical output preserves
    /// parsed framing; changed output discards it.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`,
    /// leaving all stored bytes and parsed framing unchanged.
    pub fn set_preamble_text_with_new_line_format(
        &mut self,
        text: &str,
        new_line_format: NewLineFormat,
    ) -> Result<(), EncoderOptionsError> {
        let bytes = fold_preamble_or_epilogue(text, new_line_format)?;
        if self.preamble != bytes {
            self.set_preamble(bytes);
        }
        Ok(())
    }

    /// Replaces the epilogue with UTF-8 text folded using CRLF line endings.
    ///
    /// Uses [`Self::set_preamble_text_with_new_line_format`]'s folding rules.
    /// Unlike MimeKit's raw epilogue, no delimiter-owned leading newline is
    /// inserted. Caller-supplied leading newlines are retained.
    /// Empty text clears the epilogue; byte-identical output is a no-op.
    pub fn set_epilogue_text(&mut self, text: &str) {
        self.set_epilogue_text_with_new_line_format(text, NewLineFormat::Dos)
            .expect("CRLF is a valid output format");
    }

    /// Replaces the epilogue with UTF-8 text using explicit line endings.
    ///
    /// Uses [`Self::set_preamble_text_with_new_line_format`]'s folding rules
    /// without adding a delimiter-owned leading newline. A changed epilogue
    /// discards parsed framing, restoring the closing delimiter of a truncated
    /// multipart. Empty text already represented by an empty vector is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`EncoderOptionsError::InvalidNewLineFormat`] for `Mixed`,
    /// leaving all stored bytes and parsed framing unchanged.
    pub fn set_epilogue_text_with_new_line_format(
        &mut self,
        text: &str,
        new_line_format: NewLineFormat,
    ) -> Result<(), EncoderOptionsError> {
        let bytes = fold_preamble_or_epilogue(text, new_line_format)?;
        if self.epilogue != bytes {
            // If the parsed multipart lacked an end boundary, assigning a new
            // epilogue restores the default closing delimiter.
            self.set_epilogue(bytes);
        }
        Ok(())
    }

    /// Visits all descendant leaf parts in depth-first child order.
    ///
    /// Does not parse metadata, read sources, or recurse on the call stack.
    pub fn leaf_parts(&self) -> LeafParts<'_, R> {
        LeafParts::new(&self.children)
    }

    /// Returns headers, children, preamble, and epilogue without copying or I/O.
    pub fn into_parts(self) -> (HeaderList, Vec<MimeEntity<R>>, Vec<u8>, Vec<u8>) {
        (self.headers, self.children, self.preamble, self.epilogue)
    }

    /// Parses the first Content-Type, or returns `None` when absent.
    ///
    /// Returns a detached value, without asserting that it describes multipart
    /// content or has a usable boundary. No later duplicate substitutes for an
    /// invalid first header.
    ///
    /// # Errors
    ///
    /// Returns the first header's syntax error with raw-value-relative offsets.
    pub fn content_type(&self) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.content_type_with_options(&ParameterParseOptions::default())
    }

    /// Parses the first Content-Type using explicit parameter options.
    ///
    /// # Errors
    ///
    /// Returns the first header's syntax error, as in [`Self::content_type`].
    pub fn content_type_with_options(
        &self,
        options: &ParameterParseOptions,
    ) -> Result<Option<ContentType>, ContentHeaderParseError> {
        self.headers
            .get("Content-Type")
            .map(|header| header.parse_content_type_with_options(options))
            .transpose()
    }

    /// Replaces Content-Type with a multipart value and valid boundary.
    ///
    /// Uses default header formatting. Other headers and all children/bytes
    /// remain intact. Formatting completes before duplicate-collapsing
    /// replacement at the first slot, or appending when absent.
    ///
    /// # Errors
    ///
    /// Returns validation/formatting errors without changing the container.
    pub fn set_content_type(&mut self, value: &ContentType) -> Result<(), MultipartError> {
        self.set_content_type_with_options(value, &HeaderFormatOptions::default())
    }

    /// Replaces Content-Type using explicit header formatting options.
    ///
    /// # Errors
    ///
    /// Rejects non-multipart types, missing/invalid boundaries, or formatting
    /// failures before changing any stored state.
    pub fn set_content_type_with_options(
        &mut self,
        value: &ContentType,
        options: &HeaderFormatOptions,
    ) -> Result<(), MultipartError> {
        check_multipart_type(value)?;
        validate_boundary(value.boundary().ok_or(MultipartError::MissingBoundary)?)?;
        let header = value
            .to_header_with_options(options)
            .map_err(MultipartError::HeaderFormat)?;
        self.headers
            .replace_all(header)
            .expect("typed formatters always produce a header with a parsed field name");
        Ok(())
    }

    /// Returns the current checked boundary as a detached string.
    ///
    /// Missing Content-Type or boundary returns `None`; no boundary is generated.
    ///
    /// # Errors
    ///
    /// Returns errors for malformed/non-multipart Content-Type or invalid
    /// boundary syntax. Values are not trimmed or silently repaired.
    pub fn boundary(&self) -> Result<Option<String>, MultipartError> {
        header_boundary(self.headers())
    }

    /// Replaces the boundary parameter using default header formatting.
    ///
    /// Other parameters survive. Does not rename boundaries in child metadata,
    /// scan bodies for collisions, or alter raw preamble/epilogue bytes.
    ///
    /// # Errors
    ///
    /// Returns metadata, boundary, or formatting errors without changing state.
    pub fn set_boundary(&mut self, boundary: &str) -> Result<(), MultipartError> {
        self.set_boundary_with_options(boundary, &HeaderFormatOptions::default())
    }

    /// Replaces the boundary with explicit header formatting options.
    ///
    /// Requires an existing multipart Content-Type; it can repair a missing or
    /// invalid old boundary but never supplies a missing content type.
    ///
    /// # Errors
    ///
    /// Returns errors as in [`Self::set_boundary`], leaving the container intact.
    pub fn set_boundary_with_options(
        &mut self,
        boundary: &str,
        options: &HeaderFormatOptions,
    ) -> Result<(), MultipartError> {
        validate_boundary(boundary)?;
        let mut content_type = self
            .content_type()
            .map_err(MultipartError::ContentType)?
            .ok_or(MultipartError::MissingContentType)?;
        content_type.set_boundary(Some(boundary));
        self.set_content_type_with_options(&content_type, options)
    }
}

pub(crate) fn checked_boundary<R>(multipart: &Multipart<R>) -> io::Result<String> {
    checked_header_boundary(multipart.headers())
}

fn header_boundary(headers: &HeaderList) -> Result<Option<String>, MultipartError> {
    let Some(content_type) = headers
        .get("Content-Type")
        .map(crate::Header::parse_content_type)
        .transpose()
        .map_err(MultipartError::ContentType)?
    else {
        return Ok(None);
    };
    check_multipart_type(&content_type)?;
    content_type
        .boundary()
        .map(|boundary| {
            validate_boundary(boundary)?;
            Ok(boundary.to_owned())
        })
        .transpose()
}

pub(crate) fn checked_header_boundary(headers: &HeaderList) -> io::Result<String> {
    let boundary = header_boundary(headers)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                if headers.contains("Content-Type") {
                    MultipartError::MissingBoundary
                } else {
                    MultipartError::MissingContentType
                },
            )
        })?;
    let encoding = crate::mime_part::transfer_encoding(headers)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !matches!(
        encoding,
        ContentEncoding::Unspecified
            | ContentEncoding::SevenBit
            | ContentEncoding::EightBit
            | ContentEncoding::Binary
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "multipart transfer encoding must be 7bit, 8bit, or binary",
        ));
    }
    Ok(boundary)
}

fn fold_preamble_or_epilogue(
    text: &str,
    new_line_format: NewLineFormat,
) -> Result<Vec<u8>, EncoderOptionsError> {
    let newline = match new_line_format {
        NewLineFormat::Dos => "\r\n",
        NewLineFormat::Unix => "\n",
        NewLineFormat::Mixed => {
            return Err(EncoderOptionsError::InvalidNewLineFormat(new_line_format));
        }
    };
    let mut output = String::with_capacity(text.len());
    let mut characters = text.char_indices().peekable();
    let mut line_length = 0;
    let mut index = 0;
    while index < text.len() {
        let mut start_index = index;
        while let Some(&(offset, character)) = characters.peek() {
            if !character.is_whitespace() {
                break;
            }
            characters.next();
            index = offset + character.len_utf8();
            if character == '\n' {
                output.push_str(newline);
                start_index = index;
                line_length = 0;
            }
        }
        let word_index = index;
        while let Some(&(offset, character)) = characters.peek() {
            if character.is_whitespace() {
                break;
            }
            characters.next();
            index = offset + character.len_utf8();
        }
        let mut length = index - start_index;
        if line_length > 0 && line_length + length >= 78 {
            output.push_str(newline);
            start_index = word_index;
            length = index - word_index;
            line_length = 0;
        }
        if length > 0 {
            output.push_str(&text[start_index..index]);
            line_length += length;
        }
    }
    if line_length > 0 {
        output.push_str(newline);
    }
    Ok(output.into_bytes())
}

pub(crate) fn check_multipart_type(value: &ContentType) -> Result<(), MultipartError> {
    if value.is_mime_type("multipart", "*") {
        Ok(())
    } else {
        Err(MultipartError::NotMultipart(value.mime_type()))
    }
}

pub(crate) fn validate_boundary(boundary: &str) -> Result<(), MultipartError> {
    if boundary.is_empty() {
        return Err(MultipartError::InvalidBoundary { offset: 0 });
    }
    // RFC 2046 section 5.1.1: 1..=70 bchars, ending in bcharsnospace.
    for (offset, byte) in boundary.bytes().enumerate() {
        let allowed = byte.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&byte);
        if offset >= 70 || !allowed || (byte == b' ' && offset + 1 == boundary.len()) {
            return Err(MultipartError::InvalidBoundary { offset });
        }
    }
    Ok(())
}

/// Multipart metadata could not be constructed, read, or edited.
#[derive(Debug)]
pub enum MultipartError {
    /// The operation requires a Content-Type that is absent.
    MissingContentType,
    /// A checked Content-Type replacement has no boundary parameter.
    MissingBoundary,
    /// Content-Type does not describe multipart content.
    NotMultipart(String),
    /// Boundary violates the 1..=70 byte ASCII grammar or ends with a space.
    InvalidBoundary {
        /// First invalid byte, 70 for excess length, or zero for empty input.
        offset: usize,
    },
    /// Subtype construction or raw Content-Type parsing failed.
    ContentType(ContentHeaderParseError),
    /// Checked Content-Type formatting failed.
    HeaderFormat(HeaderFormatError),
    /// The operating system could not supply bytes for a generated boundary.
    Random(getrandom::Error),
}

impl fmt::Display for MultipartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("multipart has no Content-Type header"),
            Self::MissingBoundary => f.write_str("multipart Content-Type has no boundary"),
            Self::NotMultipart(media_type) => {
                write!(f, "expected a multipart media type, got {media_type}")
            }
            Self::InvalidBoundary { offset } => {
                write!(f, "invalid multipart boundary at byte {offset}")
            }
            Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
            Self::Random(error) => write!(f, "multipart boundary randomness failed: {error}"),
        }
    }
}

impl Error for MultipartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            Self::Random(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod generation_tests {
    use super::*;

    #[test]
    fn boundary_generation_preserves_mimekits_prefix_padding_and_random_bytes() {
        for (random, expected) in [
            ([0; 16], "=-AAAAAAAAAAAAAAAAAAAAAA=="),
            ([255; 16], "=-/////////////////////w=="),
            ([251; 16], "=-+/v7+/v7+/v7+/v7+/v7+w=="),
            (
                std::array::from_fn(|index| index as u8),
                "=-AAECAwQFBgcICQoLDA0ODw==",
            ),
        ] {
            let boundary = Multipart::generate_boundary_with(|bytes| {
                assert_eq!(bytes.len(), 16);
                bytes.copy_from_slice(&random);
                Ok(())
            })
            .unwrap();
            assert_eq!(boundary, expected);
            assert_eq!(boundary.len(), 26);
            assert!(validate_boundary(&boundary).is_ok());
            let mut decoded = Vec::new();
            crate::encodings::Base64Decoder::new().decode(&boundary.as_bytes()[2..], &mut decoded);
            assert_eq!(decoded, random);
        }
    }

    #[test]
    fn boundary_entropy_failure_is_propagated_without_fallback() {
        let error = Multipart::generate_boundary_with(|bytes| {
            bytes[..8].fill(42);
            Err(getrandom::Error::UNSUPPORTED)
        })
        .unwrap_err();
        assert!(matches!(
            error,
            MultipartError::Random(getrandom::Error::UNSUPPORTED)
        ));
        assert_eq!(
            error.source().unwrap().downcast_ref::<getrandom::Error>(),
            Some(&getrandom::Error::UNSUPPORTED)
        );
        assert!(error.to_string().contains("boundary randomness"));
    }
}

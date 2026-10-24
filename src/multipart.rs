// Adapted from MimeKit's Multipart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor},
};

use crate::{
    ContentEncoding, ContentHeaderParseError, ContentType, HeaderFormatError, HeaderFormatOptions,
    HeaderList, LeafParts, MimeEntity, ParameterParseOptions,
};

/// An ordered multipart container with authoritative raw headers.
///
/// Unlike MimeKit's constructor, [`Self::new`] requires an explicit boundary.
/// Checked edits enforce boundary syntax but do not scan bodies or detect
/// collisions with ancestor/descendant boundaries. The caller must choose
/// collision-free values; no random-generation dependency is introduced.
///
/// Preamble and epilogue are raw bytes, not decoded strings or delimiter lines.
/// They are stored without newline conversion or folding. An empty vector
/// represents absence; parsed delimiter spelling/truncation is not modeled.
/// Children share one source type but may be nested containers, attached
/// messages, or leaf parts.
///
/// There is no source I/O during construction, metadata editing, or traversal.
/// Raw edits and [`Self::from_parts`] can retain missing or invalid metadata.
/// Typed getters observe it directly, without caches or silent repairs.
/// Synchronous writing normalizes output newlines and checks container metadata
/// and ancestor boundary overlap without changing stored values.
/// [`MimeEntity::load`] builds complete trees from buffered input; truncated
/// containers and original delimiter spelling are not represented.
///
/// ```
/// use mimecraft::{MimeEntity, Multipart, TextPart};
///
/// let mut alternative: Multipart = Multipart::new("alternative", "alt-boundary")?;
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
}

impl<R> Multipart<R> {
    /// Creates an empty `multipart/subtype` with a checked explicit boundary.
    ///
    /// Uses default header formatting. Does not choose a transfer encoding,
    /// insert children, inspect streams, or check boundary collisions.
    ///
    /// # Errors
    ///
    /// Returns invalid subtype/boundary or header-formatting errors.
    pub fn new(subtype: &str, boundary: &str) -> Result<Self, MultipartError> {
        let mut content_type =
            ContentType::new("multipart", subtype).map_err(MultipartError::ContentType)?;
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

    /// Returns immediate children in their stored order.
    pub fn children(&self) -> &[MimeEntity<R>] {
        &self.children
    }

    /// Allows standard vector operations on immediate children.
    ///
    /// Insertion, removal, and reordering do not edit any headers or streams.
    /// Removed children retain ownership of their sources; clearing drops them.
    /// No nesting limit or subtype-specific child constraints are imposed.
    pub fn children_mut(&mut self) -> &mut Vec<MimeEntity<R>> {
        &mut self.children
    }

    pub(crate) fn children_and_epilogue(&mut self) -> (&mut [MimeEntity<R>], &[u8]) {
        (&mut self.children, &self.epilogue)
    }

    /// Returns raw bytes preceding the first delimiter, excluding that delimiter.
    pub fn preamble(&self) -> &[u8] {
        &self.preamble
    }

    /// Replaces the raw preamble without conversion and returns the old bytes.
    pub fn set_preamble(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        std::mem::replace(&mut self.preamble, bytes)
    }

    /// Returns raw bytes after the closing delimiter line and its line ending.
    pub fn epilogue(&self) -> &[u8] {
        &self.epilogue
    }

    /// Replaces the raw epilogue without conversion and returns the old bytes.
    pub fn set_epilogue(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        std::mem::replace(&mut self.epilogue, bytes)
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
        let Some(content_type) = self.content_type().map_err(MultipartError::ContentType)? else {
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
    let boundary = multipart
        .boundary()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                if multipart.headers().contains("Content-Type") {
                    MultipartError::MissingBoundary
                } else {
                    MultipartError::MissingContentType
                },
            )
        })?;
    let encoding = crate::mime_part::transfer_encoding(multipart.headers())
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

pub(crate) fn check_multipart_type(value: &ContentType) -> Result<(), MultipartError> {
    if value.is_mime_type("multipart", "*") {
        Ok(())
    } else {
        Err(MultipartError::NotMultipart(value.mime_type()))
    }
}

fn validate_boundary(boundary: &str) -> Result<(), MultipartError> {
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
        }
    }
}

impl Error for MultipartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            _ => None,
        }
    }
}

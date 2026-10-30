// Adapted from MimeKit's MultipartAlternative.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{Cursor, Read, Seek},
};

use crate::{
    ContentHeaderParseError, MimePart, Multipart, MultipartError, TextBodyError, TextFormat,
};

/// A multipart/alternative container with preferred-body selection and decoding.
///
/// Alternatives are ordered from least to most faithful, so the last suitable
/// body wins. Composes [`Multipart`]; conversion to [`crate::MimeEntity`] retains
/// the ordinary multipart variant. Raw headers remain authoritative.
///
/// ```
/// use mimecraft::{MultipartAlternative, TextFormat, TextPart};
///
/// let mut alternative: MultipartAlternative = MultipartAlternative::new()?;
/// alternative.multipart_mut().children_mut().extend([
///     TextPart::new("plain", "hello")?.into(),
///     TextPart::new("html", "<p>hello</p>")?.into(),
/// ]);
/// assert_eq!(alternative.text_body(TextFormat::Plain)?.as_deref(), Some("hello"));
/// assert_eq!(alternative.text_body(TextFormat::Html)?.as_deref(), Some("<p>hello</p>"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MultipartAlternative<R = Cursor<Vec<u8>>> {
    multipart: Multipart<R>,
}

impl<R> MultipartAlternative<R> {
    /// Creates an empty alternative container with a generated random boundary.
    ///
    /// No children are created and no child-source I/O is performed.
    ///
    /// # Errors
    ///
    /// Returns [`Multipart::new`]'s formatting or OS-randomness errors.
    pub fn new() -> Result<Self, MultipartAlternativeError> {
        Ok(Self {
            multipart: Multipart::new("alternative")
                .map_err(MultipartAlternativeError::Multipart)?,
        })
    }

    /// Creates an empty alternative container with an explicit checked boundary.
    ///
    /// # Errors
    ///
    /// Returns boundary validation or Content-Type formatting errors.
    pub fn with_boundary(boundary: &str) -> Result<Self, MultipartAlternativeError> {
        Ok(Self {
            multipart: Multipart::with_boundary("alternative", boundary)
                .map_err(MultipartAlternativeError::Multipart)?,
        })
    }

    /// Adopts a container after checking only its first Content-Type.
    ///
    /// Accepts multipart/alternative ignoring ASCII case. Does not validate its
    /// boundary or children, repair headers, or access child sources.
    ///
    /// # Errors
    ///
    /// Missing, malformed, and other media types fail. Failure drops the owned
    /// container; borrowed sources are not read, repositioned, or closed.
    pub fn from_multipart(multipart: Multipart<R>) -> Result<Self, MultipartAlternativeError> {
        let alternative = Self { multipart };
        alternative.check_content_type()?;
        Ok(alternative)
    }

    /// Returns the underlying multipart.
    pub fn multipart(&self) -> &Multipart<R> {
        &self.multipart
    }

    /// Edits the container; subsequent body operations recheck its media type.
    pub fn multipart_mut(&mut self) -> &mut Multipart<R> {
        &mut self.multipart
    }

    /// Extracts the multipart without copying or accessing child content.
    pub fn into_multipart(self) -> Multipart<R> {
        self.multipart
    }

    /// Borrows the preferred body without reading content or checking boundaries.
    ///
    /// Adapts MimeKit's `TryGetValue`. Searches alternatives last-to-first,
    /// descending into nested multiparts using their own body-selection rules.
    /// Related containers search only their root, while mixed containers stop
    /// at the first multipart or non-attachment text candidate. Attached
    /// messages are not searched. Alternative children are not filtered by
    /// disposition; a later flowed text/plain body also matches Plain.
    ///
    /// Missing formats return `None`; a missing leaf Content-Type defaults to
    /// text/plain outside multipart/digest. Headers, framing, and source
    /// positions are unchanged. Selection requires no source trait bounds.
    ///
    /// # Errors
    ///
    /// Returns container-type and visited body-selection metadata errors.
    /// Invalid first headers do not fall back to duplicates or earlier bodies.
    /// Unvisited metadata, transfer encoding, and charset labels are not checked.
    pub fn text_body_part(
        &self,
        format: TextFormat,
    ) -> Result<Option<&MimePart<R>>, MultipartAlternativeError> {
        Ok(self
            .body_path(format)?
            .map(|path| crate::text_body::selected_part(self.multipart.children(), &path)))
    }

    fn body_path(
        &self,
        format: TextFormat,
    ) -> Result<Option<Vec<usize>>, MultipartAlternativeError> {
        self.check_content_type()?;
        // Walk alternatives backwards from greatest to least faithfulness,
        // descending into child multiparts as MimeKit does.
        crate::text_body::select_children(self.multipart.children(), format, false)
            .map_err(MultipartAlternativeError::Body)
    }

    fn check_content_type(&self) -> Result<(), MultipartAlternativeError> {
        let content_type = self
            .multipart
            .content_type()
            .map_err(MultipartAlternativeError::ContentType)?
            .ok_or(MultipartAlternativeError::MissingContentType)?;
        if !content_type.is_mime_type("multipart", "alternative") {
            return Err(MultipartAlternativeError::NotAlternative(
                content_type.mime_type(),
            ));
        }
        Ok(())
    }
}

impl<R: Read + Seek> MultipartAlternative<R> {
    /// Reads the preferred body in the requested format, unwrapping flowed text.
    ///
    /// Adapts MimeKit's `GetTextBody`, `TextBody`, and `HtmlBody` through a single
    /// format-taking method. Uses [`Self::text_body_part`]'s selection rules and
    /// [`crate::MimeEntity::text_body`]'s shared decoding: stored transfer encoding,
    /// automatic charset detection/recovery, then flowed conversion with
    /// `delsp=yes`.
    /// Missing bodies return `None`; an empty body returns `Some("")`.
    ///
    /// Only the selected source is accessed. Decoded content is materialized in
    /// memory, without changing headers, stored bytes, or parsed MIME framing.
    /// Markup is not rendered or sanitized; non-flowed newlines are unchanged.
    ///
    /// # Errors
    ///
    /// Selection errors precede source I/O. Seek, read, and final rewind
    /// errors propagate without trying an earlier alternative. Source positions
    /// follow [`crate::MimeContent::decode_to`].
    pub fn text_body(
        &mut self,
        format: TextFormat,
    ) -> Result<Option<String>, MultipartAlternativeError> {
        let Some(path) = self.body_path(format)? else {
            return Ok(None);
        };
        // This is read-only content access, not an edit to parsed MIME framing.
        let (children, _, _) = self.multipart.children_and_epilogue();
        crate::text_body::decode_selected(children, &path)
            .map(Some)
            .map_err(MultipartAlternativeError::Body)
    }
}

/// An alternative multipart could not be constructed, wrapped, or read.
#[derive(Debug)]
pub enum MultipartAlternativeError {
    /// The container's first Content-Type is absent.
    MissingContentType,
    /// The container's first Content-Type names another media type.
    NotAlternative(String),
    /// The container's first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
    /// Body selection or decoding failed.
    Body(TextBodyError),
    /// Boundary validation, generation, or Content-Type formatting failed.
    Multipart(MultipartError),
}

impl fmt::Display for MultipartAlternativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => {
                f.write_str("alternative multipart has no Content-Type header")
            }
            Self::NotAlternative(value) => write!(f, "expected multipart/alternative, got {value}"),
            Self::ContentType(error) => error.fmt(f),
            Self::Body(error) => error.fmt(f),
            Self::Multipart(error) => error.fmt(f),
        }
    }
}

impl Error for MultipartAlternativeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            Self::Body(error) => Some(error),
            Self::Multipart(error) => Some(error),
            Self::MissingContentType | Self::NotAlternative(_) => None,
        }
    }
}

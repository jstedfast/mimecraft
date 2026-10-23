// Adapted from MimeKit's MultipartRelated.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, io::Cursor};

use crate::{
    ContentHeaderParseError, ContentType, HeaderFormatOptions, MessageId, MessageIdParseError,
    MimeEntity, Multipart, MultipartError, TextBodyError,
};

/// A multipart/related container with checked root-document access and editing.
///
/// Composes [`Multipart`], using the same root-selection rules as text-body
/// lookup. Construction requires an explicit boundary. Loading and conversion
/// retain the ordinary [`MimeEntity::Multipart`] variant. Raw headers remain
/// authoritative; operations do not read, seek, or copy child content.
///
/// Root replacement updates the type and start parameters atomically. Unlike
/// MimeKit, a non-first replacement needs an existing Content-Id: no IDs are
/// generated. URI resource lookup, Content-Location/base resolution, and
/// resource opening are not implemented.
///
/// ```
/// use mimecraft::{MimeEntity, MultipartRelated, TextFormat, TextPart};
///
/// let mut related: MultipartRelated = MultipartRelated::new("related-boundary")?;
/// assert!(related.root()?.is_none());
/// related.set_root(TextPart::new("html", "<p>Root document</p>")?.into())?;
/// assert_eq!(related.root_index()?, Some(0));
/// let entity: MimeEntity = related.into();
/// assert!(entity.text_body_part(TextFormat::Html)?.is_some());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MultipartRelated<R = Cursor<Vec<u8>>> {
    multipart: Multipart<R>,
}

impl<R> MultipartRelated<R> {
    /// Creates an empty multipart/related with an explicit checked boundary.
    ///
    /// Generates no children, IDs, type/start parameters, or other headers.
    ///
    /// # Errors
    ///
    /// Returns boundary validation or Content-Type formatting errors.
    pub fn new(boundary: &str) -> Result<Self, MultipartRelatedError> {
        Ok(Self {
            multipart: Multipart::new("related", boundary)
                .map_err(MultipartRelatedError::Multipart)?,
        })
    }

    /// Adopts a container after checking only its first Content-Type.
    ///
    /// Accepts multipart/related ignoring ASCII case. Does not check boundary,
    /// root parameters, children, or source bytes, and does not repair metadata.
    ///
    /// # Errors
    ///
    /// Missing, malformed, and other media types fail. Failure drops the owned
    /// container; borrowed child sources are neither read nor repositioned.
    pub fn from_multipart(multipart: Multipart<R>) -> Result<Self, MultipartRelatedError> {
        let related = Self { multipart };
        related.checked_content_type()?;
        Ok(related)
    }

    /// Returns the underlying multipart container.
    pub fn multipart(&self) -> &Multipart<R> {
        &self.multipart
    }

    /// Edits the container; later root operations recheck authoritative metadata.
    pub fn multipart_mut(&mut self) -> &mut Multipart<R> {
        &mut self.multipart
    }

    /// Extracts the container without copying or accessing child sources.
    pub fn into_multipart(self) -> Multipart<R> {
        self.multipart
    }

    /// Returns the selected immediate child's index, or `None` for an empty tree.
    ///
    /// A present start parameter uses its first bracketed reference, or its
    /// literal decoded value when no reference is found. Matches the first
    /// child Content-Id using case-sensitive ID comparison. Only when start is
    /// absent is type compared with child media types, ignoring ASCII case.
    /// Unmatched or absent selectors fall back to the first child, not another
    /// selector. Matching and implicit child types are shared with text-body
    /// selection; malformed visited metadata returns errors rather than fallback.
    /// Does not validate the boundary or inspect descendants or dispositions.
    ///
    /// # Errors
    ///
    /// Returns container-type and visited root-selection metadata errors. Later
    /// duplicate headers never repair an invalid first header.
    pub fn root_index(&self) -> Result<Option<usize>, MultipartRelatedError> {
        let content_type = self.checked_content_type()?;
        crate::text_body::related_root(&self.multipart, &content_type)
            .map_err(MultipartRelatedError::RootSelection)
    }

    /// Borrows the selected root document, without source I/O.
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::root_index`].
    pub fn root(&self) -> Result<Option<&MimeEntity<R>>, MultipartRelatedError> {
        Ok(self
            .root_index()?
            .map(|index| &self.multipart.children()[index]))
    }

    /// Mutably borrows the selected root without synchronizing parent parameters.
    ///
    /// Raw edits are observed on subsequent lookups. Use [`Self::set_root`] to
    /// replace the document and update the related container's metadata together.
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::root_index`].
    pub fn root_mut(&mut self) -> Result<Option<&mut MimeEntity<R>>, MultipartRelatedError> {
        Ok(self
            .root_index()?
            .map(|index| &mut self.multipart.children_mut()[index]))
    }

    /// Replaces or inserts the root, returning any replaced child without copying.
    ///
    /// Uses default Content-Type formatting. See [`Self::set_root_with_options`]
    /// for ownership, selection, and atomicity rules.
    ///
    /// # Errors
    ///
    /// Metadata, boundary, and formatting errors leave this container unchanged.
    /// On failure the supplied root is dropped, without source I/O.
    pub fn set_root(
        &mut self,
        root: MimeEntity<R>,
    ) -> Result<Option<MimeEntity<R>>, MultipartRelatedError> {
        self.set_root_with_options(root, &HeaderFormatOptions::default())
    }

    /// Replaces a selector-matched root, or inserts at index zero if none matches.
    ///
    /// Unlike the getter's first-child fallback, an unmatched/absent selector
    /// causes insertion rather than replacement, preserving existing children.
    /// Empty containers insert at zero without examining the old selectors.
    ///
    /// Requires the new root's explicit, valid first Content-Type and stores
    /// its media type in the parent's type parameter. A non-first replacement
    /// also requires a parseable first Content-Id and sets start to its bracketed
    /// ID. First-child roots remove start, without inspecting or changing their
    /// Content-Id. No IDs are generated and duplicate IDs are not rejected;
    /// subsequent lookup always selects the first matching child.
    ///
    /// Preserves other parent parameters/headers, preamble, epilogue, and other
    /// children. The new root's headers and source position are unchanged.
    /// Formats and validates the parent's existing boundary before editing any
    /// stored state, then replaces Content-Type duplicates at the first slot.
    /// Unlike the source's cached model, invalid raw metadata is not repaired.
    ///
    /// # Errors
    ///
    /// Invalid visited metadata, a missing new-root Content-Type, a missing
    /// required Content-Id, boundary errors, and format errors leave this
    /// container unchanged. Failure drops the supplied root; owned sources are
    /// dropped but borrowed sources remain open and unrepositioned.
    pub fn set_root_with_options(
        &mut self,
        root: MimeEntity<R>,
        options: &HeaderFormatOptions,
    ) -> Result<Option<MimeEntity<R>>, MultipartRelatedError> {
        let mut content_type = self.checked_content_type()?;
        // An unmatched selector means insert, not replace the getter's fallback.
        let matched = if self.multipart.children().is_empty() {
            None
        } else {
            crate::text_body::related_root_match(&self.multipart, &content_type)
                .map_err(MultipartRelatedError::RootSelection)?
        };
        let root_type = root
            .headers()
            .get("Content-Type")
            .ok_or(MultipartRelatedError::MissingRootContentType)?
            .parse_content_type()
            .map_err(MultipartRelatedError::RootContentType)?;
        content_type
            .parameters_mut()
            .set_optional("type", Some(root_type.mime_type()));

        // Only use start when the root is not the first child, working around
        // Thunderbird bug 471402, as MimeKit does. Require an explicit ID rather
        // than generating one when a non-first replacement has no Content-Id.
        let start = if matched.is_some_and(|index| index > 0) {
            let header = root
                .headers()
                .get("Content-Id")
                .ok_or(MultipartRelatedError::MissingRootContentId)?;
            let (id, _) = MessageId::parse_prefix(header.raw_value())
                .map_err(MultipartRelatedError::RootContentId)?;
            Some(format!("<{}>", id.as_str()))
        } else {
            None
        };
        content_type.parameters_mut().set_optional("start", start);
        self.multipart
            .set_content_type_with_options(&content_type, options)
            .map_err(MultipartRelatedError::Multipart)?;
        if let Some(index) = matched {
            Ok(Some(std::mem::replace(
                &mut self.multipart.children_mut()[index],
                root,
            )))
        } else {
            self.multipart.children_mut().insert(0, root);
            Ok(None)
        }
    }

    fn checked_content_type(&self) -> Result<ContentType, MultipartRelatedError> {
        let value = self
            .multipart
            .content_type()
            .map_err(MultipartRelatedError::ContentType)?
            .ok_or(MultipartRelatedError::MissingContentType)?;
        if !value.is_mime_type("multipart", "related") {
            return Err(MultipartRelatedError::NotRelated(value.mime_type()));
        }
        Ok(value)
    }
}

/// A multipart/related could not be constructed, wrapped, read, or edited.
#[derive(Debug)]
pub enum MultipartRelatedError {
    /// The container's first Content-Type is absent.
    MissingContentType,
    /// The container's Content-Type names another media type.
    NotRelated(String),
    /// The container's first Content-Type is malformed.
    ContentType(ContentHeaderParseError),
    /// Visited selector or child metadata could not be interpreted.
    RootSelection(TextBodyError),
    /// The new root has no explicit Content-Type.
    MissingRootContentType,
    /// The new root's first Content-Type is malformed.
    RootContentType(ContentHeaderParseError),
    /// A non-first replacement root has no Content-Id.
    MissingRootContentId,
    /// A required first Content-Id cannot be parsed.
    RootContentId(MessageIdParseError),
    /// Container construction, boundary validation, or formatting failed.
    Multipart(MultipartError),
}

impl fmt::Display for MultipartRelatedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("related multipart has no Content-Type header"),
            Self::NotRelated(value) => write!(f, "expected multipart/related, got {value}"),
            Self::ContentType(error) => error.fmt(f),
            Self::RootSelection(error) => error.fmt(f),
            Self::MissingRootContentType => {
                f.write_str("new related root has no Content-Type header")
            }
            Self::RootContentType(error) => {
                write!(f, "invalid new related root Content-Type: {error}")
            }
            Self::MissingRootContentId => {
                f.write_str("non-first related root requires a Content-Id")
            }
            Self::RootContentId(error) => write!(f, "invalid new related root Content-Id: {error}"),
            Self::Multipart(error) => error.fmt(f),
        }
    }
}

impl Error for MultipartRelatedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) | Self::RootContentType(error) => Some(error),
            Self::RootSelection(error) => Some(error),
            Self::RootContentId(error) => Some(error),
            Self::Multipart(error) => Some(error),
            Self::MissingContentType
            | Self::NotRelated(_)
            | Self::MissingRootContentType
            | Self::MissingRootContentId => None,
        }
    }
}

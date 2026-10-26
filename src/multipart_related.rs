// Adapted from MimeKit's MultipartRelated.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Cursor, Read, Seek},
    str::Utf8Error,
};

use fluent_uri::{Uri, UriRef, resolve::ResolveError};

use crate::{
    ContentHeaderParseError, ContentType, HeaderFormatOptions, HeaderList, MessageId,
    MessageIdGenerationError, MessageIdParseError, MimeEntity, MimePart, Multipart, MultipartError,
    TextBodyError, TextFormat, filters::MimeFilter, io::FilteredReader,
};

/// A multipart/related container with checked root-document access and editing.
///
/// Composes [`Multipart`], using the same root-selection rules as text-body
/// lookup. [`Self::new`] generates a boundary; [`Self::with_boundary`] accepts
/// an explicit value instead. Loading and conversion
/// retain the ordinary [`MimeEntity::Multipart`] variant. Raw headers remain
/// authoritative; metadata operations do not read, seek, or copy child content.
///
/// Root replacement updates the type and start parameters atomically, generating
/// a missing Content-Id for a non-first root. [`Self::index_of_content_id`] and
/// [`Self::get_by_content_id`] find
/// immediate children by parsed Content-Id. [`Self::open_by_content_id`] opens
/// decoded leaf content. [`Self::index_of_uri`], [`Self::get_by_uri`], and
/// [`Self::open_by_uri`] accept string references, resolving Content-Location
/// against Content-Base internally with `fluent-uri`.
///
/// ```
/// use mimecraft::{MimeEntity, MultipartRelated, TextFormat, TextPart};
///
/// let mut related: MultipartRelated = MultipartRelated::new()?;
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

/// A related resource's MIME metadata and incrementally decoded content.
///
/// The owned metadata is a snapshot; the reader exclusively borrows the source.
/// No charset conversion is performed. Use [`ContentType::mime_type`] and
/// [`ContentType::charset`] to inspect the media type and declared charset.
pub struct RelatedResource<'a, R> {
    /// The first parsed Content-Type, defaulting to text/plain when absent.
    pub content_type: ContentType,
    /// A decoded reader with the same lifetime and framing rules as URI opening.
    pub reader: FilteredReader<&'a mut R, Box<dyn MimeFilter>>,
}

impl<R> fmt::Debug for RelatedResource<'_, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelatedResource")
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

impl<R> MultipartRelated<R> {
    /// Creates an empty multipart/related with a generated random boundary.
    ///
    /// Uses [`Multipart::new`]. Generates no children, IDs, type/start
    /// parameters, or other headers, and performs no child-source I/O.
    ///
    /// # Errors
    ///
    /// Returns Content-Type formatting or OS-randomness errors wrapped in
    /// [`MultipartRelatedError::Multipart`].
    pub fn new() -> Result<Self, MultipartRelatedError> {
        Ok(Self {
            multipart: Multipart::new("related").map_err(MultipartRelatedError::Multipart)?,
        })
    }

    /// Creates an empty multipart/related with an explicit checked boundary.
    ///
    /// Generates no children, IDs, type/start parameters, or other headers.
    ///
    /// # Errors
    ///
    /// Returns boundary validation or Content-Type formatting errors.
    pub fn with_boundary(boundary: &str) -> Result<Self, MultipartRelatedError> {
        Ok(Self {
            multipart: Multipart::with_boundary("related", boundary)
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

    /// Finds the first immediate child with the specified Content-Id.
    ///
    /// Ports the Content-Id branch of MimeKit's `MultipartRelated.IndexOf`,
    /// accepting a parsed ID rather than a `cid:` URI. Matching is case-sensitive
    /// and uses the same first-header parsing as related-root selection.
    /// No URI parsing, percent decoding, or Content-Location matching is performed.
    ///
    /// Checks the container's first Content-Type, but ignores root selectors,
    /// boundary validity, child media types, and dispositions. Missing IDs are
    /// skipped; duplicate matches select the first child, including a multipart
    /// or attached-message wrapper. Does not search descendants or read sources.
    ///
    /// # Errors
    ///
    /// Returns container-type errors or [`MultipartRelatedError::ContentId`]
    /// for a malformed visited first Content-Id. A later valid duplicate header
    /// does not repair an invalid first header. Children after a match are not
    /// examined. Headers, framing, and source positions remain unchanged.
    pub fn index_of_content_id(
        &self,
        id: &MessageId,
    ) -> Result<Option<usize>, MultipartRelatedError> {
        self.checked_content_type()?;
        crate::text_body::related_content_id_index(&self.multipart, id.as_str())
            .map_err(MultipartRelatedError::ContentId)
    }

    /// Borrows the first immediate child matching a parsed Content-Id.
    ///
    /// Uses [`Self::index_of_content_id`]'s matching and error rules. Returns
    /// `None` when no child matches; does not open or decode resource content.
    ///
    /// ```
    /// use mimecraft::{MessageId, MultipartRelated, TextPart};
    ///
    /// let mut related: MultipartRelated = MultipartRelated::new()?;
    /// let mut html = TextPart::new("html", "<p>Root document</p>")?;
    /// html.part_mut().set_content_id(Some("root@example.com"))?;
    /// related.set_root(html.into())?;
    /// let id = MessageId::from_value("root@example.com")?;
    /// assert!(related.get_by_content_id(&id)?.is_some());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::index_of_content_id`].
    pub fn get_by_content_id(
        &self,
        id: &MessageId,
    ) -> Result<Option<&MimeEntity<R>>, MultipartRelatedError> {
        Ok(self
            .index_of_content_id(id)?
            .map(|index| &self.multipart.children()[index]))
    }

    /// Checks for an immediate child with the supplied Content-Id, without I/O.
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::index_of_content_id`].
    pub fn contains_content_id(&self, id: &MessageId) -> Result<bool, MultipartRelatedError> {
        Ok(self.index_of_content_id(id)?.is_some())
    }

    /// Checks for an immediate child matching a URI, including non-leaf matches.
    ///
    /// Ports MimeKit's `Contains`; does not open the resource or read sources.
    ///
    /// # Errors
    ///
    /// Returns the same lookup errors as [`Self::index_of_uri`].
    pub fn contains_uri(&self, uri: &str) -> Result<bool, MultipartRelatedError> {
        Ok(self.index_of_uri(uri)?.is_some())
    }

    /// Finds the first immediate child matching a URI reference.
    ///
    /// Ports MimeKit's `MultipartRelated.IndexOf` with a string-based API.
    /// A case-insensitive `cid:` scheme selects Content-Id matching only.
    /// Its path is percent-decoded once, validated as a bare [`MessageId`]
    /// without header normalization, and compared case-sensitively. Authority,
    /// query, and fragment components are not accepted in a `cid:` reference;
    /// encode ID characters instead.
    ///
    /// Other references match the first Content-Location header, decoded and
    /// unfolded with [`crate::Header::decode_value`]. Relative input matches
    /// relative locations literally, without consulting Content-Base.
    /// Absolute input resolves relative locations against the child's first
    /// Content-Base, or the container's first Content-Base when absent. Without
    /// either base, the relative location is skipped. Absolute locations do
    /// not consult bases. A base must be an absolute URI without a fragment.
    ///
    /// Comparison is byte-for-byte, not .NET URI equality: scheme/host case,
    /// explicit ports, percent escapes, queries, and fragments remain significant
    /// in locations. Resolution uses `fluent-uri`'s dot-segment processing,
    /// including percent-encoded dot segments; no additional normalization is
    /// performed. No HTML base discovery, network fetching, or descendant search
    /// is performed. Container checks and first-match behavior are the same as
    /// [`Self::index_of_content_id`].
    ///
    /// # Errors
    ///
    /// Returns container-type, input URI/CID, visited Content-Id/Content-Location,
    /// required Content-Base, or URI-resolution errors before source I/O.
    /// Malformed first headers are errors, not absent values; duplicate headers
    /// and later matches never repair them. Unneeded headers are not parsed.
    /// Headers, parsed framing, and source positions remain unchanged.
    pub fn index_of_uri(&self, uri: &str) -> Result<Option<usize>, MultipartRelatedError> {
        self.checked_content_type()?;
        let reference = UriRef::parse(uri).map_err(MultipartRelatedError::Uri)?;
        if reference
            .scheme()
            .is_some_and(|scheme| scheme.as_str().eq_ignore_ascii_case("cid"))
        {
            if reference.has_authority() || reference.has_query() || reference.has_fragment() {
                return Err(MultipartRelatedError::InvalidCid);
            }
            // RFC 2392 escapes belong to the ID, not to URI component syntax.
            // Decode once; in particular, '+' is not a form-encoded space.
            let decoded = reference.path().decode().to_bytes();
            let value =
                std::str::from_utf8(&decoded).map_err(MultipartRelatedError::CidEncoding)?;
            let id = MessageId::from_value(value).map_err(MultipartRelatedError::ContentId)?;
            // A CID contains a bare ID, not a header assignment: do not silently
            // trim whitespace, remove brackets, or unfold percent-decoded data.
            if id.as_str() != value {
                return Err(MultipartRelatedError::InvalidCid);
            }
            return crate::text_body::related_content_id_index(&self.multipart, id.as_str())
                .map_err(MultipartRelatedError::ContentId);
        }

        for (index, entity) in self.multipart.children().iter().enumerate() {
            let Some(header) = entity.headers().get("Content-Location") else {
                continue;
            };
            let value = header.decode_value().text;
            let location =
                UriRef::parse(value.as_str()).map_err(MultipartRelatedError::ContentLocation)?;
            if reference.has_scheme() && !location.has_scheme() {
                let base = match content_base(entity.headers())? {
                    Some(base) => base,
                    None => match content_base(self.multipart.headers())? {
                        Some(base) => base,
                        None => continue,
                    },
                };
                let absolute = location
                    .resolve_against(&base)
                    .map_err(MultipartRelatedError::UriResolution)?;
                if absolute.as_str() == reference.as_str() {
                    return Ok(Some(index));
                }
            } else if location.as_str() == reference.as_str() {
                // MIME location comparison is literal, rather than generic URI
                // normalization (RFC 2557 section 8.2).
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// Borrows the first immediate child matching a string URI reference.
    ///
    /// Returns `None` for an absent resource, without opening its content.
    ///
    /// # Errors
    ///
    /// Returns the same metadata and URI errors as [`Self::index_of_uri`].
    pub fn get_by_uri(&self, uri: &str) -> Result<Option<&MimeEntity<R>>, MultipartRelatedError> {
        Ok(self
            .index_of_uri(uri)?
            .map(|index| &self.multipart.children()[index]))
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

    /// Selects the requested text body within the root document, without I/O.
    ///
    /// Adapts MimeKit's `TryGetValue`. Only the selected root is considered,
    /// even when it is marked as an attachment. Multipart roots use the usual
    /// body-selection rules; attached-message contents are not searched.
    ///
    /// # Errors
    ///
    /// Returns container-type or visited root/body-selection metadata errors.
    pub fn text_body_part(
        &self,
        format: TextFormat,
    ) -> Result<Option<&MimePart<R>>, MultipartRelatedError> {
        let Some(root) = self.root()? else {
            return Ok(None);
        };
        // The root may itself be a multipart such as a multipart/alternative.
        root.text_body_part_with_attachment_filter(format, false)
            .map_err(MultipartRelatedError::RootSelection)
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
    /// uses its parseable first Content-Id or generates one with
    /// [`MessageId::generate`], and sets start to its bracketed ID.
    /// First-child roots remove start, without inspecting or changing their
    /// Content-Id. Duplicate IDs are not rejected;
    /// subsequent lookup always selects the first matching child.
    ///
    /// Preserves other parent parameters/headers, preamble, epilogue, and other
    /// children. The new root's headers are unchanged except for an assigned ID;
    /// its source position is unchanged.
    /// Formats and validates the parent's existing boundary before editing any
    /// stored state, then replaces Content-Type duplicates at the first slot.
    /// Unlike the source's cached model, invalid raw metadata is not repaired.
    ///
    /// # Errors
    ///
    /// Invalid visited metadata, a missing new-root Content-Type, identifier
    /// generation errors, boundary errors, and format errors leave this
    /// container unchanged. Failure drops the supplied root; owned sources are
    /// dropped but borrowed sources remain open and unrepositioned.
    pub fn set_root_with_options(
        &mut self,
        root: MimeEntity<R>,
        options: &HeaderFormatOptions,
    ) -> Result<Option<MimeEntity<R>>, MultipartRelatedError> {
        self.set_root_with_generator(root, options, MessageId::generate)
    }

    fn set_root_with_generator(
        &mut self,
        mut root: MimeEntity<R>,
        options: &HeaderFormatOptions,
        generate: impl FnOnce() -> Result<MessageId, MessageIdGenerationError>,
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
        // Thunderbird bug 471402, as MimeKit does.
        let start = if matched.is_some_and(|index| index > 0) {
            let id = match root
                .content_id()
                .map_err(MultipartRelatedError::RootContentId)?
            {
                Some(id) => id,
                None => {
                    let id = generate().map_err(MultipartRelatedError::RootIdGeneration)?;
                    root.set_content_id(Some(id.as_str()))
                        .map_err(MultipartRelatedError::RootContentId)?;
                    id
                }
            };
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

impl<R: Read + Seek> MultipartRelated<R> {
    /// Opens a decoded reader for the first immediate child with a matching ID.
    ///
    /// Ports MimeKit's `MultipartRelated.Open` using a parsed ID rather than a
    /// URI. Uses [`Self::index_of_content_id`]'s matching rules and requires a
    /// leaf part. A matching multipart or attached-message wrapper is not an
    /// openable resource; later children with the same ID are not considered.
    /// Empty leaf content opens successfully as an empty reader.
    ///
    /// Uses [`crate::MimeContent::open`] to rewind only the selected source and
    /// decode lazily according to its stored encoding, not its current header.
    /// Does not parse the leaf's Content-Type, convert charsets, normalize
    /// newlines, or buffer the entire body. The reader exclusively borrows this
    /// container until dropped; opening again restarts with fresh decoder state.
    /// Dropping the reader does not rewind, and source read-ahead may exceed the
    /// bytes delivered. Headers, content bytes, and parsed framing are unchanged.
    ///
    /// ```
    /// use std::io::Read;
    /// use mimecraft::{MessageId, MultipartRelated, TextPart};
    ///
    /// let mut related: MultipartRelated = MultipartRelated::new()?;
    /// let mut html = TextPart::new("html", "<p>hello</p>")?;
    /// html.part_mut().set_content_id(Some("root@example.com"))?;
    /// related.set_root(html.into())?;
    /// let id = MessageId::from_value("root@example.com")?;
    /// let mut bytes = Vec::new();
    /// related.open_by_content_id(&id)?.read_to_end(&mut bytes)?;
    /// assert_eq!(bytes, b"<p>hello</p>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Metadata errors propagate before source I/O. Returns
    /// [`MultipartRelatedError::ResourceNotFound`] for an absent or non-leaf
    /// match, following MimeKit's file-not-found behavior, and
    /// [`MultipartRelatedError::Io`] for the initial rewind failure. Subsequent
    /// read errors are returned by the reader itself.
    pub fn open_by_content_id(
        &mut self,
        id: &MessageId,
    ) -> Result<FilteredReader<&mut R, Box<dyn MimeFilter>>, MultipartRelatedError> {
        let index = self
            .index_of_content_id(id)?
            .ok_or_else(|| MultipartRelatedError::ResourceNotFound(id.clone()))?;
        self.open_resource(index, || {
            MultipartRelatedError::ResourceNotFound(id.clone())
        })
    }

    /// Opens decoded content for the first immediate child matching a URI.
    ///
    /// Uses [`Self::index_of_uri`] for string-based lookup and resolution.
    /// Like [`Self::open_by_content_id`], rewinds only the selected leaf source,
    /// decodes lazily using its stored encoding, and preserves parsed framing.
    /// Empty leaves open successfully. A first matching multipart or message
    /// wrapper is not openable; no later match is considered.
    ///
    /// ```
    /// use std::io::Read;
    /// use mimecraft::{MultipartRelated, TextPart};
    ///
    /// let mut related: MultipartRelated = MultipartRelated::new()?;
    /// let mut html = TextPart::new("html", "<p>hello</p>")?;
    /// html.part_mut().set_content_id(Some("root@example.com"))?;
    /// related.set_root(html.into())?;
    /// let mut bytes = Vec::new();
    /// related.open_by_uri("cid:root%40example.com")?.read_to_end(&mut bytes)?;
    /// assert_eq!(bytes, b"<p>hello</p>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Lookup errors precede I/O. Absent and non-leaf matches return
    /// [`MultipartRelatedError::UriResourceNotFound`]. Initial rewind failures
    /// return [`MultipartRelatedError::Io`]; subsequent errors come from the reader.
    pub fn open_by_uri(
        &mut self,
        uri: &str,
    ) -> Result<FilteredReader<&mut R, Box<dyn MimeFilter>>, MultipartRelatedError> {
        let index = self
            .index_of_uri(uri)?
            .ok_or_else(|| MultipartRelatedError::UriResourceNotFound(uri.to_owned()))?;
        self.open_resource(index, || {
            MultipartRelatedError::UriResourceNotFound(uri.to_owned())
        })
    }

    /// Opens a URI resource together with its parsed MIME metadata.
    ///
    /// Adapts MimeKit's `Open(uri, out mimeType, out charset)`. Uses the same
    /// matching and decoding rules as [`Self::open_by_uri`]. Missing Content-Type
    /// defaults to text/plain without adding a header. The declared charset is
    /// returned unchanged, including unknown labels; no conversion is attempted.
    ///
    /// ```
    /// use std::io::Read;
    /// use mimecraft::{MultipartRelated, TextPart};
    ///
    /// let mut related: MultipartRelated = MultipartRelated::new()?;
    /// let mut html = TextPart::new("html", "<p>hello</p>")?;
    /// html.part_mut().set_content_id(Some("root@example.com"))?;
    /// related.set_root(html.into())?;
    /// let mut resource = related.open_by_uri_with_metadata("cid:root@example.com")?;
    /// assert_eq!(resource.content_type.mime_type(), "text/html");
    /// assert_eq!(resource.content_type.charset(), Some("utf-8"));
    /// let mut bytes = Vec::new();
    /// resource.reader.read_to_end(&mut bytes)?;
    /// assert_eq!(bytes, b"<p>hello</p>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns URI opening errors and [`MultipartRelatedError::ResourceContentType`]
    /// for a malformed first Content-Type. Metadata errors precede source I/O.
    pub fn open_by_uri_with_metadata(
        &mut self,
        uri: &str,
    ) -> Result<RelatedResource<'_, R>, MultipartRelatedError> {
        let index = self
            .index_of_uri(uri)?
            .ok_or_else(|| MultipartRelatedError::UriResourceNotFound(uri.to_owned()))?;
        self.open_resource_with_metadata(index, || {
            MultipartRelatedError::UriResourceNotFound(uri.to_owned())
        })
    }

    /// Opens a Content-Id resource together with its parsed MIME metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Self::open_by_content_id`]'s errors and the metadata errors of
    /// [`Self::open_by_uri_with_metadata`]. Metadata parsing precedes source I/O.
    pub fn open_by_content_id_with_metadata(
        &mut self,
        id: &MessageId,
    ) -> Result<RelatedResource<'_, R>, MultipartRelatedError> {
        let index = self
            .index_of_content_id(id)?
            .ok_or_else(|| MultipartRelatedError::ResourceNotFound(id.clone()))?;
        self.open_resource_with_metadata(index, || {
            MultipartRelatedError::ResourceNotFound(id.clone())
        })
    }

    fn open_resource_with_metadata(
        &mut self,
        index: usize,
        not_found: impl FnOnce() -> MultipartRelatedError,
    ) -> Result<RelatedResource<'_, R>, MultipartRelatedError> {
        let MimeEntity::Part(part) = &self.multipart.children()[index] else {
            return Err(not_found());
        };
        let content_type = part
            .content_type()
            .map_err(MultipartRelatedError::ResourceContentType)?
            .unwrap_or_else(|| ContentType::new("text", "plain").expect("valid default MIME type"));
        let reader = self.open_resource(index, not_found)?;
        Ok(RelatedResource {
            content_type,
            reader,
        })
    }

    fn open_resource(
        &mut self,
        index: usize,
        not_found: impl FnOnce() -> MultipartRelatedError,
    ) -> Result<FilteredReader<&mut R, Box<dyn MimeFilter>>, MultipartRelatedError> {
        // Opening only changes stream position. Avoid the editing accessors,
        // which would discard parsed multipart framing and leaf separators.
        let (children, _, _) = self.multipart.children_and_epilogue();
        let MimeEntity::Part(part) = &mut children[index] else {
            return Err(not_found());
        };
        part.content.open().map_err(MultipartRelatedError::Io)
    }
}

fn content_base(headers: &HeaderList) -> Result<Option<Uri<String>>, MultipartRelatedError> {
    headers
        .get("Content-Base")
        .map(|header| {
            Uri::parse(header.decode_value().text)
                .map_err(|(error, _)| MultipartRelatedError::ContentBase(error))
        })
        .transpose()
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
    /// A decoded CID or first Content-Id visited during lookup cannot be parsed.
    ContentId(MessageIdParseError),
    /// The supplied URI reference is malformed.
    Uri(fluent_uri::ParseError),
    /// A CID has extra URI components or its decoded ID needs header normalization.
    InvalidCid,
    /// A percent-decoded CID is not valid UTF-8.
    CidEncoding(Utf8Error),
    /// A visited first Content-Location is not a URI reference.
    ContentLocation(fluent_uri::ParseError),
    /// A required first Content-Base is not an absolute URI.
    ContentBase(fluent_uri::ParseError),
    /// A location cannot be resolved against its base.
    UriResolution(ResolveError),
    /// No matching leaf resource exists, including a first non-leaf match.
    ResourceNotFound(MessageId),
    /// No matching leaf resource exists for the supplied URI.
    UriResourceNotFound(String),
    /// A matched leaf's first Content-Type cannot be parsed for metadata opening.
    ResourceContentType(ContentHeaderParseError),
    /// Opening the resource failed to rewind its source.
    Io(io::Error),
    /// The new root has no explicit Content-Type.
    MissingRootContentType,
    /// The new root's first Content-Type is malformed.
    RootContentType(ContentHeaderParseError),
    /// A missing non-first root Content-Id could not be generated.
    RootIdGeneration(MessageIdGenerationError),
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
            Self::ContentId(error) => write!(f, "invalid related resource Content-Id: {error}"),
            Self::Uri(error) => write!(f, "invalid related resource URI: {error}"),
            Self::InvalidCid => {
                f.write_str("a cid URI must contain only a percent-encoded bare Content-Id")
            }
            Self::CidEncoding(error) => write!(f, "invalid UTF-8 in related resource CID: {error}"),
            Self::ContentLocation(error) => {
                write!(f, "invalid related resource Content-Location: {error}")
            }
            Self::ContentBase(error) => write!(f, "invalid related resource Content-Base: {error}"),
            Self::UriResolution(error) => write!(f, "cannot resolve related resource URI: {error}"),
            Self::ResourceNotFound(id) => {
                write!(f, "no leaf resource with Content-Id <{}>", id.as_str())
            }
            Self::UriResourceNotFound(uri) => write!(f, "no leaf resource matching URI {uri}"),
            Self::ResourceContentType(error) => {
                write!(f, "invalid related resource Content-Type: {error}")
            }
            Self::Io(error) => error.fmt(f),
            Self::MissingRootContentType => {
                f.write_str("new related root has no Content-Type header")
            }
            Self::RootContentType(error) => {
                write!(f, "invalid new related root Content-Type: {error}")
            }
            Self::RootIdGeneration(error) => {
                write!(f, "cannot generate related root Content-Id: {error}")
            }
            Self::RootContentId(error) => write!(f, "invalid new related root Content-Id: {error}"),
            Self::Multipart(error) => error.fmt(f),
        }
    }
}

impl Error for MultipartRelatedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error)
            | Self::RootContentType(error)
            | Self::ResourceContentType(error) => Some(error),
            Self::RootIdGeneration(error) => Some(error),
            Self::RootSelection(error) => Some(error),
            Self::ContentId(error) | Self::RootContentId(error) => Some(error),
            Self::Uri(error) | Self::ContentLocation(error) | Self::ContentBase(error) => {
                Some(error)
            }
            Self::CidEncoding(error) => Some(error),
            Self::UriResolution(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Multipart(error) => Some(error),
            Self::MissingContentType
            | Self::ResourceNotFound(_)
            | Self::UriResourceNotFound(_)
            | Self::InvalidCid
            | Self::NotRelated(_)
            | Self::MissingRootContentType => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MimeContent, TextPart};

    fn non_first_root() -> MultipartRelated {
        let mut related = MultipartRelated::with_boundary("b").unwrap();
        related
            .multipart
            .children_mut()
            .push(MimePart::new(MimeContent::new(Cursor::new(Vec::new()))).into());
        let mut html = TextPart::new("html", "old root").unwrap();
        html.part_mut().set_content_id(Some("old")).unwrap();
        related.multipart.children_mut().push(html.into());
        let mut content_type = related.multipart.content_type().unwrap().unwrap();
        content_type
            .parameters_mut()
            .set_optional("start", Some("<old>".into()));
        related.multipart.set_content_type(&content_type).unwrap();
        related
    }

    #[test]
    fn root_generation_failures_preserve_the_container() {
        for failure in [
            MessageIdGenerationError::InvalidDomain(crate::idna::to_ascii("").unwrap_err()),
            MessageIdGenerationError::ClockOutOfRange,
            MessageIdGenerationError::Random(getrandom::Error::UNSUPPORTED),
        ] {
            let mut related = non_first_root();
            let headers = related.multipart.headers().clone();
            let children = related.multipart.children().as_ptr();
            let description = failure.to_string();
            let error = related
                .set_root_with_generator(
                    TextPart::new("html", "replacement").unwrap().into(),
                    &HeaderFormatOptions::default(),
                    || Err(failure),
                )
                .unwrap_err();
            assert!(matches!(error, MultipartRelatedError::RootIdGeneration(_)));
            assert_eq!(error.source().unwrap().to_string(), description);
            assert!(error.to_string().contains(&description));
            assert_eq!(related.multipart.headers(), &headers);
            assert_eq!(related.multipart.children().as_ptr(), children);
            assert_eq!(related.root_index().unwrap(), Some(1));
            assert_eq!(
                related
                    .root()
                    .unwrap()
                    .unwrap()
                    .content_id()
                    .unwrap()
                    .unwrap()
                    .as_str(),
                "old"
            );
        }
    }

    #[test]
    fn root_ids_are_generated_only_when_needed() {
        let mut related = non_first_root();
        related
            .set_root_with_generator(
                TextPart::new("html", "replacement").unwrap().into(),
                &HeaderFormatOptions::default(),
                || Ok(MessageId::from_value("generated@example.com").unwrap()),
            )
            .unwrap();
        assert_eq!(related.root_index().unwrap(), Some(1));
        assert_eq!(
            related
                .multipart
                .content_type()
                .unwrap()
                .unwrap()
                .parameters()
                .get("start"),
            Some("<generated@example.com>")
        );
        let mut html = TextPart::new("html", "explicit").unwrap();
        html.part_mut().set_content_id(Some("explicit")).unwrap();
        related
            .set_root_with_generator(html.into(), &HeaderFormatOptions::default(), || {
                panic!("existing IDs must not trigger generation")
            })
            .unwrap();
        related.multipart.children_mut().clear();
        related
            .set_root_with_generator(
                TextPart::new("html", "first root").unwrap().into(),
                &HeaderFormatOptions::default(),
                || panic!("first roots must not trigger generation"),
            )
            .unwrap();
        assert_eq!(related.root().unwrap().unwrap().content_id().unwrap(), None);
        assert_eq!(
            related
                .multipart
                .content_type()
                .unwrap()
                .unwrap()
                .parameters()
                .get("start"),
            None
        );
    }
}

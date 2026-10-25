// Adapted from MimeKit's MimeEntity.cs, Multipart.cs, and MimeMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{Cursor, Read, Seek},
    iter::FusedIterator,
    slice,
};

use crate::{
    ContentDisposition, ContentHeaderParseError, Header, HeaderFormatError, HeaderFormatOptions,
    HeaderList, MessageDeliveryStatus, MessageDispositionNotification, MessageFeedbackReport,
    MessageId, MessageIdParseError, MessagePart, MessagePartial, MimePart, Multipart,
    MultipartRelated, MultipartReport, TextBodyError, TextFormat, TextPart, TextRfc822Headers,
};

/// An owned MIME tree node: a body stream, multipart container, or attached message.
///
/// Text parts convert to ordinary leaf parts without copying their headers or
/// content. Their Content-Type retains the text subtype and charset; use
/// [`TextPart::from_part`] after extracting a leaf to regain text-specific APIs.
///
/// All leaves in a tree use the same source type `R`, defaulting to in-memory
/// cursors. Sources can also be borrowed. There are no parent pointers, shared
/// children, change events, or automatically inferred container variants:
/// editing a Content-Type header does not change an enum variant.
///
/// Synchronous writing preserves parsed framing, including truncated trees.
/// [`Self::load`] reads entities into owned sources with explicit resource limits.
/// Attached messages can be constructed explicitly using [`MessagePart`].
/// Loading recognizes transparent message wrappers by default; encoded wrappers
/// and unrecognized message subtypes remain opaque leaves.
/// Programmatic nesting is unrestricted; callers must bound tree depth.
/// Leaf iteration, loading, writing, and preparation are iterative; debugging
/// and drop recurse.
#[derive(Debug)]
pub enum MimeEntity<R = Cursor<Vec<u8>>> {
    /// A leaf with one body stream, including text parts.
    Part(MimePart<R>),
    /// A container with ordered child entities.
    Multipart(Multipart<R>),
    /// A wrapper around an optional structured message.
    Message(MessagePart<R>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParsedEntity {
    pub body_separator: bool,
    pub no_content: bool,
}

pub(crate) fn write_entity_headers<'a, W: std::io::Write + ?Sized>(
    headers: impl IntoIterator<Item = &'a Header>,
    parsed: Option<ParsedEntity>,
    output: &mut W,
    options: &crate::MimePartWriteOptions,
) -> std::io::Result<()> {
    if parsed.is_some_and(|parsed| !parsed.body_separator) {
        HeaderList::write_loaded_headers(headers, output, options.new_line_format)?;
    } else {
        HeaderList::write_terminated_headers(headers, output, options.new_line_format)?;
    }
    if parsed.is_none_or(|parsed| parsed.body_separator) {
        output.write_all(options.validate()?)?;
    }
    Ok(())
}

/// An attachment flag could not be changed without losing existing metadata.
///
/// The original header list and content remain unchanged on either failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentEditError {
    /// The first Content-Disposition could not be parsed.
    ContentDisposition(ContentHeaderParseError),
    /// The changed Content-Disposition could not be formatted.
    HeaderFormat(HeaderFormatError),
}

impl fmt::Display for AttachmentEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContentDisposition(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
        }
    }
}

impl Error for AttachmentEditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentDisposition(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
        }
    }
}

impl<R> MimeEntity<R> {
    pub(crate) fn parsed(&self) -> Option<ParsedEntity> {
        match self {
            Self::Part(part) => part.parsed,
            Self::Multipart(part) => part.original_framing().and(part.parsed),
            Self::Message(part) => part.parsed,
        }
    }

    pub(crate) fn set_parsed(&mut self, parsed: ParsedEntity) {
        match self {
            Self::Part(part) => part.parsed = Some(parsed),
            Self::Multipart(part) => part.parsed = Some(parsed),
            Self::Message(part) => part.parsed = Some(parsed),
        }
    }

    pub(crate) fn ends_with_delimiter_newline(&self) -> bool {
        let mut entity = self;
        loop {
            match entity {
                Self::Part(part) => return part.parsed.is_some_and(|parsed| parsed.no_content),
                Self::Message(part) => {
                    if let Some(body) = part.message().and_then(|message| message.body()) {
                        entity = body;
                    } else {
                        return part.parsed.is_some_and(|parsed| parsed.no_content);
                    }
                }
                Self::Multipart(part) => {
                    let Some(framing) = part.original_framing() else {
                        return false;
                    };
                    if framing.end.is_some() {
                        return false;
                    }
                    // A truncated multipart ends with its trailing marker, last
                    // child, or preamble, which retains the parent's newline.
                    if let Some(trailing) = framing.boundaries.get(part.children().len()) {
                        return trailing.ends_with(b"\n");
                    }
                    if let Some(child) = part.children().last() {
                        entity = child;
                    } else {
                        return true;
                    }
                }
            }
        }
    }

    /// Returns this node's raw headers, without descending into children.
    pub fn headers(&self) -> &HeaderList {
        match self {
            Self::Part(part) => part.headers(),
            Self::Multipart(multipart) => multipart.headers(),
            Self::Message(part) => part.headers(),
        }
    }

    /// Edits this node's raw headers without changing its variant or children.
    pub fn headers_mut(&mut self) -> &mut HeaderList {
        match self {
            Self::Part(part) => part.headers_mut(),
            Self::Multipart(multipart) => multipart.headers_mut(),
            Self::Message(part) => part.headers_mut(),
        }
    }

    /// Reads this node's first Content-Id using MimeKit's tolerant prefix parser.
    ///
    /// Returns a detached [`MessageId`] without outer brackets. Reads raw value
    /// bytes without RFC 2047 decoding. Stops after the first identifier, ignoring
    /// trailing text and later duplicate headers. Missing domains and other
    /// compatibility forms follow [`MessageId::parse_prefix`]; this is not an
    /// authoring validator. Absence returns `Ok(None)`.
    ///
    /// Does not inspect descendants, generate IDs, resolve `cid:` URIs, or
    /// normalize case. Raw edits are observed immediately without caching or
    /// source I/O. A message wrapper's ID is independent of its enclosed message.
    ///
    /// # Errors
    ///
    /// An invalid first value returns its [`MessageIdParseError`], not absence
    /// or a later valid duplicate. Offsets refer to that header's raw value.
    /// This does not prevent tolerant entity or message loading.
    pub fn content_id(&self) -> Result<Option<MessageId>, MessageIdParseError> {
        content_id(self.headers())
    }

    /// Assigns Content-Id, or removes all occurrences for `None`.
    ///
    /// Validates and normalizes with [`MessageId::from_value`] before changing
    /// headers, accepting either bare or bracketed setter-compatible IDs.
    /// Writes `Content-Id: <id>` with CRLF at the first matching position and
    /// removes duplicates, or appends when absent. Like other raw-header part
    /// setters, always reformats equal values rather than reproducing MimeKit's
    /// cache-dependent no-op.
    ///
    /// Empty text is invalid; `None` removes the field. Generates no IDs and
    /// does not check uniqueness, update related-container selectors or HTML
    /// references, or touch sources. Case is retained. The underlying setter
    /// grammar is compatible rather than strict RFC validation; it rejects
    /// NUL and non-folding line breaks but imposes no physical line-length limit.
    ///
    /// # Errors
    ///
    /// Invalid assignment returns [`MessageIdParseError`] with offsets in the
    /// original argument, leaving all headers and content unchanged.
    ///
    /// ```
    /// use mimecraft::{MimeEntity, TextPart};
    ///
    /// let mut entity: MimeEntity = TextPart::new("html", "<p>Document</p>")?.into();
    /// entity.set_content_id(Some("document@example.com"))?;
    /// assert_eq!(entity.content_id()?.unwrap().as_str(), "document@example.com");
    /// entity.set_content_id(None)?;
    /// assert_eq!(entity.content_id()?, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_content_id(&mut self, value: Option<&str>) -> Result<(), MessageIdParseError> {
        set_content_id(self.headers_mut(), value)
    }

    /// Returns whether the first Content-Disposition specifies `attachment`.
    ///
    /// Missing disposition returns false. Filenames, media types, and child
    /// dispositions do not imply attachment status. No source I/O occurs.
    ///
    /// # Errors
    ///
    /// Returns the first matching header's parse error; later duplicates do not
    /// override it. Raw header edits are observed without a metadata cache.
    pub fn is_attachment(&self) -> Result<bool, ContentHeaderParseError> {
        is_attachment(self.headers())
    }

    /// Changes this node's attachment flag, preserving disposition parameters.
    ///
    /// `true` creates an `attachment` disposition when absent, or changes an
    /// existing disposition to `attachment`. `false` changes an attachment to
    /// `inline`; it does not create a missing header or change extension tokens.
    /// Uses default [`HeaderFormatOptions`].
    ///
    /// When the first header already has the requested attachment status, all
    /// raw records (including duplicates) are preserved without reformatting.
    /// Otherwise the first disposition is parsed with default parameter options,
    /// its decoded parameters are retained, and checked formatting precedes
    /// replacement at the first slot and removal of later duplicates.
    /// Malformed parameter fragments skipped by tolerant parsing are not copied
    /// into the changed header; its raw spelling and folds are not preserved.
    ///
    /// Does not change filenames, Content-Type, children, enclosed messages, or
    /// source positions. Text parts expose this through `part_mut()`.
    ///
    /// # Errors
    ///
    /// Returns [`AttachmentEditError`] for an invalid first disposition or
    /// formatting failure, without modifying headers or content. Invalid first
    /// values are not silently replaced or bypassed for a valid duplicate.
    ///
    /// ```
    /// use mimecraft::{MessagePart, MimeMessage};
    ///
    /// let mut part = MessagePart::new(MimeMessage::<()>::new());
    /// part.set_is_attachment(true)?;
    /// assert!(part.is_attachment()?);
    /// part.set_is_attachment(false)?;
    /// assert!(!part.is_attachment()?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_is_attachment(&mut self, value: bool) -> Result<(), AttachmentEditError> {
        self.set_is_attachment_with_options(value, &HeaderFormatOptions::default())
    }

    /// Changes the attachment flag with explicit header formatting options.
    ///
    /// Uses [`Self::set_is_attachment`]'s first-header and parameter semantics.
    /// Options are checked only when writing a changed disposition. No-op
    /// assignments preserve bytes even with invalid formatting options.
    ///
    /// # Errors
    ///
    /// Returns [`AttachmentEditError`] without changing any headers or content.
    pub fn set_is_attachment_with_options(
        &mut self,
        value: bool,
        options: &HeaderFormatOptions,
    ) -> Result<(), AttachmentEditError> {
        set_is_attachment(self.headers_mut(), value, options)
    }

    /// Visits non-multipart nodes in depth-first child order, including this root.
    ///
    /// Attached-message wrappers are returned without descending into their
    /// messages. Multipart nodes are skipped, even when marked as attachments,
    /// but their children are visited. No metadata parsing or source I/O occurs.
    /// Use [`Self::leaf_parts`] to descend into attached messages instead.
    pub fn body_parts(&self) -> BodyParts<'_, R> {
        BodyParts::new(slice::from_ref(self))
    }

    /// Filters [`Self::body_parts`] by their own Content-Disposition headers.
    ///
    /// Yields attached-message wrappers, not their contents. Multipart nodes
    /// themselves are excluded. No source I/O or filename-based inference occurs.
    ///
    /// # Errors
    ///
    /// Each invalid disposition yields its parse error. Iteration may continue
    /// at the next part; collecting into `Result<Vec<_>, _>` stops at the first.
    pub fn attachments(&self) -> Attachments<'_, R> {
        Attachments::new(self.body_parts())
    }

    /// Selects a text body without reading streams or entering attached messages.
    ///
    /// Alternatives are searched last-to-first. Related containers search only
    /// their root: `start` matches normalized, case-sensitive Content-Ids; when
    /// absent, `type` matches a child's media type ignoring case. No match falls
    /// back to the first child (not to `type` when `start` was supplied).
    ///
    /// Other multiparts stop at their first multipart or non-attachment text
    /// child, even if it has no body in the requested format. Root text leaves
    /// exclude attachments. As in MimeKit, alternative children and related
    /// roots are not filtered by disposition; multipart dispositions are ignored.
    ///
    /// A leaf without Content-Type is treated as text/plain, except immediately
    /// inside a digest where it defaults to message/rfc822. No header is inserted.
    /// Selection observes raw edits and first duplicates, uses an
    /// iterative depth-proportional stack, and requires no I/O trait bounds.
    /// Returned parts retain encoded content; no charset decoding, flowed-text
    /// conversion, HTML rendering, URI resolution, or sanitization is performed.
    ///
    /// # Errors
    ///
    /// Malformed metadata needed by the search is an explicit error, not a
    /// reason to try another candidate. Multipart nodes require a valid
    /// multipart Content-Type, but boundaries and transfer encodings are not
    /// inspected. Unvisited fields and attached-message contents are ignored.
    pub fn text_body_part(
        &self,
        format: TextFormat,
    ) -> Result<Option<&MimePart<R>>, TextBodyError> {
        let Some(path) = crate::text_body::select(self, format)? else {
            return Ok(None);
        };
        let mut entity = self;
        for index in path.into_iter().skip(1) {
            let Self::Multipart(part) = entity else {
                unreachable!("body selection paths traverse only multipart nodes");
            };
            entity = &part.children()[index];
        }
        let Self::Part(part) = entity else {
            unreachable!("body selection paths end at leaf parts");
        };
        Ok(Some(part))
    }

    /// Mutably borrows the same body selected by [`Self::text_body_part`].
    ///
    /// Selection finishes before granting access and never changes headers or
    /// source positions. Callers may edit the selected part or read its content.
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::text_body_part`], without
    /// changing the tree.
    pub fn text_body_part_mut(
        &mut self,
        format: TextFormat,
    ) -> Result<Option<&mut MimePart<R>>, TextBodyError> {
        let Some(path) = crate::text_body::select(self, format)? else {
            return Ok(None);
        };
        let mut entity = self;
        for index in path.into_iter().skip(1) {
            let Self::Multipart(part) = entity else {
                unreachable!("body selection paths traverse only multipart nodes");
            };
            entity = &mut part.children_mut()[index];
        }
        let Self::Part(part) = entity else {
            unreachable!("body selection paths end at leaf parts");
        };
        Ok(Some(part))
    }

    /// Visits leaf parts in depth-first child order, including a leaf root.
    ///
    /// Descends into attached messages. Empty containers and absent messages or
    /// bodies contribute no leaves. Traversal does not parse headers,
    /// read sources, or recurse on the call stack.
    pub fn leaf_parts(&self) -> LeafParts<'_, R> {
        LeafParts::new(slice::from_ref(self))
    }
}

pub(crate) fn content_id(headers: &HeaderList) -> Result<Option<MessageId>, MessageIdParseError> {
    headers
        .get("Content-Id")
        .map(|header| MessageId::parse_prefix(header.raw_value()).map(|(id, _)| id))
        .transpose()
}

pub(crate) fn is_attachment(headers: &HeaderList) -> Result<bool, ContentHeaderParseError> {
    match headers.get("Content-Disposition") {
        Some(header) => Ok(header.parse_content_disposition()?.is_attachment()),
        None => Ok(false),
    }
}

pub(crate) fn set_is_attachment(
    headers: &mut HeaderList,
    value: bool,
    options: &HeaderFormatOptions,
) -> Result<(), AttachmentEditError> {
    let disposition = match headers.get("Content-Disposition") {
        Some(header) => {
            let mut disposition = header
                .parse_content_disposition()
                .map_err(AttachmentEditError::ContentDisposition)?;
            if disposition.is_attachment() == value {
                return Ok(());
            }
            disposition.set_is_attachment(value);
            disposition
        }
        None if value => ContentDisposition::default(),
        // Like MimeKit, clearing the flag does not create an inline header.
        // Likewise, a non-attachment extension token above is left unchanged.
        None => return Ok(()),
    };
    let header = disposition
        .to_header_with_options(options)
        .map_err(AttachmentEditError::HeaderFormat)?;
    headers
        .replace_all(header)
        .expect("formatted Content-Disposition always has a parsed field name");
    Ok(())
}

pub(crate) fn set_content_id(
    headers: &mut HeaderList,
    value: Option<&str>,
) -> Result<(), MessageIdParseError> {
    if let Some(value) = value {
        // MimeKit's getter accepts loose raw syntax, but its setter uses the
        // separate value validator. Revalidate even values returned by reading;
        // a successfully parsed raw ID need not be safe to write into a header.
        let id = MessageId::from_value(value)?;
        let header = Header::from_formatted(format!("Content-Id: <{id}>\r\n"));
        headers
            .replace_all(header)
            .expect("formatted Content-Id always has a parsed field name");
    } else {
        headers.remove_all("Content-Id");
    }
    Ok(())
}

impl<R: Read + Seek> MimeEntity<R> {
    /// Reads the selected text body, unwrapping flowed text when declared.
    ///
    /// Uses [`Self::text_body_part`]'s selection rules. `None` means no matching
    /// body; an empty matching body returns `Some(String::new())`. Decoding uses
    /// the stored transfer encoding, then the declared charset or strict UTF-8
    /// when absent. There is no BOM/HTML sniffing, replacement, or fallback.
    ///
    /// For text/plain with `format=flowed`, [`crate::text::FlowedToText`] unwraps
    /// paragraphs, using `delsp=yes` (ASCII-insensitive, without trimming) to
    /// remove soft-break spaces. Both Plain and Flowed requests unwrap such a
    /// part. Converted paragraph endings are CRLF on every platform. Other
    /// formats retain decoded newlines and markup unchanged; no rendering or
    /// sanitization occurs. Read the selected part directly to retain flowed
    /// formatting or to choose a different conversion policy.
    ///
    /// Materializes transfer-decoded bytes, decoded text, and (for flowed text)
    /// converted text in memory. No raw headers or stored bytes are changed.
    ///
    /// # Errors
    ///
    /// Selection and charset-label errors precede I/O. Read/seek errors retain
    /// [`crate::MimeContent::decode_to`]'s source-position behavior; a successful
    /// byte copy rewinds the source even if charset decoding subsequently fails.
    /// Errors never cause selection of an earlier alternative.
    pub fn text_body(&mut self, format: TextFormat) -> Result<Option<String>, TextBodyError> {
        let Some(part) = self.text_body_part_mut(format)? else {
            return Ok(None);
        };
        let content_type = part.content_type().map_err(TextBodyError::ContentType)?;
        let charset = content_type
            .as_ref()
            .map(|value| value.charset_encoding())
            .transpose()
            .map_err(|error| TextBodyError::Text(crate::TextPartError::Charset(error)))?
            .flatten()
            .unwrap_or(crate::charsets::Charset::UTF8);
        let text = crate::text_part::decode_text(part, charset).map_err(TextBodyError::Text)?;
        if let Some(content_type) = content_type
            && TextFormat::Flowed.matches(&content_type)
        {
            let mut converter = crate::text::FlowedToText::new();
            converter.delete_space = content_type
                .parameters()
                .get("delsp")
                .is_some_and(|value| value.eq_ignore_ascii_case("yes"));
            Ok(Some(converter.convert(&text)))
        } else {
            Ok(Some(text))
        }
    }
}

impl<R> From<MimePart<R>> for MimeEntity<R> {
    fn from(part: MimePart<R>) -> Self {
        Self::Part(part)
    }
}

impl<R> From<Multipart<R>> for MimeEntity<R> {
    fn from(multipart: Multipart<R>) -> Self {
        Self::Multipart(multipart)
    }
}

impl<R> From<MultipartReport<R>> for MimeEntity<R> {
    fn from(report: MultipartReport<R>) -> Self {
        Self::Multipart(report.into_multipart())
    }
}

impl<R> From<MultipartRelated<R>> for MimeEntity<R> {
    fn from(related: MultipartRelated<R>) -> Self {
        Self::Multipart(related.into_multipart())
    }
}

impl<R> From<MessagePart<R>> for MimeEntity<R> {
    fn from(part: MessagePart<R>) -> Self {
        Self::Message(part)
    }
}

impl<R> From<TextRfc822Headers<R>> for MimeEntity<R> {
    fn from(headers: TextRfc822Headers<R>) -> Self {
        Self::Message(headers.into_message_part())
    }
}

impl<R: Read + Seek> From<TextPart<R>> for MimeEntity<R> {
    fn from(text: TextPart<R>) -> Self {
        Self::Part(text.into_part())
    }
}

impl<R: Read + Seek> From<MessageDeliveryStatus<R>> for MimeEntity<R> {
    fn from(status: MessageDeliveryStatus<R>) -> Self {
        Self::Part(status.into_part())
    }
}

impl<R: Read + Seek> From<MessageDispositionNotification<R>> for MimeEntity<R> {
    fn from(notification: MessageDispositionNotification<R>) -> Self {
        Self::Part(notification.into_part())
    }
}

impl<R: Read + Seek> From<MessageFeedbackReport<R>> for MimeEntity<R> {
    fn from(report: MessageFeedbackReport<R>) -> Self {
        Self::Part(report.into_part())
    }
}

impl<R: Read + Seek> From<MessagePartial<R>> for MimeEntity<R> {
    fn from(partial: MessagePartial<R>) -> Self {
        Self::Part(partial.into_part())
    }
}

/// A lazy, depth-first iterator over non-multipart nodes of a MIME tree.
///
/// Multipart containers are traversed but not returned. Attached-message
/// wrappers are returned without descending into their messages, including
/// wrappers with no message. Uses storage proportional to multipart depth,
/// with no metadata parsing, source I/O, or recursive traversal.
pub struct BodyParts<'a, R> {
    stack: Vec<slice::Iter<'a, MimeEntity<R>>>,
}

impl<'a, R> BodyParts<'a, R> {
    pub(crate) fn new(entities: &'a [MimeEntity<R>]) -> Self {
        Self {
            stack: vec![entities.iter()],
        }
    }
}

impl<'a, R> Iterator for BodyParts<'a, R> {
    type Item = &'a MimeEntity<R>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(entities) = self.stack.last_mut() {
            match entities.next() {
                Some(MimeEntity::Multipart(multipart)) => {
                    self.stack.push(multipart.children().iter());
                }
                // Attached messages are body parts themselves; do not descend
                // into the contained message when enumerating this body.
                Some(entity) => return Some(entity),
                None => {
                    self.stack.pop();
                }
            }
        }
        None
    }
}

impl<R> FusedIterator for BodyParts<'_, R> {}

/// A lazy attachment filter over [`BodyParts`].
///
/// Only non-multipart nodes whose first Content-Disposition parses as
/// `attachment` are returned. Malformed dispositions yield errors; the next
/// call resumes at the next body part. No body streams or attached-message
/// contents are read or traversed. Collect into a `Result` to stop on error.
pub struct Attachments<'a, R> {
    parts: BodyParts<'a, R>,
}

impl<'a, R> Attachments<'a, R> {
    pub(crate) fn new(parts: BodyParts<'a, R>) -> Self {
        Self { parts }
    }
}

impl<'a, R> Iterator for Attachments<'a, R> {
    type Item = Result<&'a MimeEntity<R>, ContentHeaderParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        for part in self.parts.by_ref() {
            match part.is_attachment() {
                Ok(true) => return Some(Ok(part)),
                Ok(false) => {}
                Err(error) => return Some(Err(error)),
            }
        }
        None
    }
}

impl<R> FusedIterator for Attachments<'_, R> {}

/// A lazy, depth-first iterator over the leaf parts of a MIME tree.
///
/// Descends through multipart containers and attached message bodies. Uses
/// storage proportional to nesting depth; empty containers/messages are skipped.
pub struct LeafParts<'a, R> {
    parts: BodyParts<'a, R>,
}

impl<'a, R> LeafParts<'a, R> {
    pub(crate) fn new(entities: &'a [MimeEntity<R>]) -> Self {
        Self {
            parts: BodyParts::new(entities),
        }
    }
}

impl<'a, R> Iterator for LeafParts<'a, R> {
    type Item = &'a MimePart<R>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.parts.next()? {
                MimeEntity::Part(part) => return Some(part),
                MimeEntity::Message(part) => {
                    if let Some(body) = part.message().and_then(|message| message.body()) {
                        self.parts.stack.push(slice::from_ref(body).iter());
                    }
                }
                MimeEntity::Multipart(_) => unreachable!("body-part iteration skips containers"),
            }
        }
    }
}

impl<R> FusedIterator for LeafParts<'_, R> {}

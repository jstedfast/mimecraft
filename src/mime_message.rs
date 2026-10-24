// Adapted from MimeKit's MimeMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, Cursor, Read, Seek, Write};

use crate::{
    Attachments, BodyParts, Header, HeaderList, LeafParts, MessageId, MessageIdParseError,
    MessageImportance, MessagePriority, MessagePriorityParseError, MimeEntity,
    MimeEntityParseOptions, MimePart, MimePartWriteOptions, References, TextBodyError, TextFormat,
    XMessagePriority,
};

/// An owned message with raw message headers and an optional MIME body tree.
///
/// Message headers normally contain From, To, Subject, Date, MIME-Version, and
/// other non-Content fields. Top-level `Content-*` headers belong to the body.
/// [`Self::load`] partitions them using [`crate::Header::is_content_header`];
/// raw constructors and edits preserve supplied placement without filtering,
/// validation, synchronization, or duplicate removal.
///
/// Unlike MimeKit's default constructor and writer, this model never supplies
/// the current date, generated Message-Id, empty From/Subject, or MIME-Version.
/// Supply required metadata explicitly. It has no mutable parsed-value caches
/// or change events: use existing typed header parsers and factories through
/// [`Self::headers`] and [`Self::headers_mut`].
/// Importance and priority conveniences read those raw headers on every call;
/// missing fields mean Normal, while invalid present values return errors.
/// Identifier and reply-threading conveniences likewise read current headers,
/// with explicit parse errors and assignment validation before edits.
///
/// The body and every descendant share source type `R`. Ownership replaces
/// disposal and parent tracking. Body removal returns the old tree without
/// closing or repositioning sources. An absent body is distinct in this model
/// from a leaf with empty content, but loading does not retain that distinction.
/// [`crate::MessagePart`] wraps attached messages, constructed explicitly or
/// parsed by the entity loader. Mbox markers and SMTP envelopes remain deferred.
///
/// ```
/// use mimecraft::{AddressList, Header, MimeMessage, TextPart};
///
/// let mut message = MimeMessage::with_body(TextPart::new("plain", "hello")?.into());
/// message.headers_mut().add(
///     AddressList::parse(b"Alice <alice@example.com>")?.to_header("From")?,
/// );
/// message.headers_mut().add(Header::parse(b"Subject: greeting")?);
/// message.headers_mut().add(Header::parse(b"MIME-Version: 1.0")?);
/// assert_eq!(message.leaf_parts().count(), 1);
/// let mut output = Vec::new();
/// message.write_to(&mut output)?;
/// assert!(output.ends_with(b"\r\n\r\nhello"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MimeMessage<R = Cursor<Vec<u8>>> {
    headers: HeaderList,
    body: Option<MimeEntity<R>>,
}

impl<R> MimeMessage<R> {
    /// Creates an empty message with no headers or body, without I/O.
    pub fn new() -> Self {
        Self::from_parts(HeaderList::new(), None)
    }

    /// Takes a body tree, leaving message headers empty and sources untouched.
    ///
    /// Headers already on the body are not promoted to the message header list.
    pub fn with_body(body: MimeEntity<R>) -> Self {
        Self::from_parts(HeaderList::new(), Some(body))
    }

    /// Takes raw headers and an optional body without validation or copying.
    ///
    /// Missing, duplicate, malformed, or misplaced headers remain unchanged.
    pub fn from_parts(headers: HeaderList, body: Option<MimeEntity<R>>) -> Self {
        Self { headers, body }
    }

    /// Returns message headers, without including headers from the body.
    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    /// Edits message headers without moving fields to or from the body.
    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    /// Reads the first Message-Id header without decoding encoded words.
    ///
    /// Returns `None` only when absent. Uses [`MessageId::parse_prefix`], like
    /// MimeKit's message-ID getter: suffix bytes after a bracketed ID are ignored.
    /// Raw edits are visible immediately; later duplicates and body headers are
    /// not inspected. This does not validate the identifier for authoring.
    ///
    /// # Errors
    ///
    /// Returns the first header's parse error, rather than MimeKit's null fallback.
    /// Offsets are relative to that header's raw value.
    pub fn message_id(&self) -> Result<Option<MessageId>, MessageIdParseError> {
        self.identifier("Message-Id")
    }

    /// Assigns Message-Id using [`MessageId::from_value`]'s compatibility rules.
    ///
    /// Validates before editing, even when the value compares equal. An equal
    /// normalized identifier leaves the first header and any duplicates intact.
    /// Otherwise replaces all occurrences at the first slot, or appends if absent.
    /// Malformed existing values are replaced. No source I/O occurs.
    ///
    /// Newly written headers use angle brackets, UTF-8, and CRLF, without
    /// folding or a length limit, matching MimeKit's single-ID formatting.
    /// No identifier is generated. Remove raw occurrences to restore absence.
    /// This is not strict RFC validation or sanitization of existing headers.
    ///
    /// # Errors
    ///
    /// Invalid assignment values leave all headers unchanged. Error offsets
    /// refer to the supplied string.
    pub fn set_message_id(&mut self, value: &str) -> Result<(), MessageIdParseError> {
        self.set_identifier("Message-Id", value)
    }

    /// Reads the first Resent-Message-Id using [`Self::message_id`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns the first header's parse error without checking later duplicates.
    pub fn resent_message_id(&self) -> Result<Option<MessageId>, MessageIdParseError> {
        self.identifier("Resent-Message-Id")
    }

    /// Assigns Resent-Message-Id independently using [`Self::set_message_id`]'s rules.
    ///
    /// Does not create a resent-header block or change any other field.
    ///
    /// # Errors
    ///
    /// Invalid assignment values leave all headers unchanged.
    pub fn set_resent_message_id(&mut self, value: &str) -> Result<(), MessageIdParseError> {
        self.set_identifier("Resent-Message-Id", value)
    }

    /// Reads the first bracketed identifier from the first In-Reply-To header.
    ///
    /// Uses [`References`], not the Message-Id prefix grammar: prose, comments,
    /// and quoted prose are skipped; bare identifiers are not returned.
    /// Missing headers or values with no candidates return `None`. Only the
    /// first iterator result is inspected, not later candidates or duplicates.
    ///
    /// # Errors
    ///
    /// Returns an error encountered before the first valid identifier rather
    /// than silently skipping it. Offsets refer to the first header's raw value.
    /// Use [`References::new`] directly to continue scanning after an error.
    pub fn in_reply_to(&self) -> Result<Option<MessageId>, MessageIdParseError> {
        self.header_references("In-Reply-To").next().transpose()
    }

    /// Assigns or removes In-Reply-To without updating References.
    ///
    /// `Some` follows [`Self::set_message_id`]'s validation, formatting, and
    /// equality rules, comparing with [`Self::in_reply_to`]. An equal first ID
    /// therefore preserves other IDs in that same raw header. `None` removes
    /// every occurrence, including malformed values and duplicates.
    ///
    /// # Errors
    ///
    /// Invalid assignment values leave all headers unchanged.
    pub fn set_in_reply_to(&mut self, value: Option<&str>) -> Result<(), MessageIdParseError> {
        match value {
            Some(value) => self.set_identifier("In-Reply-To", value),
            None => {
                self.headers.remove_all("In-Reply-To");
                Ok(())
            }
        }
    }

    /// Lazily enumerates identifiers from the first References header.
    ///
    /// Missing headers yield an empty iterator. Order, duplicates, explicit
    /// parse errors, recovery, and raw-value-relative offsets follow [`References`].
    /// Later header occurrences are ignored. This borrows raw bytes without
    /// allocating a list, caching parsed state, or inspecting body sources.
    pub fn references(&self) -> References<'_> {
        self.header_references("References")
    }

    /// Replaces References with an ordered sequence of assignment-style IDs.
    ///
    /// Each value uses [`MessageId::from_value`]; order and duplicate IDs are
    /// retained. An empty sequence removes every References header. Otherwise
    /// replaces all occurrences at the first slot, or appends if absent.
    /// Unlike the single-ID setters, this always reformats a nonempty sequence.
    ///
    /// Output is UTF-8 with angle brackets and CRLF. Following MimeKit's
    /// reference-header formatting, folds occur only between IDs at a soft
    /// 78-byte target. The first ID stays on the field line, and oversized IDs
    /// are never split or truncated. This is not a transport line-length check.
    /// Neither In-Reply-To nor any other header or body is changed.
    ///
    /// # Errors
    ///
    /// Stops at the first invalid assignment. No headers are changed, even
    /// when earlier values were valid. Error offsets refer to that input value.
    ///
    /// ```
    /// use mimecraft::{MessageId, MimeMessage};
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// message.set_message_id("reply@example.com")?;
    /// message.set_in_reply_to(Some("<parent@example.com>"))?;
    /// message.set_references(["root@example.com", "parent@example.com"])?;
    /// let references = message.references().collect::<Result<Vec<MessageId>, _>>()?;
    /// assert_eq!(references.len(), 2);
    /// assert_eq!(message.message_id()?.unwrap().as_str(), "reply@example.com");
    /// # Ok::<(), mimecraft::MessageIdParseError>(())
    /// ```
    pub fn set_references<I, S>(&mut self, values: I) -> Result<(), MessageIdParseError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut raw = String::from("References:");
        let mut column = raw.len();
        let mut empty = true;
        for value in values {
            let id = MessageId::from_value(value.as_ref())?;
            let length = id.as_str().len() + 2;
            // Fold between identifiers, never inside them. A single oversized
            // identifier is retained, as in MimeKit's EncodeReferencesHeader.
            if !empty && column + 1 + length > 78 {
                raw.push_str("\r\n\t");
                column = 1;
            } else {
                raw.push(' ');
                column += 1;
            }
            raw.push('<');
            raw.push_str(id.as_str());
            raw.push('>');
            column += length;
            empty = false;
        }
        // Stage the entire replacement before touching the raw header list,
        // so an invalid later ID cannot commit an incomplete reference chain.
        if empty {
            self.headers.remove_all("References");
        } else {
            raw.push_str("\r\n");
            self.replace_formatted(Header::from_formatted(raw));
        }
        Ok(())
    }

    fn identifier(&self, field: &str) -> Result<Option<MessageId>, MessageIdParseError> {
        self.headers
            .get(field)
            .map(|header| MessageId::parse_prefix(header.raw_value()).map(|(id, _)| id))
            .transpose()
    }

    fn header_references(&self, field: &str) -> References<'_> {
        References::new(self.headers.get(field).map_or(b"", Header::raw_value))
    }

    fn set_identifier(&mut self, field: &str, value: &str) -> Result<(), MessageIdParseError> {
        let id = MessageId::from_value(value)?;
        let current = if field == "In-Reply-To" {
            self.in_reply_to()
        } else {
            self.identifier(field)
        };
        if !matches!(current, Ok(Some(current)) if current == id) {
            self.replace_formatted(Header::from_formatted(format!("{field}: <{id}>\r\n")));
        }
        Ok(())
    }

    /// Reads the first Importance header, or Normal if absent.
    ///
    /// Decodes using [`Header::decode_value`] before [`MessageImportance::parse`],
    /// matching MimeKit's decoded-value access. Charset-recovery diagnostics are
    /// available by decoding that header directly. Raw edits are visible
    /// immediately; duplicate headers and the body are not inspected.
    ///
    /// # Errors
    ///
    /// Returns [`MessagePriorityParseError::Importance`] for invalid first values,
    /// rather than MimeKit's Normal fallback, even if a later duplicate is valid.
    pub fn importance(&self) -> Result<MessageImportance, MessagePriorityParseError> {
        match self.headers.get("Importance") {
            Some(header) => MessageImportance::parse(&header.decode_value().text),
            None => Ok(MessageImportance::Normal),
        }
    }

    /// Sets Importance without changing either priority header or the body.
    ///
    /// An equal parsed value is a no-op, preserving bytes and duplicates;
    /// setting an absent field to Normal does not add it. Otherwise writes a
    /// canonical CRLF record at the first slot and removes later duplicates,
    /// appending if absent. Invalid existing values are replaced, not defaulted.
    /// No source I/O occurs. Remove all raw occurrences to restore absence.
    pub fn set_importance(&mut self, value: MessageImportance) {
        // As in MimeKit, setting an absent field to Normal changes nothing.
        // Compare freshly parsed metadata rather than a potentially stale cache.
        if self.importance() != Ok(value) {
            self.replace_formatted(value.to_header());
        }
    }

    /// Reads the first Priority header, or Normal if absent.
    ///
    /// Uses [`Header::decode_value`] then [`MessagePriority::parse`]. Decoding
    /// diagnostics remain available on the raw header. Does not derive priority
    /// from Importance or X-Priority, inspect the body, or cache results.
    ///
    /// # Errors
    ///
    /// Returns [`MessagePriorityParseError::Priority`] for an invalid first value,
    /// even if a later duplicate is valid.
    pub fn priority(&self) -> Result<MessagePriority, MessagePriorityParseError> {
        match self.headers.get("Priority") {
            Some(header) => MessagePriority::parse(&header.decode_value().text),
            None => Ok(MessagePriority::Normal),
        }
    }

    /// Sets Priority independently, with [`Self::set_importance`]'s no-op,
    /// duplicate-replacement, invalid-value repair, and CRLF formatting rules.
    ///
    /// ```
    /// use mimecraft::{MessageImportance, MessagePriority, MimeMessage, XMessagePriority};
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// message.set_priority(MessagePriority::Normal);
    /// assert!(message.headers().is_empty());
    /// message.set_importance(MessageImportance::High);
    /// message.set_priority(MessagePriority::Urgent);
    /// message.set_x_priority(XMessagePriority::High);
    /// assert_eq!(message.priority()?, MessagePriority::Urgent);
    /// assert_eq!(message.x_priority()?.number(), 2);
    /// # Ok::<(), mimecraft::MessagePriorityParseError>(())
    /// ```
    pub fn set_priority(&mut self, value: MessagePriority) {
        if self.priority() != Ok(value) {
            self.replace_formatted(value.to_header());
        }
    }

    /// Reads the first X-Priority header, or Normal if absent.
    ///
    /// Uses [`XMessagePriority::parse`] on raw bytes, not decoded text. Numeric
    /// prefixes are clamped to 1 through 5 after checked 32-bit parsing, and
    /// trailing bytes are ignored. No cache, source I/O, or other fields are used.
    ///
    /// # Errors
    ///
    /// Returns [`MessagePriorityParseError::XPriority`] for missing digits or
    /// integer overflow in a present first header, rather than defaulting.
    pub fn x_priority(&self) -> Result<XMessagePriority, MessagePriorityParseError> {
        match self.headers.get("X-Priority") {
            Some(header) => XMessagePriority::parse(header.raw_value()),
            None => Ok(XMessagePriority::Normal),
        }
    }

    /// Sets X-Priority independently, with [`Self::set_importance`]'s edit rules.
    ///
    /// New records include the numeric value and a comment, e.g. `2 (High)`.
    /// An equal clamped value is a no-op: setting `0` to Highest preserves `0`.
    pub fn set_x_priority(&mut self, value: XMessagePriority) {
        if self.x_priority() != Ok(value) {
            self.replace_formatted(value.to_header());
        }
    }

    fn replace_formatted(&mut self, header: Header) {
        self.headers
            .replace_all(header)
            .expect("typed formatters always produce a header with a parsed field name");
    }

    /// Returns the body tree, if present.
    pub fn body(&self) -> Option<&MimeEntity<R>> {
        self.body.as_ref()
    }

    /// Edits the body tree without synchronizing message headers.
    pub fn body_mut(&mut self) -> Option<&mut MimeEntity<R>> {
        self.body.as_mut()
    }

    /// Replaces or removes the body, returning the old tree without source I/O.
    pub fn set_body(&mut self, body: Option<MimeEntity<R>>) -> Option<MimeEntity<R>> {
        std::mem::replace(&mut self.body, body)
    }

    /// Extracts headers and the body without copying or closing sources.
    pub fn into_parts(self) -> (HeaderList, Option<MimeEntity<R>>) {
        (self.headers, self.body)
    }

    /// Visits non-multipart body parts without entering attached messages.
    ///
    /// Returns leaves and attached-message wrappers in depth-first child order.
    /// Multipart nodes are skipped, but their children are visited. An absent
    /// body yields nothing. No metadata parsing or source I/O occurs.
    pub fn body_parts(&self) -> BodyParts<'_, R> {
        BodyParts::new(self.body.as_slice())
    }

    /// Visits body parts whose own Content-Disposition specifies `attachment`.
    ///
    /// Filters [`Self::body_parts`], including attached-message wrappers but not
    /// their contents or multipart nodes themselves. Filenames alone do not
    /// imply attachments, and no source I/O occurs.
    ///
    /// # Errors
    ///
    /// Malformed dispositions yield parse errors. Iteration may continue at the
    /// next part; collect into `Result<Vec<_>, _>` for fail-fast processing.
    ///
    /// ```
    /// use mimecraft::{Header, MessagePart, MimeMessage, TextPart};
    ///
    /// let inner = MimeMessage::with_body(TextPart::new("plain", "attached")?.into());
    /// let mut part = MessagePart::new(inner);
    /// part.headers_mut().add(Header::parse(b"Content-Disposition: attachment")?);
    /// let message = MimeMessage::with_body(part.into());
    /// let attachments = message.attachments().collect::<Result<Vec<_>, _>>()?;
    /// assert_eq!(attachments.len(), 1);
    /// assert_eq!(message.body_parts().count(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn attachments(&self) -> Attachments<'_, R> {
        Attachments::new(self.body_parts())
    }

    /// Borrows the body in a requested format, or returns `None` if absent.
    ///
    /// Uses [`MimeEntity::text_body_part`]'s alternative, related, and mixed
    /// selection rules. Does not read sources or enter attached messages.
    /// Message-level headers do not participate in selection.
    ///
    /// # Errors
    ///
    /// Returns errors for invalid metadata needed by the search.
    ///
    /// ```
    /// use mimecraft::{MimeMessage, Multipart, TextFormat, TextPart};
    ///
    /// let mut alternative = Multipart::new("alternative", "boundary")?;
    /// alternative.children_mut().push(TextPart::new("plain", "hello")?.into());
    /// alternative.children_mut().push(TextPart::new("html", "<b>hello</b>")?.into());
    /// let mut message = MimeMessage::with_body(alternative.into());
    /// assert!(message.text_body_part(TextFormat::Plain)?.is_some());
    /// let html = message.text_body_part_mut(TextFormat::Html)?.unwrap();
    /// let mut decoded = Vec::new();
    /// html.content_mut().decode_to(&mut decoded)?;
    /// assert_eq!(decoded, b"<b>hello</b>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn text_body_part(
        &self,
        format: TextFormat,
    ) -> Result<Option<&MimePart<R>>, TextBodyError> {
        match self.body.as_ref() {
            Some(body) => body.text_body_part(format),
            None => Ok(None),
        }
    }

    /// Mutably borrows the body selected by [`Self::text_body_part`].
    ///
    /// Does not read streams, rewrite metadata, or change source positions.
    ///
    /// # Errors
    ///
    /// Returns the same metadata errors as [`Self::text_body_part`].
    pub fn text_body_part_mut(
        &mut self,
        format: TextFormat,
    ) -> Result<Option<&mut MimePart<R>>, TextBodyError> {
        match self.body.as_mut() {
            Some(body) => body.text_body_part_mut(format),
            None => Ok(None),
        }
    }

    /// Visits body leaves depth-first, including those inside attached messages.
    ///
    /// Does no source I/O or metadata parsing. An absent body or empty multipart
    /// yields no leaves. Traversal is iterative; ordinary model drop and derived
    /// debugging still recurse.
    pub fn leaf_parts(&self) -> LeafParts<'_, R> {
        LeafParts::new(self.body.as_slice())
    }
}

impl<R> Default for MimeMessage<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl MimeMessage {
    /// Loads one complete message through EOF into owned in-memory body streams.
    ///
    /// Uses default [`MimeEntityParseOptions`]. See [`Self::load_with_options`]
    /// for header partitioning and limitations.
    ///
    /// # Errors
    ///
    /// Returns the entity loader's structure, metadata, resource-limit, and I/O
    /// errors, without a partial message or source-position rollback.
    pub fn load<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Self> {
        Self::load_with_options(reader, &MimeEntityParseOptions::default())
    }

    /// Loads a complete message using the entity loader's explicit limits.
    ///
    /// [`MimeEntity::load_with_options`] reads the entire message, including its
    /// headers in the input-byte limit. Depth counts the body root as one, not
    /// the message wrapper. All parsed root `Content-*` records move to the
    /// body; everything else, including MIME-Version and malformed records,
    /// moves to the message headers. Embedded message roots are partitioned
    /// separately; ordinary descendant entity headers are not repartitioned.
    /// Records retain their bytes and relative order within each list, but
    /// original interleaving between the lists is not tracked.
    ///
    /// The body is always `Some`, even for empty content. Sources start at zero
    /// and retain encoded bytes and their stored encodings. Input must contain
    /// a header/body separator and complete multipart structure. This is not
    /// message-field validation: required From/Date fields are not enforced,
    /// and non-content metadata is not parsed or repaired.
    ///
    /// Buffering, resource limits, and recovery rules are those of the entity
    /// loader, including optional structured attached-message parsing and shared
    /// limits across their bodies. No seeking/closing, mbox processing, zero-copy
    /// persistence, or async I/O is performed.
    ///
    /// # Errors
    ///
    /// Propagates [`MimeEntity::load_with_options`] errors unchanged, including
    /// `InvalidInput` before I/O for zero limits. On failure, no partial message
    /// is returned and restarting requires a fresh or repositioned source.
    ///
    /// ```
    /// use mimecraft::MimeMessage;
    ///
    /// let mut input = &b"Subject: hi\r\nContent-Type: text/plain\r\n\r\nbody"[..];
    /// let message = MimeMessage::load(&mut input)?;
    /// assert!(message.headers().contains("Subject"));
    /// assert!(!message.headers().contains("Content-Type"));
    /// assert!(message.body().unwrap().headers().contains("Content-Type"));
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn load_with_options<R: BufRead + ?Sized>(
        reader: &mut R,
        options: &MimeEntityParseOptions,
    ) -> io::Result<Self> {
        Ok(Self::from_loaded_entity(MimeEntity::load_with_options(
            reader, options,
        )?))
    }

    pub(crate) fn from_loaded_entity(mut body: MimeEntity) -> Self {
        let mut headers = HeaderList::new();
        let mut content_headers = HeaderList::new();
        // Only a message root splits Content-* fields into its body.
        // Ordinary descendant entities retain all of their headers.
        for header in std::mem::take(body.headers_mut()) {
            if header.is_content_header() {
                content_headers.add(header);
            } else {
                headers.add(header);
            }
        }
        *body.headers_mut() = content_headers;
        Self::from_parts(headers, Some(body))
    }
}

impl<R: Read + Seek> MimeMessage<R> {
    /// Reads the body in a requested format, or returns `None` when absent.
    ///
    /// Uses [`MimeEntity::text_body`] for selection, stored transfer decoding,
    /// strict charset decoding, and flowed-text unwrapping. Only the selected
    /// source is read. Other formats retain decoded text/markup and newlines.
    /// This materializes the body in memory and does not sanitize HTML.
    ///
    /// # Errors
    ///
    /// Returns metadata, charset, and I/O errors without trying another body.
    /// A successful byte copy rewinds its source before charset decoding.
    ///
    /// ```
    /// use mimecraft::{MimeMessage, TextFormat};
    ///
    /// let mut input = &b"Content-Type: text/plain; format=flowed\r\n\r\nhello \r\nworld"[..];
    /// let mut message = MimeMessage::load(&mut input)?;
    /// assert_eq!(message.text_body(TextFormat::Plain)?.as_deref(), Some("hello world\r\n"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn text_body(&mut self, format: TextFormat) -> Result<Option<String>, TextBodyError> {
        match self.body.as_mut() {
            Some(body) => body.text_body(format),
            None => Ok(None),
        }
    }

    /// Writes the message using default CRLF and body-encoding options.
    ///
    /// See [`Self::write_to_with_options`] for header order and failure behavior.
    ///
    /// # Errors
    ///
    /// Propagates body metadata and I/O errors, possibly after partial output.
    pub fn write_to<W: Write + ?Sized>(&mut self, output: &mut W) -> io::Result<()> {
        self.write_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes message headers, body headers, one separator, and body content.
    ///
    /// Message headers precede all body headers; original interleaving is not
    /// reconstructed. The existing header writer normalizes newlines and
    /// terminates truncated records without changing stored bytes. With a body,
    /// its entity writer supplies body headers and the separator; without one,
    /// this method supplies the separator. An empty message writes one newline.
    ///
    /// Body encodings, framing, and rewind rules follow [`MimeEntity::write_to_with_options`].
    /// No headers are generated, removed, or repartitioned. In particular,
    /// **Bcc and Resent-Bcc are written unchanged**; this is not SMTP transport
    /// preparation. Raw edits can introduce conflicting Content-* fields in
    /// message headers; callers are responsible for correct placement.
    ///
    /// Sources and the destination remain owned by their callers. The sink is
    /// flushed, never closed; nested encoding sinks need their own finalization.
    ///
    /// # Errors
    ///
    /// Invalid options fail with `InvalidInput` before source I/O or output.
    /// Body metadata is validated by its writer after message headers have
    /// been written. I/O errors propagate, possibly after partial output;
    /// nothing is rolled back, and retrying repeats the whole message.
    pub fn write_to_with_options<W: Write + ?Sized>(
        &mut self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        crate::mime_entity_write::write_message(self, output, options)
    }
}

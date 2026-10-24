// Adapted from MimeKit's MimeMessage.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, Cursor, Read, Seek, Write};

use crate::{
    AddressHeaderFormatError, AddressHeaderFormatOptions, AddressList, AddressParseError,
    AddressParseOptions, Attachments, BodyParts, DateTimeOffset, DateTimeOffsetError, Header,
    HeaderList, LeafParts, MailboxAddress, MailboxParseError, MailboxParseOptions, MessageId,
    MessageIdParseError, MessageImportance, MessagePriority, MessagePriorityParseError, MimeEntity,
    MimeEntityParseOptions, MimePart, MimePartWriteOptions, MimeVersion, MimeVersionParseError,
    References, TextBodyError, TextFormat, XMessagePriority,
};

/// An owned message with raw message headers and an optional MIME body tree.
///
/// Message headers normally contain From, To, Subject, Date, MIME-Version, and
/// other non-Content fields. Top-level `Content-*` headers belong to the body.
/// [`Self::load`] partitions them using [`crate::Header::is_content_header`];
/// raw constructors and edits preserve supplied placement without filtering,
/// validation, synchronization, or duplicate removal.
///
/// Constructors and writers never supply the current date, generated Message-Id,
/// empty From/Subject, or MIME-Version, unlike MimeKit's default behavior.
/// Explicit [`Self::prepare`] can insert MIME-Version for a body with headers;
/// supply other required metadata explicitly. There are no mutable parsed-value
/// caches or change events: use existing typed header parsers and factories through
/// [`Self::headers`] and [`Self::headers_mut`].
/// Importance and priority conveniences read those raw headers on every call;
/// missing fields mean Normal, while invalid present values return errors.
/// Identifier and reply-threading conveniences likewise read current headers,
/// with explicit parse errors and assignment validation before edits.
/// Date and MIME-Version accessors use existing value parsers, with no default
/// for an absent field and no cache hiding subsequent raw edits.
/// Sender accessors return detached mailboxes; explicit setters format a
/// complete replacement before editing headers.
/// Address-list accessors aggregate all matching message headers in order,
/// returning detached lists with MimeKit-style recovery and diagnostics.
/// Subject uses tolerant decoding and checked unstructured-header authoring,
/// replacing duplicate Subjects with one canonical record as in MimeKit.
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

/// Addresses recovered from message headers, together with syntax diagnostics.
///
/// Groups, duplicates, order, and per-address decoding diagnostics are retained.
/// Missing or comment-only headers contribute no addresses or issues.
/// Raw headers are never modified by recovery.
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct MessageAddresses {
    /// The recovered, detached address list.
    pub addresses: AddressList,
    /// Syntax failures encountered while recovering addresses, in parse order.
    pub issues: Vec<MessageAddressIssue>,
}

/// A syntax diagnostic from one message address header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageAddressIssue {
    /// Zero-based index in the entire message header collection at parse time.
    pub header_index: usize,
    /// The failure, with an offset relative to that header's raw value.
    pub error: AddressParseError,
}

impl<R> MimeMessage<R> {
    /// Iterates over every entity in depth-first order, including this message's
    /// body, multipart containers, and attached message bodies.
    ///
    /// [`crate::MimeIterator`] provides current-path, parent, ancestor, containing
    /// message, and subtree-skipping operations. No source I/O or header parsing
    /// occurs. The message is immutably borrowed throughout traversal.
    pub fn iter(&self) -> crate::MimeIterator<'_, R> {
        crate::MimeIterator::new(self)
    }

    /// Looks up a MIME entity by its part path without scanning unrelated siblings.
    ///
    /// Uses [`crate::MimeIterator::move_to`]'s numbering: `0` denotes a multipart
    /// message body, while `1` denotes a non-multipart message body or the first
    /// child of a multipart body. Attached messages continue the path.
    ///
    /// Returns `Ok(None)` for a valid but nonexistent path or absent body.
    /// No header parsing, source I/O, or tree mutation occurs.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MimePathParseError`] for invalid syntax, unlike MimeKit's
    /// `TryGetEntity`, which conflates invalid and nonexistent paths.
    pub fn entity_at(
        &self,
        path: &str,
    ) -> Result<Option<&MimeEntity<R>>, crate::MimePathParseError> {
        let mut iter = self.iter();
        iter.move_to(path)?;
        Ok(iter.current())
    }

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

    /// Writes just the message's complete header section using default CRLF options.
    ///
    /// Includes root-body headers and the blank header/body separator, but no
    /// content. See [`Self::write_headers_to_with_options`] for framing and errors.
    ///
    /// # Errors
    ///
    /// Propagates destination write and flush errors, possibly after partial output.
    pub fn write_headers_to<W: Write + ?Sized>(&self, output: &mut W) -> io::Result<()> {
        self.write_headers_to_with_options(output, &MimePartWriteOptions::default())
    }

    /// Writes message headers, root-body headers, and one blank separator.
    ///
    /// This is the headers-only form of [`Self::write_to_with_options`]. Message
    /// headers precede root-body headers, retaining order within each list, not
    /// original interleaving. The existing header writer normalizes newlines and
    /// adds missing record terminators without decoding, refolding, or changing
    /// stored bytes. An entirely empty message writes one newline.
    ///
    /// Does not traverse descendants, inspect multipart boundaries or transfer
    /// encodings, write preambles/epilogues, or read/seek content sources. An
    /// attached-message root contributes only its wrapper headers, not headers
    /// from the enclosed message. Requires neither mutable access nor source I/O
    /// trait bounds; malformed body metadata does not prevent header output.
    ///
    /// No headers are generated, filtered, or repartitioned. In particular,
    /// **Bcc and Resent-Bcc remain visible** and MIME-Version is not synthesized.
    /// Raw header output is not transport validation or sanitization.
    ///
    /// Validates the same options as full serialization. The newline choice
    /// applies to headers and the separator; `ensure_new_line` and valid encoder
    /// line widths have no further effect because no body is written.
    /// The destination is flushed, never closed; nested encoding sinks still
    /// require their own finalization.
    ///
    /// # Errors
    ///
    /// Invalid options return `InvalidInput` before any output or flush. Write
    /// and flush failures propagate, possibly after partial or complete output.
    /// Retrying starts from the beginning; source positions are always unchanged.
    ///
    /// ```
    /// use mimecraft::{MimeMessage, TextPart};
    ///
    /// let mut message = MimeMessage::with_body(TextPart::new("plain", "body")?.into());
    /// message.set_subject("hello")?;
    /// let mut headers = Vec::new();
    /// message.write_headers_to(&mut headers)?;
    /// assert!(headers.starts_with(b"Subject: hello\r\n"));
    /// assert!(headers.ends_with(b"\r\n\r\n"));
    /// assert!(!headers.ends_with(b"body"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn write_headers_to_with_options<W: Write + ?Sized>(
        &self,
        output: &mut W,
        options: &MimePartWriteOptions,
    ) -> io::Result<()> {
        let newline = options.validate()?;
        // Write message headers together with the body's headers, if any, and
        // the blank line separating them from content. Use our existing grouped
        // ordering rather than MimeKit's offset-based merge; do not enter bodies.
        self.headers
            .write_terminated(output, options.new_line_format)?;
        if let Some(body) = &self.body {
            body.headers()
                .write_terminated(output, options.new_line_format)?;
        }
        output.write_all(newline)?;
        output.flush()
    }

    /// Decodes the first Subject header with default RFC 2047 options.
    ///
    /// Returns `None` only when absent. An empty Subject is present empty text.
    /// See [`Self::subject_with_options`] for decoding and diagnostic details.
    pub fn subject(&self) -> Option<crate::rfc2047::DecodedHeader> {
        self.subject_with_options(&crate::rfc2047::DecodeOptions::default())
    }

    /// Decodes the first Subject header using explicit RFC 2047 options.
    ///
    /// Uses [`Header::decode_value_with_options`]: tolerant encoded-word decoding,
    /// charset fallback diagnostics, then display unfolding. Malformed wrappers
    /// remain literal text. Issue ranges refer to the first header's raw value.
    /// Later duplicates and body headers are ignored.
    ///
    /// Reads current bytes without caching or source I/O. Returned text is owned;
    /// editing it does not change the message. Decoded controls other than CR/LF
    /// remain present; this is not display-context sanitization.
    pub fn subject_with_options(
        &self,
        options: &crate::rfc2047::DecodeOptions,
    ) -> Option<crate::rfc2047::DecodedHeader> {
        self.headers
            .get("Subject")
            .map(|header| header.decode_value_with_options(options))
    }

    /// Assigns Subject with default unstructured-header formatting.
    ///
    /// See [`Self::set_subject_with_options`] for normalization and replacement.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_subject(&mut self, value: &str) -> Result<(), crate::UnstructuredHeaderFormatError> {
        self.set_subject_with_options(value, &crate::UnstructuredHeaderFormatOptions::default())
    }

    /// Assigns Subject using checked, atomic unstructured-header formatting.
    ///
    /// Like MimeKit, trims Unicode whitespace and removes CR/LF before encoding.
    /// Other controls and literal encoded-word lookalikes are encoded, not
    /// discarded or interpreted. See [`Header::from_unstructured_with_options`].
    ///
    /// Replaces all Subjects with one canonical `Subject` at the first slot,
    /// as in MimeKit's Subject setter. If absent, appends `Subject`.
    /// Always reformats, even for equal decoded text. Empty text writes an empty
    /// header; use [`HeaderList::remove_all`] through [`Self::headers_mut`] to
    /// remove Subject entirely.
    ///
    /// # Errors
    ///
    /// Invalid options or encoding failures leave all headers unchanged.
    ///
    /// ```
    /// use mimecraft::MimeMessage;
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// assert!(message.subject().is_none());
    /// message.set_subject("  Hello\r\n world  ")?;
    /// assert_eq!(message.subject().unwrap().text, "Hello world");
    /// # Ok::<(), mimecraft::UnstructuredHeaderFormatError>(())
    /// ```
    pub fn set_subject_with_options(
        &mut self,
        value: &str,
        options: &crate::UnstructuredHeaderFormatOptions,
    ) -> Result<(), crate::UnstructuredHeaderFormatError> {
        let header = Header::from_unstructured_with_options("Subject", value, options)?;
        // Singleton assignment replaces duplicates even when the value is equal.
        // Finish formatting before removing any original headers.
        self.replace_formatted(header);
        Ok(())
    }

    /// Reads all From headers with default address parsing options.
    ///
    /// See [`Self::from_with_options`] for aggregation and ownership rules.
    /// Multiple authors do not automatically create a Sender header.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn from(&self) -> MessageAddresses {
        self.from_with_options(&AddressParseOptions::default())
    }

    /// Reads all From headers using explicit address parsing options.
    ///
    /// Aggregates every matching message header in source order, preserving
    /// duplicate addresses, groups, source routes, and decoding diagnostics.
    /// Missing, empty, and comment/whitespace-only fields contribute no addresses.
    /// Uses MimeKit's internal message-header recovery: malformed entries are
    /// skipped at commas outside quotes/comments, including within groups.
    /// Unclosed groups survive; failed list separators discard that field's list
    /// (or a group's members). Later matching headers are still read.
    /// Rejected input and missing group terminators are reported in
    /// [`MessageAddresses::issues`], rather than silently lost.
    ///
    /// Each call reparses current raw bytes without caching or body I/O. The
    /// returned list is owned; edits take effect only when assigned back with
    /// [`Self::set_from`]. No Sender inference or field-specific mailbox/group
    /// restrictions are imposed beyond [`AddressParseOptions`].
    ///
    /// Diagnostics identify the original header index and raw-value byte offset.
    /// Standalone [`AddressList::parse`] remains fail-fast; syntax tolerance is
    /// controlled by the supplied options independently of message recovery.
    pub fn from_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("From", options)
    }

    /// Assigns or removes From with default address-header formatting.
    ///
    /// See [`Self::set_from_with_options`] for validation and replacement rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_from(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_from_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns or removes From using explicit address-header formatting.
    ///
    /// An empty list removes all occurrences, ignoring unused formatting options.
    /// Otherwise formats through [`AddressList::to_header_with_options`] before
    /// replacing all occurrences at the first slot, or appending if absent.
    /// Every nonempty assignment reformats, including equal lists, to preserve
    /// route-only edits. The input list and unrelated headers remain unchanged.
    ///
    /// Defaults use ASCII output, UTF-8 encoded names, IDNA domains/routes,
    /// CRLF, and enforced 78-byte physical lines. SMTPUTF8 is not supported.
    /// This does not enforce field-specific address counts or group restrictions,
    /// infer Sender, build resent blocks, or prepare an SMTP envelope.
    ///
    /// # Errors
    ///
    /// Invalid options, encoding errors, and unbreakable oversized pieces leave
    /// all existing headers unchanged, even if a later member fails.
    ///
    /// ```
    /// use mimecraft::{AddressList, MimeMessage};
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// message.set_from(&"Author <author@example.com>".parse()?)?;
    /// message.set_to(&"Friends: alice@example.com, bob@example.com;".parse()?)?;
    /// let mut recipients = message.to().addresses;
    /// recipients.as_mut_vec().push("carol@example.com".parse()?);
    /// assert_eq!(message.to().addresses.mailboxes().count(), 2);
    /// message.set_to(&recipients)?;
    /// assert_eq!(message.to().addresses.mailboxes().count(), 3);
    /// message.set_to(&AddressList::new())?;
    /// assert!(!message.headers().contains("To"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_from_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("From", value, options)
    }

    /// Reads Resent-From using [`Self::from`]'s aggregation rules.
    ///
    /// Does not fall back to From or infer Resent-Sender.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_from(&self) -> MessageAddresses {
        self.resent_from_with_options(&AddressParseOptions::default())
    }

    /// Reads Resent-From using [`Self::from_with_options`]'s rules.
    ///
    /// Aggregates occurrences across resent blocks without selecting a block.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_from_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Resent-From", options)
    }

    /// Assigns or removes Resent-From using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_from(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_from_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-From using [`Self::set_from_with_options`]'s rules.
    ///
    /// Replaces all occurrences, not just those in one resent block.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_from_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Resent-From", value, options)
    }

    /// Reads Reply-To using [`Self::from`]'s aggregation rules.
    ///
    /// An empty result does not automatically fall back to From.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn reply_to(&self) -> MessageAddresses {
        self.reply_to_with_options(&AddressParseOptions::default())
    }

    /// Reads Reply-To using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn reply_to_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Reply-To", options)
    }

    /// Assigns or removes Reply-To using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_reply_to(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_reply_to_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Reply-To using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_reply_to_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Reply-To", value, options)
    }

    /// Reads Resent-Reply-To using [`Self::from`]'s aggregation rules.
    ///
    /// Does not fall back to Reply-To or Resent-From.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_reply_to(&self) -> MessageAddresses {
        self.resent_reply_to_with_options(&AddressParseOptions::default())
    }

    /// Reads Resent-Reply-To using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_reply_to_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Resent-Reply-To", options)
    }

    /// Assigns or removes Resent-Reply-To using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_reply_to(
        &mut self,
        value: &AddressList,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_reply_to_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-Reply-To using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_reply_to_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Resent-Reply-To", value, options)
    }

    /// Reads primary recipients using [`Self::from`]'s aggregation rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn to(&self) -> MessageAddresses {
        self.to_with_options(&AddressParseOptions::default())
    }

    /// Reads To using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn to_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("To", options)
    }

    /// Assigns or removes To using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_to(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_to_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns To using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_to_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("To", value, options)
    }

    /// Reads Resent-To using [`Self::from`]'s aggregation rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_to(&self) -> MessageAddresses {
        self.resent_to_with_options(&AddressParseOptions::default())
    }

    /// Reads Resent-To using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_to_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Resent-To", options)
    }

    /// Assigns or removes Resent-To using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_to(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_to_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-To using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_to_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Resent-To", value, options)
    }

    /// Reads secondary recipients using [`Self::from`]'s aggregation rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn cc(&self) -> MessageAddresses {
        self.cc_with_options(&AddressParseOptions::default())
    }

    /// Reads Cc using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn cc_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Cc", options)
    }

    /// Assigns or removes Cc using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_cc(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_cc_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Cc using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_cc_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Cc", value, options)
    }

    /// Reads Resent-Cc using [`Self::from`]'s aggregation rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_cc(&self) -> MessageAddresses {
        self.resent_cc_with_options(&AddressParseOptions::default())
    }

    /// Reads Resent-Cc using [`Self::from_with_options`]'s rules.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_cc_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Resent-Cc", options)
    }

    /// Assigns or removes Resent-Cc using [`Self::set_from`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_cc(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_cc_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-Cc using [`Self::set_from_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_cc_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Resent-Cc", value, options)
    }

    /// Reads blind-copy recipients using [`Self::from`]'s aggregation rules.
    ///
    /// Bcc is not automatically hidden during serialization. Callers must remove
    /// it explicitly before delivery when recipient privacy requires it.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn bcc(&self) -> MessageAddresses {
        self.bcc_with_options(&AddressParseOptions::default())
    }

    /// Reads Bcc using [`Self::from_with_options`]'s rules.
    ///
    /// Does not hide or remove Bcc from the raw message headers.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn bcc_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Bcc", options)
    }

    /// Assigns or removes Bcc using [`Self::set_from`]'s rules.
    ///
    /// A nonempty Bcc header remains visible in serialized messages.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_bcc(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_bcc_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Bcc using [`Self::set_from_with_options`]'s rules.
    ///
    /// Does not prepare an SMTP envelope or hide recipients during writing.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_bcc_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Bcc", value, options)
    }

    /// Reads Resent-Bcc using [`Self::from`]'s aggregation rules.
    ///
    /// Like Bcc, Resent-Bcc is not automatically hidden during serialization.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_bcc(&self) -> MessageAddresses {
        self.resent_bcc_with_options(&AddressParseOptions::default())
    }

    /// Reads Resent-Bcc using [`Self::from_with_options`]'s rules.
    ///
    /// Does not hide or remove the raw header.
    ///
    /// Returns recovered addresses with header-indexed syntax diagnostics.
    pub fn resent_bcc_with_options(&self, options: &AddressParseOptions) -> MessageAddresses {
        self.address_headers("Resent-Bcc", options)
    }

    /// Assigns or removes Resent-Bcc using [`Self::set_from`]'s rules.
    ///
    /// A nonempty Resent-Bcc header remains visible in serialized messages.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_bcc(&mut self, value: &AddressList) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_bcc_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-Bcc using [`Self::set_from_with_options`]'s rules.
    ///
    /// Does not prepare an SMTP envelope or hide recipients during writing.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_bcc_with_options(
        &mut self,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_address_headers("Resent-Bcc", value, options)
    }

    fn address_headers(&self, field: &str, options: &AddressParseOptions) -> MessageAddresses {
        let mut result = MessageAddresses::default();
        for (header_index, header) in self.headers.iter().enumerate() {
            if !header.matches_field(field) {
                continue;
            }
            // Parse the addresses in each header and add them to our address
            // list, as in MimeKit's AddAddresses. Keep recovery diagnostics with
            // the original header index rather than silently discarding them.
            let (parsed, issues) = crate::address::parse_header_list(header.raw_value(), options);
            result.addresses.as_mut_vec().extend(parsed);
            result
                .issues
                .extend(issues.into_iter().map(|error| MessageAddressIssue {
                    header_index,
                    error,
                }));
        }
        result
    }

    fn set_address_headers(
        &mut self,
        field: &str,
        value: &AddressList,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        if value.is_empty() {
            self.headers.remove_all(field);
        } else {
            // Equality ignores mailbox routes, including those inside groups.
            // Format the whole list before replacing any of the original fields.
            let header = value.to_header_with_options(field, options)?;
            self.replace_formatted(header);
        }
        Ok(())
    }

    /// Reads the first Sender header with default mailbox parsing options.
    ///
    /// See [`Self::sender_with_options`] for prefix and ownership rules.
    ///
    /// # Errors
    ///
    /// Returns the first header's mailbox parse error, not a null fallback.
    pub fn sender(&self) -> Result<Option<MailboxAddress>, MailboxParseError> {
        self.sender_with_options(&MailboxParseOptions::default())
    }

    /// Reads the first Sender header using explicit mailbox parsing options.
    ///
    /// Returns `None` only when absent. Like MimeKit, parses one mailbox prefix,
    /// not an entire address list. Trailing comments and whitespace are consumed;
    /// other suffixes are ignored after a successful prefix parse. Groups are
    /// not accepted. Later duplicate headers and body headers are ignored.
    ///
    /// Name decoding, compatibility recovery, and diagnostics follow
    /// [`MailboxAddress::parse_prefix_with_options`]. No preliminary unstructured
    /// decoding occurs. Each call parses current raw bytes without caching or
    /// source I/O. The returned mailbox is owned: editing it does not update
    /// the message until assigned back with [`Self::set_sender`].
    ///
    /// # Errors
    ///
    /// Returns a parse error for an empty, malformed, or disallowed first value,
    /// without trying later duplicates. Offsets refer to that raw header value.
    pub fn sender_with_options(
        &self,
        options: &MailboxParseOptions,
    ) -> Result<Option<MailboxAddress>, MailboxParseError> {
        self.mailbox_header("Sender", options)
    }

    /// Assigns or removes Sender with default address-header formatting.
    ///
    /// See [`Self::set_sender_with_options`] for validation and replacement rules.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_sender(
        &mut self,
        value: Option<&MailboxAddress>,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_sender_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns or removes Sender using explicit address-header formatting.
    ///
    /// `Some` formats one mailbox through [`AddressList::to_header_with_options`],
    /// including name encoding, IDNA conversion, and enforced physical-line
    /// limits. Defaults use ASCII output, UTF-8 encoded names, CRLF, and 78-byte
    /// lines. International local parts are rejected; SMTPUTF8 is not supported.
    ///
    /// Formatting finishes before editing. Replaces all occurrences at the
    /// first slot, or appends if absent. Every `Some` assignment reformats,
    /// even when the mailbox compares equal, so route-only edits are retained.
    /// `None` removes all occurrences and ignores unused formatting options.
    /// The supplied mailbox, other headers, and body sources are unchanged.
    /// No From header, sender address, or transport envelope is synthesized.
    ///
    /// # Errors
    ///
    /// Invalid options, unencodable addresses/routes/names, and unbreakable
    /// pieces exceeding the line limit leave all existing headers unchanged.
    ///
    /// ```
    /// use mimecraft::{MailboxAddress, MimeMessage};
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// let sender = MailboxAddress::parse(b"Agent <agent@example.com>")?;
    /// message.set_sender(Some(&sender))?;
    /// let mut edited = message.sender()?.unwrap();
    /// edited.set_display_name("Delivery Agent");
    /// assert_eq!(message.sender()?.unwrap().display_name(), "Agent");
    /// message.set_sender(Some(&edited))?;
    /// assert_eq!(message.sender()?.unwrap().display_name(), "Delivery Agent");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_sender_with_options(
        &mut self,
        value: Option<&MailboxAddress>,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_mailbox_header("Sender", value, options)
    }

    /// Reads the first Resent-Sender using [`Self::sender`]'s rules.
    ///
    /// Does not fall back to Sender or infer a resent sender from Resent-From.
    ///
    /// # Errors
    ///
    /// Returns the first header's mailbox parse error.
    pub fn resent_sender(&self) -> Result<Option<MailboxAddress>, MailboxParseError> {
        self.resent_sender_with_options(&MailboxParseOptions::default())
    }

    /// Reads Resent-Sender using [`Self::sender_with_options`]'s rules.
    ///
    /// # Errors
    ///
    /// Returns the first header's mailbox parse error, with raw-value offsets.
    pub fn resent_sender_with_options(
        &self,
        options: &MailboxParseOptions,
    ) -> Result<Option<MailboxAddress>, MailboxParseError> {
        self.mailbox_header("Resent-Sender", options)
    }

    /// Assigns or removes Resent-Sender using [`Self::set_sender`]'s rules.
    ///
    /// Does not create a resent-header block or change other resent fields.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_sender(
        &mut self,
        value: Option<&MailboxAddress>,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_resent_sender_with_options(value, &AddressHeaderFormatOptions::default())
    }

    /// Assigns Resent-Sender using [`Self::set_sender_with_options`]'s rules.
    ///
    /// `None` removes all occurrences without validating unused options.
    ///
    /// # Errors
    ///
    /// Returns formatting errors without changing any headers.
    pub fn set_resent_sender_with_options(
        &mut self,
        value: Option<&MailboxAddress>,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        self.set_mailbox_header("Resent-Sender", value, options)
    }

    fn mailbox_header(
        &self,
        field: &str,
        options: &MailboxParseOptions,
    ) -> Result<Option<MailboxAddress>, MailboxParseError> {
        self.headers
            .get(field)
            .map(|header| {
                MailboxAddress::parse_prefix_with_options(header.raw_value(), options)
                    .map(|(mailbox, _)| mailbox)
            })
            .transpose()
    }

    fn set_mailbox_header(
        &mut self,
        field: &str,
        value: Option<&MailboxAddress>,
        options: &AddressHeaderFormatOptions,
    ) -> Result<(), AddressHeaderFormatError> {
        if let Some(value) = value {
            // Mailbox equality ignores source routes. Always format the supplied
            // mailbox so a route-only edit is not mistaken for an unchanged value.
            let list = AddressList::from(vec![value.clone().into()]);
            let header = list.to_header_with_options(field, options)?;
            self.replace_formatted(header);
        } else {
            self.headers.remove_all(field);
        }
        Ok(())
    }

    /// Reads the first Date header using [`DateTimeOffset::parse`].
    ///
    /// Returns `None` when absent, without supplying the current time or a
    /// minimum date. Parsing uses raw bytes, not encoded-word decoding, and
    /// retains the date parser's legacy recovery rules. Later duplicates and
    /// body headers are ignored; raw edits are visible immediately.
    ///
    /// # Errors
    ///
    /// Returns [`DateTimeOffsetError::InvalidSyntax`] if the first value cannot
    /// be interpreted, rather than MimeKit's minimum-date fallback.
    pub fn date(&self) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.date_header("Date")
    }

    /// Assigns a checked Date without changing Resent-Date or reading sources.
    ///
    /// An equal parsed value preserves original bytes only without duplicates.
    /// Equality includes local components and offset, following [`DateTimeOffset`], not
    /// just the instant: assigning the same instant with a different offset
    /// rewrites the header. Otherwise replaces every occurrence at the first
    /// slot, or appends if absent. Invalid existing values are replaced.
    ///
    /// New records use canonical English dates with seconds, a numeric zone,
    /// and CRLF. No date is generated automatically; remove raw occurrences
    /// to restore absence. Some checked values, such as dates before 1969,
    /// cannot round-trip through the compatibility parser; no cache masks this.
    pub fn set_date(&mut self, value: DateTimeOffset) {
        self.set_date_header("Date", value);
    }

    /// Reads the first Resent-Date using [`Self::date`]'s rules.
    ///
    /// Does not inspect Date or try a later resent-header block.
    ///
    /// # Errors
    ///
    /// Returns [`DateTimeOffsetError::InvalidSyntax`] for an uninterpretable
    /// first value, without falling back to Date or a later Resent-Date.
    pub fn resent_date(&self) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.date_header("Resent-Date")
    }

    /// Assigns Resent-Date independently using [`Self::set_date`]'s rules.
    ///
    /// Unlike Date, an equal parsed value preserves any duplicate occurrences.
    /// Does not create a resent-header block or update other resent fields.
    pub fn set_resent_date(&mut self, value: DateTimeOffset) {
        self.set_date_header("Resent-Date", value);
    }

    fn date_header(&self, field: &str) -> Result<Option<DateTimeOffset>, DateTimeOffsetError> {
        self.headers
            .get(field)
            .map(|header| DateTimeOffset::parse(header.raw_value()))
            .transpose()
    }

    fn set_date_header(&mut self, field: &str, value: DateTimeOffset) {
        // Preserve equal values unless a singleton has duplicates, as in MimeKit.
        // Use Rust's offset-sensitive equality so an explicit zone edit is not lost.
        if self.date_header(field) != Ok(Some(value))
            || (field == "Date" && self.has_duplicate_header(field))
        {
            self.replace_formatted(Header::from_formatted(format!("{field}: {value}\r\n")));
        }
    }

    /// Reads the first MIME-Version header using [`MimeVersion::parse`].
    ///
    /// Returns `None` when absent, without assuming version 1.0. Uses raw
    /// bytes, not decoded text; comments and compatibility forms follow the
    /// value parser. Later duplicates and body headers are ignored.
    ///
    /// # Errors
    ///
    /// Returns the first value's parse error rather than MimeKit's null fallback.
    /// Error offsets are relative to that header's raw value.
    pub fn mime_version(&self) -> Result<Option<MimeVersion>, MimeVersionParseError> {
        self.headers
            .get("MIME-Version")
            .map(|header| MimeVersion::parse(header.raw_value()))
            .transpose()
    }

    /// Assigns a checked MIME-Version without changing the body or other fields.
    ///
    /// An equal parsed version preserves bytes only without duplicates. Otherwise
    /// writes canonical decimal components with CRLF at the first slot, removing later
    /// duplicates, or appends if absent. Invalid existing values are replaced.
    /// Component counts remain significant: `1.0` differs from `1.0.0`.
    ///
    /// Remove raw occurrences to restore absence; explicit [`Self::prepare`]
    /// may add version 1.0 again. Accepting another value does not implement that
    /// version of MIME or alter parsing/writing behavior. No source I/O occurs.
    ///
    /// ```
    /// use mimecraft::{DateTimeOffset, MimeMessage, MimeVersion};
    ///
    /// let mut message: MimeMessage = MimeMessage::new();
    /// let date = DateTimeOffset::new(2007, 6, 28, 12, 47, 52, -300)?;
    /// message.set_date(date);
    /// message.set_mime_version(MimeVersion::V1_0);
    /// assert_eq!(message.date()?, Some(date));
    /// assert_eq!(message.mime_version()?, Some(MimeVersion::V1_0));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_mime_version(&mut self, value: MimeVersion) {
        if self.mime_version() != Ok(Some(value)) || self.has_duplicate_header("MIME-Version") {
            self.replace_formatted(Header::from_formatted(format!("MIME-Version: {value}\r\n")));
        }
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
    /// normalized identifier leaves original bytes intact only without duplicates.
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
    /// Unlike Message-Id, an equal normalized ID preserves duplicate occurrences.
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
    /// preserves other IDs in that same raw header only without duplicate
    /// headers. `None` removes every occurrence, including malformed values
    /// and duplicates.
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
        if !matches!(current, Ok(Some(current)) if current == id)
            || (matches!(field, "Message-Id" | "In-Reply-To") && self.has_duplicate_header(field))
        {
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
    /// An equal parsed value is a no-op only without duplicates, preserving bytes;
    /// setting an absent field to Normal does not add it. Otherwise writes a
    /// canonical CRLF record at the first slot and removes later duplicates,
    /// appending if absent. Invalid existing values are replaced, not defaulted.
    /// No source I/O occurs. Remove all raw occurrences to restore absence.
    pub fn set_importance(&mut self, value: MessageImportance) {
        // As in MimeKit, setting an absent field to Normal changes nothing.
        // Compare freshly parsed metadata rather than a potentially stale cache.
        if self.importance() != Ok(value) || self.has_duplicate_header("Importance") {
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
        if self.priority() != Ok(value) || self.has_duplicate_header("Priority") {
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
    /// Without duplicates, an equal clamped value is a no-op: setting `0` to
    /// Highest preserves `0`.
    pub fn set_x_priority(&mut self, value: XMessagePriority) {
        if self.x_priority() != Ok(value) || self.has_duplicate_header("X-Priority") {
            self.replace_formatted(value.to_header());
        }
    }

    fn has_duplicate_header(&self, field: &str) -> bool {
        self.headers.get_all(field).nth(1).is_some()
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

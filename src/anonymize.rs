// Adapted from MimeKit's MimeAnonymizer.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Byte-level masking of MIME headers and part content for inspecting layout.
//!
//! [`header`] masks a complete stored record using MimeKit's default field rules.
//! [`header_with_options`] can preserve additional fields using [`HeaderOptions`].
//! The value helpers let callers select a grammar explicitly. Record and value
//! helpers return a new buffer of exactly the input length without decoding,
//! validating, repairing, folding, or mutating headers. Each call starts fresh
//! state; do not split one value across calls. [`write_headers`] and
//! [`write_headers_with_options`] mask records and normalize output newlines.
//! [`write_message_headers`] includes root-body headers and a separator,
//! without reading any content or traversing descendants.
//! [`write_part_content`] and [`write_part_content_with_options`] mask ordinary
//! leaf content after serialization, preserving only MIME whitespace.
//! [`write_part`] adds field-aware header masking and a separator for a complete
//! ordinary part, without interpreting reports or attached messages.
//! [`write_delivery_status_content`] and [`write_disposition_notification_content`]
//! instead parse report bodies and mask their decoded header fields.
//! [`write_delivery_status`] and [`write_disposition_notification`] add masked
//! outer headers and a separator to those specialized report-content paths.
//! [`write_entity`] and [`write_message`] traverse complete MIME trees, retaining
//! delimiters and dispatching report leaves to those decoded-field paths.
//! [`write_bytes`] and [`write_bytes_with_options`] mask raw bytes with newline
//! conversion, as used for multipart preambles and epilogues in MimeKit.
//!
//! This is not guaranteed removal of sensitive information. Lengths, selected
//! syntax, and potential encoded-word charset/encoding labels remain visible;
//! address masking also preserves escaped bytes. Even malformed labels can pass
//! through verbatim. Content-header masking preserves the leading type or
//! disposition, parameter names, and selected parameter values, even if malformed.
//! Received masking preserves an entire suffix if the date parser accepts it,
//! including any unrelated text ignored by that parser.
//! Complete-record masking preserves valid field names and copies Date,
//! MIME-Version, and Content-Transfer-Encoding records verbatim, even when their
//! values contain private or malformed data. Configured preserved fields also
//! pass through in their entirety before any writer newline conversion.
//! Output may contain non-ASCII bytes, controls, or invalid UTF-8 and is not
//! necessarily valid MIME or safe display text.
//!
//! Pass a [`Header`] to [`header`], or [`Header::raw_value`] to the value helpers.
//! Writers support CRLF/LF conversion, not value reformatting.
//! Review output before sharing it. Use [`crate::filters::AnonymizeFilter`] for
//! simpler byte masking that preserves only SPACE, TAB, CR, and LF.

use std::io::{self, Read, Seek, Write};

use crate::{
    DateTimeOffset, DeliveryStatusError, DeliveryStatusParseOptions, DispositionNotificationError,
    DispositionNotificationParseOptions, Header, HeaderId, MessageDeliveryStatus,
    MessageDispositionNotification, MimeEntity, MimeMessage, MimePart, MimePartWriteOptions,
    encodings::NewLineFormat,
    filters::{AnonymizeFilter, Dos2UnixFilter, MimeFilter, Unix2DosFilter},
    io::FilteredWriter,
    parse_utils::is_whitespace,
};

const ADDRESS_SPECIALS: &[u8] = b" \t\r\n()<>[]:;@,.";
const RECEIVED_SPECIALS: &[u8] = b"<>[]:@,.";
const WHITESPACE: &[u8] = b" \t\r\n";

/// Options for masking complete header records.
///
/// Defaults to no additional preserved fields; [`header`] and
/// [`header_with_options`] then produce identical output. The three built-in
/// pass-through fields (Date, MIME-Version, Content-Transfer-Encoding) remain
/// preserved regardless of these options.
///
/// Unlike MimeKit's mutable case-insensitive set, this is an owned list using
/// the repository's ASCII header-name matching. It retains supplied spelling,
/// ordering, and duplicates, but order and duplicates do not affect masking.
/// Matching scans the list without allocating a normalized field name.
///
/// These options affect header sections and specialized report fields, not
/// raw-value helpers or ordinary-part body masking. Tree writers apply them
/// throughout the tree. Writers still normalize preserved records' newlines.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderOptions {
    /// Additional field names whose complete raw records are copied verbatim.
    ///
    /// Uses [`Header::matches_field`]: matches the complete name ignoring ASCII
    /// case, without trimming, Unicode case folding, wildcards, or prefix matching.
    /// Entries are stored as supplied rather than validated or canonicalized;
    /// names with colons, surrounding whitespace, or non-ASCII bytes do not
    /// match valid fields. `""` matches a syntactically valid empty field name.
    /// Malformed records never match and are always byte-masked.
    ///
    /// Preservation overrides every field-specific value helper, without
    /// inspecting or validating the value. This can expose private data,
    /// controls, invalid UTF-8, or embedded header-like lines. Review output
    /// before sharing it.
    pub preserve_headers: Vec<String>,
}

/// Options for masking complete messages and MIME entity trees.
///
/// No metadata is generated or changed. Preserved values and multipart boundary
/// markers can expose sensitive data; review diagnostic output before sharing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WriteOptions {
    /// Field preservation rules for every header section, including report bodies.
    pub headers: HeaderOptions,
    /// Serialization settings for headers, framing, and ordinary part content.
    ///
    /// Final-newline enforcement affects the root content, not multipart child
    /// bodies. Specialized report fields never gain a forced final newline.
    pub part: MimePartWriteOptions,
    /// Per-report delivery-status parsing limits, including global subtypes.
    ///
    /// These are not a cumulative memory or byte budget across the tree.
    pub delivery_status: DeliveryStatusParseOptions,
    /// Per-report disposition-notification parsing limits, including global subtypes.
    pub disposition_notification: DispositionNotificationParseOptions,
}

impl WriteOptions {
    pub(crate) fn validate(&self) -> io::Result<()> {
        self.part.validate()?;
        if self.delivery_status.max_decoded_bytes == 0
            || self.delivery_status.max_header_length == 0
            || self.disposition_notification.max_decoded_bytes == 0
            || self.disposition_notification.max_header_length == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "report parsing limits must be positive",
            ));
        }
        Ok(())
    }
}

/// Writes a complete masked MIME entity tree using default options.
///
/// See [`write_entity_with_options`] for traversal, report dispatch, and errors.
///
/// # Errors
///
/// Propagates metadata, report parsing, and source/destination I/O errors.
pub fn write_entity<R: Read + Seek, W: Write + ?Sized>(
    entity: &mut MimeEntity<R>,
    output: &mut W,
) -> io::Result<()> {
    write_entity_with_options(entity, output, &WriteOptions::default())
}

/// Masks an entity and all descendants without flattening their MIME structure.
///
/// Uses the ordinary writer's iterative traversal and delimiter ownership.
/// Headers use field-aware masking; multipart boundary markers remain verbatim.
/// Preambles and epilogues use [`write_bytes_with_options`]. Ordinary leaves
/// use [`write_part_content_with_options`], masking after transfer encoding.
/// Message wrappers retain complete embedded messages with interleaved headers.
///
/// A leaf's first Content-Type selects decoded-field masking for
/// message/delivery-status, message/disposition-notification, and their global
/// variants. Other leaves, including opaque message/rfc822 payloads, are not
/// reparsed as MIME trees. Missing Content-Type means ordinary masking; malformed
/// Content-Type is an error. Reports use stored encoding and can retain encoding
/// headers inconsistent with their decoded output. Report failures never fall
/// back to opaque masking. Use [`write_part`] to deliberately mask a report as
/// ordinary serialized bytes instead.
///
/// Root headers and one separator newline are included. Missing header
/// terminators are not repaired. Nonempty preambles are terminated even for
/// empty multiparts; closing delimiters always end with the selected newline.
/// Children receive independent delimiter-owned newlines, including the ordinary
/// writer's CR-preserving prefix for LF output after a bare CR.
///
/// Options are checked before any I/O. Container, leaf, and report metadata is
/// checked in traversal order **after** its masked header section is written.
/// Containers use the ordinary writer's boundary, ancestor-overlap, and
/// transparent-transfer-encoding checks, including multipart/digest defaults.
/// No boundary collision scanning, signature handling, preparation, or metadata
/// generation occurs.
///
/// Headers, bytes, hints, and tree structure are unchanged. Successful content
/// copying rewinds sources to zero. Traversal uses space proportional to depth;
/// ordinary bodies stream, while each report buffers decoded fields under its
/// own limits. The destination is flushed, not closed or finalized if filtered.
/// This is diagnostic output, not guaranteed valid MIME or safe anonymization.
///
/// # Errors
///
/// Invalid options return [`io::ErrorKind::InvalidInput`] before I/O, even for
/// unused report limits. Metadata, overlap, and report parse failures return
/// [`io::ErrorKind::InvalidData`]; source/destination I/O kinds are preserved.
/// Errors may leave a header prefix, earlier siblings, or partial content.
/// Failed content output is not finalized; no rollback occurs. Retry starts over.
///
/// ```
/// use mimecraft::{MimeEntity, Multipart, TextPart, anonymize};
///
/// let mut mixed: Multipart = Multipart::with_boundary("mixed", "example")?;
/// mixed.children_mut().push(TextPart::new("plain", "secret")?.into());
/// let mut entity: MimeEntity = mixed.into();
/// let mut output = Vec::new();
/// anonymize::write_entity(&mut entity, &mut output)?;
/// assert!(output.ends_with(b"\r\n\r\nxxxxxx\r\n--example--\r\n"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn write_entity_with_options<R: Read + Seek, W: Write + ?Sized>(
    entity: &mut MimeEntity<R>,
    output: &mut W,
    options: &WriteOptions,
) -> io::Result<()> {
    crate::mime_entity_write::write_masked_entity(entity, output, options, false)
}

/// Masks entity content with default options, omitting only its outer headers.
///
/// Descendants remain complete entities. See [`write_entity_content_with_options`].
///
/// # Errors
///
/// Propagates metadata, report parsing, and I/O errors.
pub fn write_entity_content<R: Read + Seek, W: Write + ?Sized>(
    entity: &mut MimeEntity<R>,
    output: &mut W,
) -> io::Result<()> {
    write_entity_content_with_options(entity, output, &WriteOptions::default())
}

/// Masks content without the root entity's headers or their separator.
///
/// All dispatch, framing, validation, and source rules from
/// [`write_entity_with_options`] apply. Multiparts retain complete children;
/// message wrappers retain the complete embedded message. Root metadata is
/// still required, and its boundary participates in ancestor-overlap checks.
///
/// # Errors
///
/// Invalid options or root metadata fail before I/O. Descendant metadata and
/// content/source/destination errors may leave partial output. Retry starts over.
pub fn write_entity_content_with_options<R: Read + Seek, W: Write + ?Sized>(
    entity: &mut MimeEntity<R>,
    output: &mut W,
    options: &WriteOptions,
) -> io::Result<()> {
    crate::mime_entity_write::write_masked_entity(entity, output, options, true)
}

/// Writes a complete masked message using default options.
///
/// See [`write_message_with_options`] for header interleaving and tree handling.
///
/// # Errors
///
/// Propagates metadata, report parsing, and source/destination I/O errors.
pub fn write_message<R: Read + Seek, W: Write + ?Sized>(
    message: &mut MimeMessage<R>,
    output: &mut W,
) -> io::Result<()> {
    write_message_with_options(message, output, &WriteOptions::default())
}

/// Writes interleaved masked message/body headers followed by the masked tree.
///
/// Uses [`write_message_headers_with_options`] once, then the body content-only
/// path of [`write_entity_with_options`]. Embedded messages independently merge
/// their headers using parsed byte offsets. Bodyless messages still include one
/// separator newline. No MIME-Version is generated and Bcc is masked, not removed.
///
/// Header preservation applies throughout the tree, including decoded report
/// fields. Final-newline enforcement follows the root content and propagates
/// through wrappers, not into multipart children or specialized report fields.
/// All source, buffering, destination, and disclosure rules of
/// [`write_entity_with_options`] apply.
///
/// # Errors
///
/// Invalid options fail before I/O. Body metadata errors can leave the complete
/// merged header section; descendant and I/O errors can leave partial content.
/// No rollback or silent report fallback occurs. Retry starts over.
pub fn write_message_with_options<R: Read + Seek, W: Write + ?Sized>(
    message: &mut MimeMessage<R>,
    output: &mut W,
    options: &WriteOptions,
) -> io::Result<()> {
    crate::mime_entity_write::write_masked_message(message, output, options)
}

/// Masks a complete stored header using MimeKit's default field-specific rules.
///
/// Preserves the field's spelling, blanks before its colon, and the colon.
/// Chooses a value helper using [`Header::id`], so field matching is
/// ASCII-case-insensitive and does not validate value syntax.
///
/// | Fields | Value handling |
/// | --- | --- |
/// | From, Sender, To, Cc, Bcc, Reply-To, their Resent counterparts, Disposition-Notification-To | [`address_header_value`] |
/// | Content-Id, Message-Id, Original-Message-Id, Resent-Message-Id, References, In-Reply-To | [`address_header_value`] |
/// | Received | [`received_header_value`] |
/// | Content-Type | [`content_type_value`] |
/// | Content-Disposition | [`content_disposition_value`] |
/// | Date, MIME-Version, Content-Transfer-Encoding | Copied verbatim, even invalid values |
/// | All other fields, including unknown and empty names | [`unstructured_header_value`] |
///
/// Unlisted address/date-like fields, including Return-Path and Resent-Date,
/// use unstructured masking, not inferred grammar. Authentication and signature
/// fields also use unstructured masking; no verification is performed.
///
/// If [`Header::is_invalid`] is true, masks the **entire record**, including
/// colons and apparent field text, with [`AnonymizeFilter`]. Only SPACE, TAB, CR,
/// and LF survive. A syntactically valid empty field name is not an invalid
/// record: its colon is retained and its value uses unstructured masking.
///
/// Returns exactly as many bytes as [`Header::raw`], leaving the header unchanged.
/// Unlike MimeKit's formatted stream writer, adds no newline or body separator
/// and performs no newline conversion. Individual value rules can still mask
/// escaped newlines in Received comments. This is a byte buffer, not a reparsed
/// header. Use [`header_with_options`] to preserve additional fields.
/// Use [`write_message`] for complete-message masking. See the
/// [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::{Header, anonymize};
///
/// let record = Header::parse(b"tO \t: Alice <alice@example.com>\n")?;
/// assert_eq!(
///     anonymize::header(&record),
///     b"tO \t: xxxxx <xxxxx@xxxxxxx.xxx>\n",
/// );
/// # Ok::<(), mimecraft::HeaderParseError>(())
/// ```
pub fn header(record: &Header) -> Vec<u8> {
    header_with_options(record, &HeaderOptions::default())
}

/// Masks a complete stored header while preserving explicitly selected fields.
///
/// Uses [`header`]'s field dispatch, malformed-record handling, and raw-byte
/// output rules. For a valid record, a match in [`HeaderOptions::preserve_headers`]
/// copies the entire stored record before applying any value helper. Values are
/// not validated; field spelling, whitespace, and even malformed value bytes
/// pass through unchanged. Invalid field/colon syntax is still byte-masked.
///
/// Neither input nor options are mutated. Options are checked afresh on each
/// call, so edits take effect immediately without caching. Every supplied record
/// is handled independently, including duplicate fields and records from
/// caller-parsed status groups. No automatic header-list or body traversal occurs.
///
/// Returns exactly as many bytes as [`Header::raw`], without newline conversion
/// or an added separator. Preserved data may be sensitive; see the
/// [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::{Header, anonymize::{self, HeaderOptions}};
///
/// let record = Header::parse(b"Subject: keep this exact value\n")?;
/// let options = HeaderOptions {
///     preserve_headers: vec!["sUbJeCt".into()],
/// };
/// assert_eq!(anonymize::header_with_options(&record, &options), record.raw());
/// assert_eq!(anonymize::header(&record), b"Subject: xxxx xxxx xxxxx xxxxx\n");
/// # Ok::<(), mimecraft::HeaderParseError>(())
/// ```
pub fn header_with_options(record: &Header, options: &HeaderOptions) -> Vec<u8> {
    if record.is_invalid() {
        // Invalid records have no usable field/value boundary: mask raw bytes
        // instead of dispatching on an apparent field or dropping the record.
        let mut output = Vec::with_capacity(record.raw().len());
        AnonymizeFilter::new().filter(record.raw(), &mut output);
        return output;
    }

    if options
        .preserve_headers
        .iter()
        .any(|field| record.matches_field(field))
    {
        // Don't mask a configured field: preservation takes precedence over
        // the default rules, but never over malformed-record byte masking.
        return record.raw().to_vec();
    }

    let raw_value = record.raw_value();
    let value = match record.id() {
        HeaderId::DispositionNotificationTo
        | HeaderId::ResentReplyTo
        | HeaderId::ResentSender
        | HeaderId::ResentFrom
        | HeaderId::ResentBcc
        | HeaderId::ResentCc
        | HeaderId::ResentTo
        | HeaderId::ReplyTo
        | HeaderId::Sender
        | HeaderId::From
        | HeaderId::Bcc
        | HeaderId::Cc
        | HeaderId::To => address_header_value(raw_value),
        HeaderId::Received => received_header_value(raw_value),
        HeaderId::OriginalMessageId
        | HeaderId::ResentMessageId
        | HeaderId::References
        | HeaderId::InReplyTo
        | HeaderId::MessageId
        | HeaderId::ContentId => {
            // Treat these identifiers like address headers for now, as MimeKit does.
            address_header_value(raw_value)
        }
        HeaderId::ContentDisposition => content_disposition_value(raw_value),
        HeaderId::ContentType => content_type_value(raw_value),
        HeaderId::ArcAuthenticationResults
        | HeaderId::AuthenticationResults
        | HeaderId::ArcMessageSignature
        | HeaderId::ArcSeal
        | HeaderId::DkimSignature => {
            // TODO: consider dedicated authentication/signature masking.
            unstructured_header_value(raw_value)
        }
        HeaderId::ContentTransferEncoding | HeaderId::MimeVersion | HeaderId::Date => {
            // Don't mask these headers, even if their values are malformed.
            return record.raw().to_vec();
        }
        _ => unstructured_header_value(raw_value),
    };

    let mut output = Vec::with_capacity(record.raw().len());
    output.extend_from_slice(record.raw_field());
    output.push(b':');
    output.extend_from_slice(&value);
    output
}

/// Writes masked headers with default preservation rules and CRLF newlines.
///
/// Accepts a borrowed [`crate::HeaderList`], slice, or iterator of records.
/// Retains iteration order and duplicates, without adding a body separator.
/// See [`write_headers_with_options`] for formatting, buffering, and errors.
///
/// # Errors
///
/// Propagates destination write and flush errors, possibly after partial output.
pub fn write_headers<'a, W: Write + ?Sized>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
) -> io::Result<()> {
    write_headers_with_options(
        headers,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
    )
}

/// Writes masked headers with explicit preservation and output newline settings.
///
/// Applies [`header_with_options`] independently to each complete record, then
/// streams the result through the selected CRLF/LF converter. Even preserved
/// fields have their newlines converted. No values are decoded, re-encoded,
/// refolded, or validated. Headers and options remain unchanged.
///
/// Like MimeKit's anonymizer, malformed records are byte-masked and newline
/// converted too. Unlike [`crate::HeaderList::write_to`], there is no exception
/// for carriage-return-only invalid lines: repeated LF conversion can eventually
/// turn such a line into a blank separator. Output is for inspection, not a
/// guaranteed structure-preserving round trip or removal of sensitive data.
///
/// Missing record terminators and a header/body separator are not added. Bare
/// CR bytes survive unless followed by LF. Newline state spans chunks and
/// adjacent records as one ordered byte stream, including malformed records;
/// masking state starts fresh per record. An unterminated record can therefore
/// run into the next record. Only supplied headers are visited, not MIME bodies.
///
/// Buffers one masked record at a time, not the entire collection. On success,
/// finalizes the newline filter and flushes the borrowed destination, even for
/// empty input; it does not close the destination. If the destination is itself
/// a filter writer, the caller must still finalize that outer destination.
///
/// # Errors
///
/// [`NewLineFormat::Mixed`] returns [`io::ErrorKind::InvalidInput`] before
/// obtaining an iterator or performing any I/O. Write and flush errors propagate
/// without rollback, possibly after partial or complete output. Interrupted
/// writes retry; zero writes fail with [`io::ErrorKind::WriteZero`].
/// Retrying with a fresh iterator starts over, not at the failed output byte.
///
/// ```
/// use mimecraft::{
///     HeaderList,
///     anonymize::{self, HeaderOptions},
///     encodings::NewLineFormat,
/// };
///
/// let headers = HeaderList::load(&mut &b"Subject: private\r\nTo: Alice <a@b.c>\n"[..])?;
/// let options = HeaderOptions { preserve_headers: vec!["subject".into()] };
/// let mut output = Vec::new();
/// anonymize::write_headers_with_options(&headers, &mut output, &options, NewLineFormat::Unix)?;
/// assert_eq!(output, b"Subject: private\nTo: xxxxx <x@x.x>\n");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn write_headers_with_options<'a, W: Write + ?Sized>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
) -> io::Result<()> {
    header_new_line_bytes(format)?;
    match format {
        NewLineFormat::Dos => {
            write_filtered_headers(headers, output, options, Unix2DosFilter::new())
        }
        NewLineFormat::Unix => {
            write_filtered_headers(headers, output, options, Dos2UnixFilter::new())
        }
        NewLineFormat::Mixed => unreachable!("header newlines were validated"),
    }
}

fn header_new_line_bytes(format: NewLineFormat) -> io::Result<&'static [u8]> {
    match format {
        NewLineFormat::Dos => Ok(b"\r\n"),
        NewLineFormat::Unix => Ok(b"\n"),
        NewLineFormat::Mixed => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "header new-line format must be Dos or Unix",
        )),
    }
}

fn write_filtered_headers<'a, W: Write + ?Sized, F: MimeFilter>(
    headers: impl IntoIterator<Item = &'a Header>,
    output: &mut W,
    options: &HeaderOptions,
    filter: F,
) -> io::Result<()> {
    let mut filtered = FilteredWriter::new(output, filter);
    for record in headers {
        // Mask whole values before chunked I/O: keywords, dates, and encoded
        // words can depend on bytes beyond a writer's buffer boundary.
        filtered.write_all(&header_with_options(record, options))?;
    }
    // MimeKit's filtered Flush finalizes filters; Rust's Write::flush does not.
    // Finish explicitly so a trailing CR held by Dos2Unix is not lost.
    filtered.finish()
}

/// Writes masked message and root-body headers plus a separator using CRLF.
///
/// Uses default [`HeaderOptions`]. See [`write_message_headers_with_options`]
/// for ordering, framing, and no-content-I/O guarantees.
///
/// # Errors
///
/// Propagates destination write and flush errors, possibly after partial output.
pub fn write_message_headers<R, W: Write + ?Sized>(
    message: &MimeMessage<R>,
    output: &mut W,
) -> io::Result<()> {
    write_message_headers_with_options(
        message,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
    )
}

/// Writes interleaved masked message/root-body headers and a separator newline.
///
/// Adapts the header portion of MimeKit's message anonymizer. Headers retained
/// by [`MimeMessage::load`] are interleaved in their original message/body order,
/// including duplicates and malformed records. This uses the same ordering as
/// ordinary [`MimeMessage::write_headers_to`] output.
///
/// Leading new message headers come first. Remaining loaded message and body
/// headers merge by stream byte offset, then remaining message headers precede
/// newly added body headers, as in MimeKit. Authored replacements have no stream
/// offset and follow the new-header rules. Lists are never sorted or repartitioned;
/// explicit in-list reordering is respected rather than undone. Independently authored
/// lists without loaded ordering information use message-first order.
///
/// Both lists use [`write_headers_with_options`] as one sequence, including
/// configured preservation, malformed-record masking, and newline conversion.
/// No headers are synthesized: in particular, this does not insert MIME-Version.
/// Bcc and Resent-Bcc are masked as address fields, not removed; preserving
/// those fields explicitly exposes their entire values.
///
/// Appends exactly one selected newline after the records, even when there is
/// no body or no headers. Unlike ordinary message serialization, missing record
/// terminators are **not** repaired. An unterminated record can run into the next
/// record, and the added newline need not create a blank separator. This is
/// diagnostic output, not guaranteed well-formed MIME. Original separator
/// presence is not retained by mimecraft's message model.
///
/// Does not read/seek sources, validate body metadata, write content, or visit
/// descendants. A multipart contributes only its own headers; an attached
/// message contributes only its wrapper's headers, not the enclosed message's.
/// No source I/O traits or mutable access are required. Headers, body sources,
/// source positions, and metadata remain unchanged.
///
/// Buffers one masked record at a time. The destination is flushed after the
/// header sequence and after the separator, without closing it or finalizing
/// caller-owned filter writers.
///
/// # Errors
///
/// [`NewLineFormat::Mixed`] returns [`io::ErrorKind::InvalidInput`] before
/// destination I/O. Write/flush errors propagate without rollback, possibly
/// after partial or complete output. Retrying starts the header section over.
///
/// ```
/// use mimecraft::{MimeMessage, TextPart, anonymize};
///
/// let mut message = MimeMessage::with_body(TextPart::new("plain", "private body")?.into());
/// message.set_subject("private")?;
/// let mut output = Vec::new();
/// anonymize::write_message_headers(&message, &mut output)?;
/// assert!(output.starts_with(b"Subject: xxxxxxx\r\n"));
/// assert!(output.ends_with(b"\r\n\r\n"));
/// assert!(!output.windows(12).any(|bytes| bytes == b"private body"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn write_message_headers_with_options<R, W: Write + ?Sized>(
    message: &MimeMessage<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
) -> io::Result<()> {
    let newline = header_new_line_bytes(format)?;
    // Interleave the message and root-body headers before masking them: grouping
    // the lists first would lose ordering even when all raw values are retained.
    write_headers_with_options(message.merged_headers(), output, options, format)?;
    output.write_all(newline)?;
    output.flush()
}

/// Writes masked raw bytes using CRLF, without forcing a final newline.
///
/// See [`write_bytes_with_options`] for newline conversion and empty-input
/// behavior. Does not interpret headers or serialize MIME content.
///
/// # Errors
///
/// Propagates destination write and flush errors, possibly after partial output.
pub fn write_bytes<W: Write + ?Sized>(input: &[u8], output: &mut W) -> io::Result<()> {
    write_bytes_with_options(input, output, NewLineFormat::Dos, false)
}

/// Masks raw bytes, converts newlines, and optionally ensures a final newline.
///
/// Adapts MimeKit's raw-byte writer used for multipart preambles and epilogues.
/// Each byte except SPACE, TAB, CR, and LF becomes `x`, including non-ASCII,
/// vertical tab, and form feed. Masking precedes newline conversion; there is
/// no charset or transfer decoding, header interpretation, or MIME framing.
/// Unlike [`write_part_content`], this writes supplied bytes, not serialized
/// content. It does not visit a multipart or choose which regions to mask.
///
/// CRLF output expands bare LF; LF output removes the CR in CRLF pairs. Other
/// CR bytes survive. With `ensure_new_line`, nonempty input that lacks a final
/// LF gains the selected newline. A trailing CR becomes CRLF for DOS output
/// or LF for Unix output. Existing final newlines are not doubled.
///
/// Empty input writes **and flushes nothing**, even with `ensure_new_line`.
/// This matches MimeKit's raw-byte helper, not the newline filters themselves,
/// which can emit a newline for empty input. Each call starts fresh filter
/// state: supply a complete region, rather than splitting CRLF across calls.
///
/// Input is unchanged and processed with bounded streaming buffers. Nonempty
/// input finalizes both internal filters and flushes the borrowed destination,
/// without closing it or finalizing any caller-owned filter writer.
///
/// # Errors
///
/// [`NewLineFormat::Mixed`] returns [`io::ErrorKind::InvalidInput`] before any
/// I/O, even for empty input. Write and flush errors propagate without rollback;
/// output can be partial. Interrupted writes retry; zero writes return
/// [`io::ErrorKind::WriteZero`]. Retrying this helper starts the region over.
///
/// ```
/// use mimecraft::{anonymize, encodings::NewLineFormat};
///
/// let mut output = Vec::new();
/// anonymize::write_bytes_with_options(
///     b"Private preamble\r\nSecond line\r", &mut output, NewLineFormat::Unix, true,
/// )?;
/// assert_eq!(output, b"xxxxxxx xxxxxxxx\nxxxxxx xxxx\n");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn write_bytes_with_options<W: Write + ?Sized>(
    input: &[u8],
    output: &mut W,
    format: NewLineFormat,
    ensure_new_line: bool,
) -> io::Result<()> {
    match format {
        NewLineFormat::Dos => write_filtered_bytes(
            input,
            output,
            Unix2DosFilter::with_ensure_new_line(ensure_new_line),
        ),
        NewLineFormat::Unix => write_filtered_bytes(
            input,
            output,
            Dos2UnixFilter::with_ensure_new_line(ensure_new_line),
        ),
        NewLineFormat::Mixed => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "byte new-line format must be Dos or Unix",
        )),
    }
}

fn write_filtered_bytes<W: Write + ?Sized, F: MimeFilter>(
    input: &[u8],
    output: &mut W,
    filter: F,
) -> io::Result<()> {
    // MimeKit skips empty preambles and epilogues before creating its filters:
    // ensuring a newline must not turn an absent region into a blank line.
    if input.is_empty() {
        return Ok(());
    }
    let mut newline = FilteredWriter::new(output, filter);
    let mut masked = FilteredWriter::new(&mut newline, AnonymizeFilter::new());
    masked.write_all(input)?;
    // Finalize in pipeline order. Rust's flush only drains the next writer;
    // its pending CR or ensured final newline still needs an explicit finish.
    masked.finish()?;
    newline.finish()
}

/// Writes a complete masked ordinary part using default header and body options.
///
/// See [`write_part_with_options`] for header framing, encoding, and errors.
///
/// # Errors
///
/// Propagates invalid options, required body metadata, and I/O errors.
pub fn write_part<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
) -> io::Result<()> {
    write_part_with_options(
        part,
        output,
        &HeaderOptions::default(),
        &MimePartWriteOptions::default(),
    )
}

/// Writes masked headers, a separator newline, and serialized masked content.
///
/// Uses [`write_headers_with_options`] for field-aware masking and configured
/// preservation, then [`write_part_content_with_options`] for the body. Both
/// use `options.new_line_format`; encoder width and final-newline settings
/// affect only content. Body masking occurs after transfer encoding and includes
/// generated UU wrappers, so the output need not remain valid encoded content.
/// Preserving an outer header never exempts any body bytes from masking.
///
/// Appends one separator newline even with no headers or empty content.
/// Missing header terminators are not repaired, unlike [`MimePart::write_to`];
/// records can run together and truncated headers need not form a blank line
/// before the body. Malformed records use the masking writer's newline rules,
/// not the ordinary header writer's CR-only round-trip exception.
///
/// Treats all [`MimePart`] content as opaque, including report and message
/// payloads. Does not parse fields inside the body or traverse MIME entities.
/// Use the specialized report-content writers for decoded-field masking.
/// No headers, stored bytes, encoding labels, or newline hints are changed.
///
/// Streams with one masked header record and bounded body buffers. Copying
/// starts at source offset zero and rewinds on success, not to its old position.
/// The destination is flushed but not closed; caller-owned filters still need
/// finalization. This is diagnostic output, not guaranteed MIME round-tripping
/// or removal of sensitive information.
///
/// # Errors
///
/// Invalid serialization options fail before any source or destination I/O.
/// As in MimeKit's anonymizer, headers are written before content serialization:
/// required encoding/UU metadata and source errors may leave a header prefix.
/// Destination errors can leave partial or complete output. Errors propagate
/// without rollback or finalizing failed body output; retrying starts over.
///
/// ```
/// use std::io::Cursor;
/// use mimecraft::{HeaderList, MimeContent, MimePart, anonymize};
///
/// let headers = HeaderList::load(&mut &b"X-Private: secret\n"[..])?;
/// let mut part = MimePart::from_parts(headers, MimeContent::new(Cursor::new(b"Cat")));
/// let mut output = Vec::new();
/// anonymize::write_part(&mut part, &mut output)?;
/// assert_eq!(output, b"X-Private: xxxxxx\r\n\r\nxxx");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn write_part_with_options<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
    header_options: &HeaderOptions,
    options: &MimePartWriteOptions,
) -> io::Result<()> {
    let newline = options.validate()?;
    // MimeKit writes entity headers before entering the content path. Preserve
    // that order without wrapping headers in the body's blanket byte masker.
    write_headers_with_options(
        part.headers(),
        output,
        header_options,
        options.new_line_format,
    )?;
    output.write_all(newline)?;
    write_part_content_with_options(part, output, options)
}

/// Writes masked ordinary-part content using default serialization options.
///
/// Omits headers and their separator. Uses [`MimePartWriteOptions::default`],
/// not the part's stored bytes verbatim. See [`write_part_content_with_options`]
/// for transfer encoding, newline handling, and errors.
///
/// # Errors
///
/// Propagates required metadata and source/destination I/O errors.
pub fn write_part_content<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
) -> io::Result<()> {
    write_part_content_with_options(part, output, &MimePartWriteOptions::default())
}

/// Writes ordinary-part content, masking its serialized bytes.
///
/// Adapts MimeKit's ordinary `MimePart` anonymization path: delegates to
/// [`MimePart::write_content_to_with_options`], then replaces every output byte
/// except SPACE, TAB, CR, and LF with `x`. This is **not** masking decoded bytes
/// before transfer encoding. The requested encoding, encoder width, newline
/// conversion, final-newline option, and any generated UU wrappers all take
/// effect before masking. Matching stored/requested encodings avoid transcoding.
///
/// Output has the same length and MIME-whitespace positions as ordinary content
/// serialization, not necessarily the stored or decoded body. Even UU wrapper
/// keywords, permissions, and filenames are masked. The result need not be a
/// valid encoded payload. Binary output skips newline conversion and ignores
/// `ensure_new_line`, but its non-whitespace bytes are still masked.
///
/// No headers or separator are written. This does not traverse MIME entities,
/// attached messages, or parse structured report bodies; such content is opaque
/// here. [`HeaderOptions`] does not affect body masking. It does not provide
/// specialized report masking; use [`write_delivery_status_content`] or
/// [`write_disposition_notification_content`] for that.
///
/// Uses bounded streaming buffers, without collecting the whole body. Headers,
/// content bytes, encoding labels, and newline hints remain unchanged. Copying
/// starts at source offset zero and rewinds on success, not to the old position.
/// The destination is flushed, never closed; a caller-owned filter destination
/// still requires its own finalization.
///
/// # Errors
///
/// Invalid options or required encoding/UU metadata fail before source or
/// destination I/O, using the ordinary content writer's errors. Read, seek,
/// write, and flush failures propagate, possibly after partial or complete
/// masked output. Failures do not restore source position or roll back output.
/// Retrying starts over. Buffered output is not finalized after an error.
///
/// ```
/// use std::io::Cursor;
/// use mimecraft::{ContentEncoding, MimeContent, MimePart, anonymize};
///
/// let mut part = MimePart::new(MimeContent::new(Cursor::new(b"Cat")));
/// part.set_content_transfer_encoding(ContentEncoding::Base64);
/// let mut output = Vec::new();
/// anonymize::write_part_content(&mut part, &mut output)?;
/// assert_eq!(output, b"xxxx\r\n"); // Masks Q2F0, not Cat.
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn write_part_content_with_options<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
    options: &MimePartWriteOptions,
) -> io::Result<()> {
    // Place masking after WriteContent, as in MimeKit, so transfer-encoded
    // payloads and generated wrappers retain only their whitespace layout.
    let mut filtered = FilteredWriter::new(output, AnonymizeFilter::new());
    part.write_content_to_with_options(&mut filtered, options)?;
    // The part writer flushes its destination, but does not finalize our filter.
    filtered.finish()
}

/// Writes a complete masked delivery-status part using default limits and CRLF.
///
/// See [`write_delivery_status_with_options`] for framing, decoding, and errors.
///
/// # Errors
///
/// Propagates report metadata, parse-limit, and source/destination I/O errors.
pub fn write_delivery_status<R: Read + Seek, W: Write + ?Sized>(
    status: &mut MessageDeliveryStatus<R>,
    output: &mut W,
) -> Result<(), DeliveryStatusError> {
    write_delivery_status_with_options(
        status,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
        &DeliveryStatusParseOptions::default(),
    )
}

/// Writes masked outer headers, a separator, and decoded delivery-status groups.
///
/// Combines [`write_headers_with_options`] with
/// [`write_delivery_status_content_with_options`]. The same preservation rules
/// and newline format apply to outer headers and every parsed group. Supports
/// both delivery-status subtypes and the one-time Office365 encoded remainder.
/// Unlike [`write_part`], masks parsed fields rather than opaque encoded bytes.
///
/// Adds one separator newline after outer headers, even for an empty body.
/// Missing header terminators are not repaired. Groups have one newline between
/// them, without an added trailing blank separator or forced final newline.
/// Output is decoded field text, not re-encoded to match retained outer or group
/// transfer-encoding headers. Even inconsistent or unknown outer encoding
/// values remain unchanged; decoding uses the content's stored encoding.
///
/// Headers and content are unchanged; successful decoding rewinds the source
/// to zero. Parsing buffers decoded data and groups within `parse_options`.
/// The destination is flushed, not closed or finalized if it is another filter.
/// This is diagnostic output, not guaranteed valid MIME or safe anonymization.
///
/// # Errors
///
/// `Mixed` newlines fail before any I/O. Headers and the separator are written
/// before checking report metadata and parse limits or reading the source.
/// Those failures leave the header prefix but no report fields. No error falls
/// back to opaque masking. Destination errors can leave partial output; errors
/// propagate without rollback, and retrying starts over.
///
/// ```
/// use mimecraft::{Header, MessageDeliveryStatus, anonymize};
///
/// let mut status = MessageDeliveryStatus::new();
/// status.set_status_groups(&[vec![Header::parse(b"Action: failed")?].into()])?;
/// let mut output = Vec::new();
/// anonymize::write_delivery_status(&mut status, &mut output)?;
/// assert_eq!(output,
///     b"Content-Type: message/delivery-status\r\n\r\nAction: xxxxxx\r\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn write_delivery_status_with_options<R: Read + Seek, W: Write + ?Sized>(
    status: &mut MessageDeliveryStatus<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DeliveryStatusParseOptions,
) -> Result<(), DeliveryStatusError> {
    let newline = header_new_line_bytes(format).map_err(DeliveryStatusError::Io)?;
    // MimeKit emits the entity's header section before its specialized report
    // body. Keep the group parser's explicit errors instead of masking a fallback.
    write_headers_with_options(status.part().headers(), output, options, format)
        .map_err(DeliveryStatusError::Io)?;
    output.write_all(newline).map_err(DeliveryStatusError::Io)?;
    write_delivery_status_content_with_options(status, output, options, format, parse_options)
}

/// Writes masked delivery-status groups using default limits, rules, and CRLF.
///
/// See [`write_delivery_status_content_with_options`] for decoded-body framing,
/// preservation, and errors. No outer MIME headers or separator are written.
///
/// # Errors
///
/// Propagates report metadata, parse-limit, and source/destination I/O errors.
pub fn write_delivery_status_content<R: Read + Seek, W: Write + ?Sized>(
    status: &mut MessageDeliveryStatus<R>,
    output: &mut W,
) -> Result<(), DeliveryStatusError> {
    write_delivery_status_content_with_options(
        status,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
        &DeliveryStatusParseOptions::default(),
    )
}

/// Parses and masks a delivery-status body as decoded header groups.
///
/// Uses [`MessageDeliveryStatus::status_groups_with_options`], including global
/// subtypes, skipped empty groups, and the one-time Office365 encoded-remainder
/// workaround. Stored content encoding controls decoding, not the outer transfer
/// header. Parsing completes before writing any output and holds decoded data
/// and groups in memory within the parser's documented limits.
///
/// Each group uses [`write_headers_with_options`], including configured field
/// preservation, default pass-through fields, and malformed-record masking.
/// Writes one selected newline **between** groups, never after the last group.
/// Existing record terminators are converted but missing ones are not repaired.
/// An empty body produces no bytes and still flushes the destination.
///
/// As in MimeKit, output is decoded field text, not re-encoded to match retained
/// outer or group Content-Transfer-Encoding fields. Those fields may therefore
/// be inconsistent with the output: this is diagnostic masking, not round-trip
/// report serialization. No outer headers or header/body separator are written.
///
/// Source bytes, metadata, and headers remain unchanged. Successful decoding
/// rewinds the source, even if later parsing or writing fails. The destination
/// is flushed, not closed; caller-owned filters still need finalization.
///
/// # Errors
///
/// `Mixed` output newlines, zero limits, and invalid media metadata fail before
/// source/destination I/O. Parse limits, active group encodings, and source
/// errors propagate before destination I/O; no partial parsed report is written.
/// Unlike MimeKit's exception fallback, failures never silently switch to
/// ordinary-part byte masking. Destination errors use [`DeliveryStatusError::Io`]
/// and may leave partial output. No rollback occurs; retrying starts over.
///
/// ```
/// use mimecraft::{Header, MessageDeliveryStatus, anonymize};
///
/// let mut status = MessageDeliveryStatus::new();
/// status.set_status_groups(&[
///     vec![Header::parse(b"Action: failed")?].into(),
///     vec![Header::parse(b"Status: 5.1.1")?].into(),
/// ])?;
/// let mut output = Vec::new();
/// anonymize::write_delivery_status_content(&mut status, &mut output)?;
/// assert_eq!(output, b"Action: xxxxxx\r\n\r\nStatus: xxxxx\r\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn write_delivery_status_content_with_options<R: Read + Seek, W: Write + ?Sized>(
    status: &mut MessageDeliveryStatus<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DeliveryStatusParseOptions,
) -> Result<(), DeliveryStatusError> {
    write_delivery_status_part_content(status.part_mut(), output, options, format, parse_options)
}

fn write_delivery_status_part_content<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DeliveryStatusParseOptions,
) -> Result<(), DeliveryStatusError> {
    let newline = header_new_line_bytes(format).map_err(DeliveryStatusError::Io)?;
    // Parse detached groups before output; do not rewrite the stored report
    // or hide parse failures behind the source anonymizer's byte-mask fallback.
    let groups = MessageDeliveryStatus::read_status_groups(part, parse_options)?;
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            // MimeKit separates groups, but adds no trailing blank separator.
            output.write_all(newline).map_err(DeliveryStatusError::Io)?;
        }
        write_headers_with_options(group, output, options, format)
            .map_err(DeliveryStatusError::Io)?;
    }
    if groups.is_empty() {
        output.flush().map_err(DeliveryStatusError::Io)?;
    }
    Ok(())
}

/// Writes a complete masked disposition notification using default limits and CRLF.
///
/// See [`write_disposition_notification_with_options`] for framing and errors.
///
/// # Errors
///
/// Propagates report metadata, parse-limit, and source/destination I/O errors.
pub fn write_disposition_notification<R: Read + Seek, W: Write + ?Sized>(
    notification: &mut MessageDispositionNotification<R>,
    output: &mut W,
) -> Result<(), DispositionNotificationError> {
    write_disposition_notification_with_options(
        notification,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
        &DispositionNotificationParseOptions::default(),
    )
}

/// Writes masked outer headers, a separator, and decoded notification fields.
///
/// Combines [`write_headers_with_options`] with
/// [`write_disposition_notification_content_with_options`]. Both outer headers
/// and the first decoded field block use the same preservation and newline
/// settings. Supports both disposition-notification subtypes. Unlike
/// [`write_part`], applies field-aware masking inside the report body.
///
/// Adds one separator newline after outer headers, even for an empty body or
/// initial empty field block. Missing record terminators are not repaired and
/// no trailing blank separator or final newline is added to the fields.
/// Later blocks are not emitted, but the entire decoded body counts toward
/// limits. Retained transfer-encoding fields do not cause re-encoding: output
/// is decoded text and can be inconsistent with its outer metadata.
///
/// Headers and content are unchanged; decoding uses stored encoding and rewinds
/// the source to zero on success. Parsing buffers data within `parse_options`.
/// The destination is flushed, not closed or finalized if it is another filter.
/// This is diagnostic output, not guaranteed valid MIME or safe anonymization.
///
/// # Errors
///
/// `Mixed` newlines fail before any I/O. Report metadata, parse limits, and source
/// errors are checked after writing outer headers and the separator, but before
/// any fields are emitted. Failures never fall back to opaque masking. Write
/// and flush errors may leave partial output; no rollback occurs. Retry starts
/// over.
///
/// ```
/// use mimecraft::{Header, MessageDispositionNotification, anonymize};
///
/// let mut notification = MessageDispositionNotification::new();
/// notification.set_fields(&vec![Header::parse(b"Disposition: displayed")?].into())?;
/// let mut output = Vec::new();
/// anonymize::write_disposition_notification(&mut notification, &mut output)?;
/// assert_eq!(output,
///     b"Content-Type: message/disposition-notification\r\n\r\nDisposition: xxxxxxxxx\r\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn write_disposition_notification_with_options<R: Read + Seek, W: Write + ?Sized>(
    notification: &mut MessageDispositionNotification<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DispositionNotificationParseOptions,
) -> Result<(), DispositionNotificationError> {
    let newline = header_new_line_bytes(format).map_err(DispositionNotificationError::Io)?;
    // Keep outer headers separate from the decoded field block, as in MimeKit:
    // blanket body masking would hide field names and configured preserved data.
    write_headers_with_options(notification.part().headers(), output, options, format)
        .map_err(DispositionNotificationError::Io)?;
    output
        .write_all(newline)
        .map_err(DispositionNotificationError::Io)?;
    write_disposition_notification_content_with_options(
        notification,
        output,
        options,
        format,
        parse_options,
    )
}

/// Writes masked disposition-notification fields using default limits and CRLF.
///
/// See [`write_disposition_notification_content_with_options`] for first-block
/// selection and errors. No outer MIME headers or separator are written.
///
/// # Errors
///
/// Propagates report metadata, parse-limit, and source/destination I/O errors.
pub fn write_disposition_notification_content<R: Read + Seek, W: Write + ?Sized>(
    notification: &mut MessageDispositionNotification<R>,
    output: &mut W,
) -> Result<(), DispositionNotificationError> {
    write_disposition_notification_content_with_options(
        notification,
        output,
        &HeaderOptions::default(),
        NewLineFormat::Dos,
        &DispositionNotificationParseOptions::default(),
    )
}

/// Parses and masks the first decoded disposition-notification field block.
///
/// Uses [`MessageDispositionNotification::fields_with_options`], including global
/// subtypes and stored-encoding decoding. Initial blank lines yield an empty
/// block; later blocks are not emitted. The entire body, including ignored
/// trailing data, is decoded and counted toward limits before any output.
/// Field-level transfer-encoding headers are ordinary data, not decoding commands.
///
/// Writes via [`write_headers_with_options`], applying configured preservation
/// and newline conversion. Adds no outer headers, blank separator, or missing
/// record terminator. Empty fields still flush the destination. Output remains
/// decoded field text, even if retained transfer-encoding metadata says otherwise.
/// This is diagnostic masking, not round-trip report serialization.
///
/// Headers, stored bytes, and metadata are unchanged; successful decoding
/// rewinds the source even before a subsequent parsing or writing failure.
/// Decoded data and fields are held in memory under the parser's limits.
/// The destination is flushed, not closed or finalized if it is another filter.
///
/// # Errors
///
/// Invalid newlines, zero limits, and invalid media metadata fail before I/O.
/// Parse-limit and source errors propagate before destination I/O, without
/// silently falling back to ordinary-part masking. Destination errors use
/// [`DispositionNotificationError::Io`] and can leave partial output.
/// No rollback occurs; retrying starts over.
pub fn write_disposition_notification_content_with_options<R: Read + Seek, W: Write + ?Sized>(
    notification: &mut MessageDispositionNotification<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DispositionNotificationParseOptions,
) -> Result<(), DispositionNotificationError> {
    write_disposition_notification_part_content(
        notification.part_mut(),
        output,
        options,
        format,
        parse_options,
    )
}

fn write_disposition_notification_part_content<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
    options: &HeaderOptions,
    format: NewLineFormat,
    parse_options: &DispositionNotificationParseOptions,
) -> Result<(), DispositionNotificationError> {
    header_new_line_bytes(format).map_err(DispositionNotificationError::Io)?;
    // Use the detached field view, not WriteContent: the source's MDN path
    // masks decoded fields without emitting the first block's blank separator.
    let fields = MessageDispositionNotification::read_fields(part, parse_options)?;
    write_headers_with_options(&fields, output, options, format)
        .map_err(DispositionNotificationError::Io)
}

pub(crate) fn write_leaf_content<R: Read + Seek, W: Write + ?Sized>(
    part: &mut MimePart<R>,
    output: &mut W,
    options: &WriteOptions,
    part_options: &MimePartWriteOptions,
) -> io::Result<()> {
    let content_type = part
        .content_type()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // The Rust tree stores reports as ordinary leaves rather than subclasses.
    // Dispatch from authoritative metadata without cloning headers or content.
    if let Some(content_type) = content_type {
        if content_type.is_mime_type("message", "delivery-status")
            || content_type.is_mime_type("message", "global-delivery-status")
        {
            return write_delivery_status_part_content(
                part,
                output,
                &options.headers,
                part_options.new_line_format,
                &options.delivery_status,
            )
            .map_err(|error| match error {
                DeliveryStatusError::Io(error) => error,
                error => io::Error::new(io::ErrorKind::InvalidData, error),
            });
        }
        if content_type.is_mime_type("message", "disposition-notification")
            || content_type.is_mime_type("message", "global-disposition-notification")
        {
            return write_disposition_notification_part_content(
                part,
                output,
                &options.headers,
                part_options.new_line_format,
                &options.disposition_notification,
            )
            .map_err(|error| match error {
                DispositionNotificationError::Io(error) => error,
                error => io::Error::new(io::ErrorKind::InvalidData, error),
            });
        }
    }
    write_part_content_with_options(part, output, part_options)
}

/// Masks an unstructured raw header value, retaining potential encoded-word syntax.
///
/// Ordinary bytes become `x`, except SPACE, TAB, CR, LF, `=`, and `?`.
/// A permissive state machine preserves potential charset and encoding labels,
/// masks payload bytes except whitespace and `?`, and retains a closing `?=`.
/// No label validation or encoded-word decoding occurs. Malformed and incomplete
/// sequences follow MimeKit's transitions, not the RFC 2047 decoder's grammar.
///
/// Quotation marks and backslashes have no special treatment outside labels.
/// Returns exactly one byte per input byte; see the [module documentation](self)
/// for scope and limitations.
///
/// ```
/// use mimecraft::anonymize::unstructured_header_value;
///
/// assert_eq!(
///     unstructured_header_value(b" =?utf-8?q?hello_world?=\r\n"),
///     b" =?utf-8?q?xxxxxxxxxxx?=\r\n",
/// );
/// ```
pub fn unstructured_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut word = EncodedWordState::None;
    raw_value
        .iter()
        .map(|&byte| word.mask(byte, WHITESPACE))
        .collect()
}

/// Masks an address-like raw value while retaining punctuation, quotes, and escapes.
///
/// Outside quoted strings, preserves SPACE, TAB, CR, LF, `()<>[]:;@,.`, `=`,
/// and `?`, with the same potential encoded-word handling as
/// [`unstructured_header_value`]. This does not parse mailboxes, comments, groups,
/// or domain literals; malformed address syntax is retained rather than repaired.
///
/// Double quotes and backslashes are retained. An odd run of backslashes escapes
/// the following byte, which is copied verbatim, even inside quotes. Unescaped
/// quotes toggle quoted-string handling: only CR/LF survive masking inside
/// quotes, while spaces and tabs become `x`. Encoded-word state is paused, not
/// reset, while processing quotes, escapes, or quoted text.
///
/// Returns exactly one byte per input byte. Escaped data and potential labels
/// may remain readable; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::address_header_value;
///
/// assert_eq!(
///     address_header_value(b" Alice <alice@example.com>\r\n"),
///     b" xxxxx <xxxxx@xxxxxxx.xxx>\r\n",
/// );
/// ```
pub fn address_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(raw_value.len());
    let mut word = EncodedWordState::None;
    let mut escaped = false;
    let mut quoted = false;

    for &byte in raw_value {
        let masked = match byte {
            b'\\' => {
                escaped = !escaped;
                byte
            }
            b'"' => {
                if escaped {
                    escaped = false;
                } else {
                    quoted = !quoted;
                }
                byte
            }
            _ if escaped => {
                escaped = false;
                byte
            }
            _ if quoted => {
                // Inside quoted strings, preserve CR/LF for inspecting folding,
                // but mask spaces and tabs along with the other quoted text.
                if matches!(byte, b'\r' | b'\n') {
                    byte
                } else {
                    b'x'
                }
            }
            _ => word.mask(byte, ADDRESS_SPECIALS),
        };
        output.push(masked);
    }
    output
}

/// Masks a raw Received value, retaining trace syntax and recognized date suffixes.
///
/// Preserves SPACE, TAB, CR, LF, `<>[]:@,.`, and semicolons outside comments.
/// The keywords `from`, `by`, `via`, `with`, `id`, and `for` retain their original
/// spelling when matched ASCII-case-insensitively at a token boundary and
/// followed by MIME whitespace. An exact keyword at EOF is masked. Comments
/// also delimit tokens; punctuation such as `<` does not start a new token.
///
/// Nested comments are scanned iteratively. Parentheses, backslashes, the same
/// punctuation, and whitespace survive inside comments, but keywords do not.
/// An odd backslash run masks the following byte, including whitespace or a
/// parenthesis, instead of interpreting it. Unclosed comments remain comments
/// through EOF; unmatched closing parentheses outside comments are masked.
///
/// After each semicolon outside comments, skips MIME whitespace and tries
/// [`DateTimeOffset::parse`] on the remaining slice. If accepted, the **entire
/// suffix** is copied verbatim, including comments, ignored tokens, and trailing
/// data. Otherwise masking resumes. This is tolerant date recognition, not
/// validation or formatting; missing times and other recovered forms can pass.
///
/// Returns a new buffer of exactly the input length. Each call starts fresh;
/// lookahead means masking a prefix can differ from that prefix of the full
/// result. No encoded-word or charset decoding occurs. Preserved dates and
/// unrelated suffix data may be sensitive; see the [module documentation](self).
///
/// ```
/// use mimecraft::anonymize::received_header_value;
///
/// assert_eq!(
///     received_header_value(b" from mail.example; 15 Nov 2017 14:16:18 -0000\r\n"),
///     b" from xxxx.xxxxxxx; 15 Nov 2017 14:16:18 -0000\r\n",
/// );
/// ```
pub fn received_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let mut index = 0;

    while index < raw_value.len() {
        if is_whitespace(raw_value[index]) {
            index += 1;
        } else if raw_value[index] == b'(' {
            // Consume '(' and scan the comment, including nested comments.
            let mut depth = 1usize;
            let mut escaped = false;
            index += 1;

            while index < raw_value.len() && depth > 0 {
                let byte = raw_value[index];
                match byte {
                    b'\\' => escaped = !escaped,
                    _ if escaped => {
                        // Escaped bytes are masked even when they are whitespace
                        // or would otherwise open or close a comment.
                        output[index] = b'x';
                        escaped = false;
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ if !is_whitespace(byte) && !RECEIVED_SPECIALS.contains(&byte) => {
                        // Mask everything else in the comment.
                        output[index] = b'x';
                    }
                    _ => {}
                }
                index += 1;
            }
        } else if raw_value[index] == b';' {
            index += 1;
            while index < raw_value.len() && is_whitespace(raw_value[index]) {
                index += 1;
            }

            // This might be a date. Like MimeKit, don't mask any of the suffix
            // if the date parser accepts it, even bytes the parser ignores.
            if DateTimeOffset::parse(&raw_value[index..]).is_ok() {
                break;
            }
        } else if let Some(length) = received_keyword_length(&raw_value[index..]) {
            index += length;
        } else {
            while index < raw_value.len() {
                let byte = raw_value[index];
                if is_whitespace(byte) || matches!(byte, b';' | b'(') {
                    break;
                }
                if !RECEIVED_SPECIALS.contains(&byte) {
                    output[index] = b'x';
                }
                index += 1;
            }
        }
    }
    output
}

fn received_keyword_length(input: &[u8]) -> Option<usize> {
    // Look for: from, by, via, with, id, for. All are at most four bytes,
    // and even an exact match must have a following MIME whitespace byte.
    let length = input.iter().take(5).position(|&byte| is_whitespace(byte))?;
    let word = &input[..length];
    if word.eq_ignore_ascii_case(b"from")
        || word.eq_ignore_ascii_case(b"by")
        || word.eq_ignore_ascii_case(b"via")
        || word.eq_ignore_ascii_case(b"with")
        || word.eq_ignore_ascii_case(b"id")
        || word.eq_ignore_ascii_case(b"for")
    {
        Some(length)
    } else {
        None
    }
}

/// Masks parameter values in a raw Content-Type value, preserving MIME structure.
///
/// Copies everything through the first semicolon verbatim, without validating
/// the media type. Parameter names and continuation suffixes also pass through.
/// Values named `boundary`, `charset`, `delsp`, or `format` are preserved using
/// ASCII-case-insensitive comparison of the name before its first `*`.
/// Names are not trimmed or decoded, and continuation suffixes are not validated.
///
/// Other value bytes become `x`. Outside quotes, SPACE, TAB, CR, LF, and
/// separating semicolons survive. Inside quoted values, quotes, backslashes,
/// and CR/LF survive; all other bytes, including spaces, tabs, and escaped text,
/// are masked. No RFC 2231 or encoded-word decoding occurs.
///
/// Follows MimeKit's permissive parameter states, not [`crate::ParameterList`]
/// parsing. In particular, an incomplete starred parameter followed by `;`
/// retains its accumulated base name when reading the next parameter. This can
/// change which subsequent values are preserved. An unclosed quote keeps later
/// semicolons inside that value. Missing semicolons do not start new parameters.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// Leading text, names, and preserved values may contain sensitive data or
/// malformed bytes; see the [module documentation](self) before sharing output.
///
/// ```
/// use mimecraft::anonymize::content_type_value;
///
/// assert_eq!(
///     content_type_value(b" text/plain; charset=utf-8; name=report.txt\r\n"),
///     b" text/plain; charset=utf-8; name=xxxxxxxxxx\r\n",
/// );
/// ```
pub fn content_type_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the MIME type; only parameter values are candidates.
    content_header_value(raw_value)
}

/// Masks parameter values in a raw Content-Disposition value.
///
/// Preserves everything through the first semicolon, including an unrecognized
/// disposition token. Uses exactly the same parameter rules as
/// [`content_type_value`], including preservation of `boundary`, `charset`,
/// `delsp`, and `format` values even on Content-Disposition. Other values,
/// including filenames, sizes, and dates, are masked rather than decoded.
///
/// Returns a new buffer of exactly the input length, with fresh state per call.
/// This does not validate syntax or guarantee removal of sensitive information;
/// see the [module documentation](self) for limitations.
///
/// ```
/// use mimecraft::anonymize::content_disposition_value;
///
/// assert_eq!(
///     content_disposition_value(b" attachment; filename=\"my file.txt\"\r\n"),
///     b" attachment; filename=\"xxxxxxxxxxx\"\r\n",
/// );
/// ```
pub fn content_disposition_value(raw_value: &[u8]) -> Vec<u8> {
    // Don't mask the "attachment" or "inline" part.
    content_header_value(raw_value)
}

#[derive(Clone, Copy)]
enum ParameterState {
    Semicolon,
    Name,
    NameStar,
    Value,
}

fn is_preserved_parameter_name(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"boundary")
        || name.eq_ignore_ascii_case(b"charset")
        || name.eq_ignore_ascii_case(b"delsp")
        || name.eq_ignore_ascii_case(b"format")
}

fn content_header_value(raw_value: &[u8]) -> Vec<u8> {
    let mut output = raw_value.to_vec();
    let Some(start) = raw_value.iter().position(|&byte| byte == b';') else {
        return output;
    };
    let mut state = ParameterState::Semicolon;
    let mut name = Vec::with_capacity(16);
    let mut escaped = false;
    let mut quoted = false;
    let mut preserve = false;

    for (&byte, masked) in raw_value[start + 1..].iter().zip(&mut output[start + 1..]) {
        if matches!(state, ParameterState::Semicolon) && byte != b';' && !is_whitespace(byte) {
            state = ParameterState::Name;
        }
        match state {
            ParameterState::Semicolon => {
                // Preserve repeated semicolons and intervening whitespace.
            }
            ParameterState::Name => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // An incomplete parameter name: recover at the semicolon.
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                b'*' => state = ParameterState::NameStar,
                _ => name.push(byte),
            },
            ParameterState::NameStar => match byte {
                b'=' => {
                    preserve = is_preserved_parameter_name(&name);
                    state = ParameterState::Value;
                }
                b';' => {
                    // This starred parameter seems incomplete. Like MimeKit,
                    // resume at the next name without clearing the old base name.
                    state = ParameterState::Semicolon;
                }
                _ => {}
            },
            ParameterState::Value => match byte {
                b'"' => {
                    if escaped {
                        escaped = false;
                    } else {
                        quoted = !quoted;
                    }
                }
                b'\\' if quoted => escaped = !escaped,
                b'\r' | b'\n' if quoted => {
                    // Keep CR/LF for inspecting folding within a quoted value.
                    escaped = false;
                }
                _ if quoted => {
                    if !preserve {
                        *masked = b'x';
                    }
                    escaped = false;
                }
                b';' => {
                    state = ParameterState::Semicolon;
                    name.clear();
                }
                _ if !preserve && !is_whitespace(byte) => *masked = b'x',
                _ => {}
            },
        }
    }
    output
}

#[derive(Clone, Copy)]
enum EncodedWordState {
    None,
    Equals,
    EqualsQuestion,
    Charset,
    CharsetQuestion,
    Encoding,
    EncodingQuestion,
    Payload,
    PayloadQuestion,
}

impl EncodedWordState {
    fn mask(&mut self, byte: u8, specials: &[u8]) -> u8 {
        match byte {
            b'=' => match self {
                Self::None => {
                    *self = Self::Equals;
                    byte
                }
                Self::Payload => {
                    // Mask '=' inside the payload, including encoding padding.
                    b'x'
                }
                _ => {
                    // Close ?= or leave an interrupted encoded-word sequence.
                    *self = Self::None;
                    byte
                }
            },
            b'?' => {
                *self = match self {
                    Self::Equals => Self::EqualsQuestion,
                    Self::Charset => Self::CharsetQuestion,
                    Self::Encoding => Self::EncodingQuestion,
                    Self::Payload => Self::PayloadQuestion,
                    // A normal '?' passes through too; unexpected transitions
                    // leave encoded-word mode rather than rejecting the value.
                    _ => Self::None,
                };
                byte
            }
            _ => match self {
                Self::EqualsQuestion | Self::Charset => {
                    // Let the potential charset name pass through unchanged.
                    *self = Self::Charset;
                    byte
                }
                Self::CharsetQuestion | Self::Encoding => {
                    // Likewise preserve the encoding name without validation.
                    *self = Self::Encoding;
                    byte
                }
                Self::EncodingQuestion | Self::Payload => {
                    *self = Self::Payload;
                    // Whitespace inside a payload can affect MIME tokenization,
                    // especially malformed folding, so keep it visible.
                    if is_whitespace(byte) { byte } else { b'x' }
                }
                Self::None | Self::Equals | Self::PayloadQuestion => {
                    // Reprocess unexpected bytes as ordinary text. Returning
                    // their masked value avoids MimeKit's one-byte cursor rewind.
                    *self = Self::None;
                    if specials.contains(&byte) { byte } else { b'x' }
                }
            },
        }
    }
}

// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! MIME processing with an idiomatic Rust API, ported from MimeKit's core library.
//!
//! This crate is under development. Currently it provides incremental MIME
//! Base64, quoted-printable, UU, yEnc, and pass-through codecs, plus parameter
//! percent and RFC 2047 B/Q payload codecs, and owned message/MIME tree I/O.
//! Encoded message-wrapper interpretation and truncated-structure recovery are deferred.
//! Cryptography is outside the scope of this crate.
//! Wrapping body encoders default to CRLF line endings, with LF available through
//! [`encodings::NewLineFormat`]. Decoders accept both CRLF and LF.
//! Pass-through codecs preserve bytes verbatim, including line endings.
//! Header payload encoders do not add wrappers, fold lines, or convert charsets.
//! [`MimeTypes`] guesses filename MIME-types and looks up preferred extensions,
//! with immutable compatibility tables and instance-local registrations.
//! [`encodings::MimeEncoder`] and [`encodings::MimeDecoder`] provide common
//! incremental interfaces with [`ContentEncoding`] metadata.
//! The [`filters`] module adds codec adapters, pass-through filtering, and
//! streaming CRLF/LF conversion, `From ` escaping/armoring, and trailing-whitespace
//! removal; these operate on buffers, not I/O streams.
//! [`filters::AnonymizeFilter`] masks non-whitespace bytes while preserving byte
//! counts and layout; it does not provide full-message anonymization.
//! [`anonymize`] masks raw address and unstructured header values while retaining
//! syntax for inspection, including potential encoded-word labels and escapes.
//! [`filters::BestEncodingFilter`] analyzes content to select a transfer encoding
//! under an [`EncodingConstraint`] without changing the bytes.
//! [`io::FilteredReader`] integrates filters with synchronous [`std::io::Read`]
//! and [`std::io::BufRead`], with EOF finalization and nested composition.
//! [`io::FilteredWriter`] provides buffered [`std::io::Write`] integration with
//! non-finalizing flush and explicit, retryable end-of-input finalization.
//! [`MimeContent`] wraps seekable body streams, with borrowed decoded readers,
//! raw copying, and decoded copying using the existing transfer codecs.
//! [`MimePart`] combines a body stream with authoritative raw headers, detached
//! typed metadata, and checked setters without source I/O or automatic transcoding.
//! Content descriptions use tolerant header decoding and checked unstructured
//! authoring; content durations expose whole seconds with explicit value errors.
//! Content-MD5 accessors retain tolerant text metadata and checked formatting,
//! without computing or verifying checksums.
//! [`MimePart::file_name`] reads disposition-first decoded filenames, and
//! [`MimePart::set_file_name`] atomically updates both MIME filename parameters.
//! [`FileNameEditError`] reports missing metadata, parse, and formatting failures.
//! [`MimePart::best_encoding`] recommends an encoding using decoded text/message
//! analysis or MimeKit's non-text shortcut, without changing headers or content.
//! [`BestEncodingError`] retains metadata, line-limit, and source I/O failures.
//! [`MimePart::prepare`] applies transport encoding selection to a leaf, retaining
//! already-safe requested encodings; [`PartPrepareError`] preserves failures.
//! [`MimeEntity::prepare`] extends this to trees with iterative traversal.
//! [`MimeMessage::prepare`] also inserts a missing MIME-Version for a body with
//! headers; [`MimePrepareError`] reports traversal-stopping failures.
//! The part's synchronous writer serializes complete single parts with streaming
//! transfer conversion, UU wrappers, and [`MimePartWriteOptions`] newline settings.
//! [`TextPart`] adds strict charset-aware text reads and atomic in-memory text
//! updates, preserving raw metadata and reusing single-part serialization.
//! Format-based constructors and metadata predicates share [`TextFormat`] body
//! matching, without implicitly converting or validating text formats.
//! [`MessageDeliveryStatus`] adds bounded, detached delivery-status header groups,
//! one encoded-remainder compatibility pass, and atomic in-memory group edits.
//! [`MessageDispositionNotification`] provides bounded, detached notification
//! fields and atomic in-memory replacement without interpreting inner encodings.
//! [`MessageFeedbackReport`] provides the same field-block access for
//! machine-readable feedback reports without imposing field-level semantics.
//! [`MessagePartial`] provides checked fragment metadata and bounded reassembly
//! of decoded fragment streams, with original-message header merging.
//! [`MultipartReport`] adds checked report-type metadata to ordinary multipart
//! containers without imposing a report layout or accessing child sources.
//! [`MultipartRelated`] shares text-body root selection and adds atomic root
//! replacement with checked type/start parameters, without generating IDs.
//! [`MimeEntity`] and [`Multipart`] own nested MIME trees, with checked boundary
//! edits, raw preamble/epilogue bytes, and iterative [`LeafParts`] traversal.
//! Their synchronous writers use iterative traversal, shared leaf encoding,
//! checked ancestor boundaries, and independent delimiter-owned newlines.
//! [`MimeEntity::write_content_to`] omits only the root's headers and separator,
//! retaining transfer encoding, multipart framing, and complete enclosed messages.
//! [`MimeEntity::load`] reads complete entity trees from buffered sources into
//! owned body streams, preserving encoded bytes and raw headers with explicit
//! [`MimeEntityParseOptions`] limits. Recovery of truncated entities is deferred.
//! [`MimeMessage`] owns message headers and an optional body tree. Loading
//! partitions root Content-* headers onto the body, preserving raw records;
//! writing groups message headers before body headers without generating fields.
//! [`MimeMessage::write_headers_to`] writes only that header section and separator
//! with an immutable borrow, without inspecting body metadata or reading sources.
//! Original header interleaving and automatic transport preparation are deferred.
//! [`MessageImportance`], [`MessagePriority`], and [`XMessagePriority`] add
//! independent typed header access with checked parsing and canonical setters.
//! Message identifiers and reply references also have raw-header-backed accessors,
//! assignment-validated setters, and explicit parsing and recovery errors.
//! Date, Resent-Date, and MIME-Version accessors reuse the existing value types,
//! with explicit absence, parse errors, and independent canonical setters.
//! Sender accessors return owned mailbox prefixes and provide atomic, checked
//! replacement with configurable parsing and address-header formatting.
//! Standard and resent address-list accessors aggregate matching headers with
//! MimeKit-style recovery and [`MessageAddresses`] diagnostics. Their explicit
//! setters validate complete replacements atomically; Bcc is not hidden on write.
//! [`MimeMessage::recipients`] selects ordinary or resent recipient fields,
//! returning flattened, duplicate-preserving [`MessageRecipients`] with diagnostics.
//! Subject accessors reuse tolerant header decoding and checked
//! [`Header::from_unstructured`] authoring with bounded folding, normalized
//! assignment, and atomic duplicate-collapsing replacement.
//! [`MessagePart`] adds owned attached messages with checked
//! wrapper metadata, iterative writing, and leaf traversal through their bodies.
//! [`TextRfc822Headers`] uses the same structured-message representation for
//! text/rfc822-headers and message/global-headers returned-header containers.
//! Loading recognizes transparent message wrappers with shared tree-wide limits
//! and an opaque opt-out; encoded wrappers remain ordinary leaves.
//! Untyped multipart/digest children default to attached messages without
//! synthesizing headers; tree writing honors that immediate-parent context.
//! [`MimeEntity::content_id`] and the concrete entity accessors parse the first
//! raw Content-Id tolerantly, with explicit errors rather than silent absence.
//! [`MimeEntity::set_content_id`] validates before canonical duplicate-collapsing
//! replacement; neither operation reads sources or generates identifiers.
//! [`BodyParts`] traverses non-multipart nodes without entering attached messages;
//! [`Attachments`] filters their dispositions with explicit parse errors.
//! [`MimeEntity::set_is_attachment`] and concrete entity setters preserve
//! disposition parameters, with atomic edits and MimeKit-compatible no-ops.
//! [`AttachmentEditError`] distinguishes parse failures from formatting errors.
//! [`MimeIterator`] visits every entity, including attached-message bodies, with
//! part paths, ancestry, subtree skipping, and direct depth-proportional navigation.
//! [`MimeMessage::entity_at`] distinguishes malformed paths from missing parts.
//! [`MimeMessage::text_body_part`] selects a borrowed body in a [`TextFormat`],
//! honoring alternative preference, related roots, and mixed-part stopping rules.
//! Selection performs no stream I/O, decoding, or text-format conversion.
//! [`MimeMessage::text_body`] separately reads the selected content with strict
//! charset decoding and [`text::FlowedToText`] paragraph unwrapping when declared.
//! [`text::TextToFlowed`] wraps plain text with space stuffing and UTF-8-safe
//! soft breaks for caller-authored `format=flowed; delsp=yes` text parts.
//! [`text::TextToText`] copies decoded text verbatim with optional text headers
//! and footers, without inserting separators or normalizing line endings.
//! [`text::PlainTextPreviewer`] produces length-limited, whitespace-normalized
//! previews of decoded plain text without selecting or reading MIME bodies.
//! [`text::html_encode`] and [`text::html_attribute_encode`] escape character
//! data and quoted attribute values without parsing or rendering HTML.
//! [`text::HtmlWriter`] builds HTML in memory with checked tag/attribute
//! operations, escaped text, explicit states, and trusted raw markup.
//! [`text::HtmlTagId`] and [`text::HtmlAttributeId`] recognize canonical names;
//! [`text::HtmlAttributeCollection`] preserves checked attributes in order,
//! including duplicate names and valueless attributes.
//! [`text::HtmlToken`] serializes owned tags, comments, character data, CDATA,
//! script data, and checked doctypes without providing an HTML tokenizer.
//! [`text::html_decode`] decodes named and numeric character references using
//! MimeKit's permissive character-data rules without stripping markup.
//! [`text::html_attribute_decode`] preserves ambiguous legacy references in
//! attribute contents, while [`text::HtmlEntityDecoder`] accepts one character
//! at a time with bounded state and context-sensitive value retrieval.
//! [`text::html_decode_with_options`] and
//! [`text::html_attribute_decode_with_options`] add reference diagnostics with
//! original UTF-8 byte offsets and optional per-violation reporting limits.
//! [`text::RtfTokenizer`] reads bounded, owned RTF tokens from buffered bytes,
//! retaining source positions and optionally reporting lexical recoveries.
//! [`text::RtfTokenizerOptions`] bounds those reports independently per violation
//! across streaming reads, with one marker when an issue is first suppressed.
//! It skips binary payloads but does not decode code pages or render RTF.
//! [`text::RtfValidator`] adds missing-header, unmatched-closing-brace, and
//! trailing-content checks without changing tokens. [`text::RtfValidationOptions`]
//! caps the combined lexical/structural report with independent violation quotas.
//! [`text::RtfToText`] renders original buffered RTF bytes into Unicode text,
//! with bounded group/font state, code-page decoding, Unicode fallback, tables,
//! optional attachment placeholders, and capped interpretation diagnostics.
//! [`text::RtfToHtml`] shares that interpreter to render HTML documents or
//! fragments with character styles, paragraph alignment, colors, and field links.
//! Its optional tag callbacks use [`text::HtmlTagContext`] to customize generated
//! tags and suppress content. [`text::RtfHtmlExtractor`] separately recovers raw
//! encapsulated HTML without sanitization, filtering, or synthesized markup.
//! [`text::parse_rtf_hyperlink`] extracts borrowed, scheme-filtered targets
//! from decoded RTF field instructions without tokenizing or rendering RTF.
//! [`text::TextToHtml`] renders decoded plain text with line breaks, quote blocks,
//! automatic links, and optional text or trusted-HTML headers and footers.
//! [`text::FlowedToHtml`] joins flowed lines into HTML paragraphs with the same
//! links and decorations, preserving quote-depth boundaries and `delsp` behavior.
//! Both converters accept per-conversion [`text::HtmlTagContext`] callbacks
//! to customize body tags and suppress link labels, paragraphs, or quotations.
//! [`io::BoundedStream`] exposes a seekable source range without copying it,
//! suitable for storing a MIME body's subrange in [`MimeContent`].
//! [`io::ChainedStream`] joins seekable sources into one logical stream with
//! cross-boundary reads/writes and combined-offset seeking.
//! [`io::MemoryBlockStream`] supplies block-backed, growable in-memory storage
//! with zero-initialized resizing, avoiding repeated copies of existing bytes.
//! [`io::MeasuringStream`] measures serialized or encoded output without
//! retaining bytes, with seeking and logical resizing.
//! [`HeaderParser`] incrementally separates raw header records from body bytes,
//! preserving folding and malformed records without depending on I/O traits.
//! [`Header::parse`] parses standalone fields with terminal-newline repair;
//! [`Header::decode_value`] unfolds RFC 2047 text with charset diagnostics.
//! [`Header::parse_addresses`], [`Header::parse_content_type`], and
//! [`Header::parse_content_disposition`] instead parse raw structured values.
//! Their typed models create checked replacement records through `to_header`
//! methods, reusing the existing formatters and [`HeaderList::replace_all`].
//! [`HeaderId`] identifies known fields without changing their original spelling.
//! [`HeaderList`] keeps raw headers in order, including duplicates, with
//! case-insensitive field lookup and explicit, order-preserving list edits.
//! Its synchronous loading leaves body bytes in the caller's buffered reader;
//! writing preserves raw bytes or converts line endings without adding a separator.
//! [`HeaderParser::read_from`] also exposes separator metadata and resumable reads.
//! [`ContentEncoding::parse`] and [`MimeVersion::parse`] interpret transfer-encoding
//! and version values using MimeKit's tolerant parsing rules.
//! [`MessageId`] parses Message-Id and Content-Id values, with complete-value
//! and prefix APIs, UTF-8 validation, and legacy compatibility handling.
//! [`MessageId::from_value`] separately applies MimeKit's assignment checks.
//! [`References`] lazily extracts bracketed IDs from References/In-Reply-To
//! values, with explicit errors and MimeKit-compatible recovery.
//! [`DomainList`] parses obsolete mailbox routes with ordered UTF-8 domains,
//! comments, literals, and complete/prefix APIs sharing message-ID domain parsing.
//! [`idna`] converts international domains with explicit validity/MIME-safety
//! errors; [`DomainList::to_ascii`] separately formats validated ASCII routes.
//! [`AddrSpec`] parses the local-part/domain portion of an address, with quoted
//! words, compatibility options, and explicit Latin-1 recovery diagnostics.
//! [`MailboxAddress`] adds decoded display names, angle brackets, source routes,
//! trailing-comment names, and single-mailbox compatibility parsing.
//! [`Address`], [`GroupAddress`], and [`AddressList`] add groups and lists with
//! bounded nesting, fail-fast parsing, and depth-first mailbox traversal.
//! Mailboxes and groups also support construction and editing with typed
//! addr-specs; editing is separate from header-authoring validation.
//! Address models implement unencoded display formatting and separately
//! provide checked ASCII value encoding via [`AddressList::to_ascii`] and
//! corresponding mailbox/group methods. [`AddressList::to_header_string`] adds
//! checked field names and hard-bounded folding using [`AddressHeaderFormatOptions`].
//! Encoded name controls are preserved, not sanitized.
//! Address equality and hashing compare exact names/address text and ordered
//! members, excluding source routes and decoding diagnostics.
//! [`utils`] provides shared quoting, tolerant string/byte unquoting, and unfolding.
//! [`DateTimeOffset`] parses legacy date-header values and formats canonical dates
//! with checked calendar components and a fixed numeric UTC offset.
//! [`charsets`] resolves MIME charset labels and converts complete text buffers,
//! with explicit conversion errors and legacy codecs supplied by `encoding_rs`.
//! [`filters::CharsetFilter`] converts between charsets incrementally, retaining
//! split characters and shift state with explicit replacement diagnostics.
//! [`rfc2047`] decodes encoded words in text and phrases, with loose/strict
//! tokenization, adjacent-word recovery, and explicit charset diagnostics.
//! Its encoding helpers produce ASCII text/phrase values with complete,
//! size-bounded encoded words and explicit charset conversion errors.
//! [`ParameterList`] parses MIME parameters, including RFC 2231 continuations
//! and extended values, with explicit syntax errors and recovery diagnostics.
//! [`ContentType`] and [`ContentDisposition`] parse complete MIME header values
//! with decoded parameters and accessors for common metadata. They also support
//! construction and editing through validated tokens and ordered parameter lists;
//! [`HeaderFormatOptions`] controls their ASCII header serialization, with
//! RFC 2231 parameters, folding, and selectable CRLF/LF endings.
//!
//! The default `simd` feature accelerates Base64 and UU with runtime-detected
//! SSSE3/AVX2 on x86/x86-64 and NEON on AArch64. Other CPUs use scalar codecs. Disable
//! default features to disable mimecraft's SIMD and forbid unsafe code in this
//! crate. Dependencies such as `encoding_rs` have their own acceleration policies.
//!
//! ```
//! use mimecraft::encodings::{Base64Decoder, Base64Encoder};
//!
//! let mut encoded = Vec::new();
//! let mut encoder = Base64Encoder::new();
//! encoder.encode(b"hello ", &mut encoded);
//! encoder.encode(b"world", &mut encoded);
//! encoder.finish(&mut encoded);
//! assert_eq!(encoded, b"aGVsbG8gd29ybGQ=\r\n");
//!
//! let mut decoded = Vec::new();
//! let mut decoder = Base64Decoder::new();
//! for chunk in encoded.chunks(3) {
//!     decoder.decode(chunk, &mut decoded);
//! }
//! assert_eq!(decoded, b"hello world");
//! ```

#![deny(unsafe_code)]
#![cfg_attr(not(feature = "simd"), forbid(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod addr_spec;
mod address;
mod address_encode;
mod address_header_format;
mod address_list;
pub mod anonymize;
pub mod charsets;
mod content_disposition;
mod content_encoding;
mod content_header;
mod content_type;
mod date_time_offset;
mod domain_list;
mod encoding_constraint;
pub mod encodings;
pub mod filters;
mod group_address;
mod header;
mod header_format;
mod header_id;
mod header_list;
pub mod idna;
pub mod io;
mod mailbox_address;
mod message_delivery_status;
mod message_disposition_notification;
mod message_feedback_report;
mod message_id;
mod message_part;
mod message_partial;
mod message_priority;
mod mime_content;
mod mime_entity;
mod mime_entity_load;
mod mime_entity_prepare;
mod mime_entity_write;
mod mime_iterator;
mod mime_message;
mod mime_part;
mod mime_part_write;
mod mime_types;
mod mime_types_data;
mod mime_version;
mod multipart;
mod multipart_related;
mod multipart_report;
mod parameter;
mod parse_utils;
mod report_fields;
pub mod rfc2047;
pub mod text;
mod text_body;
mod text_part;
mod text_rfc822_headers;
mod unstructured_header_format;
pub mod utils;

pub use addr_spec::{
    AddrSpec, AddrSpecParseError, AddrSpecParseErrorKind, AddrSpecParseOptions,
    AddressComplianceMode,
};
pub use address::{Address, AddressParseError, AddressParseErrorKind, AddressParseOptions};
pub use address_encode::{AddressEncodeError, AddressEncodeErrorKind};
pub use address_header_format::{AddressHeaderFormatError, AddressHeaderFormatOptions};
pub use address_list::{AddressList, Mailboxes};
pub use content_disposition::{ContentDisposition, ContentDispositionSizeError};
pub use content_encoding::{ContentEncoding, UnknownContentEncoding};
pub use content_header::{ContentHeaderParseError, ContentHeaderParseErrorKind};
pub use content_type::ContentType;
pub use date_time_offset::{DateTimeOffset, DateTimeOffsetError};
pub use domain_list::{
    DomainList, DomainListEncodeError, DomainListParseError, DomainListParseErrorKind,
};
pub use encoding_constraint::EncodingConstraint;
pub use group_address::GroupAddress;
pub use header::{Header, HeaderParseError, HeaderParser, HeaderTooLong};
pub use header_format::{HeaderFormatError, HeaderFormatOptions};
pub use header_id::HeaderId;
pub use header_list::{HeaderList, HeaderListEditError};
pub use mailbox_address::{
    MailboxAddress, MailboxParseError, MailboxParseErrorKind, MailboxParseOptions,
};
pub use message_delivery_status::{
    DeliveryStatusError, DeliveryStatusParseOptions, MessageDeliveryStatus,
};
pub use message_disposition_notification::{
    DispositionNotificationError, DispositionNotificationParseOptions,
    MessageDispositionNotification,
};
pub use message_feedback_report::{
    FeedbackReportError, FeedbackReportParseOptions, MessageFeedbackReport,
};
pub use message_id::{MessageId, MessageIdParseError, MessageIdParseErrorKind, References};
pub use message_part::{MessagePart, MessagePartError};
pub use message_partial::{MessagePartial, MessagePartialError};
pub use message_priority::{
    MessageImportance, MessagePriority, MessagePriorityParseError, XMessagePriority,
};
pub use mime_content::MimeContent;
pub use mime_entity::{AttachmentEditError, Attachments, BodyParts, LeafParts, MimeEntity};
pub use mime_entity_load::MimeEntityParseOptions;
pub use mime_entity_prepare::MimePrepareError;
pub use mime_iterator::{MimeIterator, MimeIteratorPositionError, MimePathParseError};
pub use mime_message::{MessageAddressIssue, MessageAddresses, MessageRecipients, MimeMessage};
pub use mime_part::{
    BestEncodingError, FileNameEditError, InvalidContentDuration, MimePart, PartPrepareError,
};
pub use mime_part_write::MimePartWriteOptions;
pub use mime_types::{MimeTypeRegistrationError, MimeTypes};
pub use mime_version::{MimeVersion, MimeVersionParseError, MimeVersionParseErrorKind};
pub use multipart::{Multipart, MultipartError};
pub use multipart_related::{MultipartRelated, MultipartRelatedError};
pub use multipart_report::{MultipartReport, MultipartReportError};
pub use parameter::{
    Parameter, ParameterComplianceMode, ParameterIssue, ParameterIssueKind, ParameterList,
    ParameterListEditError, ParameterParseError, ParameterParseErrorKind, ParameterParseOptions,
};
pub use text_body::{TextBodyError, TextFormat};
pub use text_part::{TextPart, TextPartError};
pub use text_rfc822_headers::{TextRfc822Headers, TextRfc822HeadersError};
pub use unstructured_header_format::{
    UnstructuredHeaderFormatError, UnstructuredHeaderFormatOptions,
};

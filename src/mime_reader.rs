// Adapted from MimeKit's MimeReader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, BufRead},
    num::NonZeroUsize,
};

use crate::{
    ContentEncoding, HeaderList, HeaderParser, MimeComplianceLogger, MimeComplianceOptions,
    encodings::NewLineFormat, mime_compliance::Reporting,
};

mod engine;
mod options;
mod scan;

use engine::ReadContext;
pub use options::MimeEntityParseOptions;

/// A position in the reader's original input, before newline normalization.
///
/// Offsets start at zero and lines at one, regardless of an underlying stream's
/// seek position. LF advances the line number; a bare CR does not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimePosition {
    /// Original byte offset relative to the start of this reader's input.
    pub offset: u64,
    /// One-based physical line number.
    pub line: u64,
}

impl Default for MimePosition {
    fn default() -> Self {
        Self { offset: 0, line: 1 }
    }
}

impl MimePosition {
    pub(crate) fn advance(self, bytes: &[u8]) -> Self {
        Self {
            offset: self.offset + bytes.len() as u64,
            line: self.line + memchr::memchr_iter(b'\n', bytes).count() as u64,
        }
    }
}

/// The structural interpretation of an entity, not its exact media subtype.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeEntityKind {
    /// Opaque, transfer-encoded content, including text and report leaves.
    Part,
    /// A multipart container.
    Multipart,
    /// A transparent wrapper around an optional embedded message.
    MessagePart,
}

/// Which part of an entity a content callback describes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeContentKind {
    /// Stored, undecoded leaf content.
    Body,
    /// Bytes before the first multipart delimiter.
    Preamble,
    /// Bytes after a multipart closing delimiter.
    Epilogue,
}

/// A synchronous MIME reader notification.
///
/// Begin/end events nest in wire order. A message contains one root entity;
/// its root's header list includes both message and Content-* fields. An empty
/// message wrapper contains no message events. Content slices are borrowed
/// only for the callback; headers are owned and can be retained without cloning.
///
/// End positions are exclusive. An entity ending at a parent delimiter excludes
/// the delimiter-owned preceding newline. Leaf newline hints describe only
/// emitted content. Error or callback failure may leave begin events unmatched.
/// Standalone header and status-group reading emit only [`Headers`](Self::Headers).
#[derive(Debug)]
pub enum MimeEvent<'a> {
    /// An mbox envelope line before the corresponding message.
    ///
    /// Emitted only by [`MimeReader::read_mbox_message`]. The sender/date is not
    /// interpreted, and this line is not part of the message's headers.
    MboxMarker {
        /// Original marker bytes, including the LF or CRLF ending.
        bytes: &'a [u8],
        /// Start of the marker's `From ` prefix.
        position: MimePosition,
        /// Exclusive position after the marker's newline.
        end: MimePosition,
    },
    /// A standalone header block, without interpreting MIME structure.
    ///
    /// Emitted by [`MimeReader::read_headers`] (including for empty input) or
    /// [`MimeReader::read_status_group`] (only for nonempty groups).
    Headers {
        /// Raw headers, retaining duplicates, malformed records, and offsets.
        headers: HeaderList,
        /// Position before the first header.
        position: MimePosition,
        /// Position after the separator, or at EOF if there is no separator.
        headers_end: MimePosition,
        /// Exact separator (LF, CRLF, or lone CR at EOF), borrowed for the callback.
        ///
        /// `None` means the block ended at EOF without a separator.
        body_separator: Option<&'a [u8]>,
    },
    /// Beginning of the requested message or a present embedded message.
    MessageBegin {
        /// Position before its first header.
        position: MimePosition,
    },
    /// A complete header section and its checked structural interpretation.
    EntityBegin {
        /// Raw headers, including original byte offsets and duplicates.
        headers: HeaderList,
        /// Selected structural kind.
        kind: MimeEntityKind,
        /// Stored transfer encoding; payload bytes remain undecoded.
        encoding: ContentEncoding,
        /// Position before its first header.
        position: MimePosition,
        /// Position after the separator, or at EOF/an enclosing marker when
        /// [`MimeEntityParseOptions::allow_header_only`] completes present headers.
        headers_end: MimePosition,
    },
    /// A nonempty chunk of body, preamble, or epilogue bytes.
    Content {
        /// Which portion of the current entity is being read.
        kind: MimeContentKind,
        /// Original bytes, excluding multipart delimiter framing.
        bytes: &'a [u8],
        /// Position of the first byte.
        position: MimePosition,
    },
    /// End of the current entity.
    EntityEnd {
        /// Structural kind matching the corresponding begin event.
        kind: MimeEntityKind,
        /// Exclusive end of the entity.
        position: MimePosition,
        /// Leaf content's observed newline format; `None` for containers.
        new_line_format: Option<NewLineFormat>,
    },
    /// End of the requested message or an embedded message.
    MessageEnd {
        /// Exclusive end of the message.
        position: MimePosition,
    },
}

/// Forward-only synchronous MIME scanning without constructing a MIME tree.
///
/// Uses [`MimeEntityParseOptions`] limits and MimeKit-style recovery for
/// truncated headers/containers and malformed MIME metadata. Fallible compliance
/// loggers can reject selected violations. Async I/O and persistent source
/// ranges are not implemented.
///
/// Ordinary content borrows buffered input directly, including across physical
/// lines. With the default `simd` feature, runtime-detected SSE2/AVX2 or NEON
/// scans fuse boundary-prefix detection, line counting, and newline formats.
/// Scalar matching is the fallback; bulk byte searches use `memchr`.
/// The reader retains headers, nesting state, and lookahead, not complete bodies.
/// Ambiguous delimiter lines with long transport padding must be buffered until
/// their classification is known; their storage is bounded by the input limit,
/// not a fixed chunk size. Content callback slices are at most 4096 bytes.
///
/// Entity/message reading consumes through EOF; standalone header reading stops
/// after the separator or at EOF, without consuming body bytes.
/// Each installed source supports one standalone header/entity/message operation,
/// incremental status groups via [`Self::read_status_group`], or explicit mbox
/// iteration via [`Self::read_mbox_message`].
/// Modes cannot be mixed. Repeating a standalone operation or continuing after
/// a failed group is an `InvalidInput` error. Use
/// [`Self::replace_source`] to start fresh, or recover the source with
/// [`Self::into_inner`], rather than attempting to resume a partial parse.
/// Borrow a source with `MimeReader::new(&mut source)` to keep ownership outside
/// the reader. Its underlying buffered read-ahead is not a logical byte offset.
///
/// ```
/// use mimecraft::{MimeContentKind, MimeEvent, MimeReader};
///
/// let mut reader = MimeReader::new(b"Subject: example\r\n\r\nbody".as_slice());
/// let mut body_bytes = 0;
/// reader.read_message(|event| {
///     if let MimeEvent::Content { kind: MimeContentKind::Body, bytes, .. } = event {
///         body_bytes += bytes.len();
///     }
///     Ok(())
/// })?;
/// assert_eq!(body_bytes, 4);
/// assert_eq!(reader.position().offset, 24);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct MimeReader<R> {
    source: R,
    options: MimeEntityParseOptions,
    compliance_options: MimeComplianceOptions,
    position: MimePosition,
    state: ReadState,
    buffer: engine::ScanBuffer,
}

// Parser-only framing hooks leave the public content/event contract unchanged.
// Closure readers do not retain delimiters or allocate a second input buffer.
pub(crate) trait EventSink {
    fn event(&mut self, event: MimeEvent<'_>) -> io::Result<()>;

    fn body_separator(&mut self, _present: bool) {}

    fn boundary(&mut self, _bytes: &[u8], _closing: bool, _end: MimePosition) {}

    fn end_boundary(&mut self, _position: Option<MimePosition>) {}
}

impl<F: FnMut(MimeEvent<'_>) -> io::Result<()>> EventSink for F {
    fn event(&mut self, event: MimeEvent<'_>) -> io::Result<()> {
        self(event)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReadState {
    Ready,
    Used,
    EndOfStream,
    StatusGroups,
    StatusGroupsEndOfStream,
    Mbox,
    MboxEndOfStream,
}

impl<R: BufRead> MimeReader<R> {
    /// Creates a reader with default limits, without performing I/O.
    pub fn new(source: R) -> Self {
        Self {
            source,
            options: MimeEntityParseOptions::default(),
            compliance_options: MimeComplianceOptions::default(),
            position: MimePosition::default(),
            state: ReadState::Ready,
            buffer: Default::default(),
        }
    }

    /// Creates a reader with explicit limits, without performing I/O.
    ///
    /// # Errors
    ///
    /// Zero limits return `InvalidInput`.
    pub fn with_options(source: R, options: MimeEntityParseOptions) -> io::Result<Self> {
        options.validate()?;
        Ok(Self {
            options,
            ..Self::new(source)
        })
    }

    /// Returns the supplied options.
    pub fn options(&self) -> &MimeEntityParseOptions {
        &self.options
    }

    /// Returns reporting settings; ordinary reads without a logger ignore them.
    pub fn compliance_options(&self) -> &MimeComplianceOptions {
        &self.compliance_options
    }

    /// Sets reporting settings for subsequent operations, without changing parsing.
    ///
    /// Each operation snapshots these settings and starts fresh per-violation
    /// budgets, so one mbox message cannot exhaust the next message's quotas.
    pub fn set_compliance_options(&mut self, options: MimeComplianceOptions) {
        self.compliance_options = options;
    }

    /// Returns the consumed input position, which may be ahead of an event.
    pub fn position(&self) -> MimePosition {
        self.position
    }

    /// Returns whether a successful operation finished at an observed EOF.
    ///
    /// This inspects reader state without performing I/O. It is initially false,
    /// even for an empty source. Entity/message success sets it to true.
    /// Header/group reading sets it only when EOF terminates the block; a separator
    /// stops reading without probing whether any body bytes follow it.
    /// False therefore does not prove that more bytes are available.
    ///
    /// A source, parse, or callback failure does not mark completion. Replacing
    /// the source resets this state. Group iteration may return false at EOF
    /// repeatedly; standalone operations remain one-shot. In mbox mode, only
    /// the final message (or empty archive) marks EOF, not a next-message marker.
    pub fn is_end_of_stream(&self) -> bool {
        matches!(
            self.state,
            ReadState::EndOfStream
                | ReadState::StatusGroupsEndOfStream
                | ReadState::MboxEndOfStream
        )
    }

    /// Installs a new source and returns the previous buffered source.
    ///
    /// Preserves options but resets byte/line positions, completion state, and
    /// the operation-mode guard, including after a failed operation. The new
    /// input starts at offset zero and line one regardless of its seek position.
    /// Neither source is read, sought, flushed, or closed. The returned source
    /// retains its current position and unread buffered bytes.
    /// Mbox prefix lookahead belongs to this reader, not the source: save
    /// [`Self::pending_input`] before replacement if that data is needed.
    ///
    /// This starts a new parse; it does not resume the previous operation or
    /// rewind either source. To change the source's Rust type, construct a new
    /// reader instead.
    pub fn replace_source(&mut self, source: R) -> R {
        // Like MimeKit's SetStream/Initialize, reset stream-dependent state
        // without changing options. Return the old buffer instead of dropping
        // read-ahead that may be needed by a non-seekable source's caller.
        self.position = MimePosition::default();
        self.state = ReadState::Ready;
        self.buffer.reset();
        std::mem::replace(&mut self.source, source)
    }

    /// Borrows the underlying buffered source without changing its position.
    pub fn get_ref(&self) -> &R {
        &self.source
    }

    /// Returns the source, retaining its buffering and current position.
    ///
    /// In mbox mode use [`Self::into_parts`] to also recover reader-owned prefix
    /// lookahead. This method discards that prefix.
    pub fn into_inner(self) -> R {
        self.source
    }

    /// Returns reader-owned lookahead already consumed from the underlying source.
    ///
    /// This is empty except at an mbox message boundary, where it is `b"From "`.
    /// It belongs to the next message's marker. Positions include its consumption.
    /// Replacing the source discards it; save it first when extracting a partial
    /// archive, or use [`Self::into_parts`] to recover both pieces together.
    pub fn pending_input(&self) -> &[u8] {
        self.buffer.pending_input()
    }

    /// Returns the underlying source and unread reader-owned lookahead.
    ///
    /// Replay the second tuple member before reading the source. Unlike
    /// [`Self::into_inner`], this also preserves an mbox boundary's five-byte
    /// lookahead; any read-ahead within the source itself stays in that source.
    pub fn into_parts(self) -> (R, Vec<u8>) {
        let pending = self.buffer.pending_input().to_vec();
        (self.source, pending)
    }

    /// Reads a standalone header block, leaving the body in the buffered source.
    ///
    /// Emits one [`MimeEvent::Headers`] event after the separator or EOF, with
    /// no entity/message events or MIME metadata validation. Empty input and
    /// unterminated headers are accepted. Like [`HeaderList::load`], this retains
    /// even an invalid first record, unlike MimeKit's higher-level parser.
    /// Input limits count only consumed header bytes and the separator, not
    /// buffered body bytes. Depth and message interpretation do not apply.
    ///
    /// # Errors
    ///
    /// Source and callback errors propagate unchanged; interrupted reads retry.
    /// Exceeded input/header limits return [`io::ErrorKind::InvalidData`];
    /// header-length errors retain [`crate::HeaderTooLong`] as their cause.
    /// A failure before completion emits no event. Source consumption is not
    /// rolled back, and another operation returns `InvalidInput` after either
    /// success or failure.
    ///
    /// ```
    /// use mimecraft::{MimeEvent, MimeReader};
    ///
    /// let mut reader = MimeReader::new(b"Subject: example\r\n\r\nbody".as_slice());
    /// reader.read_headers(|event| {
    ///     if let MimeEvent::Headers { headers, body_separator, .. } = event {
    ///         assert!(headers.contains("Subject"));
    ///         assert_eq!(body_separator, Some(b"\r\n".as_slice()));
    ///     }
    ///     Ok(())
    /// })?;
    /// assert_eq!(reader.into_inner(), b"body");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn read_headers<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_headers_impl(&mut callback, None)
    }

    /// Reads standalone headers with synchronous, fallible compliance reporting.
    ///
    /// Reports raw-header defects, repeated MIME fields, and metadata issues.
    /// No required message fields are demanded. Otherwise follows
    /// [`Self::read_headers`]. Logger errors propagate unchanged.
    pub fn read_headers_with_logger<F>(
        &mut self,
        logger: &mut dyn MimeComplianceLogger,
        mut callback: F,
    ) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_headers_impl(&mut callback, Some(logger))
    }

    fn read_headers_impl<F>(
        &mut self,
        callback: &mut F,
        logger: Option<&mut dyn MimeComplianceLogger>,
    ) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.begin_read(ReadState::Ready)?;
        let mut reporting = Reporting::new(logger, self.compliance_options, false);
        let eof = self.read_header_block(callback, false, &mut reporting)?;
        // Publish completion only after the final callback succeeds, as in
        // MimeKit's ReadHeaders. Do not probe beyond a consumed separator.
        if eof {
            self.state = ReadState::EndOfStream;
        }
        Ok(())
    }

    /// Reads the next nonempty delivery-status header group.
    ///
    /// Skips empty CRLF/LF lines and emits one [`MimeEvent::Headers`], returning
    /// true. Returns false without an event once EOF leaves no group, including
    /// a lone trailing CR. Repeated calls after that return false without I/O.
    /// Unterminated and malformed records follow [`Self::read_headers`].
    ///
    /// Calling this first selects status-group iteration for this source.
    /// Positions and the input budget include all prior groups and skipped
    /// separators. Only one group is retained at a time; fields are not
    /// interpreted or transfer-decoded. The source stops after each group's
    /// separator (or EOF), leaving a separately encoded remainder untouched.
    ///
    /// # Errors
    ///
    /// Follows [`Self::read_headers`]'s source, callback, and limit errors.
    /// A failed group emits no successful result and prevents further reading
    /// until [`Self::replace_source`]. Mixing group iteration with standalone
    /// header, entity, or message operations returns `InvalidInput`.
    pub fn read_status_group<F>(&mut self, mut callback: F) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_status_group_impl(&mut callback, None)
    }

    /// Reads a status group with the header diagnostics of
    /// [`Self::read_headers_with_logger`]. Budgets reset for each call.
    pub fn read_status_group_with_logger<F>(
        &mut self,
        logger: &mut dyn MimeComplianceLogger,
        mut callback: F,
    ) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_status_group_impl(&mut callback, Some(logger))
    }

    fn read_status_group_impl<F>(
        &mut self,
        callback: &mut F,
        logger: Option<&mut dyn MimeComplianceLogger>,
    ) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        if matches!(self.state, ReadState::StatusGroupsEndOfStream) {
            return Ok(false);
        }
        self.begin_read(ReadState::StatusGroups)?;
        let mut reporting = Reporting::new(logger, self.compliance_options, false);
        let mut emitted = false;
        let eof = self.read_header_block(
            &mut |event| {
                callback(event)?;
                emitted = true;
                Ok(())
            },
            true,
            &mut reporting,
        )?;
        self.state = if eof {
            ReadState::StatusGroupsEndOfStream
        } else {
            ReadState::StatusGroups
        };
        Ok(emitted)
    }

    fn read_header_block<F>(
        &mut self,
        callback: &mut F,
        skip_empty: bool,
        reporting: &mut Reporting<'_>,
    ) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        let limit = NonZeroUsize::new(self.options.max_header_length).expect("validated limit");
        // Positions cannot exceed the validated usize input budget. Keep that
        // budget across groups rather than granting each recipient a new one.
        let mut remaining = self.options.max_input_bytes - self.position.offset as usize;
        loop {
            let position = self.position;
            let mut parser = HeaderParser::with_max_header_length(limit);
            parser.set_stream_offset(position.offset);
            let mut headers = Vec::new();
            let mut eof = false;
            while !parser.is_finished() {
                let input = match self.source.fill_buf() {
                    Ok(input) => input,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                if input.is_empty() {
                    // EOF can follow a complete header or interrupt its value.
                    // Keep the raw record without inventing a body separator.
                    parser
                        .finish(&mut headers)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    eof = true;
                    break;
                }
                if remaining == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MIME input byte limit exceeded",
                    ));
                }
                // Consume only the parsed prefix, not buffered read-ahead: callers
                // must still be able to read the body from a non-seekable source.
                let consumed = parser
                    .parse(&input[..input.len().min(remaining)], &mut headers)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                self.position = self.position.advance(&input[..consumed]);
                self.source.consume(consumed);
                remaining -= consumed;
            }
            if skip_empty && headers.is_empty() {
                // Status groups are separated by one or more blank lines.
                // EOF while skipping them means there are no more groups.
                // A lone trailing CR also produces no headers, as in
                // MimeKit's SkipBlankLines/ParseStatusGroup combination.
                if eof {
                    return Ok(true);
                }
                continue;
            }
            reporting.headers(
                &headers,
                position,
                self.position,
                parser.body_separator().into(),
                false,
                true,
            )?;
            callback(MimeEvent::Headers {
                headers: HeaderList::from(headers),
                position,
                headers_end: self.position,
                body_separator: parser.body_separator(),
            })?;
            return Ok(eof);
        }
    }

    /// Scans an entity through EOF, emitting structural and content events.
    ///
    /// # Errors
    ///
    /// Returns invalid initial-header, limit, or source errors. Interrupted
    /// source reads retry. Callback errors propagate unchanged and stop scanning;
    /// neither source consumption nor callback side effects are rolled back.
    pub fn read_entity<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read(false, &mut callback, None)
    }

    /// Scans a message through EOF, enclosing its root in message events.
    ///
    /// Otherwise follows [`Self::read_entity`], including its errors.
    pub fn read_message<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read(true, &mut callback, None)
    }

    /// Reads an entity with the reporting and error semantics of
    /// [`Self::read_message_with_logger`], without required message-header checks.
    pub fn read_entity_with_logger<F>(
        &mut self,
        logger: &mut dyn MimeComplianceLogger,
        mut callback: F,
    ) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read(false, &mut callback, Some(logger))
    }

    /// Reads a message with a borrowed, synchronous compliance logger.
    ///
    /// Reports header syntax and line format, repeated and missing message fields,
    /// MIME metadata, framing/depth failures, and body line format/null/high bytes.
    /// Headers are checked after their block completes, before entity-begin.
    /// Body checks are incremental and do not retain content. Base64,
    /// quoted-printable, and UU validators run by default; select them with
    /// [`MimeComplianceOptions::enabled_validators`]. Deep address validation
    /// is not yet implemented.
    ///
    /// Encoded bytes are checked without decoding or repairing them. Validators
    /// finish before the part's end event; they inspect the newline preceding a
    /// boundary, but not the boundary marker itself.
    ///
    /// Accepting a diagnostic preserves the same recovery as an unlogged read.
    /// Return an error from the logger to reject selected violations. The error
    /// propagates unchanged, stops callbacks and poisons the source just like a
    /// content-callback error. Resource and source errors remain fatal.
    ///
    /// Without a logger, ordinary read methods skip all compliance checks and
    /// retain the fused SIMD scanning path. A logged read performs additional
    /// inspection; it does not promise the same throughput.
    ///
    /// ```
    /// use mimecraft::{MimeComplianceIssue, MimeComplianceViolation, MimeReader};
    /// use std::io;
    ///
    /// let mut reader = MimeReader::new(b"Subject: hello\r\n\r\nbody".as_slice());
    /// reader.read_message_with_logger(&mut |issue: MimeComplianceIssue| {
    ///     if issue.violation == MimeComplianceViolation::RepeatedContentType {
    ///         return Err(io::Error::new(io::ErrorKind::InvalidData, issue.to_string()));
    ///     }
    ///     Ok(())
    /// }, |_| Ok(()))?;
    /// # Ok::<(), io::Error>(())
    /// ```
    pub fn read_message_with_logger<F>(
        &mut self,
        logger: &mut dyn MimeComplianceLogger,
        mut callback: F,
    ) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read(true, &mut callback, Some(logger))
    }

    /// Reads one message from an explicitly selected Unix mbox stream.
    ///
    /// Returns true for a message, false at EOF (including an empty archive).
    /// Calls may be repeated until EOF; other operation modes cannot be mixed.
    /// Emits one [`MimeEvent::MboxMarker`] followed by the normal message/tree
    /// events. Only five bytes of the next marker are consumed as lookahead;
    /// use [`Self::pending_input`] or [`Self::into_parts`] when recovering a
    /// partially read source. Byte/line positions remain archive-relative.
    ///
    /// Like MimeKit, a line beginning with exactly `From ` marks a message;
    /// its date/sender is not validated. Initial non-marker lines are skipped.
    /// A missing or unterminated marker is an error. `>From ` is ordinary data
    /// and is not unescaped. Content-Length-based mbox variants are not supported.
    /// The newline preceding a next marker is framing, not message content;
    /// at actual EOF all trailing content bytes are preserved.
    ///
    /// The input budget resets per message and includes its marker, framing,
    /// and initial skipped data, but not the next marker's lookahead. Marker
    /// lines are bounded by `max_header_length`, including their line ending.
    /// Depth and header limits apply normally. Truncated headers and containers
    /// recover at EOF or the next message's marker. Consecutive markers recover
    /// an empty message between them; a marker followed immediately by EOF
    /// without headers or a separator remains an error.
    ///
    /// # Errors
    ///
    /// Invalid initial-header, limit, source, and callback errors propagate
    /// as for [`Self::read_entity`], without resynchronizing or returning a
    /// partial message. Failures prevent further reads until source replacement.
    pub fn read_mbox_message<F>(&mut self, mut callback: F) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_mbox_message_impl(&mut callback, None)
    }

    /// Reads one mbox message with the reporting of
    /// [`Self::read_message_with_logger`]. Positions are archive-relative;
    /// reporting budgets reset for each message, not each source.
    pub fn read_mbox_message_with_logger<F>(
        &mut self,
        logger: &mut dyn MimeComplianceLogger,
        mut callback: F,
    ) -> io::Result<bool>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.read_mbox_message_impl(&mut callback, Some(logger))
    }

    pub(crate) fn read_mbox_message_impl<F>(
        &mut self,
        callback: &mut F,
        logger: Option<&mut dyn MimeComplianceLogger>,
    ) -> io::Result<bool>
    where
        F: EventSink,
    {
        if self.state == ReadState::MboxEndOfStream {
            return Ok(false);
        }
        self.begin_read(ReadState::Mbox)?;
        let Some(remaining) = engine::read_mbox_marker(
            &mut self.source,
            &self.options,
            &mut self.position,
            &mut self.buffer,
            callback,
        )?
        else {
            self.state = ReadState::MboxEndOfStream;
            return Ok(false);
        };
        callback.event(MimeEvent::MessageBegin {
            position: self.position,
        })?;
        let mut context = ReadContext {
            limits: self.options,
            reporting: Reporting::new(logger, self.compliance_options, true),
            message: true,
        };
        let end = engine::read_tree(
            &mut self.source,
            &mut context,
            &mut self.position,
            &mut self.buffer,
            true,
            remaining,
            callback,
        )?;
        callback.event(MimeEvent::MessageEnd {
            position: end.position,
        })?;
        self.state = if end.eof {
            ReadState::MboxEndOfStream
        } else {
            ReadState::Mbox
        };
        Ok(true)
    }

    fn begin_read(&mut self, continuation: ReadState) -> io::Result<()> {
        if !matches!(self.state, ReadState::Ready) && self.state != continuation {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MIME reader has already been used",
            ));
        }
        self.state = ReadState::Used;
        Ok(())
    }

    pub(crate) fn read<F>(
        &mut self,
        message: bool,
        callback: &mut F,
        logger: Option<&mut dyn MimeComplianceLogger>,
    ) -> io::Result<()>
    where
        F: EventSink,
    {
        self.begin_read(ReadState::Ready)?;
        if message {
            callback.event(MimeEvent::MessageBegin {
                position: self.position,
            })?;
        }
        let mut context = ReadContext {
            limits: self.options,
            reporting: Reporting::new(logger, self.compliance_options, message),
            message,
        };
        engine::read_tree(
            &mut self.source,
            &mut context,
            &mut self.position,
            &mut self.buffer,
            false,
            self.options.max_input_bytes,
            callback,
        )?;
        if message {
            callback.event(MimeEvent::MessageEnd {
                position: self.position,
            })?;
        }
        self.state = ReadState::EndOfStream;
        Ok(())
    }
}

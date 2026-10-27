// Adapted from MimeKit's MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, BufReader, Cursor, Read, Seek};
use std::ops::Range;

use crate::{
    ContentEncoding, Header, HeaderList, MessagePart, MimeContent, MimeContentKind,
    MimeContentStorage, MimeContentStream, MimeEntity, MimeEntityKind, MimeEntityParseOptions,
    MimeEvent, MimeMessage, MimePart, MimePosition, MimeReader, Multipart,
    io::{BoundedStream, SharedStream},
};

/// Constructs owned MIME models from the synchronous [`MimeReader`] events.
///
/// Leaf content is stored undecoded at position zero, in memory by default.
/// [`Self::with_storage`] can instead retain bounded seekable-source ranges.
/// Message roots
/// partition Content-* headers onto their bodies; ordinary child entities keep
/// all their headers. Header offsets retain their original input origin.
///
/// Each installed source supports one standalone header block, or one entity
/// or message through EOF with default malformed-message recovery.
/// Alternatively, [`Self::parse_status_group`] incrementally reads a sequence
/// of delivery-status header groups on the same source.
/// [`Self::parse_mbox_message`] selects incremental mbox iteration instead.
/// Async parsing is not provided. Use [`MimeReader`] directly to process content
/// incrementally without constructing a model.
///
/// ```
/// use mimecraft::MimeParser;
///
/// let mut parser = MimeParser::new(b"Subject: example\r\n\r\nbody".as_slice());
/// let message = parser.parse_message()?;
/// assert!(message.headers().contains("Subject"));
/// assert_eq!(parser.position().offset, 24);
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct MimeParser<R, C = Cursor<Vec<u8>>> {
    reader: MimeReader<R>,
    factory: ContentFactory<C>,
    factory_for: fn(&R, MimeContentStorage) -> ContentFactory<C>,
}

impl<R: BufRead> MimeParser<R> {
    /// Creates a parser with default limits, without I/O.
    pub fn new(source: R) -> Self {
        Self {
            reader: MimeReader::new(source),
            factory: memory_factory(&(), MimeContentStorage::Memory),
            factory_for: memory_factory,
        }
    }

    /// Creates a parser with explicit resource limits, without I/O.
    ///
    /// # Errors
    ///
    /// Zero limits return `InvalidInput`.
    pub fn with_options(source: R, options: MimeEntityParseOptions) -> io::Result<Self> {
        Ok(Self {
            reader: MimeReader::with_options(source, options)?,
            factory: memory_factory(&(), MimeContentStorage::Memory),
            factory_for: memory_factory,
        })
    }
}

impl<'a> MimeParser<&'a [u8], MimeContentStream<Cursor<&'a [u8]>>> {
    /// Creates a parser that borrows an in-memory slice without an input buffer.
    ///
    /// Uses [`MimeContentStorage::Source`]: leaf bodies retain ranges in `source`
    /// without copying their bytes. Headers, multipart framing, and the mutable
    /// model remain owned. Bodies can be replaced with
    /// [`MimeContentStream::Memory`] without modifying the original slice.
    /// Use [`MimeParser::new`] with a slice instead to copy bodies into memory.
    ///
    /// Returned models remain readable after parser replacement or drop, but
    /// cannot outlive `source`. [`Self::replace_source`] installs another slice.
    /// Positions and body ranges are relative to the start of the supplied slice.
    /// Parsing and diagnostics follow the same rules as the streaming parser.
    ///
    /// ```
    /// use mimecraft::MimeParser;
    ///
    /// let input = b"Subject: example\r\n\r\nbody";
    /// let mut parser = MimeParser::from_slice(input);
    /// let mut message = parser.parse_message()?;
    /// drop(parser);
    /// let mut wire = Vec::new();
    /// message.write_to(&mut wire)?;
    /// assert_eq!(wire, input);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// Borrowed bodies cannot outlive the input:
    ///
    /// ```compile_fail
    /// use mimecraft::MimeParser;
    ///
    /// let mut message = {
    ///     let input = b"\r\nbody".to_vec();
    ///     MimeParser::from_slice(&input).parse_message().unwrap()
    /// };
    /// message.write_to(&mut Vec::new()).unwrap();
    /// ```
    pub fn from_slice(source: &'a [u8]) -> Self {
        Self {
            reader: MimeReader::new(source),
            factory: slice_factory(&source, MimeContentStorage::Source),
            factory_for: slice_factory,
        }
    }

    /// Creates a slice-backed parser with explicit resource limits.
    ///
    /// Ownership and storage follow [`Self::from_slice`].
    ///
    /// # Errors
    ///
    /// Zero limits return `InvalidInput`.
    pub fn from_slice_with_options(
        source: &'a [u8],
        options: MimeEntityParseOptions,
    ) -> io::Result<Self> {
        Ok(MimeParser {
            reader: MimeReader::with_options(source, options)?,
            factory: slice_factory(&source, MimeContentStorage::Source),
            factory_for: slice_factory,
        })
    }
}

impl MimeParser<()> {
    /// Creates a parser with explicit body storage and default limits.
    ///
    /// Accepts an owned or borrowed `Read + Seek` source at its current position.
    /// Both modes use a 4 KiB buffer; use the ordinary [`MimeParser::new`] constructor
    /// for nonseekable input and the existing in-memory body type.
    ///
    /// Source-backed bodies retain the input independently of the parser and
    /// remain readable after source replacement or parser drop. Each body and
    /// the parser have independent cursors, including buffered mbox read-ahead.
    /// Offsets reported by the parser are relative to the initial input position;
    /// body ranges refer to absolute positions in the backing source.
    /// The return type carries `S`'s lifetime even in `Memory` mode; use
    /// [`MimeParser::new`] for copies independent of a borrowed source's lifetime.
    ///
    /// # Errors
    ///
    /// Propagates the initial source-position query failure.
    ///
    /// ```
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContentStorage, MimeParser};
    ///
    /// let input = Cursor::new(b"Subject: example\r\n\r\nbody".to_vec());
    /// let mut parser = MimeParser::with_storage(input, MimeContentStorage::Source)?;
    /// let mut message = parser.parse_message()?;
    /// drop(parser);
    /// let mut wire = Vec::new();
    /// message.write_to(&mut wire)?;
    /// assert_eq!(wire, b"Subject: example\r\n\r\nbody");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// Borrowed content cannot outlive its source:
    ///
    /// ```compile_fail
    /// use std::io::Cursor;
    /// use mimecraft::{MimeContentStorage, MimeParser};
    ///
    /// let mut message = {
    ///     let mut input = Cursor::new(b"\r\nbody");
    ///     let mut parser = MimeParser::with_storage(&mut input, MimeContentStorage::Source).unwrap();
    ///     parser.parse_message().unwrap()
    /// };
    /// message.write_to(&mut Vec::new()).unwrap();
    /// ```
    ///
    /// Nonseekable sources cannot select source storage (there is no fallback):
    ///
    /// ```compile_fail
    /// use mimecraft::{MimeContentStorage, MimeParser};
    ///
    /// let parser = MimeParser::with_storage(b"\r\nbody".as_slice(), MimeContentStorage::Source);
    /// ```
    pub fn with_storage<S: Read + Seek>(
        source: S,
        storage: MimeContentStorage,
    ) -> io::Result<MimeParser<BufReader<SharedStream<S>>, MimeContentStream<S>>> {
        Self::with_storage_and_options(source, storage, MimeEntityParseOptions::default())
    }

    /// Creates a seekable-source parser with explicit storage and limits.
    ///
    /// Validates limits before querying the source position. Ownership,
    /// buffering, and source errors follow [`Self::with_storage`].
    pub fn with_storage_and_options<S: Read + Seek>(
        source: S,
        storage: MimeContentStorage,
        options: MimeEntityParseOptions,
    ) -> io::Result<MimeParser<BufReader<SharedStream<S>>, MimeContentStream<S>>> {
        options.validate()?;
        let source = BufReader::with_capacity(4096, SharedStream::new(source)?);
        Ok(MimeParser {
            factory: source_factory(&source, storage),
            factory_for: source_factory,
            reader: MimeReader::with_options(source, options)?,
        })
    }
}

impl<S: Read + Seek> MimeParser<BufReader<SharedStream<S>>, MimeContentStream<S>> {
    /// Replaces the backing source, retaining the selected storage and options.
    ///
    /// The old source and buffered read-ahead are returned. Previously parsed
    /// bodies keep the old source; new bodies reference the replacement.
    /// Queries the new source position before changing parser state; a failed
    /// query leaves the parser unchanged. Save pending mbox lookahead separately.
    pub fn replace_storage_source(&mut self, source: S) -> io::Result<BufReader<SharedStream<S>>> {
        let source = BufReader::with_capacity(4096, SharedStream::new(source)?);
        Ok(self.replace_source(source))
    }
}

impl<R: BufRead, C: Read + Seek> MimeParser<R, C> {
    /// Returns the selected body storage.
    pub fn storage(&self) -> MimeContentStorage {
        self.factory.storage
    }

    /// Returns the supplied options.
    pub fn options(&self) -> &MimeEntityParseOptions {
        self.reader.options()
    }

    /// Returns settings used by the `*_with_logger` methods.
    pub fn compliance_options(&self) -> &crate::MimeComplianceOptions {
        self.reader.compliance_options()
    }

    /// Sets reporting settings for subsequent operations, without changing parsing.
    pub fn set_compliance_options(&mut self, options: crate::MimeComplianceOptions) {
        self.reader.set_compliance_options(options);
    }

    /// Parses headers with a borrowed, synchronous compliance logger.
    ///
    /// Follows [`MimeReader::read_headers_with_logger`]; logger errors propagate
    /// unchanged and no partial result is returned.
    pub fn parse_headers_with_logger(
        &mut self,
        logger: &mut dyn crate::MimeComplianceLogger,
    ) -> io::Result<HeaderList> {
        let mut parsed = None;
        self.reader.read_headers_with_logger(logger, |event| {
            let MimeEvent::Headers { headers, .. } = event else {
                unreachable!("standalone reading emits headers");
            };
            parsed = Some(headers);
            Ok(())
        })?;
        Ok(parsed.expect("successful header reading emits one block"))
    }

    /// Parses the next status group with fallible compliance reporting.
    ///
    /// Follows [`MimeReader::read_status_group_with_logger`], including
    /// per-operation budgets and unchanged logger errors.
    pub fn parse_status_group_with_logger(
        &mut self,
        logger: &mut dyn crate::MimeComplianceLogger,
    ) -> io::Result<Option<HeaderList>> {
        let mut parsed = None;
        self.reader.read_status_group_with_logger(logger, |event| {
            let MimeEvent::Headers { headers, .. } = event else {
                unreachable!("status reading emits headers");
            };
            parsed = Some(headers);
            Ok(())
        })?;
        Ok(parsed)
    }

    /// Parses an entity with fallible compliance reporting.
    ///
    /// Follows [`MimeReader::read_entity_with_logger`]. Logger errors propagate
    /// unchanged, poison the installed source, and discard the partial tree.
    pub fn parse_entity_with_logger(
        &mut self,
        logger: &mut dyn crate::MimeComplianceLogger,
    ) -> io::Result<MimeEntity<C>> {
        let mut builder = Builder::new(&self.factory);
        self.reader.read(false, &mut builder, Some(logger))?;
        Ok(builder
            .entity
            .expect("successful entity events produce a root"))
    }

    /// Parses a message with fallible compliance reporting.
    ///
    /// Follows [`MimeReader::read_message_with_logger`], including its current
    /// diagnostic coverage. No strict mode is introduced; callers may reject
    /// selected issues by returning an error from the logger.
    pub fn parse_message_with_logger(
        &mut self,
        logger: &mut dyn crate::MimeComplianceLogger,
    ) -> io::Result<MimeMessage<C>> {
        let mut builder = Builder::new(&self.factory);
        self.reader.read(true, &mut builder, Some(logger))?;
        Ok(builder
            .message
            .expect("successful message events produce a root"))
    }

    /// Parses one mbox message with a fresh reporting budget, or returns `None`.
    ///
    /// Follows [`MimeReader::read_mbox_message_with_logger`]. Previously returned
    /// messages survive a logger failure; no partial current message is returned.
    pub fn parse_mbox_message_with_logger(
        &mut self,
        logger: &mut dyn crate::MimeComplianceLogger,
    ) -> io::Result<Option<MimeMessage<C>>> {
        let mut builder = Builder::new(&self.factory);
        if self
            .reader
            .read_mbox_message_impl(&mut builder, Some(logger))?
        {
            Ok(Some(
                builder
                    .message
                    .expect("successful message events produce a root"),
            ))
        } else {
            Ok(None)
        }
    }

    /// Returns the consumed original-input position.
    pub fn position(&self) -> MimePosition {
        self.reader.position()
    }

    /// Returns whether a successful parse finished at an observed EOF.
    ///
    /// Delegates to [`MimeReader::is_end_of_stream`] without performing I/O.
    /// Header parsing stops at a separator without probing the remaining input;
    /// false does not necessarily mean more bytes remain.
    pub fn is_end_of_stream(&self) -> bool {
        self.reader.is_end_of_stream()
    }

    /// Installs a new source, returning the previous buffered source.
    ///
    /// Follows [`MimeReader::replace_source`]: preserves options, resets
    /// positions and completion/failure state, and performs no source I/O.
    /// Save [`Self::pending_input`] first if mbox prefix lookahead is needed.
    /// Previously returned models keep their copied content or shared old source.
    ///
    /// ```
    /// use mimecraft::MimeParser;
    ///
    /// let mut parser = MimeParser::new(b"Subject: first\r\n\r\nbody".as_slice());
    /// let headers = parser.parse_headers()?;
    /// let old_source = parser.replace_source(b"Subject: second\r\n\r\nnext".as_slice());
    /// assert_eq!(old_source, b"body");
    /// assert_eq!(parser.position().offset, 0);
    /// assert!(!parser.is_end_of_stream());
    /// let message = parser.parse_message()?;
    /// assert!(parser.is_end_of_stream());
    /// assert_eq!(message.headers().get("Subject").unwrap().raw_value(), b" second\r\n");
    /// assert_eq!(headers.get("Subject").unwrap().raw_value(), b" first\r\n");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn replace_source(&mut self, source: R) -> R {
        self.factory = (self.factory_for)(&source, self.factory.storage);
        self.reader.replace_source(source)
    }

    /// Borrows the underlying buffered source.
    pub fn get_ref(&self) -> &R {
        self.reader.get_ref()
    }

    /// Returns the source with its buffered read-ahead intact.
    ///
    /// This discards reader-owned mbox prefix lookahead; [`Self::into_parts`]
    /// recovers that separately for replay before the source.
    pub fn into_inner(self) -> R {
        self.reader.into_inner()
    }

    /// Parses raw headers through the separator or EOF without consuming the body.
    ///
    /// Retains all fields in wire order, including Content-* headers, malformed
    /// records, and original byte offsets. Empty input yields an empty list.
    /// Does not interpret MIME metadata or require a header/body separator.
    /// The separator is consumed but not stored in [`HeaderList`]; use
    /// [`MimeReader::read_headers`] to inspect it.
    ///
    /// # Errors
    ///
    /// Follows [`MimeReader::read_headers`]'s limits and errors. Partial results
    /// are discarded on failure. Each installed source supports only one
    /// operation, even when reading just headers or when that operation fails.
    ///
    /// ```
    /// use mimecraft::MimeParser;
    ///
    /// let mut parser = MimeParser::new(b"Subject: example\r\n\r\nbody".as_slice());
    /// let headers = parser.parse_headers()?;
    /// assert!(headers.contains("Subject"));
    /// assert_eq!(parser.position().offset, 20);
    /// assert_eq!(parser.into_inner(), b"body");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn parse_headers(&mut self) -> io::Result<HeaderList> {
        let mut parsed = None;
        self.reader.read_headers(|event| {
            let MimeEvent::Headers { headers, .. } = event else {
                unreachable!("standalone header reading emits a header block");
            };
            // Like MimeKit's ParseHeaders, retain the entire ordered block;
            // Content-* partitioning belongs to message construction instead.
            parsed = Some(headers);
            Ok(())
        })?;
        Ok(parsed.expect("successful header reading emits one block"))
    }

    /// Parses the next nonempty delivery-status header group, or `None` at EOF.
    ///
    /// Selects incremental group parsing for this source. Blank lines and a lone
    /// trailing CR do not create empty groups. Retains malformed records, folds,
    /// duplicates, and byte offsets relative to the entire supplied input.
    /// The input budget applies across all groups, not separately to each call;
    /// the header-length limit still applies to individual folded records.
    ///
    /// Does not decode content or interpret group fields. Recover the buffered
    /// source with [`Self::into_inner`] to process any encoded remainder.
    /// Repeated calls after group EOF return `None` without I/O.
    ///
    /// # Errors
    ///
    /// Follows [`MimeReader::read_status_group`]. Failure returns no partial
    /// current group; previously returned groups remain valid. Other parse
    /// operations cannot be mixed with this mode until [`Self::replace_source`].
    ///
    /// ```
    /// use mimecraft::MimeParser;
    ///
    /// let input = b"\r\nReporting-MTA: dns; example.com\r\n\r\n\
    ///               Final-Recipient: rfc822; user@example.com\r\n\r\n";
    /// let mut parser = MimeParser::new(input.as_slice());
    /// assert!(parser.parse_status_group()?.unwrap().contains("Reporting-MTA"));
    /// assert!(parser.parse_status_group()?.unwrap().contains("Final-Recipient"));
    /// assert!(parser.parse_status_group()?.is_none());
    /// assert!(parser.is_end_of_stream());
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn parse_status_group(&mut self) -> io::Result<Option<HeaderList>> {
        let mut parsed = None;
        self.reader.read_status_group(|event| {
            let MimeEvent::Headers { headers, .. } = event else {
                unreachable!("status-group reading emits only headers");
            };
            parsed = Some(headers);
            Ok(())
        })?;
        Ok(parsed)
    }

    /// Parses an entity, retaining stored body bytes without transfer decoding.
    ///
    /// # Errors
    ///
    /// Propagates reader errors without returning a partial tree. A second parse
    /// attempt returns `InvalidInput`, even if the first attempt failed.
    pub fn parse_entity(&mut self) -> io::Result<MimeEntity<C>> {
        let mut builder = Builder::new(&self.factory);
        self.reader.read(false, &mut builder, None)?;
        Ok(builder
            .entity
            .expect("successful entity events produce a root"))
    }

    /// Parses a message, separating its headers from its root body's headers.
    ///
    /// Follows [`Self::parse_entity`]'s ownership, limits, and error behavior.
    pub fn parse_message(&mut self) -> io::Result<MimeMessage<C>> {
        let mut builder = Builder::new(&self.factory);
        self.reader.read(true, &mut builder, None)?;
        Ok(builder
            .message
            .expect("successful message events produce a root"))
    }

    /// Parses one mbox message, or `None` at the end of the archive.
    ///
    /// Selects mbox iteration for this source. Follows
    /// [`MimeReader::read_mbox_message`]'s marker recognition, per-message limits,
    /// recovery, archive-relative offsets, and failure lifecycle.
    /// Envelope markers are not stored in the model; use the reader's marker
    /// events to retain them. Previously returned messages remain independent.
    /// Enable [`MimeEntityParseOptions::respect_content_length`] to protect
    /// body `From ` lines using the outer message's declared length.
    ///
    /// ```
    /// use mimecraft::MimeParser;
    ///
    /// let archive = b"From sender@example.com Sat Oct 24 12:00:00 2026\n\
    ///                 Subject: example\n\nbody\n";
    /// let mut parser = MimeParser::new(archive.as_slice());
    /// assert!(parser.parse_mbox_message()?.is_some());
    /// assert!(parser.parse_mbox_message()?.is_none());
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn parse_mbox_message(&mut self) -> io::Result<Option<MimeMessage<C>>> {
        let mut builder = Builder::new(&self.factory);
        if self.reader.read_mbox_message_impl(&mut builder, None)? {
            Ok(Some(
                builder
                    .message
                    .expect("successful message events produce a root"),
            ))
        } else {
            Ok(None)
        }
    }

    /// Returns next-marker lookahead; see [`MimeReader::pending_input`].
    pub fn pending_input(&self) -> &[u8] {
        self.reader.pending_input()
    }

    /// Returns the source and lookahead to replay; see [`MimeReader::into_parts`].
    pub fn into_parts(self) -> (R, Vec<u8>) {
        self.reader.into_parts()
    }
}

struct ContentFactory<C> {
    // A prototype and constructor keep the builder generic without changing
    // the default Cursor body type or requiring a public storage-strategy trait.
    seed: C,
    storage: MimeContentStorage,
    create: fn(&C, Vec<u8>, Range<u64>) -> io::Result<C>,
}

fn memory_factory<R>(_: &R, _: MimeContentStorage) -> ContentFactory<Cursor<Vec<u8>>> {
    ContentFactory {
        seed: Cursor::new(Vec::new()),
        storage: MimeContentStorage::Memory,
        create: |_, bytes, _| Ok(Cursor::new(bytes)),
    }
}

fn source_factory<R: Read + Seek>(
    source: &BufReader<SharedStream<R>>,
    storage: MimeContentStorage,
) -> ContentFactory<MimeContentStream<R>> {
    // Replacement may already have unread buffered input. The reader starts
    // there, not at the backing handle's post-refill position.
    let origin = source.get_ref().position() - source.buffer().len() as u64;
    ContentFactory {
        seed: match storage {
            MimeContentStorage::Memory => MimeContentStream::Memory(Cursor::new(Vec::new())),
            MimeContentStorage::Source => MimeContentStream::Source(
                BoundedStream::new(source.get_ref().clone(), origin, Some(origin))
                    .expect("empty range is valid"),
            ),
        },
        storage,
        create: create_source_content,
    }
}

fn slice_factory<'a>(
    source: &&'a [u8],
    _: MimeContentStorage,
) -> ContentFactory<MimeContentStream<Cursor<&'a [u8]>>> {
    let source = SharedStream::new(Cursor::new(*source)).expect("slice cursor position is valid");
    ContentFactory {
        seed: MimeContentStream::Source(
            BoundedStream::new(source, 0, Some(0)).expect("empty range is valid"),
        ),
        storage: MimeContentStorage::Source,
        create: create_source_content,
    }
}

fn create_source_content<R: Read + Seek>(
    seed: &MimeContentStream<R>,
    bytes: Vec<u8>,
    range: Range<u64>,
) -> io::Result<MimeContentStream<R>> {
    match seed {
        MimeContentStream::Memory(_) => Ok(MimeContentStream::Memory(Cursor::new(bytes))),
        MimeContentStream::Source(source) => {
            let origin = source.start_boundary();
            let start = origin.checked_add(range.start);
            let end = origin.checked_add(range.end);
            let (Some(start), Some(end)) = (start, end) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "body source offset overflow",
                ));
            };
            Ok(MimeContentStream::Source(BoundedStream::new(
                source.get_ref().clone(),
                start,
                Some(end),
            )?))
        }
    }
}

struct Builder<'a, C> {
    factory: &'a ContentFactory<C>,
    stack: Vec<Frame<C>>,
    entity: Option<MimeEntity<C>>,
    message: Option<MimeMessage<C>>,
    body_separator: bool,
    end_boundary: Option<MimePosition>,
}

impl<'a, C> Builder<'a, C> {
    fn new(factory: &'a ContentFactory<C>) -> Self {
        Self {
            factory,
            stack: Vec::new(),
            entity: None,
            message: None,
            body_separator: false,
            end_boundary: None,
        }
    }
}

enum Frame<C> {
    Entity {
        headers: HeaderList,
        content: Content<C>,
        encoding: ContentEncoding,
        parsed: crate::mime_entity::ParsedEntity,
        headers_end: MimePosition,
    },
    Message {
        headers: HeaderList,
        body: Option<MimeEntity<C>>,
    },
}

enum Content<C> {
    Part {
        bytes: Vec<u8>,
        length: u64,
    },
    Multipart {
        children: Vec<MimeEntity<C>>,
        preamble: Vec<u8>,
        epilogue: Vec<u8>,
        framing: crate::multipart::ParsedMultipart,
        boundary_end: u64,
    },
    MessagePart(Option<MimeMessage<C>>),
}

impl<C: Read + Seek> crate::mime_reader::EventSink for Builder<'_, C> {
    fn entity_begin(
        &mut self,
        headers: Vec<Header>,
        kind: MimeEntityKind,
        encoding: ContentEncoding,
        _position: MimePosition,
        headers_end: MimePosition,
    ) -> io::Result<()> {
        let headers = if let Some(Frame::Message {
            headers: message_headers,
            ..
        }) = self.stack.last_mut()
        {
            // Like MimeKit's message and entity constructors, count Content-*
            // headers so both final HeaderLists have the correct capacity.
            // Ordinary child entities retain every header without partitioning.
            let content_count = headers
                .iter()
                .filter(|header| header.is_content_header())
                .count();
            *message_headers = HeaderList::with_capacity(headers.len() - content_count);
            let mut content_headers = HeaderList::with_capacity(content_count);
            for header in headers {
                if header.is_content_header() {
                    content_headers.add(header);
                } else {
                    message_headers.add(header);
                }
            }
            content_headers
        } else {
            HeaderList::from(headers)
        };
        self.stack.push(Frame::Entity {
            headers,
            headers_end,
            encoding,
            parsed: crate::mime_entity::ParsedEntity {
                body_separator: self.body_separator,
                no_content: false,
            },
            content: match kind {
                MimeEntityKind::Part => Content::Part {
                    bytes: Vec::new(),
                    length: 0,
                },
                MimeEntityKind::Multipart => Content::Multipart {
                    children: Vec::new(),
                    preamble: Vec::new(),
                    epilogue: Vec::new(),
                    framing: Default::default(),
                    boundary_end: 0,
                },
                MimeEntityKind::MessagePart => Content::MessagePart(None),
            },
        });
        Ok(())
    }

    fn end_boundary(&mut self, position: Option<MimePosition>) {
        self.end_boundary = position;
    }

    fn body_separator(&mut self, present: bool) {
        self.body_separator = present;
    }

    fn multipart_boundary(&mut self, boundary: Option<String>) {
        let Some(Frame::Entity {
            content: Content::Multipart { framing, .. },
            ..
        }) = self.stack.last_mut()
        else {
            unreachable!("decoded boundary belongs to the current multipart")
        };
        // The reader no longer needs this boundary once the multipart ends.
        // Move it into the saved framing instead of parsing its header again.
        framing.boundary = boundary;
    }

    fn boundary(&mut self, bytes: &[u8], closing: bool, end: MimePosition) {
        let Some(Frame::Entity {
            content:
                Content::Multipart {
                    framing,
                    boundary_end,
                    ..
                },
            ..
        }) = self.stack.last_mut()
        else {
            unreachable!("boundary belongs to the current multipart")
        };
        if closing {
            framing.end = Some(bytes.to_vec());
            *boundary_end = end.offset;
        } else {
            framing.boundaries.push(bytes.to_vec());
        }
    }

    fn event(&mut self, event: MimeEvent<'_>) -> io::Result<()> {
        match event {
            MimeEvent::MboxMarker { .. } => {}
            MimeEvent::Headers { .. } | MimeEvent::EntityBegin { .. } => {
                unreachable!("tree construction receives plain entity headers");
            }
            MimeEvent::MessageBegin { .. } => self.stack.push(Frame::Message {
                headers: HeaderList::new(),
                body: None,
            }),
            MimeEvent::Content {
                kind,
                bytes,
                position,
            } => {
                let Some(Frame::Entity {
                    content,
                    headers_end,
                    ..
                }) = self.stack.last_mut()
                else {
                    unreachable!("content events belong to an entity");
                };
                // The reader already excludes the newline owned by a following
                // delimiter. Copy stored bytes without decoding or normalizing.
                match (kind, content) {
                    (
                        MimeContentKind::Body,
                        Content::Part {
                            bytes: body,
                            length,
                        },
                    ) => {
                        assert_eq!(
                            position.offset,
                            headers_end.offset + *length,
                            "body ranges are contiguous"
                        );
                        *length += bytes.len() as u64;
                        // MimeKit keeps no memory stream in Source mode. The
                        // reader still scans content, but only its range is kept.
                        if self.factory.storage == MimeContentStorage::Memory {
                            body.extend_from_slice(bytes);
                        }
                    }
                    (MimeContentKind::Preamble, Content::Multipart { preamble, .. }) => {
                        preamble.extend_from_slice(bytes)
                    }
                    (MimeContentKind::Epilogue, Content::Multipart { epilogue, .. }) => {
                        epilogue.extend_from_slice(bytes)
                    }
                    _ => unreachable!("content section matches its enclosing entity"),
                }
            }
            MimeEvent::EntityEnd {
                kind,
                new_line_format,
                position,
                ..
            } => {
                let Some(Frame::Entity {
                    headers,
                    content,
                    encoding,
                    mut parsed,
                    headers_end,
                }) = self.stack.pop()
                else {
                    unreachable!("entity end matches an entity frame");
                };
                // A successfully parsed encoding is never Unspecified. With a
                // header present, this is the reader's recovery from an invalid
                // value. Multipart framing retains its raw encoding separately.
                let recovered_encoding = if kind != MimeEntityKind::Multipart
                    && encoding == ContentEncoding::Unspecified
                {
                    headers
                        .get("content-transfer-encoding")
                        .map(|header| header.raw_value().to_vec())
                } else {
                    None
                };
                let mut entity: MimeEntity<C> = match (kind, content) {
                    (MimeEntityKind::Part, Content::Part { bytes, length }) => {
                        // With no body, the header block's final newline also
                        // precedes the next boundary. Rust retains an empty
                        // stream rather than MimeKit's null Content.
                        parsed.no_content = length == 0
                            && new_line_format.is_none()
                            && self
                                .end_boundary
                                .is_none_or(|boundary| boundary.offset <= headers_end.offset);
                        // MimeReader passes only bytes within the content range,
                        // excluding delimiter-owned newlines. At an empty body,
                        // its header newline can also belong to the delimiter.
                        let end = headers_end.offset.checked_add(length).ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidData, "body end offset overflow")
                        })?;
                        let source = (self.factory.create)(
                            &self.factory.seed,
                            bytes,
                            headers_end.offset..end,
                        )?;
                        let mut body = MimeContent::with_encoding(source, encoding);
                        body.set_new_line_format(new_line_format);
                        let mut part = MimePart::from_parts(headers, body);
                        part.recovered_encoding = recovered_encoding;
                        part.into()
                    }
                    (
                        MimeEntityKind::Multipart,
                        Content::Multipart {
                            children,
                            preamble,
                            epilogue,
                            mut framing,
                            boundary_end,
                        },
                    ) => {
                        if let Some(end) = &mut framing.end {
                            // A closing marker's newline may also belong to the
                            // next enclosing delimiter, which writes it itself.
                            if boundary_end > position.offset {
                                let trim = (boundary_end - position.offset) as usize;
                                assert!(
                                    (trim == 1 && end.ends_with(b"\n"))
                                        || (trim == 2 && end.ends_with(b"\r\n")),
                                    "only a delimiter newline can overlap an enclosing marker"
                                );
                                end.truncate(end.len() - trim);
                            }
                        }
                        framing.content_type = headers
                            .get("content-type")
                            .expect("parsed multipart type")
                            .raw_value()
                            .to_vec();
                        framing.transfer_encoding = headers
                            .get("content-transfer-encoding")
                            .map(|header| header.raw_value().to_vec());
                        let mut multipart =
                            Multipart::from_parts(headers, children, preamble, epilogue);
                        multipart.framing = Some(framing);
                        multipart.into()
                    }
                    (MimeEntityKind::MessagePart, Content::MessagePart(message)) => {
                        parsed.no_content = message.is_none()
                            && self
                                .end_boundary
                                .is_none_or(|boundary| boundary.offset <= headers_end.offset);
                        let mut part = MessagePart::from_parts(headers, message);
                        part.recovered_encoding = recovered_encoding;
                        part.into()
                    }
                    _ => unreachable!("entity end matches its begin kind"),
                };
                entity.set_parsed(parsed);
                // Like MimeKit's construction stack, attach a completed entity
                // to its immediate multipart or message without recursive calls.
                match self.stack.last_mut() {
                    Some(Frame::Entity {
                        content: Content::Multipart { children, .. },
                        ..
                    }) => children.push(entity),
                    Some(Frame::Message { body, .. }) => *body = Some(entity),
                    None => self.entity = Some(entity),
                    _ => unreachable!("entity belongs to a multipart, message, or root"),
                }
            }
            MimeEvent::MessageEnd { .. } => {
                let Some(Frame::Message {
                    headers,
                    body: Some(body),
                }) = self.stack.pop()
                else {
                    unreachable!("a complete message contains its root entity");
                };
                let message = MimeMessage::from_parts(headers, Some(body));
                match self.stack.last_mut() {
                    Some(Frame::Entity {
                        content: Content::MessagePart(slot),
                        ..
                    }) => *slot = Some(message),
                    None => self.message = Some(message),
                    _ => unreachable!("message belongs to a message wrapper or root"),
                }
            }
        }
        Ok(())
    }
}

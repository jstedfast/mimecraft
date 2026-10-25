// Adapted from MimeKit's MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead, Cursor};

use crate::{
    ContentEncoding, HeaderList, MessagePart, MimeContent, MimeContentKind, MimeEntity,
    MimeEntityKind, MimeEntityParseOptions, MimeEvent, MimeMessage, MimePart, MimePosition,
    MimeReader, Multipart,
};

/// Constructs owned MIME models from the synchronous [`MimeReader`] events.
///
/// Leaf content is stored undecoded in memory at position zero. Message roots
/// partition Content-* headers onto their bodies; ordinary child entities keep
/// all their headers. Header offsets retain their original input origin.
///
/// Each installed source supports one standalone header block, or one entity
/// or message through EOF with default malformed-message recovery.
/// Alternatively, [`Self::parse_status_group`] incrementally reads a sequence
/// of delivery-status header groups on the same source.
/// [`Self::parse_mbox_message`] selects incremental mbox iteration instead.
/// It does not provide persistent-source bodies or async parsing. Models still buffer
/// content; use [`MimeReader`] directly to process content incrementally.
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
pub struct MimeParser<R> {
    reader: MimeReader<R>,
}

impl<R: BufRead> MimeParser<R> {
    /// Creates a parser with default limits, without I/O.
    pub fn new(source: R) -> Self {
        Self {
            reader: MimeReader::new(source),
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
        })
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
    ) -> io::Result<MimeEntity> {
        let mut builder = Builder::default();
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
    ) -> io::Result<MimeMessage> {
        let mut builder = Builder::default();
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
    ) -> io::Result<Option<MimeMessage>> {
        let mut builder = Builder::default();
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
    /// Previously returned models own their content and remain unchanged.
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
    pub fn parse_entity(&mut self) -> io::Result<MimeEntity> {
        let mut builder = Builder::default();
        self.reader.read(false, &mut builder, None)?;
        Ok(builder
            .entity
            .expect("successful entity events produce a root"))
    }

    /// Parses a message, separating its headers from its root body's headers.
    ///
    /// Follows [`Self::parse_entity`]'s ownership, limits, and error behavior.
    pub fn parse_message(&mut self) -> io::Result<MimeMessage> {
        let mut builder = Builder::default();
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
    pub fn parse_mbox_message(&mut self) -> io::Result<Option<MimeMessage>> {
        let mut builder = Builder::default();
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

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    entity: Option<MimeEntity>,
    message: Option<MimeMessage>,
    body_separator: bool,
    end_boundary: Option<MimePosition>,
}

enum Frame {
    Entity {
        headers: HeaderList,
        content: Content,
        parsed: crate::mime_entity::ParsedEntity,
        headers_end: MimePosition,
    },
    Message(Option<MimeEntity>),
}

enum Content {
    Part {
        encoding: ContentEncoding,
        bytes: Vec<u8>,
    },
    Multipart {
        children: Vec<MimeEntity>,
        preamble: Vec<u8>,
        epilogue: Vec<u8>,
        framing: crate::multipart::ParsedMultipart,
        boundary_end: u64,
    },
    MessagePart(Option<MimeMessage>),
}

impl crate::mime_reader::EventSink for Builder {
    fn end_boundary(&mut self, position: Option<MimePosition>) {
        self.end_boundary = position;
    }

    fn body_separator(&mut self, present: bool) {
        self.body_separator = present;
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
            MimeEvent::Headers { .. } => {
                unreachable!("tree construction does not read standalone headers");
            }
            MimeEvent::MessageBegin { .. } => self.stack.push(Frame::Message(None)),
            MimeEvent::EntityBegin {
                headers,
                kind,
                encoding,
                headers_end,
                ..
            } => {
                self.stack.push(Frame::Entity {
                    headers,
                    headers_end,
                    parsed: crate::mime_entity::ParsedEntity {
                        body_separator: self.body_separator,
                        no_content: false,
                    },
                    content: match kind {
                        MimeEntityKind::Part => Content::Part {
                            encoding,
                            bytes: Vec::new(),
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
            }
            MimeEvent::Content { kind, bytes, .. } => {
                let Some(Frame::Entity { content, .. }) = self.stack.last_mut() else {
                    unreachable!("content events belong to an entity");
                };
                // The reader already excludes the newline owned by a following
                // delimiter. Copy stored bytes without decoding or normalizing.
                match (kind, content) {
                    (MimeContentKind::Body, Content::Part { bytes: body, .. }) => {
                        body.extend_from_slice(bytes)
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
                    mut parsed,
                    headers_end,
                }) = self.stack.pop()
                else {
                    unreachable!("entity end matches an entity frame");
                };
                let mut entity: MimeEntity = match (kind, content) {
                    (MimeEntityKind::Part, Content::Part { bytes, encoding }) => {
                        // With no body, the header block's final newline also
                        // precedes the next boundary. Rust retains an empty
                        // stream rather than MimeKit's null Content.
                        parsed.no_content = bytes.is_empty()
                            && new_line_format.is_none()
                            && self
                                .end_boundary
                                .is_none_or(|boundary| boundary.offset <= headers_end.offset);
                        let mut body = MimeContent::with_encoding(Cursor::new(bytes), encoding);
                        body.set_new_line_format(new_line_format);
                        let recovered_encoding = crate::mime_part::transfer_encoding(&headers)
                            .is_err()
                            .then(|| {
                                headers
                                    .get("Content-Transfer-Encoding")
                                    .expect("invalid encoding header")
                                    .raw_value()
                                    .to_vec()
                            });
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
                        framing.boundary = headers.get("Content-Type").and_then(|header| {
                            crate::ContentType::parse_for_reader(header.raw_value())
                                .0
                                .boundary()
                                .map(str::to_owned)
                        });
                        framing.content_type = headers
                            .get("Content-Type")
                            .expect("parsed multipart type")
                            .raw_value()
                            .to_vec();
                        framing.transfer_encoding = headers
                            .get("Content-Transfer-Encoding")
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
                        MessagePart::from_parts(headers, message).into()
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
                    Some(Frame::Message(body)) => *body = Some(entity),
                    None => self.entity = Some(entity),
                    _ => unreachable!("entity belongs to a multipart, message, or root"),
                }
            }
            MimeEvent::MessageEnd { .. } => {
                let Some(Frame::Message(Some(body))) = self.stack.pop() else {
                    unreachable!("a complete message contains its root entity");
                };
                let message = MimeMessage::from_loaded_entity(body);
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

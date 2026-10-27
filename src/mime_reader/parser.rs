// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use super::{
    EventSink, MimeEntityParseOptions, MimeEvent, MimePosition,
    engine::{MarkerParser, ReadContext, ScanBuffer, TreeParser},
};
use crate::{MimeComplianceLogger, MimeComplianceOptions, mime_compliance::Reporting};

/// The framing interpreted by a byte-fed [`MimeEventParser`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeEventFormat {
    /// One MIME entity, without outer message events.
    Entity,
    /// One message, including outer message begin/end events.
    Message,
    /// A Unix mbox archive, with a marker and message events for each entry.
    Mbox,
}

enum State {
    Ready,
    Marker(MarkerParser),
    Tree(TreeParser),
    Finished,
    Failed,
}

/// Resumable MIME event parsing without reading, seeking, or awaiting I/O.
///
/// Feed arbitrary byte slices to [`parse`](Self::parse), then explicitly signal
/// EOF with [`finish`](Self::finish). Empty slices do not signal EOF. This is the
/// same scanner and tree state machine used by [`super::MimeReader`], including
/// malformed-input recovery, resource limits, mbox lookahead, and content-length
/// handling. Use [`crate::HeaderParser`] for standalone header blocks.
///
/// The parser retains headers, nesting state, and ambiguous lookahead, not
/// complete bodies. Ordinary content borrows the caller's input; ambiguous
/// delimiter lines may require buffering up to the configured input limit.
/// Content event slices are at most 4096 bytes and live only for the callback.
/// Chunk boundaries may change content event sizes, not content or positions.
///
/// All callbacks run synchronously. An async driver should await input outside
/// these calls and arrange bounded input chunks or owned event storage when
/// downstream processing needs to await. Pausing between calls preserves state;
/// a callback error is terminal, not a backpressure or retry signal.
///
/// ```
/// use mimecraft::{MimeContentKind, MimeEvent, MimeEventFormat, MimeEventParser};
///
/// let mut parser = MimeEventParser::new(MimeEventFormat::Message);
/// let mut body = Vec::new();
/// let mut collect = |event: MimeEvent<'_>| {
///     if let MimeEvent::Content { kind: MimeContentKind::Body, bytes, .. } = event {
///         body.extend_from_slice(bytes);
///     }
///     Ok(())
/// };
/// parser.parse(b"Subject: example\r", &mut collect)?;
/// parser.parse(b"\n\r\nhello", &mut collect)?;
/// parser.finish(&mut collect)?;
/// assert_eq!(body, b"hello");
/// assert!(parser.is_finished());
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct MimeEventParser<
    'a,
    L: MimeComplianceLogger + ?Sized = dyn MimeComplianceLogger + Send + 'a,
> {
    format: MimeEventFormat,
    context: ReadContext<'a, L>,
    position: MimePosition,
    buffer: ScanBuffer,
    state: State,
}

impl<'a> MimeEventParser<'a> {
    /// Creates a parser with default limits and no compliance reporting.
    pub fn new(format: MimeEventFormat) -> Self {
        Self::create(
            format,
            MimeEntityParseOptions::default(),
            MimeComplianceOptions::default(),
            None,
        )
    }

    /// Creates a parser with explicit limits and no compliance reporting.
    ///
    /// # Errors
    ///
    /// Zero limits return [`io::ErrorKind::InvalidInput`].
    pub fn with_options(
        format: MimeEventFormat,
        options: MimeEntityParseOptions,
    ) -> io::Result<Self> {
        options.validate()?;
        Ok(Self::create(
            format,
            options,
            MimeComplianceOptions::default(),
            None,
        ))
    }
}

impl<'a, L: MimeComplianceLogger + ?Sized> MimeEventParser<'a, L> {
    /// Creates a parser with a borrowed, fallible compliance logger.
    ///
    /// Reporting follows [`super::MimeReader::read_message_with_logger`].
    /// Entity mode omits required message-header checks; mbox mode resets
    /// reporting budgets for each message. The logger remains borrowed until
    /// the parser is dropped. It must not be used to await downstream work.
    /// The parser is `Send` when the logger is `Send`; local-only loggers are
    /// also accepted without imposing thread-safety on synchronous callers.
    ///
    /// # Errors
    ///
    /// Zero limits return [`io::ErrorKind::InvalidInput`].
    pub fn with_logger(
        format: MimeEventFormat,
        options: MimeEntityParseOptions,
        compliance: MimeComplianceOptions,
        logger: &'a mut L,
    ) -> io::Result<Self> {
        options.validate()?;
        Ok(Self::create(format, options, compliance, Some(logger)))
    }

    fn create(
        format: MimeEventFormat,
        limits: MimeEntityParseOptions,
        compliance: MimeComplianceOptions,
        logger: Option<&'a mut L>,
    ) -> Self {
        let message = format != MimeEventFormat::Entity;
        Self {
            format,
            context: ReadContext {
                limits,
                reporting: Reporting::new(logger, compliance, message),
                message,
                mbox_content_end: (format == MimeEventFormat::Mbox
                    && limits.respect_content_length)
                    .then_some(0),
            },
            position: MimePosition::default(),
            buffer: ScanBuffer::default(),
            state: State::Ready,
        }
    }

    /// Returns the configured limits and interpretation options.
    pub fn options(&self) -> &MimeEntityParseOptions {
        &self.context.limits
    }

    /// Returns the consumed byte/line position, including retained lookahead.
    pub fn position(&self) -> MimePosition {
        self.position
    }

    /// Returns whether explicit EOF was processed successfully.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Finished)
    }

    /// Consumes bytes and emits events without performing I/O.
    ///
    /// Success consumes the entire slice and returns its length. Empty input
    /// is a no-op, not EOF. Drop or reuse the input after the callbacks return.
    /// Continue with the same logical event consumer across calls.
    ///
    /// # Errors
    ///
    /// Parse, limit, logger, and callback errors stop parsing permanently;
    /// callback side effects and consumed bytes are not rolled back.
    /// Calls after failure or completion return [`io::ErrorKind::InvalidInput`].
    /// Error types use `io::Error` for compatibility with the synchronous
    /// adapter; this method itself never reads or writes a stream.
    pub fn parse<F>(&mut self, input: &[u8], mut callback: F) -> io::Result<usize>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        self.check_active()?;
        if !input.is_empty() {
            self.process(input, false, &mut callback)?;
        }
        Ok(input.len())
    }

    /// Signals actual EOF, flushing pending bytes and applying EOF recovery.
    ///
    /// Successful repeated calls do nothing and emit no duplicate end events.
    /// No further input may be supplied after success.
    ///
    /// # Errors
    ///
    /// Follows [`Self::parse`], including terminal callback/logger failures.
    /// Empty entity/message input is invalid; an empty mbox archive is valid.
    pub fn finish<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(MimeEvent<'_>) -> io::Result<()>,
    {
        if self.is_finished() {
            return Ok(());
        }
        self.check_active()?;
        self.process(&[], true, &mut callback)
    }

    fn check_active(&self) -> io::Result<()> {
        if matches!(self.state, State::Failed | State::Finished) {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MIME event parser is no longer active",
            ))
        } else {
            Ok(())
        }
    }

    fn process<F: EventSink>(
        &mut self,
        input: &[u8],
        eof: bool,
        callback: &mut F,
    ) -> io::Result<()> {
        let result = self.step(input, eof, callback);
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }

    fn step<F: EventSink>(
        &mut self,
        mut input: &[u8],
        eof: bool,
        callback: &mut F,
    ) -> io::Result<()> {
        loop {
            match &mut self.state {
                State::Ready => {
                    if input.is_empty() && !eof {
                        return Ok(());
                    }
                    if self.format == MimeEventFormat::Mbox {
                        self.context.reporting.reset();
                        self.context.mbox_content_end =
                            self.context.respect_content_length.then_some(0);
                        self.state = State::Marker(MarkerParser::new(
                            &self.context,
                            self.position,
                            &mut self.buffer,
                            self.position.offset == 0,
                        )?);
                    } else {
                        if self.context.message {
                            callback.event(MimeEvent::MessageBegin {
                                position: self.position,
                            })?;
                        }
                        self.start_tree(self.context.max_input_bytes);
                    }
                }
                State::Marker(parser) => {
                    let result =
                        parser.parse(&mut input, eof, &self.context, &mut self.buffer, callback);
                    self.position = parser.position();
                    match result? {
                        None => return Ok(()),
                        Some(None) => {
                            self.state = State::Finished;
                            return Ok(());
                        }
                        Some(Some(remaining)) => {
                            callback.event(MimeEvent::MessageBegin {
                                position: self.position,
                            })?;
                            self.start_tree(remaining);
                        }
                    }
                }
                State::Tree(parser) => {
                    let result = parser.parse(
                        &mut input,
                        eof,
                        &mut self.context,
                        &mut self.buffer,
                        callback,
                    );
                    self.position = parser.position();
                    let Some(end) = result? else {
                        return Ok(());
                    };
                    if self.context.message {
                        callback.event(MimeEvent::MessageEnd {
                            position: end.position,
                        })?;
                    }
                    if end.eof {
                        self.state = State::Finished;
                        return Ok(());
                    }
                    self.state = State::Ready;
                }
                State::Finished | State::Failed => unreachable!("checked before processing input"),
            }
        }
    }

    fn start_tree(&mut self, remaining: usize) {
        self.state = State::Tree(TreeParser::new(
            &self.context,
            self.position,
            &mut self.buffer,
            self.format == MimeEventFormat::Mbox,
            remaining,
        ));
    }
}

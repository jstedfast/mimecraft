// Adapted from MimeKit's MimeReader.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, BufRead},
    num::NonZeroUsize,
};

use crate::{
    ContentEncoding, Header, HeaderList, HeaderParser, MimeContentKind, MimeEntityKind,
    MimeEntityParseOptions, MimeEvent, MimePosition, encodings::NewLineFormat,
    mime_part::transfer_encoding,
};

use super::{EventSink, scan};

pub(crate) struct ReadContext<'a> {
    pub(crate) limits: MimeEntityParseOptions,
    pub(crate) reporting: crate::mime_compliance::Reporting<'a>,
    pub(crate) message: bool,
}

impl std::ops::Deref for ReadContext<'_> {
    type Target = MimeEntityParseOptions;

    fn deref(&self) -> &Self::Target {
        &self.limits
    }
}

#[derive(Default)]
pub(crate) struct ScanBuffer {
    line: Vec<u8>,
    pending: Vec<u8>,
    mbox_prefix: Option<MimePosition>,
    scanner: scan::Scanner,
}

impl ScanBuffer {
    pub(crate) fn pending_input(&self) -> &[u8] {
        if self.mbox_prefix.is_some() {
            b"From "
        } else {
            b""
        }
    }

    pub(crate) fn reset(&mut self) {
        self.line.clear();
        self.pending.clear();
        self.mbox_prefix = None;
    }
}

pub(crate) struct TreeEnd {
    pub position: MimePosition,
    pub eof: bool,
}

pub(crate) fn read_mbox_marker<R, F>(
    reader: &mut R,
    options: &MimeEntityParseOptions,
    position: &mut MimePosition,
    buffer: &mut ScanBuffer,
    callback: &mut F,
) -> io::Result<Option<usize>>
where
    R: BufRead + ?Sized,
    F: EventSink,
{
    let origin = buffer.mbox_prefix.unwrap_or(*position);
    if buffer.mbox_prefix.is_none() {
        let mut input = Lines {
            reader,
            remaining: options.max_input_bytes,
            position,
            at_line_start: true,
            piece_at_line_start: true,
            piece_position: origin,
            mbox: true,
            mbox_prefix: None,
            borrowed: 0,
            eof: false,
            scanner: buffer.scanner,
        };
        // Like MimeKit's StepMboxMarkerStart, skip whole lines until a line
        // begins with "From ". A split line must never match in its middle.
        while input.next(&mut buffer.line, &[], false, options.max_header_length)? {}
        buffer.mbox_prefix = input.mbox_prefix;
        if buffer.mbox_prefix.is_none() {
            return if position.offset == origin.offset {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing mbox marker",
                ))
            };
        }
    }
    let marker_position = buffer.mbox_prefix.take().expect("marker prefix found");
    let consumed =
        usize::try_from(position.offset - origin.offset).map_err(|_| input_limit_error())?;
    let mut remaining = options
        .max_input_bytes
        .checked_sub(consumed)
        .ok_or_else(input_limit_error)?;
    buffer.line.clear();
    buffer.line.extend_from_slice(b"From ");
    loop {
        if buffer.line.len() > options.max_header_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mbox marker length limit exceeded",
            ));
        }
        let input = match reader.fill_buf() {
            Ok(input) => input,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if input.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete mbox marker",
            ));
        }
        if remaining == 0 {
            return Err(input_limit_error());
        }
        let available = remaining.min(options.max_header_length - buffer.line.len());
        if available == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mbox marker length limit exceeded",
            ));
        }
        let input = &input[..input.len().min(available)];
        let count = memchr::memchr(b'\n', input).map_or(input.len(), |index| index + 1);
        let complete = input[count - 1] == b'\n';
        buffer.line.extend_from_slice(&input[..count]);
        *position = position.advance(&input[..count]);
        reader.consume(count);
        remaining -= count;
        if complete {
            callback.event(MimeEvent::MboxMarker {
                bytes: &buffer.line,
                position: marker_position,
                end: *position,
            })?;
            return Ok(Some(remaining));
        }
    }
}

pub(crate) fn read_tree<R, F>(
    reader: &mut R,
    options: &mut ReadContext<'_>,
    position: &mut MimePosition,
    buffer: &mut ScanBuffer,
    mbox: bool,
    remaining: usize,
    callback: &mut F,
) -> io::Result<TreeEnd>
where
    R: BufRead + ?Sized,
    F: EventSink,
{
    let header_limit = NonZeroUsize::new(options.max_header_length).expect("validated limit");
    let mut input = Lines {
        reader,
        remaining,
        position,
        at_line_start: true,
        piece_at_line_start: true,
        piece_position: MimePosition::default(),
        mbox,
        mbox_prefix: None,
        borrowed: 0,
        eof: false,
        scanner: buffer.scanner,
    };
    // Explicit message and multipart frames avoid recursive calls that
    // could exhaust the call stack on deeply nested input.
    let mut stack: Vec<Frame> = Vec::new();
    let mut state = State::headers(header_limit, *input.position);
    let ScanBuffer {
        line,
        pending,
        mbox_prefix,
        ..
    } = buffer;
    line.clear();
    pending.clear();
    let mut pending_position = *input.position;

    loop {
        if !matches!(state, State::Headers { .. })
            && input.forward_content(
                &mut state,
                pending,
                &mut pending_position,
                &stack,
                &mut options.reporting,
                callback,
            )?
        {
            continue;
        }
        if !input.next(
            line,
            &stack,
            matches!(state, State::Headers { .. }),
            options.max_header_length,
        )? {
            break;
        }
        let line_position = input.piece_position;
        let delimiter = input
            .piece_at_line_start
            .then(|| find_boundary(&stack, line))
            .flatten();
        // Withhold a piece so a delimiter's newline cannot become an
        // absent message's separator. Existing headers and preambles keep theirs.
        let trim = matches!(state, State::Leaf(_) | State::Epilogue { .. })
            || (matches!(state, State::Headers { started: false, .. })
                && matches!(stack.last(), Some(Frame::Message { .. })));
        let had_pending = !pending.is_empty();
        let framing = if delimiter.is_some() && trim {
            trim_delimiter_newline(pending)
        } else {
            b""
        };
        let end = if had_pending {
            pending_position.advance(pending)
        } else if delimiter.is_some() {
            state.end_before_boundary(line_position)
        } else {
            line_position
        };
        consume_line(
            pending,
            pending_position,
            &mut state,
            &mut stack,
            options,
            header_limit,
            callback,
        )?;
        pending.clear();
        options.reporting.content(framing, end)?;
        // The pending separator may have introduced a multipart boundary.
        let delimiter = input
            .piece_at_line_start
            .then(|| find_boundary(&stack, line))
            .flatten();
        if delimiter.is_some() {
            options.reporting.framing(line, line_position)?;
            state = at_boundary(
                state,
                &mut stack,
                line,
                options,
                end,
                (line_position, line_position.advance(line)),
                callback,
            )?;
        } else {
            std::mem::swap(pending, line);
            pending_position = line_position;
        }
    }

    *mbox_prefix = input.mbox_prefix;
    callback.end_boundary(*mbox_prefix);
    let eof = mbox_prefix.is_none();
    let had_pending = !pending.is_empty();
    let framing = if !eof
        && (matches!(state, State::Leaf(_) | State::Epilogue { .. })
            || (matches!(state, State::Headers { started: false, .. })
                && matches!(stack.last(), Some(Frame::Message { .. }))))
    {
        // An mbox marker is an outer boundary. The preceding newline belongs
        // to its framing, just as for MIME delimiters; EOF retains that newline.
        trim_delimiter_newline(pending)
    } else {
        b""
    };
    let end = if eof {
        *input.position
    } else if had_pending {
        pending_position.advance(pending)
    } else {
        state.end_before_boundary(mbox_prefix.expect("stopped at an mbox marker"))
    };
    consume_line(
        pending,
        pending_position,
        &mut state,
        &mut stack,
        options,
        header_limit,
        callback,
    )?;
    options.reporting.content(framing, end)?;
    complete_headers(
        &mut state,
        &mut stack,
        options,
        header_limit,
        end,
        !eof,
        callback,
    )?;
    if eof
        && !mbox
        && !stack
            .iter()
            .any(|frame| matches!(frame, Frame::Multipart { .. }))
        && matches!(state, State::Headers { started: false, .. })
        && matches!(stack.last(), Some(Frame::Message { begun: false }))
    {
        // Without any enclosing boundary, MimeKit still constructs the empty
        // embedded message at EOF. With a boundary it leaves the wrapper empty.
        check_depth(stack.len(), options, end)?;
        if let Some(Frame::Message { begun }) = stack.last_mut() {
            *begun = true;
        }
        callback.event(MimeEvent::MessageBegin { position: end })?;
        complete_headers(
            &mut state,
            &mut stack,
            options,
            header_limit,
            end,
            false,
            callback,
        )?;
    }
    let diagnostic = if let Some(marker) = *mbox_prefix {
        (marker, 1)
    } else {
        (end, options.reporting.column(end))
    };
    finish_entity(
        state,
        &mut stack,
        if eof {
            io::ErrorKind::UnexpectedEof
        } else {
            io::ErrorKind::InvalidData
        },
        end,
        diagnostic,
        &mut options.reporting,
        callback,
    )?;
    loop {
        wrap_messages(&mut stack, end, &mut options.reporting, callback)?;
        if stack.is_empty() {
            break;
        }
        // EOF and an mbox marker can truncate several enclosing multiparts.
        // Unwind the explicit frames just as an ancestor boundary does.
        finish_entity(
            State::Preamble,
            &mut stack,
            io::ErrorKind::UnexpectedEof,
            end,
            diagnostic,
            &mut options.reporting,
            callback,
        )?;
    }
    Ok(TreeEnd { position: end, eof })
}

enum Frame {
    Multipart {
        boundary: Option<String>,
        closed: bool,
        digest: bool,
    },
    Message {
        begun: bool,
    },
}

fn find_boundary(stack: &[Frame], line: &[u8]) -> Option<(usize, bool)> {
    // Like MimeKit's IsPossibleBoundary, reject ordinary lines before walking
    // ancestors. Otherwise deeply nested headers cause quadratic scanning.
    // Mbox From-lines are recognized separately by the input scanner.
    if !line.starts_with(b"--") {
        return None;
    }
    // Check the immediate container before its ancestors, including those
    // outside an embedded message.
    stack.iter().enumerate().rev().find_map(|(index, frame)| {
        if let Frame::Multipart {
            boundary,
            closed: false,
            ..
        } = frame
        {
            boundary
                .as_deref()
                .and_then(|boundary| boundary_line(line, boundary).map(|close| (index, close)))
        } else {
            None
        }
    })
}

fn complete_headers<F: EventSink>(
    state: &mut State,
    stack: &mut Vec<Frame>,
    options: &mut ReadContext<'_>,
    header_limit: NonZeroUsize,
    end: MimePosition,
    interrupted: bool,
    callback: &mut F,
) -> io::Result<()> {
    let State::Headers {
        parser,
        headers,
        started,
        start,
    } = state
    else {
        return Ok(());
    };
    if !*started && matches!(stack.last(), Some(Frame::Message { begun: false })) {
        return Ok(());
    }
    // A boundary can interrupt the header block; EOF can arrive immediately
    // after a header or even immediately after a subpart's opening marker.
    // Finish the raw record without inventing a separator or swallowing the
    // ancestor's delimiter as an invalid header.
    parser
        .finish(headers)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let message = (stack.is_empty() && options.reporting.message())
        || matches!(stack.last(), Some(Frame::Message { .. }));
    options.reporting.headers(
        headers,
        *start,
        end,
        match parser.body_separator() {
            Some(separator) => crate::mime_compliance::HeaderEnd::Separator(separator),
            None if interrupted => crate::mime_compliance::HeaderEnd::Boundary,
            None => crate::mime_compliance::HeaderEnd::Eof,
        },
        message,
        stack.is_empty(),
    )?;
    if headers.is_empty() && parser.body_separator().is_none() {
        if !interrupted {
            options.reporting.issue(
                crate::MimeComplianceViolation::MissingBodySeparator,
                end,
                1,
                crate::MimeCompliancePositionKind::ElementStart,
            )?;
        }
        // An mbox From-line can terminate even an empty header block. EOF
        // without any headers, however, does not contain a standalone message.
        if stack.is_empty() && !interrupted {
            return Ok(());
        }
    }
    if !options.allow_header_only && parser.body_separator().is_none() {
        return Ok(());
    }
    if stack.is_empty()
        && options.message
        && parser.body_separator().is_none()
        && !headers.is_empty()
        && headers.iter().all(|header| {
            header.is_invalid()
                && (header.raw().starts_with(b"From ") || header.raw().starts_with(b">From "))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "no valid MIME headers",
        ));
    }
    check_depth(stack.len(), options, *start)?;
    callback.body_separator(parser.body_separator().is_some());
    *state = start_entity(
        HeaderList::from(std::mem::take(headers)),
        *start,
        end,
        stack,
        options,
        header_limit,
        callback,
    )?;
    Ok(())
}

fn consume_line<F: EventSink>(
    line: &[u8],
    position: MimePosition,
    state: &mut State,
    stack: &mut Vec<Frame>,
    options: &mut ReadContext<'_>,
    header_limit: NonZeroUsize,
    callback: &mut F,
) -> io::Result<()> {
    if line.is_empty() {
        return Ok(());
    }
    match state {
        State::Headers {
            parser,
            headers,
            started,
            start,
        } => {
            check_depth(stack.len(), options, position)?;
            if !*started {
                // Use the original input position, including withheld lines
                // and multipart framing, rather than restarting at each block.
                parser.set_stream_offset(position.offset);
                *start = position;
                if let Some(Frame::Message { begun }) = stack.last_mut() {
                    *begun = true;
                    callback.event(MimeEvent::MessageBegin { position })?;
                }
            }
            *started = true;
            parser
                .parse(line, headers)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if parser.is_finished() {
                let message = (stack.is_empty() && options.reporting.message())
                    || matches!(stack.last(), Some(Frame::Message { .. }));
                options.reporting.headers(
                    headers,
                    *start,
                    position.advance(line),
                    parser.body_separator().into(),
                    message,
                    stack.is_empty(),
                )?;
                callback.body_separator(parser.body_separator().is_some());
                *state = start_entity(
                    HeaderList::from(std::mem::take(headers)),
                    *start,
                    position.advance(line),
                    stack,
                    options,
                    header_limit,
                    callback,
                )?;
            }
        }
        State::Leaf(_) | State::Preamble | State::Epilogue { .. } => emit_content(
            state,
            line,
            position,
            None,
            &mut options.reporting,
            callback,
        )?,
    }
    Ok(())
}

fn emit_content<F: EventSink>(
    state: &mut State,
    bytes: &[u8],
    mut position: MimePosition,
    scanned: Option<scan::Scan>,
    reporting: &mut crate::mime_compliance::Reporting<'_>,
    callback: &mut F,
) -> io::Result<()> {
    let kind = match state {
        State::Leaf(newlines) => {
            if let Some(scanned) = scanned {
                newlines.push_scanned(bytes, scanned);
            } else {
                newlines.push(bytes);
            }
            MimeContentKind::Body
        }
        State::Preamble => MimeContentKind::Preamble,
        State::Epilogue { .. } => MimeContentKind::Epilogue,
        State::Headers { .. } => unreachable!("content follows header completion"),
    };
    let mut chunks = bytes.chunks(CHUNK_SIZE).peekable();
    while let Some(bytes) = chunks.next() {
        // Inspect bytes separately so the disabled path need not materialize
        // the entire event for a dynamically dispatched diagnostic sink.
        reporting.content(bytes, position)?;
        callback.event(MimeEvent::Content {
            kind,
            bytes,
            position,
        })?;
        if chunks.peek().is_some() {
            position = position.advance(bytes);
        }
    }
    Ok(())
}

fn start_entity<F: EventSink>(
    headers: HeaderList,
    position: MimePosition,
    headers_end: MimePosition,
    stack: &mut Vec<Frame>,
    options: &mut ReadContext<'_>,
    header_limit: NonZeroUsize,
    callback: &mut F,
) -> io::Result<State> {
    if stack.is_empty() {
        // Before the first valid header, MimeKit permits only (munged) mbox
        // From-lines. A file containing ordinary garbage is not a MIME entity.
        for header in headers
            .as_slice()
            .iter()
            .take_while(|header| header.is_invalid())
        {
            if !options.message
                || (!header.raw().starts_with(b"From ") && !header.raw().starts_with(b">From "))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid first MIME header",
                ));
            }
        }
    }
    let content_type = headers
        .get("Content-Type")
        .filter(|header| !header.is_invalid())
        .map(|header| crate::ContentType::parse_for_reader(header.raw_value()).0);
    // The logger has already reported an unknown encoding at its value's
    // position. Like MimeKit, retain the bytes and use the unspecified encoding.
    let encoding = transfer_encoding(&headers).unwrap_or(ContentEncoding::Unspecified);
    if content_type
        .as_ref()
        .is_some_and(|value| value.is_mime_type("multipart", "*"))
    {
        let digest = content_type
            .as_ref()
            .is_some_and(|value| value.is_mime_type("multipart", "digest"));
        // A missing boundary leaves all content in the preamble. Invalid
        // markers can still match (except those containing a newline). Check
        // the nearest container first, even when ancestors reuse its marker.
        let boundary = content_type
            .as_ref()
            .and_then(|value| value.boundary())
            .map(str::to_owned);
        options.reporting.emit(
            callback,
            MimeEvent::EntityBegin {
                headers,
                kind: MimeEntityKind::Multipart,
                encoding,
                position,
                headers_end,
            },
        )?;
        stack.push(Frame::Multipart {
            boundary,
            closed: false,
            digest,
        });
        Ok(State::Preamble)
    } else {
        let message_type = content_type.as_ref().map_or_else(
            // Only the immediate entity parent supplies the digest default.
            || matches!(stack.last(), Some(Frame::Multipart { digest: true, .. })),
            |value| {
                crate::text_rfc822_headers::is_headers_type(value)
                    || (value.is_mime_type("message", "*")
                        && ["rfc822", "news", "global", "external-body", "rfc2822"]
                            .iter()
                            .any(|subtype| value.media_subtype().eq_ignore_ascii_case(subtype)))
            },
        );
        // Broken senders transfer-encode message/rfc822 despite RFC 2046.
        // Keep encoded wrappers as opaque leaves rather than parsing their
        // encoded bytes as message headers.
        let structured = options.parse_message_parts
            && message_type
            && matches!(
                encoding,
                ContentEncoding::Unspecified
                    | ContentEncoding::SevenBit
                    | ContentEncoding::EightBit
                    | ContentEncoding::Binary
            );
        options.reporting.emit(
            callback,
            MimeEvent::EntityBegin {
                headers,
                kind: if structured {
                    MimeEntityKind::MessagePart
                } else {
                    MimeEntityKind::Part
                },
                encoding,
                position,
                headers_end,
            },
        )?;
        if structured {
            stack.push(Frame::Message { begun: false });
            Ok(State::headers(header_limit, headers_end))
        } else {
            Ok(State::Leaf(NewLines::default()))
        }
    }
}

fn check_depth(
    ancestors: usize,
    options: &mut ReadContext<'_>,
    position: MimePosition,
) -> io::Result<()> {
    if ancestors >= options.max_depth {
        options.reporting.issue(
            crate::MimeComplianceViolation::MimeDepthLimitExceeded,
            position,
            1,
            crate::MimeCompliancePositionKind::ElementStart,
        )?;
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MIME entity depth limit exceeded",
        ))
    } else {
        Ok(())
    }
}

fn at_boundary<F: EventSink>(
    mut state: State,
    stack: &mut Vec<Frame>,
    line: &[u8],
    options: &mut ReadContext<'_>,
    position: MimePosition,
    marker: (MimePosition, MimePosition),
    callback: &mut F,
) -> io::Result<State> {
    callback.end_boundary(Some(marker.0));
    let header_limit = NonZeroUsize::new(options.max_header_length).expect("validated limit");
    complete_headers(
        &mut state,
        stack,
        options,
        header_limit,
        position,
        true,
        callback,
    )?;
    // Completing interrupted headers can install a nearer boundary with the
    // same spelling. Reclassify the marker before unwinding its ancestors.
    let (index, close) = find_boundary(stack, line).expect("matched boundary remains active");
    if !matches!(state, State::Preamble) || stack.len() != index + 1 {
        finish_entity(
            state,
            stack,
            io::ErrorKind::InvalidData,
            position,
            (marker.0, 1),
            &mut options.reporting,
            callback,
        )?;
        wrap_messages(stack, position, &mut options.reporting, callback)?;
    }
    while stack.len() > index + 1 {
        // We found a parent's boundary before this multipart's end marker.
        // End the truncated child and let that same marker drive its owner.
        finish_entity(
            State::Preamble,
            stack,
            io::ErrorKind::InvalidData,
            position,
            (marker.0, 1),
            &mut options.reporting,
            callback,
        )?;
        wrap_messages(stack, position, &mut options.reporting, callback)?;
    }
    let Some(Frame::Multipart { closed, .. }) = stack.last_mut() else {
        unreachable!("a matched boundary belongs to a multipart")
    };
    callback.boundary(line, close, marker.1);
    if close {
        *closed = true;
        let marker_line = line.strip_suffix(b"\n").unwrap_or(line);
        let marker_line = if line.ends_with(b"\n") {
            marker_line.strip_suffix(b"\r").unwrap_or(marker_line)
        } else {
            marker_line
        };
        Ok(State::Epilogue {
            start: marker.1,
            boundary_end: marker.0.advance(marker_line),
        })
    } else if line.ends_with(b"\n") {
        check_depth(stack.len(), options, position)?;
        Ok(State::headers(header_limit, marker.1))
    } else {
        // SkipBoundaryMarker could not reach the end of this opening line.
        // No child exists, so do not charge a depth level for its headers.
        Ok(State::Preamble)
    }
}

fn finish_entity<F: EventSink>(
    state: State,
    stack: &mut Vec<Frame>,
    incomplete: io::ErrorKind,
    position: MimePosition,
    diagnostic: (MimePosition, u64),
    reporting: &mut crate::mime_compliance::Reporting<'_>,
    callback: &mut F,
) -> io::Result<()> {
    match state {
        State::Leaf(newlines) => reporting.emit(
            callback,
            MimeEvent::EntityEnd {
                kind: MimeEntityKind::Part,
                position,
                new_line_format: newlines.format(),
            },
        ),
        State::Epilogue { .. } => {
            let Some(Frame::Multipart { closed: true, .. }) = stack.pop() else {
                unreachable!("epilogue belongs to a closed multipart")
            };
            reporting.emit(
                callback,
                MimeEvent::EntityEnd {
                    kind: MimeEntityKind::Multipart,
                    position,
                    new_line_format: None,
                },
            )
        }
        State::Headers { started: false, .. }
            if matches!(stack.last(), Some(Frame::Message { begun: false })) =>
        {
            stack.pop();
            reporting.emit(
                callback,
                MimeEvent::EntityEnd {
                    kind: MimeEntityKind::MessagePart,
                    position,
                    new_line_format: None,
                },
            )
        }
        State::Headers { .. } => Err(io::Error::new(
            incomplete,
            "missing MIME header/body separator",
        )),
        State::Preamble => {
            let Some(Frame::Multipart { boundary, .. }) = stack.pop() else {
                unreachable!("preamble belongs to a multipart")
            };
            if boundary.is_some() {
                reporting.issue(
                    crate::MimeComplianceViolation::MissingMultipartBoundary,
                    diagnostic.0,
                    diagnostic.1,
                    crate::MimeCompliancePositionKind::Exact,
                )?;
            }
            reporting.emit(
                callback,
                MimeEvent::EntityEnd {
                    kind: MimeEntityKind::Multipart,
                    position,
                    new_line_format: None,
                },
            )
        }
    }
}

fn wrap_messages<F: EventSink>(
    stack: &mut Vec<Frame>,
    position: MimePosition,
    reporting: &mut crate::mime_compliance::Reporting<'_>,
    callback: &mut F,
) -> io::Result<()> {
    while matches!(stack.last(), Some(Frame::Message { begun: true })) {
        stack.pop();
        callback.event(MimeEvent::MessageEnd { position })?;
        reporting.emit(
            callback,
            MimeEvent::EntityEnd {
                kind: MimeEntityKind::MessagePart,
                position,
                new_line_format: None,
            },
        )?;
    }
    Ok(())
}

enum State {
    Headers {
        parser: HeaderParser,
        headers: Vec<Header>,
        started: bool,
        start: MimePosition,
    },
    Leaf(NewLines),
    Preamble,
    Epilogue {
        start: MimePosition,
        boundary_end: MimePosition,
    },
}

impl State {
    fn end_before_boundary(&self, position: MimePosition) -> MimePosition {
        if let Self::Epilogue {
            start,
            boundary_end,
        } = self
            && position == *start
        {
            // The newline preceding a boundary (or mbox From-line) belongs
            // to that boundary, even when it also ends a closing marker.
            // Empty epilogues have no pending content from which to trim it.
            *boundary_end
        } else {
            position
        }
    }

    fn headers(limit: NonZeroUsize, start: MimePosition) -> Self {
        Self::Headers {
            parser: HeaderParser::with_max_header_length(limit),
            headers: Vec::new(),
            started: false,
            start,
        }
    }
}

struct Lines<'a, R: ?Sized> {
    reader: &'a mut R,
    remaining: usize,
    position: &'a mut MimePosition,
    at_line_start: bool,
    piece_at_line_start: bool,
    piece_position: MimePosition,
    mbox: bool,
    mbox_prefix: Option<MimePosition>,
    borrowed: usize,
    eof: bool,
    scanner: scan::Scanner,
}

const CHUNK_SIZE: usize = 4096;

fn input_limit_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "MIME input byte limit exceeded")
}

impl<R: BufRead + ?Sized> Lines<'_, R> {
    fn forward_content<F: EventSink>(
        &mut self,
        state: &mut State,
        pending: &mut Vec<u8>,
        pending_position: &mut MimePosition,
        stack: &[Frame],
        reporting: &mut crate::mime_compliance::Reporting<'_>,
        callback: &mut F,
    ) -> io::Result<bool> {
        if self.eof {
            return Ok(false);
        }
        let input = loop {
            match self.reader.fill_buf() {
                Ok(input) => break input,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        };
        if input.is_empty() {
            self.eof = true;
            return Ok(false);
        }
        if self.remaining == 0 {
            return Ok(false);
        }
        if pending.last() == Some(&b'\r') && input[0] == b'\n' {
            // MimeKit withholds a trailing CR because it may be half a CRLF.
            // Keep the pair together until the next line can be classified.
            pending.push(b'\n');
            *self.position = self.position.advance(b"\n");
            self.at_line_start = true;
            self.reader.consume(1);
            self.remaining -= 1;
            return Ok(true);
        }
        let multipart = stack
            .iter()
            .any(|frame| matches!(frame, Frame::Multipart { closed: false, .. }));
        let possible = |byte: u8| (multipart && byte == b'-') || (self.mbox && byte == b'F');
        if self.at_line_start && possible(input[0]) {
            return Ok(false);
        }
        let input = &input[..input.len().min(self.remaining).min(CHUNK_SIZE)];
        // Fuse candidate matching, line counting and newline-format detection,
        // rather than searching this same content again for each observation.
        let mut scanned =
            self.scanner
                .scan(input, multipart, self.mbox, pending.last() == Some(&b'\r'));
        let count = scanned.len;
        let bytes = &input[..count];
        let end = MimePosition {
            offset: self.position.offset + count as u64,
            line: self.position.line + scanned.lines as u64,
        };
        let withheld = if bytes.ends_with(b"\r\n") {
            2
        } else if matches!(bytes.last(), Some(b'\r' | b'\n')) {
            1
        } else {
            0
        };
        let content_len = count - withheld;
        scanned.len = content_len;
        if bytes.ends_with(b"\n") {
            scanned.lines -= 1;
            scanned.dos -= usize::from(withheld == 2);
        }
        // As in MimeKit's ScanContent, ordinary content borrows the input
        // buffer directly. Only the ambiguous trailing newline is copied;
        // delimiter candidates fall back to the bounded line scanner.
        emit_content(state, pending, *pending_position, None, reporting, callback)?;
        pending.clear();
        emit_content(
            state,
            &bytes[..content_len],
            *self.position,
            Some(scanned),
            reporting,
            callback,
        )?;
        *pending_position = MimePosition {
            offset: end.offset - withheld as u64,
            line: end.line - u64::from(withheld != 0 && bytes.ends_with(b"\n")),
        };
        pending.extend_from_slice(&bytes[content_len..]);
        *self.position = end;
        self.at_line_start = bytes.ends_with(b"\n");
        self.reader.consume(count);
        self.remaining -= count;
        Ok(true)
    }

    fn next(
        &mut self,
        line: &mut Vec<u8>,
        stack: &[Frame],
        reading_headers: bool,
        max_header_length: usize,
    ) -> io::Result<bool> {
        line.clear();
        self.piece_position = *self.position;
        self.piece_at_line_start = self.at_line_start;
        if self.eof {
            return Ok(false);
        }
        let mut padded_boundary = None;
        let mut padding_checked = 0;
        let marker_length = if reading_headers {
            // The pending separator can introduce a boundary that is not yet
            // on the stack. Its size is bounded by the raw header limit.
            max_header_length.saturating_add(4)
        } else {
            stack
                .iter()
                .filter_map(|frame| match frame {
                    Frame::Multipart {
                        boundary: Some(boundary),
                        closed: false,
                        ..
                    } => Some(boundary.len() + 4),
                    _ => None,
                })
                .max()
                .unwrap_or(0)
                .max(74)
        };
        loop {
            if self.mbox && self.piece_at_line_start && line == b"From " {
                // MimeKit's mbox boundary matching only checks the first five
                // bytes, not the sender/date. Retain this prefix for the next
                // call instead of buffering the next message or its marker.
                self.mbox_prefix = Some(self.piece_position);
                return Ok(false);
            }
            let prefix = self.mbox
                && self.piece_at_line_start
                && line.len() < 5
                && b"From ".starts_with(line);
            if !prefix && self.borrowed != 0 {
                return Err(input_limit_error());
            }
            let input = match self.reader.fill_buf() {
                Ok(input) => input,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if input.is_empty() {
                if self.borrowed != 0 {
                    return Err(input_limit_error());
                }
                self.eof = true;
                return Ok(!line.is_empty());
            }
            let prefix = prefix && (!line.is_empty() || input[0] == b'F');
            // Bound ordinary content lookahead even when no newline occurs.
            // Invalid boundaries can exceed the RFC's 70-byte limit. Keep a
            // possible marker until its known length, then only permit padding.
            let candidate = if line.len() < CHUNK_SIZE {
                false
            } else if line.len() < marker_length {
                self.piece_at_line_start && line.starts_with(b"--")
            } else {
                let candidate = padded_boundary.get_or_insert_with(|| {
                    self.piece_at_line_start
                        && line.starts_with(b"--")
                        && line[marker_length..].iter().all(|byte| matches!(byte, b' ' | b'\t'))
                        // A pending header separator may introduce a new boundary.
                        // Elsewhere only active ancestors can claim this line.
                        && (reading_headers || find_boundary(stack, line).is_some())
                });
                if *candidate && padding_checked != 0 {
                    // Check only newly appended padding, not the entire growing
                    // line on every refill (which would make scanning quadratic).
                    *candidate = line[padding_checked..]
                        .iter()
                        .all(|byte| matches!(byte, b' ' | b'\t'));
                }
                padding_checked = line.len();
                *candidate
            };
            if line.len() >= CHUNK_SIZE && !candidate {
                // Keep CRLF together across chunks. A trailing lone CR is
                // withheld until lookahead tells us whether LF follows it.
                if line.last() != Some(&b'\r') || input[0] != b'\n' {
                    return Ok(true);
                }
            }
            if self.remaining == 0 && !prefix {
                return Err(input_limit_error());
            }
            let capacity = if prefix {
                5 - line.len()
            } else if line.len() < CHUNK_SIZE {
                CHUNK_SIZE - line.len()
            } else if candidate {
                CHUNK_SIZE
            } else {
                1
            };
            // A next-message prefix is lookahead, not part of this message's
            // budget. Permit at most five extra bytes while it is ambiguous;
            // a mismatch or partial prefix at EOF still fails the exact limit.
            let available = if prefix {
                capacity
            } else {
                self.remaining.min(capacity)
            };
            let input = &input[..input.len().min(available)];
            let count = memchr::memchr(b'\n', input).map_or(input.len(), |index| index + 1);
            let complete = input[count - 1] == b'\n';
            line.extend_from_slice(&input[..count]);
            *self.position = self.position.advance(&input[..count]);
            self.at_line_start = complete;
            self.reader.consume(count);
            self.borrowed += count.saturating_sub(self.remaining);
            self.remaining = self.remaining.saturating_sub(count);
            if complete {
                if self.borrowed != 0 {
                    return Err(input_limit_error());
                }
                return Ok(true);
            }
        }
    }
}

fn boundary_line(line: &[u8], boundary: &str) -> Option<bool> {
    // MimeKit removes transport whitespace before comparing the whole marker.
    // A boundary value ending in whitespace therefore cannot match an opening
    // delimiter, but its closing delimiter can still match.
    let end = line
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(0, |index| index + 1);
    let line = &line[..end];
    let rest = line
        .strip_prefix(b"--")?
        .strip_prefix(boundary.as_bytes())?;
    let (close, padding) = match rest.strip_prefix(b"--") {
        Some(padding) => (true, padding),
        None => (false, rest),
    };
    // Matching a boundary prefix is not enough: only transport padding may
    // follow the marker and its optional closing "--".
    padding
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then_some(close)
}

fn trim_delimiter_newline(bytes: &mut Vec<u8>) -> &'static [u8] {
    // Only the immediately preceding newline belongs to the delimiter.
    // At EOF there is no delimiter, so callers must retain the final newline.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
            return b"\r\n";
        }
        return b"\n";
    }
    b""
}

#[derive(Default)]
struct NewLines {
    dos: bool,
    unix: bool,
    cr: bool,
}

impl NewLines {
    fn push_scanned(&mut self, bytes: &[u8], scanned: scan::Scan) {
        debug_assert_eq!(bytes.len(), scanned.len);
        if !bytes.is_empty() {
            self.dos |= scanned.dos != 0;
            self.unix |= scanned.lines != scanned.dos;
            self.cr = bytes.last() == Some(&b'\r');
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if !(self.dos && self.unix) {
            for index in memchr::memchr_iter(b'\n', bytes) {
                if (index == 0 && self.cr) || (index > 0 && bytes[index - 1] == b'\r') {
                    self.dos = true;
                } else {
                    self.unix = true;
                }
                if self.dos && self.unix {
                    break;
                }
            }
        }
        self.cr = bytes.last() == Some(&b'\r');
    }

    fn format(&self) -> Option<NewLineFormat> {
        match (self.dos, self.unix) {
            (true, true) => Some(NewLineFormat::Mixed),
            (true, false) => Some(NewLineFormat::Dos),
            (false, true) => Some(NewLineFormat::Unix),
            (false, false) => None,
        }
    }
}

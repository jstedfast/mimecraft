// Adapted from MimeKit's MimeReader.cs and MimeParser.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    io::{self, BufRead, Cursor},
    num::NonZeroUsize,
};

use crate::{
    ContentEncoding, Header, HeaderList, HeaderParser, MessagePart, MimeContent, MimeEntity,
    MimeMessage, MimePart, Multipart, encodings::NewLineFormat, mime_part::transfer_encoding,
    multipart::checked_boundary,
};

/// Resource limits and message interpretation for loading an owned MIME tree.
///
/// All limits must be positive. These are not transport constraints and do
/// not limit decoded payload size; transfer decoding happens only on request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimeEntityParseOptions {
    /// Maximum consumed input bytes, including all framing. Defaults to 64 MiB.
    ///
    /// This bounds wire data, not total allocation overhead or reader read-ahead.
    pub max_input_bytes: usize,
    /// Maximum entity depth, counting the root as one. Defaults to 64.
    ///
    /// Message-part wrappers and embedded body roots each count as entities.
    /// Loading is iterative, but model debugging/drop recurse. Choose a safe
    /// bound for your application rather than disabling this protection.
    pub max_depth: usize,
    /// Maximum raw bytes per header record, including folds. Defaults to 16 KiB.
    pub max_header_length: usize,
    /// Parse recognized, transparently encoded message parts structurally.
    ///
    /// Defaults to true. Set false to retain all message parts as opaque leaves.
    /// Recognizes message/rfc822, news, global, global-headers, external-body,
    /// and rfc2822, plus text/rfc822-headers and immediate multipart/digest
    /// children without Content-Type (implicitly message/rfc822).
    /// No missing header is synthesized.
    /// Base64, quoted-printable, and UU wrappers remain opaque regardless of
    /// this option. Present inner messages still require a header/body separator,
    /// including returned-header parts; existing bodies are not discarded.
    pub parse_message_parts: bool,
}

impl Default for MimeEntityParseOptions {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_depth: 64,
            max_header_length: 16 * 1024,
            parse_message_parts: true,
        }
    }
}

impl MimeEntityParseOptions {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.max_input_bytes == 0 || self.max_depth == 0 || self.max_header_length == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MIME parser limits must be positive",
            ));
        }
        Ok(())
    }
}

impl MimeEntity {
    /// Loads one MIME entity through EOF into owned in-memory body streams.
    ///
    /// Uses default [`MimeEntityParseOptions`]. For plain [`io::Read`] sources,
    /// wrap the source in [`io::BufReader`]. No seeking or source closing occurs.
    /// See [`Self::load_with_options`] for structural rules and limitations.
    ///
    /// # Errors
    ///
    /// Returns input, metadata, resource-limit, or I/O errors without a partial tree.
    ///
    /// ```
    /// use mimecraft::{MimeEntity, TextPart};
    ///
    /// let mut input = &b"Content-Type: text/plain; charset=utf-8\r\n\r\nhello"[..];
    /// let MimeEntity::Part(part) = MimeEntity::load(&mut input)? else {
    ///     unreachable!()
    /// };
    /// assert_eq!(TextPart::from_part(part)?.text()?, "hello");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load<R: BufRead + ?Sized>(reader: &mut R) -> io::Result<Self> {
        Self::load_with_options(reader, &MimeEntityParseOptions::default())
    }

    /// Loads a complete entity with explicit limits and message interpretation.
    ///
    /// Reads incrementally, buffering up to two physical lines and retaining
    /// headers, preambles, epilogues, and encoded leaf bodies in memory. This is not a
    /// constant-memory, zero-copy, persistent-source, or event-based parser.
    /// The resulting source type is [`Cursor<Vec<u8>>`], positioned at zero.
    /// Body encoding comes from the first Content-Transfer-Encoding header;
    /// missing means `Default`. Body bytes are not decoded or newline-normalized.
    /// Newline hints describe stored LF/CRLF occurrences, excluding framing.
    ///
    /// Raw headers retain spelling, folds, duplicates, and malformed records
    /// using [`HeaderParser`]. Missing Content-Type selects an opaque leaf,
    /// except that immediate multipart/digest children default to message/rfc822.
    /// That default does not extend into the embedded body or other containers.
    /// No missing header is synthesized. Invalid present Content-Type/encoding
    /// is an error rather than a silent fallback. `multipart/*` creates a
    /// container. Recognized, transparently encoded message parts create
    /// [`MessagePart`] nodes unless `parse_message_parts` is false; see
    /// [`MimeEntityParseOptions::parse_message_parts`] for supported types.
    /// Each embedded message partitions its own root Content-* headers onto
    /// its body. Limits are shared across the entire tree, not reset per message.
    /// Encoded wrappers and other types remain opaque leaves.
    ///
    /// Delimiters match whole, case-sensitive lines with optional trailing
    /// space/tab. CRLF and LF are accepted, as is a closing marker at EOF.
    /// Preambles retain their ending newline. Leaf content and nested epilogues
    /// exclude exactly the LF or CRLF immediately preceding a parent delimiter;
    /// at EOF all trailing content bytes are retained. Epilogues begin after
    /// the closing marker's line ending, matching the [`Multipart`] model.
    ///
    /// A header/body separator is required for every entity, including empty
    /// ones. Empty input, header-only entities, premature ancestor delimiters,
    /// and missing closing delimiters are errors. Empty containers (just a
    /// closing marker) are accepted. A message part with no payload has no
    /// attached message; a present message needs its own separator. Existing
    /// header separators can also frame empty children, but delimiter-only
    /// blank lines do not synthesize an attached message.
    /// Container encodings and boundary syntax/
    /// ancestor overlap follow the writer's checks. Unlike MimeKit's recovery
    /// paths, no truncated containers or delimiter spelling are retained.
    /// Writing may normalize headers/framing and distinguish empty bodies
    /// differently; raw-wire byte-for-byte round trips are not promised.
    ///
    /// # Errors
    ///
    /// Zero-valued limits return [`io::ErrorKind::InvalidInput`] before I/O.
    /// Malformed metadata, misplaced boundaries, and exceeded limits return
    /// [`io::ErrorKind::InvalidData`]; incomplete structure returns
    /// [`io::ErrorKind::UnexpectedEof`]. Header-length errors retain
    /// [`crate::HeaderTooLong`] as their cause. Other I/O errors propagate and
    /// interrupted reads retry. Source position is not restored; retrying needs
    /// a fresh or repositioned source. Reader read-ahead can exceed consumed
    /// bytes; retain the same buffered reader after an error.
    pub fn load_with_options<R: BufRead + ?Sized>(
        reader: &mut R,
        options: &MimeEntityParseOptions,
    ) -> io::Result<Self> {
        options.validate()?;
        let header_limit = NonZeroUsize::new(options.max_header_length).expect("validated limit");
        let mut input = Lines {
            reader,
            remaining: options.max_input_bytes,
        };
        // Explicit message and multipart frames avoid recursive calls that
        // could exhaust the call stack on deeply nested input.
        let mut stack: Vec<Frame> = Vec::new();
        let mut state = State::headers(header_limit);
        let mut line = Vec::new();
        let mut pending = Vec::new();

        while input.next(&mut line)? {
            let delimiter = find_boundary(&stack, &line);
            // Withhold a line so a delimiter's newline cannot become an
            // absent message's separator. Existing headers and preambles keep theirs.
            let trim = matches!(state, State::Leaf { .. } | State::Epilogue(_))
                || (matches!(state, State::Headers { started: false, .. })
                    && matches!(stack.last(), Some(Frame::Message(_))));
            if delimiter.is_some() && trim {
                trim_delimiter_newline(&mut pending);
            }
            consume_line(&pending, &mut state, &mut stack, options, header_limit)?;
            pending.clear();
            // The pending separator may have introduced a multipart boundary.
            let delimiter = delimiter.or_else(|| find_boundary(&stack, &line));
            if let Some((index, close)) = delimiter {
                state = at_boundary(state, &mut stack, index, close, options, header_limit)?;
            } else {
                std::mem::swap(&mut pending, &mut line);
            }
        }

        consume_line(&pending, &mut state, &mut stack, options, header_limit)?;
        let entity = finish_entity(state, &mut stack, io::ErrorKind::UnexpectedEof)?;
        let entity = wrap_messages(entity, &mut stack);
        if stack.is_empty() {
            Ok(entity)
        } else {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "missing multipart closing boundary",
            ))
        }
    }
}

enum Frame {
    Multipart {
        part: Multipart,
        boundary: String,
        closed: bool,
        digest: bool,
    },
    Message(HeaderList),
}

fn find_boundary(stack: &[Frame], line: &[u8]) -> Option<(usize, bool)> {
    // Check the immediate container before its ancestors, including those
    // outside an embedded message.
    stack.iter().enumerate().rev().find_map(|(index, frame)| {
        if let Frame::Multipart {
            boundary,
            closed: false,
            ..
        } = frame
        {
            boundary_line(line, boundary).map(|close| (index, close))
        } else {
            None
        }
    })
}

fn consume_line(
    line: &[u8],
    state: &mut State,
    stack: &mut Vec<Frame>,
    options: &MimeEntityParseOptions,
    header_limit: NonZeroUsize,
) -> io::Result<()> {
    if line.is_empty() {
        return Ok(());
    }
    match state {
        State::Headers {
            parser,
            headers,
            started,
        } => {
            check_depth(stack.len(), options)?;
            *started = true;
            parser
                .parse(line, headers)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if parser.is_finished() {
                let headers = HeaderList::from(std::mem::take(headers));
                let content_type = headers
                    .get("Content-Type")
                    .map(Header::parse_content_type)
                    .transpose()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                if content_type
                    .as_ref()
                    .is_some_and(|value| value.is_mime_type("multipart", "*"))
                {
                    let digest = content_type
                        .as_ref()
                        .is_some_and(|value| value.is_mime_type("multipart", "digest"));
                    let part = Multipart::from_parts(headers, Vec::new(), Vec::new(), Vec::new());
                    let boundary = checked_boundary(&part)?;
                    if stack.iter().any(|frame| {
                        matches!(frame,
                            Frame::Multipart { boundary: ancestor, .. }
                            if boundary.starts_with(ancestor) || ancestor.starts_with(&boundary)
                        )
                    }) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "multipart boundary overlaps an ancestor boundary",
                        ));
                    }
                    stack.push(Frame::Multipart {
                        part,
                        boundary,
                        closed: false,
                        digest,
                    });
                    *state = State::Preamble(Vec::new());
                } else {
                    let encoding = transfer_encoding(&headers)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    let message_type = content_type.as_ref().map_or_else(
                        // Only the immediate entity parent supplies the digest default.
                        || matches!(stack.last(), Some(Frame::Multipart { digest: true, .. })),
                        |value| {
                            crate::text_rfc822_headers::is_headers_type(value)
                                || (value.is_mime_type("message", "*")
                                    && ["rfc822", "news", "global", "external-body", "rfc2822"]
                                        .iter()
                                        .any(|subtype| {
                                            value.media_subtype().eq_ignore_ascii_case(subtype)
                                        }))
                        },
                    );
                    // Broken senders transfer-encode message/rfc822 despite
                    // RFC 2046. Keep encoded wrappers as opaque leaves rather
                    // than parsing their encoded bytes as message headers.
                    let structured = options.parse_message_parts
                        && message_type
                        && matches!(
                            encoding,
                            ContentEncoding::Default
                                | ContentEncoding::SevenBit
                                | ContentEncoding::EightBit
                                | ContentEncoding::Binary
                        );
                    if structured {
                        stack.push(Frame::Message(headers));
                        *state = State::headers(header_limit);
                    } else {
                        *state = State::Leaf {
                            headers,
                            encoding,
                            body: Vec::new(),
                        };
                    }
                }
            }
        }
        State::Leaf { body, .. } | State::Preamble(body) | State::Epilogue(body) => {
            body.extend_from_slice(line);
        }
    }
    Ok(())
}

fn check_depth(ancestors: usize, options: &MimeEntityParseOptions) -> io::Result<()> {
    if ancestors >= options.max_depth {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "MIME entity depth limit exceeded",
        ))
    } else {
        Ok(())
    }
}

fn at_boundary(
    state: State,
    stack: &mut Vec<Frame>,
    index: usize,
    close: bool,
    options: &MimeEntityParseOptions,
    header_limit: NonZeroUsize,
) -> io::Result<State> {
    let (preamble, entity) = if let State::Preamble(bytes) = state {
        (Some(bytes), None)
    } else {
        let entity = finish_entity(state, stack, io::ErrorKind::InvalidData)?;
        (None, Some(wrap_messages(entity, stack)))
    };
    if stack.len() != index + 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ancestor boundary before nested multipart closed",
        ));
    }
    let Some(Frame::Multipart { part, closed, .. }) = stack.last_mut() else {
        unreachable!("a matched boundary belongs to a multipart")
    };
    if let Some(bytes) = preamble {
        part.set_preamble(bytes);
    }
    if let Some(entity) = entity {
        part.children_mut().push(entity);
    }
    if close {
        *closed = true;
        Ok(State::Epilogue(Vec::new()))
    } else {
        check_depth(stack.len(), options)?;
        Ok(State::headers(header_limit))
    }
}

fn finish_entity(
    state: State,
    stack: &mut Vec<Frame>,
    incomplete: io::ErrorKind,
) -> io::Result<MimeEntity> {
    match state {
        State::Leaf {
            headers,
            encoding,
            body,
        } => Ok(finish_leaf(headers, encoding, body)),
        State::Epilogue(bytes) => {
            let Some(Frame::Multipart {
                mut part,
                closed: true,
                ..
            }) = stack.pop()
            else {
                unreachable!("epilogue belongs to a closed multipart")
            };
            part.set_epilogue(bytes);
            Ok(part.into())
        }
        State::Headers { started: false, .. }
            if matches!(stack.last(), Some(Frame::Message(_))) =>
        {
            let Some(Frame::Message(headers)) = stack.pop() else {
                unreachable!()
            };
            Ok(MessagePart::from_parts(headers, None).into())
        }
        State::Headers { .. } => Err(io::Error::new(
            incomplete,
            "missing MIME header/body separator",
        )),
        State::Preamble(_) => Err(io::Error::new(
            incomplete,
            "missing multipart closing boundary",
        )),
    }
}

fn wrap_messages(mut entity: MimeEntity, stack: &mut Vec<Frame>) -> MimeEntity {
    while matches!(stack.last(), Some(Frame::Message(_))) {
        let Some(Frame::Message(headers)) = stack.pop() else {
            unreachable!()
        };
        let message = MimeMessage::from_loaded_entity(entity);
        entity = MessagePart::from_parts(headers, Some(message)).into();
    }
    entity
}

enum State {
    Headers {
        parser: HeaderParser,
        headers: Vec<Header>,
        started: bool,
    },
    Leaf {
        headers: HeaderList,
        encoding: ContentEncoding,
        body: Vec<u8>,
    },
    Preamble(Vec<u8>),
    Epilogue(Vec<u8>),
}

impl State {
    fn headers(limit: NonZeroUsize) -> Self {
        Self::Headers {
            parser: HeaderParser::with_max_header_length(limit),
            headers: Vec::new(),
            started: false,
        }
    }
}

struct Lines<'a, R: ?Sized> {
    reader: &'a mut R,
    remaining: usize,
}

impl<R: BufRead + ?Sized> Lines<'_, R> {
    fn next(&mut self, line: &mut Vec<u8>) -> io::Result<bool> {
        line.clear();
        loop {
            let input = match self.reader.fill_buf() {
                Ok(input) => input,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if input.is_empty() {
                return Ok(!line.is_empty());
            }
            if self.remaining == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MIME input byte limit exceeded",
                ));
            }
            let input = &input[..input.len().min(self.remaining)];
            let count = input
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(input.len(), |index| index + 1);
            let complete = input[count - 1] == b'\n';
            line.extend_from_slice(&input[..count]);
            self.reader.consume(count);
            self.remaining -= count;
            if complete {
                return Ok(true);
            }
        }
    }
}

fn boundary_line(line: &[u8], boundary: &str) -> Option<bool> {
    let line = match line.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    };
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

fn trim_delimiter_newline(bytes: &mut Vec<u8>) {
    // Only the immediately preceding newline belongs to the delimiter.
    // At EOF there is no delimiter, so callers must retain the final newline.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
}

fn finish_leaf(headers: HeaderList, encoding: ContentEncoding, body: Vec<u8>) -> MimeEntity {
    let mut dos = false;
    let mut unix = false;
    for (index, &byte) in body.iter().enumerate() {
        if byte == b'\n' {
            if index > 0 && body[index - 1] == b'\r' {
                dos = true;
            } else {
                unix = true;
            }
        }
    }
    let mut content = MimeContent::with_encoding(Cursor::new(body), encoding);
    content.set_new_line_format(match (dos, unix) {
        (true, true) => Some(NewLineFormat::Mixed),
        (true, false) => Some(NewLineFormat::Dos),
        (false, true) => Some(NewLineFormat::Unix),
        (false, false) => None,
    });
    MimePart::from_parts(headers, content).into()
}

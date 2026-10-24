// Adapted from MimeKit's MessagePartial.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    io::{self, BufReader, Cursor, Read, Seek},
};

use crate::{
    ContentHeaderParseError, ContentType, HeaderFormatError, HeaderId, HeaderList, MimeContent,
    MimeEntityParseOptions, MimeMessage, MimePart,
};

/// An opaque message/partial fragment with checked metadata and reassembly.
///
/// Composes [`MimePart`]; loading and conversion keep the ordinary leaf variant.
/// Identifiers are opaque, case-sensitive strings, not parsed Message-Ids.
/// Raw headers remain authoritative. Metadata access and wrapping never read
/// or reposition sources, and returned parameter values are detached.
///
/// [`Self::join`] reassembles existing fragments using the first enclosing
/// message's headers. Automatic splitting, generated IDs, and transport
/// preparation are not implemented.
///
/// ```
/// use std::io::Cursor;
/// use mimecraft::{Header, HeaderList, MessagePartial, MimeContent};
///
/// let envelope: HeaderList = vec![Header::parse(b"From: sender@example.com")?].into();
/// let mut fragments = vec![
///     MessagePartial::with_content("id", 2, 2, MimeContent::new(Cursor::new(b"body".as_slice())))?,
///     MessagePartial::with_content("id", 1, 2, MimeContent::new(Cursor::new(
///         b"Subject: Original\r\n\r\n".as_slice(),
///     )))?,
/// ];
/// let joined = MessagePartial::join(&envelope, &mut fragments)?.unwrap();
/// assert!(joined.headers().contains("From"));
/// assert!(joined.headers().contains("Subject"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct MessagePartial<R = Cursor<Vec<u8>>> {
    part: MimePart<R>,
}

impl MessagePartial {
    /// Creates a numbered fragment with empty, unencoded owned content.
    ///
    /// Sets id, number, and total in Content-Type without generating any other
    /// headers. The ID is preserved verbatim, including empty strings.
    ///
    /// # Errors
    ///
    /// Requires `1 <= number <= total <= i32::MAX`; formatting errors propagate.
    pub fn new(id: &str, number: u32, total: u32) -> Result<Self, MessagePartialError> {
        Self::with_content(id, number, total, MimeContent::new(Cursor::new(Vec::new())))
    }
}

impl<R: Read + Seek> MessagePartial<R> {
    /// Creates a numbered fragment owning or borrowing the supplied content.
    ///
    /// Preserves stored encoding, newline hint, and source position without I/O.
    /// No outer transfer-encoding header is generated.
    ///
    /// # Errors
    ///
    /// Requires `1 <= number <= total <= i32::MAX`. Metadata formatting errors
    /// propagate; failure drops owned content but does not access borrowed sources.
    pub fn with_content(
        id: &str,
        number: u32,
        total: u32,
        content: MimeContent<R>,
    ) -> Result<Self, MessagePartialError> {
        if number == 0 || number > i32::MAX as u32 {
            return Err(MessagePartialError::InvalidNumber(number));
        }
        if total < number || total > i32::MAX as u32 {
            return Err(MessagePartialError::InvalidTotal { number, total });
        }
        let mut value =
            ContentType::new("message", "partial").expect("literal message/partial type is valid");
        for (name, value_text) in [
            ("id", id.to_owned()),
            ("number", number.to_string()),
            ("total", total.to_string()),
        ] {
            value.parameters_mut().set_optional(name, Some(value_text));
        }
        Ok(Self {
            part: MimePart::with_content_type(&value, content)
                .map_err(MessagePartialError::HeaderFormat)?,
        })
    }

    /// Adopts a leaf after checking its first Content-Type without source I/O.
    ///
    /// Accepts message/partial ignoring ASCII case. Does not require or validate
    /// parameters, transfer encoding, or fragment bytes.
    ///
    /// # Errors
    ///
    /// Missing, malformed, and other media types return errors. Failure drops
    /// owned content; borrowed sources remain open and unrepositioned.
    pub fn from_part(part: MimePart<R>) -> Result<Self, MessagePartialError> {
        let partial = Self { part };
        partial.checked_content_type()?;
        Ok(partial)
    }

    /// Returns the underlying leaf.
    pub fn part(&self) -> &MimePart<R> {
        &self.part
    }

    /// Edits the leaf; metadata access and joining recheck its first Content-Type.
    pub fn part_mut(&mut self) -> &mut MimePart<R> {
        &mut self.part
    }

    /// Extracts the leaf without copying or accessing its source.
    pub fn into_part(self) -> MimePart<R> {
        self.part
    }

    /// Returns the decoded ID without trimming or Message-Id validation.
    ///
    /// # Errors
    ///
    /// Returns media-type errors. Only an absent parameter returns `None`.
    pub fn id(&self) -> Result<Option<String>, MessagePartialError> {
        Ok(self
            .checked_content_type()?
            .parameters()
            .get("id")
            .map(str::to_owned))
    }

    /// Returns the declared fragment number, including zero if present.
    ///
    /// Accepts ASCII digits with surrounding ASCII space or tab-through-CR
    /// whitespace, but no sign. Values cannot exceed `i32::MAX`. Unlike MimeKit's
    /// nullable getter, invalid present values are errors, not missing values.
    ///
    /// # Errors
    ///
    /// Returns metadata or numeric errors. Only absence returns `None`.
    pub fn number(&self) -> Result<Option<u32>, MessagePartialError> {
        number_parameter(&self.checked_content_type()?, "number")
    }

    /// Returns the declared total with the same numeric rules as [`Self::number`].
    ///
    /// Does not compare the total with this fragment's number.
    ///
    /// # Errors
    ///
    /// Returns metadata or numeric errors. Only absence returns `None`.
    pub fn total(&self) -> Result<Option<u32>, MessagePartialError> {
        number_parameter(&self.checked_content_type()?, "total")
    }

    /// Reassembles fragments with default parser limits.
    ///
    /// `enclosing_headers` must be the first fragment's enclosing message headers,
    /// not the fragment's own Content-* headers. See [`Self::join_with_options`].
    ///
    /// # Errors
    ///
    /// Returns metadata, sequence, parser-limit, and I/O errors.
    pub fn join(
        enclosing_headers: &HeaderList,
        partials: &mut [Self],
    ) -> Result<Option<MimeMessage>, MessagePartialError> {
        Self::join_with_options(
            enclosing_headers,
            partials,
            &MimeEntityParseOptions::default(),
        )
    }

    /// Sorts borrowed fragments by number and parses their concatenated decoded bytes.
    ///
    /// Empty input returns `None`. Every fragment must have a matching,
    /// case-sensitive ID and a number. Unlike MimeKit, absent IDs are rejected.
    /// Empty IDs are accepted. Numbers must be exactly 1 through the fragment
    /// count, and the highest-numbered fragment's total must equal that count.
    /// Totals on earlier fragments are ignored, matching MimeKit. All metadata
    /// and sequence validation precedes source I/O, including for a single fragment.
    ///
    /// Does not reorder the caller's slice or modify any fragment headers/bytes.
    /// Each source is opened at zero and transfer-decoded using its stored
    /// encoding, not an edited outer transfer header. Reads chain incrementally
    /// without combining fragment data in an intermediate buffer. The resulting
    /// message owns parsed body buffers. Source cursors are not restored, even
    /// on success; decoding/read-ahead and errors follow [`MimeContent::open`].
    ///
    /// Parser limits apply to the combined decoded wire message, including
    /// headers, with shared inner depth/header limits. They do not bound encoded
    /// input consumed by tolerant decoders, buffered read-ahead, the enclosing
    /// headers, fragment metadata, or allocation overhead.
    ///
    /// Enclosed Subject, Message-Id, Encrypted, and MIME-Version fields are kept;
    /// other enclosed non-Content fields are discarded. Enclosing non-Content
    /// fields are copied in order except those four special fields, whose slots
    /// take matching enclosed fields when present. Remaining enclosed special
    /// fields follow. Body Content-* headers remain those of the enclosed message.
    /// Raw field spelling, duplicates, folds, and values are retained. Combined
    /// message and root-body headers have their stream offsets cleared, so
    /// serialization writes the combined message headers before root-body fields.
    /// Descendant offsets and embedded-message header interleaving are preserved.
    /// The enclosing headers and input fragment headers retain their offsets.
    ///
    /// # Errors
    ///
    /// Zero parser limits fail before I/O, even for empty input. Invalid metadata,
    /// missing/duplicate fragments, mismatched IDs, incomplete MIME structure,
    /// limit breaches, and source failures return no partial message. Sources
    /// may have been repositioned or consumed when I/O or parsing fails.
    pub fn join_with_options(
        enclosing_headers: &HeaderList,
        partials: &mut [Self],
        options: &MimeEntityParseOptions,
    ) -> Result<Option<MimeMessage>, MessagePartialError> {
        options.validate().map_err(MessagePartialError::Io)?;
        if partials.is_empty() {
            return Ok(None);
        }
        let mut ordered = Vec::with_capacity(partials.len());
        let mut id = None;
        for (index, partial) in partials.iter_mut().enumerate() {
            let content_type = partial.checked_content_type()?;
            let current = content_type
                .parameters()
                .get("id")
                .ok_or(MessagePartialError::MissingParameter("id"))?;
            if id.as_deref().is_some_and(|id| id != current) {
                return Err(MessagePartialError::MismatchedId { index });
            }
            if id.is_none() {
                id = Some(current.to_owned());
            }
            let number = number_parameter(&content_type, "number")?
                .ok_or(MessagePartialError::MissingParameter("number"))?;
            ordered.push((number, partial));
        }
        // Join has already validated every number, including a single fragment
        // that would never reach a sort comparer. Compare without subtraction.
        ordered.sort_unstable_by_key(|(number, _)| *number);
        let total = ordered
            .last()
            .expect("nonempty fragments")
            .1
            .total()?
            .ok_or(MessagePartialError::MissingParameter("total"))?;
        if usize::try_from(total).ok() != Some(ordered.len()) {
            return Err(MessagePartialError::CountMismatch {
                expected: total,
                actual: ordered.len(),
            });
        }
        for (index, (number, _)) in ordered.iter().enumerate() {
            if usize::try_from(*number).ok() != Some(index + 1) {
                return Err(MessagePartialError::UnexpectedNumber {
                    expected: index + 1,
                    actual: *number,
                });
            }
        }
        // Chain all the partial content streams in numbered order. Each decoder
        // ends at its own fragment boundary; the MIME parser sees one byte stream.
        let mut readers = VecDeque::with_capacity(ordered.len());
        for (_, partial) in ordered {
            readers.push_back(
                partial
                    .part
                    .content_mut()
                    .open()
                    .map_err(MessagePartialError::Io)?,
            );
        }
        let mut reader = BufReader::new(FragmentReader(readers));
        let mut joined = MimeMessage::load_with_options(&mut reader, options)
            .map_err(MessagePartialError::Io)?;
        combine_headers(enclosing_headers, &mut joined);
        Ok(Some(joined))
    }

    fn checked_content_type(&self) -> Result<ContentType, MessagePartialError> {
        let value = self
            .part
            .content_type()
            .map_err(MessagePartialError::ContentType)?
            .ok_or(MessagePartialError::MissingContentType)?;
        if !value.is_mime_type("message", "partial") {
            return Err(MessagePartialError::NotPartial(value.mime_type()));
        }
        Ok(value)
    }
}

fn number_parameter(
    value: &ContentType,
    name: &'static str,
) -> Result<Option<u32>, MessagePartialError> {
    let Some(raw) = value.parameters().get(name) else {
        return Ok(None);
    };
    let text = raw.trim_matches(|ch: char| matches!(ch, ' ' | '\t'..='\r'));
    let number = (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<u32>().ok())
        .flatten()
        .filter(|number| *number <= i32::MAX as u32)
        .ok_or_else(|| MessagePartialError::InvalidParameter {
            name,
            value: raw.to_owned(),
        })?;
    Ok(Some(number))
}

fn is_enclosed_field(id: HeaderId) -> bool {
    matches!(
        id,
        HeaderId::Subject | HeaderId::MessageId | HeaderId::Encrypted | HeaderId::MimeVersion
    )
}

fn combine_headers(enclosing: &HeaderList, joined: &mut MimeMessage) {
    // RFC 2046 drops enclosed non-Content fields except Subject, Message-Id,
    // Encrypted, and MIME-Version. Content-* fields already belong to the body.
    let mut enclosed: Vec<_> = std::mem::take(joined.headers_mut())
        .into_iter()
        .filter(|header| is_enclosed_field(header.id()))
        .collect();
    let mut headers = HeaderList::new();
    // Copy the initial enclosing message's other non-Content fields in order,
    // using enclosed special fields in the corresponding enclosing slots.
    for header in enclosing {
        if header.is_content_header() {
            continue;
        }
        if is_enclosed_field(header.id()) {
            if let Some(index) = enclosed.iter().position(|inner| inner.id() == header.id()) {
                headers.add(enclosed.remove(index));
            }
        } else {
            headers.add(header.clone());
        }
    }
    headers.extend(enclosed);
    // The enclosing and enclosed headers came from different streams; their
    // offsets no longer describe the combined order. Like MimeKit, clear the
    // message and root-body offsets, not those of independently ordered
    // descendants such as attached messages.
    headers.clear_offsets();
    *joined.headers_mut() = headers;
    if let Some(body) = joined.body_mut() {
        body.headers_mut().clear_offsets();
    }
}

struct FragmentReader<R>(VecDeque<R>);

impl<R: Read> Read for FragmentReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while let Some(reader) = self.0.front_mut() {
            let count = reader.read(buffer)?;
            if count != 0 {
                return Ok(count);
            }
            self.0.pop_front();
        }
        Ok(0)
    }
}

/// A partial-message fragment could not be constructed, read, or reassembled.
#[derive(Debug)]
pub enum MessagePartialError {
    /// The first Content-Type header is absent.
    MissingContentType,
    /// The first Content-Type describes a different media type.
    NotPartial(String),
    /// The first Content-Type cannot be parsed.
    ContentType(ContentHeaderParseError),
    /// Constructor metadata cannot be serialized.
    HeaderFormat(HeaderFormatError),
    /// Constructor number is zero or greater than i32::MAX.
    InvalidNumber(u32),
    /// Constructor total is less than number or greater than i32::MAX.
    InvalidTotal {
        /// Requested fragment number.
        number: u32,
        /// Requested total.
        total: u32,
    },
    /// An optional numeric parameter is present but invalid.
    InvalidParameter {
        /// Numeric parameter name.
        name: &'static str,
        /// Original decoded parameter value.
        value: String,
    },
    /// Joining requires a parameter absent from a fragment.
    MissingParameter(&'static str),
    /// ID differs from the first supplied fragment's ID; index is zero-based.
    MismatchedId {
        /// Position in the caller's original, unsorted slice.
        index: usize,
    },
    /// The final fragment's total differs from the number of supplied fragments.
    CountMismatch {
        /// Total declared by the highest-numbered fragment.
        expected: u32,
        /// Number of supplied fragments.
        actual: usize,
    },
    /// Sorted numbers are not the sequence 1 through the fragment count.
    UnexpectedNumber {
        /// Required one-based position in sorted order.
        expected: usize,
        /// Declared fragment number.
        actual: u32,
    },
    /// Opening, decoding, or parsing failed.
    Io(io::Error),
}

impl fmt::Display for MessagePartialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingContentType => f.write_str("partial message has no Content-Type"),
            Self::NotPartial(value) => write!(f, "expected message/partial, got {value}"),
            Self::ContentType(error) => error.fmt(f),
            Self::HeaderFormat(error) => error.fmt(f),
            Self::InvalidNumber(value) => write!(f, "invalid partial message number: {value}"),
            Self::InvalidTotal { number, total } => write!(
                f,
                "invalid total {total} for partial message number {number}"
            ),
            Self::InvalidParameter { name, value } => {
                write!(f, "invalid partial message {name}: {value:?}")
            }
            Self::MissingParameter(name) => write!(f, "partial message has no {name} parameter"),
            Self::MismatchedId { index } => {
                write!(f, "partial message at index {index} has a mismatching id")
            }
            Self::CountMismatch { expected, actual } => {
                write!(f, "expected {expected} partial messages, got {actual}")
            }
            Self::UnexpectedNumber { expected, actual } => write!(
                f,
                "expected partial message number {expected}, got {actual}"
            ),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl Error for MessagePartialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ContentType(error) => Some(error),
            Self::HeaderFormat(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

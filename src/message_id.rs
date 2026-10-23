// Adapted from MimeKit's Utils/ParseUtils.cs and Utils/MimeUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, iter::FusedIterator, str::FromStr};

use crate::parse_utils::{
    DomainParseErrorKind, is_atom, is_whitespace, skip_comments_and_whitespace, skip_quoted,
    skip_whitespace,
};

/// A normalized Message-Id or Content-Id value parsed with MimeKit's loose grammar.
///
/// The outer angle brackets and unquoted local-part whitespace are removed,
/// CR/LF are removed from quoted local parts, and domain comments/whitespace
/// are removed. Quotes, quoted-pair escapes, and case are retained. International
/// local parts and domain atoms must be UTF-8; no IDN conversion is performed.
///
/// The reading APIs are compatibility parsers, **not authoring validators**:
/// missing domains, unusual local-part characters (including controls), repeated
/// `@` signs, and other malformed forms may be accepted. Its output is not
/// guaranteed safe to insert into a header. Use [`Self::from_value`] for
/// MimeKit's setter-style checks. ID generation is not implemented yet.
///
/// [`Display`](fmt::Display) returns the normalized value without angle brackets;
/// use [`crate::Header::raw`] to preserve an original header unchanged.
///
/// ```
/// use mimecraft::MessageId;
///
/// let id = MessageId::parse(b" <local@@example.com.> (comment)")?;
/// assert_eq!(id.as_str(), "local@example.com");
/// let (id, consumed) = MessageId::parse_prefix(b"<first@example.com> <second@example.com>")?;
/// assert_eq!(id.as_str(), "first@example.com");
/// assert_eq!(&b"<first@example.com> <second@example.com>"[consumed..],
///            b" <second@example.com>");
/// # Ok::<(), mimecraft::MessageIdParseError>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MessageId {
    value: String,
}

impl MessageId {
    /// Normalizes a value using MimeKit's message-ID property setter rules.
    ///
    /// Unlike [`Self::parse`] and [`FromStr`], this rejects NULs and non-folding
    /// CR/LF within the trimmed value. It first trims outer Unicode whitespace.
    /// Interior CRLF, bare CR, or bare LF must be followed by a space or tab;
    /// the line break is removed but the following whitespace is retained.
    ///
    /// Values beginning with `<` use the complete bracketed grammar. Other
    /// values are wrapped in angle brackets before parsing, and unquoted
    /// spaces/tabs are rejected rather than silently collapsed. Consequently,
    /// `<a b@c>` normalizes to `ab@c`, but `a b@c` is rejected. Quotes and quoted
    /// pairs follow the same loose compatibility rules as MimeKit.
    ///
    /// This is not strict RFC validation: local-only IDs, missing domains,
    /// unusual local-part characters, and other compatibility forms remain
    /// accepted. It does not enforce length limits or format a header.
    /// Successful results contain no NUL, CR, or LF.
    ///
    /// ```
    /// use mimecraft::MessageId;
    ///
    /// let id = MessageId::from_value("\"x\r\n y\"@example.com")?;
    /// assert_eq!(id.as_str(), "\"x y\"@example.com");
    /// assert!(MessageId::from_value("a b@example.com").is_err());
    /// assert!(MessageId::from_value("a@b\r\nBcc: other@example.com").is_err());
    /// # Ok::<(), mimecraft::MessageIdParseError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`MessageIdParseError`] for invalid assignment values or malformed
    /// syntax. Offsets refer to bytes in the original string, before trimming,
    /// unfolding, or adding brackets. Errors on synthetic brackets map to the
    /// corresponding boundary of the trimmed value.
    pub fn from_value(value: &str) -> Result<Self, MessageIdParseError> {
        let start = value.len() - value.trim_start().len();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::MissingId,
                start,
            ));
        }

        let wrapped = !trimmed.starts_with('<');
        let bytes = trimmed.as_bytes();
        let mut unfolded = Vec::with_capacity(bytes.len() + 2);
        if wrapped {
            unfolded.push(b'<');
        }
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == 0 {
                return Err(MessageIdParseError::new(
                    MessageIdParseErrorKind::NulCharacter,
                    start + index,
                ));
            }
            if matches!(byte, b'\r' | b'\n') {
                let offset = start + index;
                if byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                index += 1;
                if !matches!(bytes.get(index), Some(b' ' | b'\t')) {
                    return Err(MessageIdParseError::new(
                        MessageIdParseErrorKind::InvalidLineBreak,
                        offset,
                    ));
                }
            } else {
                unfolded.push(byte);
                index += 1;
            }
        }
        if wrapped {
            unfolded.push(b'>');
        }

        // Reconstruct an original offset only on failure, rather than retaining
        // a per-byte source map for every successfully parsed identifier.
        let original_offset = |offset: usize| {
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| !matches!(byte, b'\r' | b'\n'))
                .nth(offset.saturating_sub(usize::from(wrapped)))
                .map_or(start + bytes.len(), |(index, _)| start + index)
        };
        // Parsed getters never return unquoted whitespace. Do not let implicit
        // wrapping silently turn an assigned bare "a b@c" into "ab@c".
        if wrapped && let Some(offset) = find_unquoted_whitespace(&unfolded) {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::UnquotedWhitespace,
                original_offset(offset),
            ));
        }
        Self::parse(&unfolded).map_err(|mut error| {
            error.offset = original_offset(error.offset);
            error
        })
    }

    /// Parses one complete value, with or without enclosing angle brackets.
    ///
    /// Leading/trailing comments and MIME whitespace are accepted. After the
    /// identifier, any other input (including a second identifier) is an error.
    /// This matches MimeKit's complete-token parser, not its stricter property
    /// setter validation. In particular, an unmatched closing `>` is tolerated.
    ///
    /// # Errors
    ///
    /// Returns [`MessageIdParseError`] for malformed or incomplete tokens,
    /// invalid UTF-8 in a local part or domain atom, or trailing non-comment input.
    pub fn parse(input: &[u8]) -> Result<Self, MessageIdParseError> {
        let (id, mut index) = Self::parse_prefix(input)?;
        skip_cfws(input, &mut index)?;
        if index != input.len() {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::TrailingInput,
                index,
            ));
        }
        Ok(id)
    }

    /// Parses an identifier at the start of a slice and returns bytes consumed.
    ///
    /// Leading comments and MIME whitespace count toward the consumed length.
    /// Parsing stops just after a closing `>` when present; subsequent bytes
    /// are not consumed. Without brackets, the loose local-part grammar may
    /// absorb whitespace-separated text. This does not search for an embedded
    /// identifier or enumerate a References header.
    ///
    /// This is the prefix behavior of MimeKit's `MimeUtils.ParseMessageId`.
    /// The input must contain a complete token; this is not a streaming parser.
    /// On success the consumed length is nonzero and within the input slice.
    ///
    /// # Errors
    ///
    /// Returns [`MessageIdParseError`] on invalid or incomplete input. Offsets
    /// refer to bytes in this slice, not characters in the normalized value.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), MessageIdParseError> {
        let mut index = 0;
        let id = Self::parse_at(input, &mut index)?;
        Ok((id, index))
    }

    // Retain cursor progress on failure: References resumes where MimeKit's
    // parser stopped, not at the diagnostic offset or the next closing '>'.
    fn parse_at(input: &[u8], index: &mut usize) -> Result<Self, MessageIdParseError> {
        skip_cfws(input, index)?;
        if *index == input.len() {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::MissingId,
                *index,
            ));
        }

        let start = *index;
        let angle_addr = input[*index] == b'<';
        if angle_addr {
            *index += 1;
        }
        skip_whitespace(input, index);
        if *index == input.len() {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::IncompleteId,
                *index,
            ));
        }

        // Exchange sometimes wraps the entire local@domain in square brackets.
        // Retain the opening bracket in the loose local part and its closing
        // bracket after the domain (MimeKit issue #912).
        let square_brackets = input[*index] == b'[';
        let mut value = String::new();

        // The local part is intentionally much looser than an address atom:
        // real-world IDs contain punctuation and embedded quotes (#472).
        loop {
            let segment_start = *index;
            let quoted = input[*index] == b'"';
            if quoted {
                skip_quoted(input, index).map_err(|offset| {
                    MessageIdParseError::new(
                        MessageIdParseErrorKind::IncompleteQuotedString,
                        offset,
                    )
                })?;
            } else {
                while *index < input.len()
                    && !matches!(input[*index], b'.' | b'@' | b'>')
                    && !is_whitespace(input[*index])
                {
                    *index += 1;
                }
            }
            let segment = utf8_segment(input, segment_start, *index)?;
            if quoted {
                // Unfold quoted local parts without removing quotes or escapes.
                value.extend(segment.chars().filter(|&ch| ch != '\r' && ch != '\n'));
            } else {
                value.push_str(segment);
            }

            skip_whitespace(input, index);
            if *index == input.len() {
                if angle_addr {
                    return Err(MessageIdParseError::new(
                        MessageIdParseErrorKind::IncompleteId,
                        *index,
                    ));
                }
                break;
            }
            if matches!(input[*index], b'@' | b'>') {
                break;
            }
            if input[*index] == b'.' {
                value.push('.');
                *index += 1;
                skip_whitespace(input, index);
            }
            if *index == input.len() {
                return Err(MessageIdParseError::new(
                    MessageIdParseErrorKind::IncompleteId,
                    *index,
                ));
            }
        }

        if input.get(*index) == Some(&b'@') {
            value.push('@');
            *index += 1;
            // Some broken IDs use <local@@domain>; collapse the repeated
            // initial separators, matching MimeKit issue #962.
            while input.get(*index) == Some(&b'@') {
                *index += 1;
            }
            skip_cfws(input, index)?;
            if *index < input.len() && input[*index] != b'>' {
                // Preserve multiple domains such as <local@domain1@domain2>,
                // as required for compatibility with MailKit issue #138.
                loop {
                    parse_domain(input, index, &mut value)?;
                    if input.get(*index) != Some(&b'@') {
                        break;
                    }
                    value.push('@');
                    *index += 1;
                    if *index == input.len() {
                        break;
                    }
                }
                skip_cfws(input, index)?;
            }
            // An absent domain, as in <local@>, is intentionally accepted
            // for compatibility with MimeKit issue #102.
        }

        if square_brackets && input.get(*index) == Some(&b']') {
            value.push(']');
            *index += 1;
        }
        if angle_addr && input.get(*index) != Some(&b'>') {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::IncompleteId,
                *index,
            ));
        }
        if input.get(*index) == Some(&b'>') {
            *index += 1;
        }
        if value.is_empty() {
            return Err(MessageIdParseError::new(
                MessageIdParseErrorKind::EmptyId,
                start,
            ));
        }

        Ok(Self { value })
    }

    /// Returns the normalized identifier, without outer angle brackets.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Consumes the identifier and returns its normalized string.
    pub fn into_string(self) -> String {
        self.value
    }
}

/// Lazily parses bracketed message identifiers from References or In-Reply-To.
///
/// This follows MimeKit's `MimeUtils.EnumerateReferences` scanning and recovery
/// rules: comments, quoted prose, bare words, and other non-candidate bytes are
/// skipped. Only a `<` outside those constructs starts an identifier. Successful
/// entries use [`MessageId`]'s loose grammar and normalization; order and
/// duplicates are preserved.
///
/// Unlike MimeKit's silently tolerant enumeration, each malformed candidate,
/// unterminated comment, or unterminated quoted string yields an explicit
/// [`MessageIdParseError`]. Its byte offset is relative to the entire supplied
/// slice. Continuing after an error resumes at the parser's stopping point,
/// which may be later than the diagnostic offset. A broken comment or quoted
/// string can consume the rest of the slice, hiding apparent identifiers
/// inside it. An error does not necessarily end iteration.
///
/// This borrows a complete input slice, not a chunk-fed stream. Identifiers
/// are parsed and allocated on demand; no list is built in advance. The iterator
/// is cloneable and fused. It does not validate identifiers for header authoring.
///
/// ```
/// use mimecraft::{MessageId, MessageIdParseErrorKind, References};
///
/// let mut references = References::new(b"prose <first@example.com> <> <last@example.com>");
/// assert_eq!(references.next().unwrap()?.as_str(), "first@example.com");
/// assert_eq!(references.next().unwrap().unwrap_err().kind(),
///            MessageIdParseErrorKind::EmptyId);
/// assert_eq!(references.next().unwrap()?.as_str(), "last@example.com");
/// assert!(references.next().is_none());
///
/// // Collecting Results stops on the first malformed entry.
/// let ids = References::new(b"<one@example.com> <two@example.com>")
///     .collect::<Result<Vec<MessageId>, _>>()?;
/// assert_eq!(ids.len(), 2);
/// # Ok::<(), mimecraft::MessageIdParseError>(())
/// ```
#[derive(Clone, Debug)]
pub struct References<'a> {
    input: &'a [u8],
    index: usize,
}

impl<'a> References<'a> {
    /// Creates a lazy iterator over a complete raw header value.
    ///
    /// For string input, pass `value.as_bytes()`. For a subrange, pass a slice;
    /// any error offsets are relative to that slice. Construction does not
    /// parse or validate the input.
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, index: 0 }
    }
}

impl Iterator for References<'_> {
    type Item = Result<MessageId, MessageIdParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.input.len() {
            if let Err(error) = skip_cfws(self.input, &mut self.index) {
                return Some(Err(error));
            }
            let &byte = self.input.get(self.index)?;
            if byte == b'<' {
                return Some(MessageId::parse_at(self.input, &mut self.index));
            }
            if byte == b'"' {
                if let Err(offset) = skip_quoted(self.input, &mut self.index) {
                    return Some(Err(MessageIdParseError::new(
                        MessageIdParseErrorKind::IncompleteQuotedString,
                        offset,
                    )));
                }
            } else if is_atom(byte) {
                while self
                    .input
                    .get(self.index)
                    .is_some_and(|&byte| is_atom(byte))
                {
                    self.index += 1;
                }
            } else {
                self.index += 1;
            }
        }
        None
    }
}

impl FusedIterator for References<'_> {}

fn find_unquoted_whitespace(input: &[u8]) -> Option<usize> {
    let mut quoted = false;
    let mut index = 0;
    while index < input.len() {
        let byte = input[index];
        if quoted {
            if byte == b'\\' {
                index += 1;
            } else if byte == b'"' {
                quoted = false;
            }
        } else if byte == b'"' {
            quoted = true;
        } else if matches!(byte, b' ' | b'\t') {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn skip_cfws(input: &[u8], index: &mut usize) -> Result<(), MessageIdParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| {
        MessageIdParseError::new(MessageIdParseErrorKind::IncompleteComment, offset)
    })
}

fn utf8_segment(input: &[u8], start: usize, end: usize) -> Result<&str, MessageIdParseError> {
    std::str::from_utf8(&input[start..end]).map_err(|error| {
        MessageIdParseError::new(
            MessageIdParseErrorKind::InvalidUtf8,
            start + error.valid_up_to(),
        )
    })
}

fn parse_domain(
    input: &[u8],
    index: &mut usize,
    output: &mut String,
) -> Result<(), MessageIdParseError> {
    crate::parse_utils::parse_domain(input, index, b">@", output).map_err(|error| {
        let kind = match error.kind {
            DomainParseErrorKind::InvalidDomain => MessageIdParseErrorKind::InvalidDomain,
            DomainParseErrorKind::IncompleteLiteral => {
                MessageIdParseErrorKind::IncompleteDomainLiteral
            }
            DomainParseErrorKind::IncompleteComment => MessageIdParseErrorKind::IncompleteComment,
            DomainParseErrorKind::InvalidUtf8 => MessageIdParseErrorKind::InvalidUtf8,
        };
        MessageIdParseError::new(kind, error.offset)
    })
}

impl FromStr for MessageId {
    type Err = MessageIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

/// The reason a message identifier could not be parsed or accepted for assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageIdParseErrorKind {
    /// No identifier followed leading comments and whitespace.
    MissingId,
    /// An identifier ended prematurely or an expected closing `>` was absent.
    IncompleteId,
    /// The parsed token contained no identifier bytes.
    EmptyId,
    /// A comment was not closed.
    IncompleteComment,
    /// A quoted string (a local-part segment or reference-list prose) was not closed.
    IncompleteQuotedString,
    /// A domain atom or literal contained an invalid byte or lacked an atom.
    InvalidDomain,
    /// A domain literal was not closed with `]`.
    IncompleteDomainLiteral,
    /// A local-part segment or domain atom contained invalid UTF-8.
    InvalidUtf8,
    /// A complete-value parse found non-comment data after the identifier.
    TrailingInput,
    /// An assigned value contained a NUL character.
    NulCharacter,
    /// An assigned value contained an interior CR/LF not followed by space or tab.
    InvalidLineBreak,
    /// An assigned unbracketed value contained unquoted space or tab.
    UnquotedWhitespace,
}

/// An identifier parsing or assignment failure and its zero-based byte offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageIdParseError {
    kind: MessageIdParseErrorKind,
    offset: usize,
}

impl MessageIdParseError {
    fn new(kind: MessageIdParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the reason parsing failed.
    pub fn kind(self) -> MessageIdParseErrorKind {
        self.kind
    }

    /// Returns a byte offset relative to the supplied input.
    ///
    /// Unclosed comments, quotes, and domain literals point to their opener;
    /// empty IDs point to the token start. Other errors point to the offending
    /// byte or to the input length when required input is missing.
    /// For [`MessageId::from_value`], offsets refer to the original string;
    /// synthetic brackets map to the trimmed value's start or end.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for MessageIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            MessageIdParseErrorKind::MissingId => "no message identifier found",
            MessageIdParseErrorKind::IncompleteId => "incomplete message identifier",
            MessageIdParseErrorKind::EmptyId => "empty message identifier",
            MessageIdParseErrorKind::IncompleteComment => "unterminated comment",
            MessageIdParseErrorKind::IncompleteQuotedString => "unterminated quoted string",
            MessageIdParseErrorKind::InvalidDomain => "invalid message identifier domain",
            MessageIdParseErrorKind::IncompleteDomainLiteral => "unterminated domain literal",
            MessageIdParseErrorKind::InvalidUtf8 => "invalid UTF-8 in message identifier",
            MessageIdParseErrorKind::TrailingInput => "unexpected input after message identifier",
            MessageIdParseErrorKind::NulCharacter => "NUL in assigned message identifier",
            MessageIdParseErrorKind::InvalidLineBreak => {
                "non-folding line break in assigned message identifier"
            }
            MessageIdParseErrorKind::UnquotedWhitespace => {
                "unquoted whitespace in assigned message identifier"
            }
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for MessageIdParseError {}

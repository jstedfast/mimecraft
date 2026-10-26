// Adapted from MimeKit's Received.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::{
    DateTimeOffset,
    charsets::{Charset, decode_best},
    parse_utils::{is_whitespace, skip_whitespace},
};

const KEYWORDS: [&str; 6] = ["from", "by", "via", "with", "id", "for"];

/// The recognized clauses in a Received trace header.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReceivedClauseId {
    /// Comments preceding the first keyword, with an empty keyword and value.
    None,
    /// The sending host.
    From,
    /// The receiving host.
    By,
    /// The physical link or transport.
    Via,
    /// The mail protocol or mechanism.
    With,
    /// A server-assigned message or queue identifier.
    Id,
    /// The recipient.
    For,
    /// An unrecognized keyword, retained with its value and comments.
    Unknown,
}

impl ReceivedClauseId {
    fn from_keyword(keyword: &[u8]) -> Self {
        match KEYWORDS
            .iter()
            .position(|word| keyword.eq_ignore_ascii_case(word.as_bytes()))
        {
            Some(0) => Self::From,
            Some(1) => Self::By,
            Some(2) => Self::Via,
            Some(3) => Self::With,
            Some(4) => Self::Id,
            Some(5) => Self::For,
            _ => Self::Unknown,
        }
    }
}

/// Character decoding preferences for [`Received`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReceivedParseOptions {
    /// Preferred charset for tokens/comments that are not valid UTF-8.
    ///
    /// UTF-8 always wins when valid; otherwise an error-free preferred decoding
    /// is used, with lossless Latin-1 as the final fallback. Defaults to `None`.
    /// Encoded words are not interpreted.
    pub fallback_charset: Option<Charset>,
}

/// One parsed Received clause, without authoring validation.
///
/// Keyword spelling is retained. Value tokens are joined with one ASCII space.
/// Comments may precede or follow the value and retain their encounter order.
/// The outer parentheses and escape backslashes are removed; nested parentheses
/// remain. CR/LF are dropped and tabs become spaces, including escaped whitespace.
/// These strings are inspection data, not safe-to-insert header text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedClause {
    id: ReceivedClauseId,
    keyword: String,
    value: String,
    comments: Vec<String>,
}

impl ReceivedClause {
    /// Returns the case-insensitive keyword classification.
    pub fn id(&self) -> ReceivedClauseId {
        self.id
    }

    /// Returns the original decoded keyword spelling.
    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    /// Returns the decoded value, possibly empty for a missing value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns all decoded comments in source order, without outer parentheses.
    pub fn comments(&self) -> &[String] {
        &self.comments
    }
}

/// A read-only parsed Received header value.
///
/// Retains clause order, unknown keywords, comments, and the unfolded date
/// spelling. Known keywords are ASCII case-insensitive and may occur only once,
/// in any order. Unknown keywords may repeat. Leading comments form a
/// [`ReceivedClauseId::None`] clause. Empty input, absent dates, and empty
/// values are accepted; a present, nonempty date must parse.
///
/// This follows MimeKit's tolerant trace-header reader, not strict SMTP grammar.
/// Multi-word values such as `with Microsoft SMTP Server` are accepted. A known
/// keyword always starts the next clause, even if it might be a value.
/// Parentheses start comments and semicolons start dates; quotes and address
/// literals do not shield these delimiters. Hosts, protocols, identifiers, and
/// recipients are not validated. A successful parse does not authenticate a hop
/// or establish that its content is trustworthy.
///
/// Raw fields are decoded using [`crate::charsets::decode_best`], independently
/// for each keyword, value token, complete comment, and date. Encoded words and
/// IDNA domains are not decoded. [`Self::had_charset_fallback`] reports recovery.
/// Raw bytes are not retained. Comment nesting uses an iterative counter; memory
/// usage is proportional to input size. No stream I/O, editing, serialization,
/// or automatic message-header processing is performed.
///
/// ```
/// use mimecraft::{Header, Received};
///
/// let header = Header::parse(
///     b"Received: from source.example ([192.0.2.1]) by target.example with ESMTP; Tue, 30 Apr 2019 19:10:19 +0000"
/// )?;
/// let received = Received::parse(header.raw_value())?;
/// assert_eq!(received.from(), Some("source.example"));
/// assert_eq!(received.from_tcp_info(), Some("[192.0.2.1]"));
/// assert_eq!(received.by(), Some("target.example"));
/// assert_eq!(received.with(), Some("ESMTP"));
/// assert_eq!(received.date_time().unwrap().year(), 2019);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Received {
    clauses: Vec<ReceivedClause>,
    date_time: Option<DateTimeOffset>,
    date_time_text: Option<String>,
    had_charset_fallback: bool,
}

impl Received {
    /// Parses a complete raw field value, excluding the `Received:` name.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedParseError`] for an unterminated comment, a duplicate
    /// known clause, or an unparseable nonempty date after the semicolon.
    pub fn parse(input: &[u8]) -> Result<Self, ReceivedParseError> {
        Self::parse_with_options(input, &ReceivedParseOptions::default())
    }

    /// Parses a complete raw field value with explicit charset preferences.
    ///
    /// Whitespace means SP, TAB, CR, or LF. Comments normalize whitespace before
    /// charset decoding, matching MimeKit. Date parsing uses [`DateTimeOffset`]'s
    /// tolerant recovery; the separately retained date spelling removes CR/LF
    /// and converts tabs to spaces, but does not collapse spaces or canonicalize
    /// the date. An empty suffix after `;` is equivalent to no date.
    ///
    /// Pass a subslice to parse a byte range. Error offsets are relative to that
    /// slice, not to a larger enclosing header or message.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedParseError`] under the same conditions as [`Self::parse`].
    pub fn parse_with_options(
        input: &[u8],
        options: &ReceivedParseOptions,
    ) -> Result<Self, ReceivedParseError> {
        Parser {
            input,
            index: 0,
            options,
            had_charset_fallback: false,
        }
        .parse()
    }

    /// Returns all clauses in source order, including leading comments.
    pub fn clauses(&self) -> &[ReceivedClause] {
        &self.clauses
    }

    /// Returns the first clause with this classification, if present.
    pub fn clause(&self, id: ReceivedClauseId) -> Option<&ReceivedClause> {
        self.clauses.iter().find(|clause| clause.id == id)
    }

    /// Returns the sending host, without domain or IP validation.
    pub fn from(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::From)
            .map(ReceivedClause::value)
    }

    /// Returns the first comment on the sending-host clause, if any.
    pub fn from_tcp_info(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::From)?
            .comments
            .first()
            .map(String::as_str)
    }

    /// Returns the receiving host, without domain or IP validation.
    pub fn by(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::By).map(ReceivedClause::value)
    }

    /// Returns the first comment on the receiving-host clause, if any.
    pub fn by_tcp_info(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::By)?
            .comments
            .first()
            .map(String::as_str)
    }

    /// Returns the physical link or transport value.
    pub fn via(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::Via)
            .map(ReceivedClause::value)
    }

    /// Returns the mail protocol or mechanism value.
    pub fn with(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::With)
            .map(ReceivedClause::value)
    }

    /// Returns the server's identifier value, without Message-Id validation.
    pub fn id(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::Id).map(ReceivedClause::value)
    }

    /// Returns the `for` clause value, without mailbox parsing or validation.
    pub fn for_recipient(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::For)
            .map(ReceivedClause::value)
    }

    /// Returns the parsed date and fixed UTC offset, when supplied.
    pub fn date_time(&self) -> Option<DateTimeOffset> {
        self.date_time
    }

    /// Returns the decoded, unfolded date spelling, without its semicolon.
    ///
    /// Preserves named zones, comments, and noncanonical date components rather
    /// than using [`DateTimeOffset`]'s canonical display representation.
    pub fn date_time_text(&self) -> Option<&str> {
        self.date_time_text.as_deref()
    }

    /// Reports whether any token, comment, or date used non-UTF-8 decoding.
    ///
    /// This includes both preferred-charset decoding and the Latin-1 fallback.
    /// It does not report normalized whitespace or tolerated syntax.
    pub fn had_charset_fallback(&self) -> bool {
        self.had_charset_fallback
    }
}

impl FromStr for Received {
    type Err = ReceivedParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

/// The reason a Received header value could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceivedParseErrorKind {
    /// A comment's closing parenthesis was missing.
    IncompleteComment,
    /// A recognized clause occurred more than once.
    DuplicateClause(ReceivedClauseId),
    /// A nonempty date suffix could not be interpreted.
    InvalidDateTime,
}

/// A Received parsing failure with zero-based source byte offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceivedParseError {
    kind: ReceivedParseErrorKind,
    token_offset: usize,
    offset: usize,
}

impl ReceivedParseError {
    /// Returns the reason parsing failed.
    pub fn kind(self) -> ReceivedParseErrorKind {
        self.kind
    }

    /// Returns the start of the offending keyword, comment, or date.
    pub fn token_offset(self) -> usize {
        self.token_offset
    }

    /// Returns where the error was detected in the supplied slice.
    ///
    /// This is EOF for an incomplete comment, the exclusive end of a duplicate
    /// keyword, or the end of the nonempty date after trimming ASCII whitespace.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for ReceivedParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ReceivedParseErrorKind::IncompleteComment => {
                f.write_str("unterminated Received comment")?
            }
            ReceivedParseErrorKind::DuplicateClause(id) => {
                write!(f, "duplicate Received {id:?} clause")?
            }
            ReceivedParseErrorKind::InvalidDateTime => f.write_str("invalid Received date-time")?,
        }
        write!(f, " at byte {}", self.token_offset)
    }
}

impl Error for ReceivedParseError {}

struct Parser<'a> {
    input: &'a [u8],
    index: usize,
    options: &'a ReceivedParseOptions,
    had_charset_fallback: bool,
}

impl Parser<'_> {
    fn decode(&mut self, value: &[u8]) -> String {
        let (text, charset) = decode_best(value, self.options.fallback_charset);
        self.had_charset_fallback |= charset != Charset::UTF8;
        text.into_owned()
    }

    fn token(&self) -> &[u8] {
        let remaining = &self.input[self.index..];
        let length = remaining
            .iter()
            .position(|byte| b"\t\r\n (;".contains(byte))
            .unwrap_or(remaining.len());
        &remaining[..length]
    }

    fn keyword(&mut self) -> (ReceivedClauseId, String) {
        let token = self.token();
        let id = ReceivedClauseId::from_keyword(token);
        let length = token.len();
        let text = self.decode(&self.input[self.index..self.index + length]);
        self.index += length;
        (id, text)
    }

    fn value(&mut self) -> Option<String> {
        let token = self.token();
        if ReceivedClauseId::from_keyword(token) != ReceivedClauseId::Unknown {
            return None;
        }
        let length = token.len();
        let text = self.decode(&self.input[self.index..self.index + length]);
        self.index += length;
        Some(text)
    }

    fn comment(&mut self) -> Result<String, ReceivedParseError> {
        let start = self.index;
        // Skip over the '('.
        self.index += 1;
        let mut bytes = Vec::new();
        let mut escaped = false;
        let mut depth = 1usize;
        while let Some(&byte) = self.input.get(self.index) {
            if !escaped {
                match byte {
                    b'\\' => {
                        // Escape sequence.
                        escaped = true;
                        self.index += 1;
                        continue;
                    }
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            // Skip over ')'.
                            self.index += 1;
                            return Ok(self.decode(&bytes));
                        }
                    }
                    _ => {}
                }
            }
            if !is_whitespace(byte) {
                bytes.push(byte);
            } else if !matches!(byte, b'\r' | b'\n') {
                bytes.push(b' ');
            }
            escaped = false;
            self.index += 1;
        }
        Err(ReceivedParseError {
            kind: ReceivedParseErrorKind::IncompleteComment,
            token_offset: start,
            offset: self.index,
        })
    }

    fn comments(&mut self, comments: &mut Vec<String>) -> Result<(), ReceivedParseError> {
        loop {
            skip_whitespace(self.input, &mut self.index);
            if self.input.get(self.index) != Some(&b'(') {
                return Ok(());
            }
            comments.push(self.comment()?);
        }
    }

    fn parse(mut self) -> Result<Received, ReceivedParseError> {
        let mut clauses = Vec::new();
        let mut mask = 0u8;
        let mut leading_comments = Vec::new();

        // Skip leading whitespace, collecting any comments along the way.
        self.comments(&mut leading_comments)?;
        if !leading_comments.is_empty() {
            clauses.push(ReceivedClause {
                id: ReceivedClauseId::None,
                keyword: String::new(),
                value: String::new(),
                comments: leading_comments,
            });
        }

        // Continue parsing clauses until ';' or the end of the header value.
        while self.index < self.input.len() && self.input[self.index] != b';' {
            let keyword_start = self.index;
            // Parse the keyword token.
            let (id, keyword) = self.keyword();

            // Update the parser state and check for duplicate known clauses.
            if id != ReceivedClauseId::Unknown {
                let bit = 1u8 << id as u8;
                if mask & bit != 0 {
                    return Err(ReceivedParseError {
                        kind: ReceivedParseErrorKind::DuplicateClause(id),
                        token_offset: keyword_start,
                        offset: self.index,
                    });
                }
                mask |= bit;
            }

            let mut comments = Vec::new();
            // Skip whitespace, collecting comments before the value.
            self.comments(&mut comments)?;
            let mut value = String::new();
            // Check for the end of the header or ';' before parsing a value.
            // If value() returns None, the next token looks like a keyword;
            // retain an empty clause and leave that token for the next pass.
            if self.index < self.input.len()
                && self.input[self.index] != b';'
                && let Some(first) = self.value()
            {
                value = first;
                // Some servers use multi-string values, e.g. "with Microsoft
                // SMTP Server" or "via Frontend Transport".
                loop {
                    // Skip whitespace after the value token.
                    skip_whitespace(self.input, &mut self.index);
                    // Stop at EOF, ';', a comment, or the next known keyword.
                    if self.index == self.input.len()
                        || matches!(self.input[self.index], b';' | b'(')
                    {
                        break;
                    }
                    // Check that the next token is not a keyword, then append it.
                    let Some(token) = self.value() else { break };
                    value.push(' ');
                    value.push_str(&token);
                }
                // Parse any comments after the value.
                self.comments(&mut comments)?;
            }
            clauses.push(ReceivedClause {
                id,
                keyword,
                value,
                comments,
            });
        }

        let mut date_time = None;
        let mut date_time_text = None;
        if self.index < self.input.len() {
            self.index += 1;
            skip_whitespace(self.input, &mut self.index);
            let mut end = self.input.len();
            // Trim trailing whitespace.
            while end > self.index && is_whitespace(self.input[end - 1]) {
                end -= 1;
            }
            if self.index < end {
                date_time = Some(DateTimeOffset::parse(&self.input[self.index..end]).map_err(
                    |_| ReceivedParseError {
                        kind: ReceivedParseErrorKind::InvalidDateTime,
                        token_offset: self.index,
                        offset: end,
                    },
                )?);
                // Cache the formatting of the date. Unlike general header
                // unfolding, Received also replaces continuation tabs with spaces.
                let text = self.decode(&self.input[self.index..end]);
                date_time_text = Some(
                    text.chars()
                        .filter_map(|ch| match ch {
                            '\r' | '\n' => None,
                            '\t' => Some(' '),
                            _ => Some(ch),
                        })
                        .collect(),
                );
            }
        }
        Ok(Received {
            clauses,
            date_time,
            date_time_text,
            had_charset_fallback: self.had_charset_fallback,
        })
    }
}

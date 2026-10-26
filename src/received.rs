// Adapted from MimeKit's Received.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, net::IpAddr, str::FromStr};

use crate::{
    DateTimeOffset, MailboxAddress, MailboxParseError, MessageId, MessageIdParseError,
    ReceivedFormatError, ReceivedFormatOptions,
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

/// One Received clause, with checked construction and tolerant parsing.
///
/// [`Self::new`] and [`Self::set_value`] apply MimeKit's clause-specific assignment
/// checks, not strict SMTP validation. Assigned spelling is retained verbatim.
/// Comments are freely editable and are not authoring-validated.
///
/// When parsed, keyword spelling is retained and value tokens are joined with
/// one ASCII space.
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
    /// Constructs a clause, preserving keyword and value spelling.
    ///
    /// Keywords contain ASCII letters and hyphens, beginning with a letter.
    /// Recognized keywords are classified without regard to ASCII case.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an invalid keyword or value. See
    /// [`Self::set_value`] for the clause-specific assignment rules.
    pub fn new(keyword: &str, value: &str) -> Result<Self, ReceivedEditError> {
        let id = validate_keyword(keyword)?;
        // Like MimeKit's authoring constructors, validate the value; the parser
        // constructs the fields directly so it can retain malformed input.
        validate_value(id, value)?;
        Ok(Self {
            id,
            keyword: keyword.to_owned(),
            value: value.to_owned(),
            comments: Vec::new(),
        })
    }

    /// Constructs a checked clause with one unvalidated, unmodified comment.
    ///
    /// The comment is its content, without enclosing parentheses. Unlike TCP-info
    /// setters, this does not unfold it, matching MimeKit's clause constructor.
    ///
    /// # Errors
    ///
    /// Returns the same keyword/value errors as [`Self::new`].
    pub fn with_comment(
        keyword: &str,
        value: &str,
        comment: &str,
    ) -> Result<Self, ReceivedEditError> {
        let mut clause = Self::new(keyword, value)?;
        clause.comments.push(comment.to_owned());
        Ok(clause)
    }

    /// Returns the case-insensitive keyword classification.
    pub fn id(&self) -> ReceivedClauseId {
        self.id
    }

    /// Returns the stored keyword spelling.
    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    /// Returns the stored value, possibly empty for a missing parsed value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Checks and replaces the value without changing its spelling.
    ///
    /// Values must be nonempty and contain no ASCII controls (including TAB).
    /// `from`/`by` use MimeKit's permissive hostname checks; bracketed values
    /// must contain an IP address accepted by [`IpAddr`]. This excludes .NET's
    /// legacy abbreviated, octal/hex IPv4 and scoped IPv6 forms.
    /// `id` uses the complete, tolerant [`MessageId::parse`] grammar, and `for`
    /// uses [`MailboxAddress::parse`]. Unlike MimeKit, assigned mailbox strings
    /// also reject controls because the original value is retained verbatim.
    /// Other clauses additionally reject `(`, `)`, and `;`, but allow spaces.
    ///
    /// These are compatibility checks, not DNS, IDNA, SMTP, or safe header
    /// serialization. In particular, hostnames are not limited to DNS letters,
    /// digits and hyphens. Parsed clauses bypass these checks.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] without changing the clause on failure.
    pub fn set_value(&mut self, value: &str) -> Result<(), ReceivedEditError> {
        validate_value(self.id, value)?;
        self.value = value.to_owned();
        Ok(())
    }

    /// Returns stored comment contents in their current order.
    pub fn comments(&self) -> &[String] {
        &self.comments
    }

    /// Exposes ordinary comment edits, without validation or unfolding.
    ///
    /// Store comment content without enclosing parentheses. Direct edits can
    /// contain controls and are not safe-to-insert header text.
    pub fn comments_mut(&mut self) -> &mut Vec<String> {
        &mut self.comments
    }
}

/// A Received header value with tolerant parsing and checked convenience setters.
///
/// Parsing retains clause order, unknown keywords, comments, and the unfolded date
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
/// usage is proportional to input size.
///
/// [`Self::new`] constructs an empty value. Convenience setters validate assigned
/// values, update the first matching clause, and sort clauses by classification
/// only when adding a new clause. [`Self::clauses_mut`] permits ordinary vector
/// edits, including duplicates and reordered parsed clauses. Assignment checks
/// do not make arbitrary parsed or edited content safe to serialize.
/// [`Self::to_header_string`] separately checks emitted text and physical-line
/// lengths. No stream I/O or automatic message-header processing is performed.
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
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Received {
    clauses: Vec<ReceivedClause>,
    date_time: Option<DateTimeOffset>,
    date_time_text: Option<String>,
    had_charset_fallback: bool,
}

impl Received {
    /// Constructs an empty value with no clauses, date, or recovery diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Formats a complete `Received:` header with default options.
    ///
    /// Uses ASCII output, CRLF, and a maximum of 78 bytes per physical line.
    /// Includes the field name and exactly one terminating newline.
    ///
    /// ```
    /// use mimecraft::Received;
    ///
    /// let mut received = Received::new();
    /// received.set_from(Some("source.example"))?;
    /// received.set_by(Some("target.example"))?;
    /// assert_eq!(received.to_header_string()?,
    ///     "Received: from source.example by target.example\r\n");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the text or line-length errors documented on
    /// [`Self::to_header_string_with_options`].
    pub fn to_header_string(&self) -> Result<String, ReceivedFormatError> {
        self.to_header_string_with_options(&ReceivedFormatOptions::default())
    }

    /// Formats a complete header with explicit line-ending and UTF-8 options.
    ///
    /// Preserves current clause order, keyword/value spelling, and cached date
    /// spelling. Authored dates use [`DateTimeOffset`]'s canonical format.
    /// Leading-comment clauses emit only their comments. Empty values and
    /// duplicate clauses are retained; this is not strict trace validation.
    ///
    /// Folds between keywords, complete values, comments, and the date; it never
    /// splits their contents. Comments drop CR/LF, change tabs to spaces, and
    /// escape backslashes. Balanced nested parentheses remain; if unbalanced,
    /// all parentheses are escaped to keep the content inside its comment.
    /// Other ASCII controls are rejected. Output is ASCII unless
    /// [`ReceivedFormatOptions::international`] is enabled; there is no IDNA or
    /// encoded-word conversion. These checks do not authenticate trace data or
    /// guarantee that directly edited values satisfy the parser's clause grammar.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedFormatError`] for invalid options, forbidden controls,
    /// non-ASCII text without explicit opt-in, or a piece that cannot fit on a
    /// continuation line. No partial output is returned and nothing is mutated.
    pub fn to_header_string_with_options(
        &self,
        options: &ReceivedFormatOptions,
    ) -> Result<String, ReceivedFormatError> {
        crate::received_format::format_header(self, options)
    }

    /// Constructs `from`/`by` clauses with bracketed IP comments and a date.
    ///
    /// IP comments use Rust's canonical address spelling, without an `IPv6:`
    /// prefix. Host spelling is retained.
    ///
    /// ```
    /// use mimecraft::{DateTimeOffset, Received};
    ///
    /// let mut received = Received::with_hosts(
    ///     "source.example", "192.0.2.1".parse()?,
    ///     "target.example", "127.0.0.1".parse()?,
    ///     DateTimeOffset::new(2026, 4, 18, 20, 5, 38, -240)?,
    /// )?;
    /// received.set_with(Some("ESMTPS"))?;
    /// received.set_for_recipient(Some("user@example.com"))?;
    /// assert_eq!(received.from_tcp_info(), Some("[192.0.2.1]"));
    /// assert_eq!(received.with(), Some("ESMTPS"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] when either host fails the assignment
    /// checks documented on [`ReceivedClause::set_value`].
    pub fn with_hosts(
        from: &str,
        from_ip: IpAddr,
        by: &str,
        by_ip: IpAddr,
        date_time: DateTimeOffset,
    ) -> Result<Self, ReceivedEditError> {
        Ok(Self {
            clauses: vec![
                ReceivedClause::with_comment("from", from, &format!("[{from_ip}]"))?,
                ReceivedClause::with_comment("by", by, &format!("[{by_ip}]"))?,
            ],
            date_time: Some(date_time),
            ..Self::default()
        })
    }

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

    /// Returns all clauses in their current order, including leading comments.
    pub fn clauses(&self) -> &[ReceivedClause] {
        &self.clauses
    }

    /// Exposes ordinary vector edits, without duplicate or ordering checks.
    ///
    /// Constructors and value setters still validate individual assignments,
    /// but parsed clauses can contain unchecked values. Convenience getters,
    /// updates, and removals act on the first matching clause if duplicates exist.
    pub fn clauses_mut(&mut self) -> &mut Vec<ReceivedClause> {
        &mut self.clauses
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

    /// Sets the sending host, or removes its first clause and comments for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an invalid host, without mutation.
    /// See [`ReceivedClause::set_value`] for the compatibility rules.
    pub fn set_from(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::From, "from", value)
    }

    /// Returns the first comment on the sending-host clause, if any.
    pub fn from_tcp_info(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::From)?
            .comments
            .first()
            .map(String::as_str)
    }

    /// Replaces all sending-host comments with one unfolded comment, or clears them.
    ///
    /// CR/LF are removed and tabs become spaces; other bytes and outer spaces
    /// are retained. A non-`None` comment creates a missing `from` clause with
    /// an empty value. `None` does not create or remove a clause. No IP or
    /// comment-syntax validation is performed.
    pub fn set_from_tcp_info(&mut self, comment: Option<&str>) {
        self.set_clause_comment(ReceivedClauseId::From, "from", comment);
    }

    /// Returns the receiving host, without domain or IP validation.
    pub fn by(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::By).map(ReceivedClause::value)
    }

    /// Sets the receiving host, or removes its first clause and comments for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an invalid host, without mutation.
    /// See [`ReceivedClause::set_value`] for the compatibility rules.
    pub fn set_by(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::By, "by", value)
    }

    /// Returns the first comment on the receiving-host clause, if any.
    pub fn by_tcp_info(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::By)?
            .comments
            .first()
            .map(String::as_str)
    }

    /// Replaces all receiving-host comments, following [`Self::set_from_tcp_info`].
    pub fn set_by_tcp_info(&mut self, comment: Option<&str>) {
        self.set_clause_comment(ReceivedClauseId::By, "by", comment);
    }

    /// Returns the physical link or transport value.
    pub fn via(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::Via)
            .map(ReceivedClause::value)
    }

    /// Sets the transport value, or removes its first clause for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an empty value, ASCII control, `(`,
    /// `)`, or `;`, without mutation. Multi-word values are accepted.
    pub fn set_via(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::Via, "via", value)
    }

    /// Returns the mail protocol or mechanism value.
    pub fn with(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::With)
            .map(ReceivedClause::value)
    }

    /// Sets the protocol value, or removes its first clause for `None`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::set_via`], without mutation.
    pub fn set_with(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::With, "with", value)
    }

    /// Returns the server's identifier value, without Message-Id validation.
    pub fn id(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::Id).map(ReceivedClause::value)
    }

    /// Sets an identifier, retaining spelling, or removes its first clause for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an empty value, ASCII control, or failure
    /// of [`MessageId::parse`], without mutation.
    pub fn set_id(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::Id, "id", value)
    }

    /// Returns the `for` clause value, without mailbox parsing or validation.
    pub fn for_recipient(&self) -> Option<&str> {
        self.clause(ReceivedClauseId::For)
            .map(ReceivedClause::value)
    }

    /// Sets a mailbox string, retaining spelling, or removes its first clause for `None`.
    ///
    /// # Errors
    ///
    /// Returns [`ReceivedEditError`] for an empty value, ASCII control, or failure
    /// of [`MailboxAddress::parse`], without mutation. Parsing does not validate
    /// SMTP deliverability or normalize the retained spelling.
    pub fn set_for_recipient(&mut self, value: Option<&str>) -> Result<(), ReceivedEditError> {
        self.set_clause_value(ReceivedClauseId::For, "for", value)
    }

    /// Returns the parsed date and fixed UTC offset, when supplied.
    pub fn date_time(&self) -> Option<DateTimeOffset> {
        self.date_time
    }

    /// Replaces or removes the date and discards its cached parsed spelling.
    ///
    /// Even assigning the same date clears [`Self::date_time_text`]. Editing
    /// other properties leaves the cached date spelling intact.
    pub fn set_date_time(&mut self, value: Option<DateTimeOffset>) {
        self.date_time = value;
        self.date_time_text = None;
    }

    /// Returns the decoded, unfolded date spelling, without its semicolon.
    ///
    /// Preserves named zones, comments, and noncanonical date components rather
    /// than using [`DateTimeOffset`]'s canonical display representation.
    /// Returns `None` for authored dates and after [`Self::set_date_time`].
    pub fn date_time_text(&self) -> Option<&str> {
        self.date_time_text.as_deref()
    }

    /// Reports whether any token, comment, or date used non-UTF-8 decoding.
    ///
    /// This includes both preferred-charset decoding and the Latin-1 fallback.
    /// It does not report normalized whitespace or tolerated syntax.
    /// This records the original parse; subsequent edits do not clear it, nor
    /// do copied clauses transfer their source object's diagnostics.
    pub fn had_charset_fallback(&self) -> bool {
        self.had_charset_fallback
    }

    fn set_clause_value(
        &mut self,
        id: ReceivedClauseId,
        keyword: &str,
        value: Option<&str>,
    ) -> Result<(), ReceivedEditError> {
        let index = self.clauses.iter().position(|clause| clause.id == id);
        match (index, value) {
            (Some(index), Some(value)) => self.clauses[index].set_value(value)?,
            (Some(index), None) => {
                self.clauses.remove(index);
            }
            (None, Some(value)) => {
                let clause = ReceivedClause::new(keyword, value)?;
                self.clauses.push(clause);
                self.clauses.sort_by_key(|clause| clause.id as u8);
            }
            (None, None) => {}
        }
        Ok(())
    }

    fn set_clause_comment(&mut self, id: ReceivedClauseId, keyword: &str, comment: Option<&str>) {
        let comment = comment.map(unfold);
        if let Some(clause) = self.clauses.iter_mut().find(|clause| clause.id == id) {
            clause.comments.clear();
            clause.comments.extend(comment);
        } else if let Some(comment) = comment {
            // Like MimeKit's parser-only constructor, bypass value validation
            // so a TCP-info comment can exist before its host value is assigned.
            self.clauses.push(ReceivedClause {
                id,
                keyword: keyword.to_owned(),
                value: String::new(),
                comments: vec![comment],
            });
            self.clauses.sort_by_key(|clause| clause.id as u8);
        }
    }
}

impl FromStr for Received {
    type Err = ReceivedParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

/// A Received clause could not be constructed or assigned.
///
/// Failed assignments leave the original object unchanged. Offsets refer to
/// UTF-8 bytes in the supplied keyword or value, without normalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceivedEditError {
    /// A keyword is empty, starts with a hyphen, or contains other than ASCII letters/hyphens.
    InvalidKeyword {
        /// Invalid byte offset, or zero for an empty keyword.
        offset: usize,
    },
    /// The assigned value is empty.
    EmptyValue {
        /// Clause being assigned.
        clause: ReceivedClauseId,
    },
    /// A `from` or `by` value failed the hostname/IP-literal checks.
    InvalidDomain {
        /// Clause being assigned.
        clause: ReceivedClauseId,
    },
    /// The value contains an ASCII control or a forbidden clause delimiter.
    InvalidValue {
        /// Clause being assigned.
        clause: ReceivedClauseId,
        /// Invalid UTF-8 byte offset.
        offset: usize,
    },
    /// The complete identifier parser rejected an `id` value.
    MessageId(MessageIdParseError),
    /// The mailbox parser rejected a `for` value.
    Mailbox(MailboxParseError),
}

impl fmt::Display for ReceivedEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKeyword { offset } => {
                write!(f, "invalid Received keyword at byte {offset}")
            }
            Self::EmptyValue { clause } => write!(f, "empty Received {clause:?} value"),
            Self::InvalidDomain { clause } => write!(f, "invalid Received {clause:?} domain"),
            Self::InvalidValue { clause, offset } => {
                write!(f, "invalid Received {clause:?} value at byte {offset}")
            }
            Self::MessageId(error) => write!(f, "invalid Received identifier: {error}"),
            Self::Mailbox(error) => write!(f, "invalid Received recipient: {error}"),
        }
    }
}

impl Error for ReceivedEditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MessageId(error) => Some(error),
            Self::Mailbox(error) => Some(error),
            _ => None,
        }
    }
}

fn validate_keyword(keyword: &str) -> Result<ReceivedClauseId, ReceivedEditError> {
    if keyword.is_empty() || keyword.starts_with('-') {
        return Err(ReceivedEditError::InvalidKeyword { offset: 0 });
    }
    if let Some(offset) = keyword
        .bytes()
        .position(|byte| !byte.is_ascii_alphabetic() && byte != b'-')
    {
        return Err(ReceivedEditError::InvalidKeyword { offset });
    }
    Ok(ReceivedClauseId::from_keyword(keyword.as_bytes()))
}

fn validate_value(id: ReceivedClauseId, value: &str) -> Result<(), ReceivedEditError> {
    if value.is_empty() {
        return Err(ReceivedEditError::EmptyValue { clause: id });
    }
    // MimeKit checks identifier controls because the value is serialized
    // verbatim. Apply that safeguard to mailbox assignments as well: parsing
    // alone may accept controls or folded input while we retain the original.
    if let Some(offset) = value.bytes().position(|byte| byte.is_ascii_control()) {
        return Err(ReceivedEditError::InvalidValue { clause: id, offset });
    }
    match id {
        ReceivedClauseId::From | ReceivedClauseId::By => {
            let valid = if let Some(literal) = value.strip_prefix('[') {
                literal
                    .strip_suffix(']')
                    .is_some_and(|ip| ip.parse::<IpAddr>().is_ok())
            } else {
                let mut dot = true;
                value.chars().all(|ch| {
                    if ch <= ' ' || matches!(ch, '[' | '\\' | ']') {
                        return false;
                    }
                    if dot {
                        if matches!(ch, '.' | '-') {
                            return false;
                        }
                        dot = false;
                    } else if ch == '.' {
                        dot = true;
                    }
                    true
                })
            };
            if !valid {
                return Err(ReceivedEditError::InvalidDomain { clause: id });
            }
        }
        ReceivedClauseId::Id => {
            MessageId::parse(value.as_bytes()).map_err(ReceivedEditError::MessageId)?;
        }
        ReceivedClauseId::For => {
            // Validate the value by parsing it as a mailbox address.
            MailboxAddress::parse(value.as_bytes()).map_err(ReceivedEditError::Mailbox)?;
        }
        _ => {
            if let Some(offset) = value.bytes().position(|byte| b"();".contains(&byte)) {
                return Err(ReceivedEditError::InvalidValue { clause: id, offset });
            }
        }
    }
    Ok(())
}

pub(crate) fn unfold(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\r' | '\n' => None,
            '\t' => Some(' '),
            _ => Some(ch),
        })
        .collect()
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
                date_time_text = Some(unfold(&text));
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

// Adapted from MimeKit's Utils/ParseUtils.cs and Utils/MimeUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    iter::FusedIterator,
    str::FromStr,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

static DEFAULT_MESSAGE_ID_DOMAIN: OnceLock<String> = OnceLock::new();

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
/// MimeKit's setter-style checks, or [`Self::generate`] to author a new ID.
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
    /// Configures the process-wide domain for generated message and content IDs.
    ///
    /// Call once at startup, before [`Self::generate`] or [`Self::default_domain`].
    /// The domain uses the same IDNA validation and lowercase normalization as
    /// [`Self::generate_with_domain`], with an optional final root dot removed.
    /// Invalid input does not initialize the setting. Concurrent initialization
    /// is atomic: only one caller succeeds, even if the domains are identical.
    ///
    /// ```
    /// use mimecraft::MessageId;
    ///
    /// MessageId::set_default_domain("EXAMPLE.COM.")?;
    /// assert_eq!(MessageId::default_domain(), "example.com");
    /// assert!(MessageId::generate()?.as_str().ends_with("@example.com"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`MessageIdDomainError::InvalidDomain`] for invalid input, or
    /// [`MessageIdDomainError::AlreadyInitialized`] after configuration or first
    /// use. Validation precedes the initialization attempt.
    pub fn set_default_domain(domain: &str) -> Result<(), MessageIdDomainError> {
        set_default_domain(&DEFAULT_MESSAGE_ID_DOMAIN, domain)
    }

    /// Returns the configured domain, initializing it if necessary.
    ///
    /// Without prior configuration, permanently selects `localhost.localdomain`.
    /// No hostname discovery or DNS lookup is performed.
    pub fn default_domain() -> &'static str {
        default_domain(&DEFAULT_MESSAGE_ID_DOMAIN)
    }

    /// Generates a Message-Id or Content-Id using the process-wide default domain.
    ///
    /// Adapts MimeKit's parameterless `MimeUtils.GenerateMessageId`, using
    /// [`Self::default_domain`] instead of discovering the local hostname.
    /// Set the domain once at startup with [`Self::set_default_domain`], or use
    /// [`Self::generate_with_domain`] for an independent per-call override.
    /// The first attempt freezes the default even if generation fails.
    ///
    /// # Errors
    ///
    /// Returns clock or OS-randomness errors from [`Self::generate_with_domain`].
    pub fn generate() -> Result<Self, MessageIdGenerationError> {
        Self::generate_with(Self::default_domain(), SystemTime::now(), getrandom::fill)
    }

    /// Generates a new Message-Id or Content-Id using an explicit domain.
    ///
    /// Ports MimeKit's `MimeUtils.GenerateMessageId(domain)`: a UTC timestamp
    /// and eight OS-random bytes form two uppercase base36 components, followed
    /// by `@` and the domain. Digits are emitted least-significant first.
    /// The result has no angle brackets and uniqueness is probabilistic.
    /// Identifiers are not authentication tokens.
    ///
    /// Unlike the loose parsing APIs, domains undergo [`crate::idna::to_ascii`]
    /// validation and lowercase ASCII normalization. An optional final DNS root
    /// dot is omitted, matching this type's normalized representation.
    /// Does not read or initialize the global default domain, discover a hostname,
    /// or assign a header.
    ///
    /// ```
    /// use mimecraft::{MessageId, MimeMessage};
    ///
    /// let id = MessageId::generate_with_domain("example.com")?;
    /// let mut message: MimeMessage = MimeMessage::new();
    /// message.set_message_id(id.as_str())?;
    /// assert!(id.as_str().ends_with("@example.com"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`MessageIdGenerationError`] for invalid domains, an OS clock
    /// outside years 1 through 9999, or failure to obtain OS randomness.
    /// Domain and clock validation precede the entropy request. No weak random
    /// source or substitute identifier is used on failure.
    pub fn generate_with_domain(domain: &str) -> Result<Self, MessageIdGenerationError> {
        Self::generate_with(domain, SystemTime::now(), getrandom::fill)
    }

    fn generate_with(
        domain: &str,
        now: SystemTime,
        fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
    ) -> Result<Self, MessageIdGenerationError> {
        let domain =
            crate::idna::to_ascii(domain).map_err(MessageIdGenerationError::InvalidDomain)?;
        let ticks = message_id_ticks(now)?;
        let mut random = [0; 8];
        fill(&mut random).map_err(MessageIdGenerationError::Random)?;

        let mut value = String::with_capacity(28 + domain.len());
        append_base36(&mut value, ticks);
        value.push('.');
        // MimeKit assembles the random integer most-significant byte first,
        // independently of the host's native byte order.
        append_base36(&mut value, u64::from_be_bytes(random));
        value.push('@');
        value.push_str(domain.trim_end_matches('.'));
        Ok(Self { value })
    }

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

fn message_id_ticks(now: SystemTime) -> Result<u64, MessageIdGenerationError> {
    // MimeKit uses 100-nanosecond UTC ticks since 0001-01-01, not Unix time.
    const UNIX_EPOCH_TICKS: u128 = 621_355_968_000_000_000;
    const MAX_TICKS: u128 = 3_155_378_975_999_999_999;

    let ticks = match now.duration_since(UNIX_EPOCH) {
        Ok(duration) => Some(UNIX_EPOCH_TICKS + duration.as_nanos() / 100),
        // Round toward the earlier tick, including immediately before 1970.
        Err(error) => UNIX_EPOCH_TICKS.checked_sub(error.duration().as_nanos().div_ceil(100)),
    };
    ticks
        .filter(|ticks| *ticks <= MAX_TICKS)
        .map(|ticks| ticks as u64)
        .ok_or(MessageIdGenerationError::ClockOutOfRange)
}

fn append_base36(output: &mut String, mut value: u64) {
    const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    // Preserve MimeKit's reversed digit order, including one digit for zero.
    loop {
        output.push(char::from(DIGITS[(value % 36) as usize]));
        value /= 36;
        if value == 0 {
            break;
        }
    }
}

fn default_domain(domain: &OnceLock<String>) -> &str {
    domain.get_or_init(|| "localhost.localdomain".to_owned())
}

fn set_default_domain(domain: &OnceLock<String>, value: &str) -> Result<(), MessageIdDomainError> {
    let value = crate::idna::to_ascii(value).map_err(MessageIdDomainError::InvalidDomain)?;
    domain
        .set(value.trim_end_matches('.').to_owned())
        .map_err(|_| MessageIdDomainError::AlreadyInitialized)
}

/// The process-wide message/content-ID domain could not be configured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageIdDomainError {
    /// The supplied domain failed IDNA or MIME-safe ASCII validation.
    InvalidDomain(crate::idna::IdnaError),
    /// Configuration or first use has already fixed the domain.
    AlreadyInitialized,
}

impl fmt::Display for MessageIdDomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDomain(error) => {
                write!(f, "invalid default message identifier domain: {error}")
            }
            Self::AlreadyInitialized => {
                f.write_str("default message identifier domain is already initialized")
            }
        }
    }
}

impl Error for MessageIdDomainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDomain(error) => Some(error),
            Self::AlreadyInitialized => None,
        }
    }
}

/// A new message or content identifier could not be generated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageIdGenerationError {
    /// The domain failed IDNA or MIME-safe ASCII validation.
    InvalidDomain(crate::idna::IdnaError),
    /// The OS clock lies outside MimeKit's supported years 1 through 9999.
    ClockOutOfRange,
    /// The operating system could not supply random bytes.
    Random(getrandom::Error),
}

impl fmt::Display for MessageIdGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDomain(error) => write!(f, "invalid message identifier domain: {error}"),
            Self::ClockOutOfRange => {
                f.write_str("message identifier clock is outside years 1 through 9999")
            }
            Self::Random(error) => write!(f, "message identifier randomness failed: {error}"),
        }
    }
}

impl Error for MessageIdGenerationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDomain(error) => Some(error),
            Self::Random(error) => Some(error),
            Self::ClockOutOfRange => None,
        }
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

#[cfg(test)]
mod generation_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn base36_keeps_mimekits_digit_order_and_zero() {
        for (value, expected) in [(0, "0"), (35, "Z"), (36, "01"), (1295, "ZZ"), (1296, "001")] {
            let mut output = String::from("prefix:");
            append_base36(&mut output, value);
            assert_eq!(output, format!("prefix:{expected}"));
        }
        let mut output = String::new();
        append_base36(&mut output, u64::MAX);
        assert_eq!(
            u64::from_str_radix(&output.chars().rev().collect::<String>(), 36),
            Ok(u64::MAX)
        );
        assert_eq!(output.len(), 13);
    }

    #[test]
    fn timestamps_use_utc_ticks_and_floor_subtick_precision() {
        let epoch = 621_355_968_000_000_000;
        for (nanoseconds, ticks) in [(0, 0), (1, 0), (99, 0), (100, 1), (101, 1)] {
            let duration = Duration::from_nanos(nanoseconds);
            let now = UNIX_EPOCH + duration;
            // Windows SystemTime cannot represent sub-100-nanosecond steps.
            if now.duration_since(UNIX_EPOCH).unwrap() != duration {
                continue;
            }
            assert_eq!(message_id_ticks(now).unwrap(), epoch + ticks);
        }
        for (nanoseconds, ticks) in [(1, 1), (99, 1), (100, 1), (101, 2)] {
            let duration = Duration::from_nanos(nanoseconds);
            let now = UNIX_EPOCH - duration;
            if UNIX_EPOCH.duration_since(now).unwrap() != duration {
                continue;
            }
            assert_eq!(message_id_ticks(now).unwrap(), epoch - ticks);
        }
        let end = UNIX_EPOCH + Duration::from_secs(253_402_300_800);
        assert_eq!(
            message_id_ticks(end - Duration::from_nanos(100)).unwrap(),
            3_155_378_975_999_999_999
        );
        assert!(matches!(
            message_id_ticks(end),
            Err(MessageIdGenerationError::ClockOutOfRange)
        ));
        // Some platforms cannot represent year 1 as a SystemTime.
        if let Some(start) = UNIX_EPOCH.checked_sub(Duration::from_secs(62_135_596_800)) {
            assert_eq!(message_id_ticks(start).unwrap(), 0);
            if let Some(before) = start.checked_sub(Duration::from_nanos(100)) {
                assert!(matches!(
                    message_id_ticks(before),
                    Err(MessageIdGenerationError::ClockOutOfRange)
                ));
            }
        }
    }

    #[test]
    fn generation_uses_eight_big_endian_random_bytes() {
        let id = MessageId::generate_with("EXAMPLE.COM.", UNIX_EPOCH, |bytes| {
            assert_eq!(bytes.len(), 8);
            bytes.copy_from_slice(&[0, 0, 0, 0, 0, 0, 1, 0]);
            Ok(())
        })
        .unwrap();
        let (local, domain) = id.as_str().split_once('@').unwrap();
        let (timestamp, random) = local.split_once('.').unwrap();
        assert_eq!(domain, "example.com");
        assert_eq!(random, "47"); // 256, least-significant digit first.
        assert_eq!(
            u64::from_str_radix(&timestamp.chars().rev().collect::<String>(), 36),
            Ok(621_355_968_000_000_000)
        );
        let zero = MessageId::generate_with("example.com", UNIX_EPOCH, |bytes| {
            bytes.fill(0);
            Ok(())
        })
        .unwrap();
        assert!(zero.as_str().ends_with(".0@example.com"));
    }

    #[test]
    fn validation_precedes_entropy_and_failures_have_no_fallback() {
        let no_entropy = |_: &mut [u8]| -> Result<(), getrandom::Error> {
            panic!("invalid inputs must fail before requesting entropy")
        };
        let error = MessageId::generate_with("", UNIX_EPOCH, no_entropy).unwrap_err();
        assert!(matches!(error, MessageIdGenerationError::InvalidDomain(_)));
        assert!(error.source().unwrap().is::<crate::idna::IdnaError>());
        assert!(error.to_string().contains("domain"));

        let end = UNIX_EPOCH + Duration::from_secs(253_402_300_800);
        let error = MessageId::generate_with("example.com", end, no_entropy).unwrap_err();
        assert!(matches!(error, MessageIdGenerationError::ClockOutOfRange));
        assert!(error.source().is_none());
        assert!(error.to_string().contains("clock"));

        let error = MessageId::generate_with("example.com", UNIX_EPOCH, |bytes| {
            bytes[0] = 42;
            Err(getrandom::Error::UNSUPPORTED)
        })
        .unwrap_err();
        assert!(matches!(error, MessageIdGenerationError::Random(_)));
        assert_eq!(
            error.source().unwrap().downcast_ref::<getrandom::Error>(),
            Some(&getrandom::Error::UNSUPPORTED)
        );
        assert!(error.to_string().contains("randomness"));
    }

    #[test]
    fn configured_domains_are_normalized_and_cannot_be_replaced() {
        for (value, expected) in [
            ("LOCALHOST", "localhost"),
            ("Host.EXAMPLE.COM.", "host.example.com"),
            ("Mj\u{f6}lnir", "xn--mjlnir-xxa"),
        ] {
            let domain = OnceLock::new();
            set_default_domain(&domain, value).unwrap();
            assert_eq!(default_domain(&domain), expected);
            let error = set_default_domain(&domain, expected).unwrap_err();
            assert_eq!(error, MessageIdDomainError::AlreadyInitialized);
            assert!(error.source().is_none());
            assert!(!error.to_string().is_empty());
            assert_eq!(default_domain(&domain), expected);
        }
    }

    #[test]
    fn invalid_configuration_does_not_initialize_the_default() {
        let domain = OnceLock::new();
        for value in [
            "",
            ".",
            "invalid domain",
            "host\r\nBcc: victim@example.com",
            "a\0b",
        ] {
            let error = set_default_domain(&domain, value).unwrap_err();
            assert!(matches!(error, MessageIdDomainError::InvalidDomain(_)));
            assert!(error.source().unwrap().is::<crate::idna::IdnaError>());
            assert!(error.to_string().contains("domain"));
            assert!(domain.get().is_none());
        }
        set_default_domain(&domain, "example.com").unwrap();
        assert_eq!(default_domain(&domain), "example.com");
    }

    #[test]
    fn first_use_permanently_selects_localhost() {
        let domain = OnceLock::new();
        assert_eq!(default_domain(&domain), "localhost.localdomain");
        assert_eq!(
            set_default_domain(&domain, "example.com"),
            Err(MessageIdDomainError::AlreadyInitialized)
        );
        assert_eq!(default_domain(&domain), "localhost.localdomain");
    }

    #[test]
    fn concurrent_configuration_has_exactly_one_winner() {
        let domain = OnceLock::new();
        let barrier = std::sync::Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        set_default_domain(&domain, "EXAMPLE.COM.")
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(MessageIdDomainError::AlreadyInitialized))
                .count(),
            7
        );
        assert_eq!(default_domain(&domain), "example.com");
    }
}

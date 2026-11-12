// Adapted from MimeKit's DomainList.cs and Utils/ParseUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::parse_utils::{self, DomainParseErrorKind, skip_comments_and_whitespace};

/// An ordered list of domains in an obsolete mailbox source route.
///
/// For example, `@relay.example,@gateway.example` is the route portion of
/// `<@relay.example,@gateway.example:user@example.com>`. This type does not
/// parse the surrounding mailbox, its local part, or its display name.
///
/// Parsing preserves case, duplicate domains, and UTF-8 atoms, strips comments
/// and domain whitespace, and tolerates obsolete empty comma entries. Domain
/// literals are parsed as tokens, not validated as IP addresses. Parsing does
/// not perform IDNA conversion or SMTP validation; [`Self::to_ascii`] separately
/// converts and validates route domains for ASCII value formatting.
///
/// Construction and mutation require nonempty dot-atoms or bracketed literals
/// without whitespace or controls. Unicode and single-label names are supported;
/// values are not trimmed or checked against DNS. Parsing and cloning preserve
/// tolerated malformed domains. [`Display`](fmt::Display) does not validate,
/// escape, or fold them; use checked formatting for header output.
///
/// ```
/// use mimecraft::DomainList;
///
/// let route = DomainList::parse(b"@relay (note) . example,,@gateway.example")?;
/// assert_eq!(route.as_slice(), &["relay.example", "gateway.example"]);
/// assert_eq!(route.to_string(), "@relay.example,@gateway.example");
/// let input = b"@relay.example:user@example.com";
/// let (route, consumed) = DomainList::parse_prefix(input)?;
/// assert_eq!(&input[consumed..], b":user@example.com");
/// # Ok::<(), mimecraft::DomainListParseError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct DomainList {
    domains: Vec<String>,
}

impl DomainList {
    /// Creates an empty list without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of stored domains.
    pub fn len(&self) -> usize {
        self.domains.len()
    }

    /// Returns whether no strings are stored.
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// Returns stored strings in order, for indexing, iteration, or exact lookup.
    ///
    /// String comparison is case-sensitive, matching MimeKit's list.
    pub fn as_slice(&self) -> &[String] {
        &self.domains
    }

    /// Constructs a list, validating each domain without normalization.
    ///
    /// Returns the index of the first invalid domain; no partial list is returned.
    pub fn try_from_iter<S: Into<String>>(
        domains: impl IntoIterator<Item = S>,
    ) -> Result<Self, DomainListValidationError> {
        Self::try_from(domains.into_iter().map(Into::into).collect::<Vec<_>>())
    }

    /// Appends a valid domain, preserving spelling and duplicates.
    ///
    /// An invalid domain leaves the list unchanged.
    pub fn push(&mut self, domain: impl Into<String>) -> Result<(), DomainListValidationError> {
        let domain = domain.into();
        validate_domain(&domain, self.len())?;
        self.domains.push(domain);
        Ok(())
    }

    /// Inserts a valid domain, leaving the list unchanged on a validation error.
    ///
    /// # Panics
    ///
    /// Panics if `index > self.len()`, like [`Vec::insert`].
    pub fn insert(
        &mut self,
        index: usize,
        domain: impl Into<String>,
    ) -> Result<(), DomainListValidationError> {
        let domain = domain.into();
        validate_domain(&domain, index)?;
        self.domains.insert(index, domain);
        Ok(())
    }

    /// Replaces a domain, leaving the list unchanged on a validation error.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`.
    pub fn set(
        &mut self,
        index: usize,
        domain: impl Into<String>,
    ) -> Result<(), DomainListValidationError> {
        let domain = domain.into();
        validate_domain(&domain, index)?;
        self.domains[index] = domain;
        Ok(())
    }

    /// Removes and returns a domain, shifting later entries left.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.len()`, like [`Vec::remove`].
    pub fn remove(&mut self, index: usize) -> String {
        self.domains.remove(index)
    }

    /// Removes all domains.
    pub fn clear(&mut self) {
        self.domains.clear();
    }

    /// Returns the stored vector without copying its strings.
    pub fn into_vec(self) -> Vec<String> {
        self.domains
    }

    /// Formats an ASCII route without modifying stored domains.
    ///
    /// Like display, this skips whitespace-only entries and joins domains with
    /// commas and `@` prefixes. Unlike display, it validates every emitted entry:
    /// names use [`crate::idna::to_ascii`], including lowercase normalization and
    /// DNS length checks; bracketed literals use the MIME domain parser, removing
    /// internal whitespace without validating their content as an IP address.
    ///
    /// This returns route value syntax only: no enclosing angle brackets,
    /// mailbox-separating colon, folding, or terminating newline.
    ///
    /// ```
    /// use mimecraft::DomainList;
    ///
    /// let route = DomainList::try_from_iter(["b\u{fc}cher.example", "[IPv6:::1]"])?;
    /// assert_eq!(route.to_ascii()?, "@xn--bcher-kva.example,@[IPv6:::1]");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`DomainListEncodeError`] with the original stored index of the
    /// first invalid domain. No partial output is returned, and Unicode is never
    /// silently passed through when conversion fails.
    pub fn to_ascii(&self) -> Result<String, DomainListEncodeError> {
        self.encode(false)
    }

    pub(crate) fn encode(&self, international: bool) -> Result<String, DomainListEncodeError> {
        let mut output = String::new();
        for (index, domain) in self.domains.iter().enumerate() {
            if domain.trim().is_empty() {
                continue;
            }
            let formatted = if domain.starts_with('[') {
                let mut literal = String::new();
                let mut consumed = 0;
                parse_utils::parse_domain(domain.as_bytes(), &mut consumed, b"", &mut literal)
                    .map_err(|_| DomainListEncodeError::InvalidLiteral { index })?;
                if consumed != domain.len() {
                    return Err(DomainListEncodeError::InvalidLiteral { index });
                }
                std::borrow::Cow::Owned(literal)
            } else {
                let ascii = crate::idna::to_ascii(domain)
                    .map_err(|source| DomainListEncodeError::Idna { index, source })?;
                // MimeKit retains domain spelling in international mode. Unlike
                // unchecked display output, also validate recovered route entries.
                if international {
                    std::borrow::Cow::Borrowed(domain.as_str())
                } else {
                    ascii
                }
            };
            if !output.is_empty() {
                output.push(',');
            }
            output.push('@');
            output.push_str(&formatted);
        }
        Ok(output)
    }

    /// Parses a complete route, allowing leading/trailing comments and whitespace.
    ///
    /// Requires an explicit leading `@`. Unlike MimeKit's public prefix-style
    /// wrapper, trailing non-route input is rejected. In particular, the colon
    /// preceding a mailbox is not part of a complete standalone route.
    ///
    /// # Errors
    ///
    /// Returns [`DomainListParseError`] for missing or malformed domains,
    /// unterminated comments/literals, invalid UTF-8, or trailing input.
    pub fn parse(input: &[u8]) -> Result<Self, DomainListParseError> {
        let (list, index) = Self::parse_prefix(input)?;
        if index != input.len() {
            return Err(DomainListParseError::new(
                DomainListParseErrorKind::TrailingInput,
                index,
            ));
        }
        Ok(list)
    }

    /// Parses a route prefix and returns the number of consumed input bytes.
    ///
    /// The first `@` is required after optional leading comments/whitespace.
    /// Like MimeKit, repeated/trailing commas and adjacent `@domain` entries
    /// are tolerated. A domain must immediately follow each `@`; comments and
    /// whitespace are accepted around dots, commas, and after domains.
    /// A trailing dot before a comma, colon, or EOF is stripped.
    ///
    /// The colon before a mailbox and other trailing non-route input are left
    /// unconsumed. Trailing comments, whitespace, and commas are consumed.
    /// Whitespace-only Unicode domains are omitted, so a valid prefix can
    /// produce an empty list. The input must contain a complete route prefix;
    /// this is not an incremental parser.
    ///
    /// # Errors
    ///
    /// Returns [`DomainListParseError`] for a missing `@`, malformed domains,
    /// invalid UTF-8, or unterminated comments/literals. No partial list is returned.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), DomainListParseError> {
        let mut index = 0;
        skip_cfws(input, &mut index)?;
        if input.get(index) != Some(&b'@') {
            return Err(DomainListParseError::new(
                DomainListParseErrorKind::MissingAt,
                index,
            ));
        }

        let mut domains = Vec::new();
        loop {
            // Skip over the '@'.
            index += 1;
            let mut domain = String::new();
            parse_utils::parse_domain(input, &mut index, b",:", &mut domain).map_err(|error| {
                let kind = match error.kind {
                    DomainParseErrorKind::InvalidDomain => DomainListParseErrorKind::InvalidDomain,
                    DomainParseErrorKind::IncompleteLiteral => {
                        DomainListParseErrorKind::IncompleteDomainLiteral
                    }
                    DomainParseErrorKind::IncompleteComment => {
                        DomainListParseErrorKind::IncompleteComment
                    }
                    DomainParseErrorKind::InvalidUtf8 => DomainListParseErrorKind::InvalidUtf8,
                };
                DomainListParseError::new(kind, error.offset)
            })?;

            // Display skips whitespace-only domains (Unicode spaces can parse
            // as atoms), so skip them here too to keep round trips consistent.
            if !domain.trim().is_empty() {
                domains.push(domain);
            }
            // Note: obs-domain-list allows null domains between commas.
            loop {
                skip_cfws(input, &mut index)?;
                if input.get(index) != Some(&b',') {
                    break;
                }
                index += 1;
            }
            if input.get(index) != Some(&b'@') {
                break;
            }
        }
        Ok((Self { domains }, index))
    }
}

fn skip_cfws(input: &[u8], index: &mut usize) -> Result<(), DomainListParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| {
        DomainListParseError::new(DomainListParseErrorKind::IncompleteComment, offset)
    })
}

impl TryFrom<Vec<String>> for DomainList {
    type Error = DomainListValidationError;

    fn try_from(domains: Vec<String>) -> Result<Self, Self::Error> {
        for (index, domain) in domains.iter().enumerate() {
            validate_domain(domain, index)?;
        }
        Ok(Self { domains })
    }
}

fn validate_domain(domain: &str, index: usize) -> Result<(), DomainListValidationError> {
    let literal = domain.starts_with('[');
    let text = if literal {
        domain.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
    } else {
        Some(domain)
    };
    let valid = text.is_some_and(|text| {
        let mut atom_length = 0;
        for ch in text.chars() {
            if !literal && ch == '.' {
                if atom_length == 0 {
                    return false;
                }
                atom_length = 0;
                continue;
            }
            // Rust strings already contain valid Unicode scalars, so no
            // surrogate-pair checks are needed for internationalized domains.
            let valid = if ch.is_ascii() {
                if literal {
                    ch.is_ascii_graphic() && !matches!(ch, '[' | '\\' | ']')
                } else {
                    parse_utils::is_atom(ch as u8)
                }
            } else {
                !ch.is_whitespace() && !ch.is_control()
            };
            if !valid {
                return false;
            }
            atom_length += 1;
        }
        literal || atom_length > 0
    });
    if valid {
        Ok(())
    } else {
        Err(DomainListValidationError { index })
    }
}

/// An invalid authored route domain. Parsing uses separate, tolerant rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainListValidationError {
    index: usize,
}

impl DomainListValidationError {
    /// Returns the attempted insertion/replacement index, or invalid input index.
    pub fn domain_index(self) -> usize {
        self.index
    }
}

impl fmt::Display for DomainListValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid route domain at index {}", self.index)
    }
}

impl Error for DomainListValidationError {}

impl IntoIterator for DomainList {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.domains.into_iter()
    }
}

impl<'a> IntoIterator for &'a DomainList {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.domains.iter()
    }
}

impl FromStr for DomainList {
    type Err = DomainListParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl fmt::Display for DomainList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for domain in &self.domains {
            if domain.trim().is_empty() {
                continue;
            }
            write!(f, "{separator}@{domain}")?;
            separator = ",";
        }
        Ok(())
    }
}

/// A domain-route ASCII formatting failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainListEncodeError {
    /// A stored name could not be converted to MIME-safe ASCII.
    Idna {
        /// Zero-based index in the stored list, including skipped blank entries.
        index: usize,
        /// The domain conversion failure.
        source: crate::idna::IdnaError,
    },
    /// A stored bracketed literal was malformed or had trailing input.
    InvalidLiteral {
        /// Zero-based index in the stored list, including skipped blank entries.
        index: usize,
    },
}

impl DomainListEncodeError {
    /// Returns the index of the failed entry in the original stored list.
    pub fn domain_index(self) -> usize {
        match self {
            Self::Idna { index, .. } | Self::InvalidLiteral { index } => index,
        }
    }
}

impl fmt::Display for DomainListEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idna { index, source } => write!(f, "route domain at index {index}: {source}"),
            Self::InvalidLiteral { index } => {
                write!(f, "invalid route domain literal at index {index}")
            }
        }
    }
}

impl Error for DomainListEncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Idna { source, .. } => Some(source),
            Self::InvalidLiteral { .. } => None,
        }
    }
}

/// The reason a domain route could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainListParseErrorKind {
    /// No `@` followed the leading comments and whitespace.
    MissingAt,
    /// A domain was missing or contained invalid atom/literal syntax.
    InvalidDomain,
    /// A bracketed domain literal was not closed.
    IncompleteDomainLiteral,
    /// A comment was not closed.
    IncompleteComment,
    /// A domain atom contained invalid UTF-8.
    InvalidUtf8,
    /// Non-route input followed a complete-value parse.
    TrailingInput,
}

/// A domain-route parsing failure with a zero-based input byte offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainListParseError {
    kind: DomainListParseErrorKind,
    offset: usize,
}

impl DomainListParseError {
    fn new(kind: DomainListParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the reason parsing failed.
    pub fn kind(self) -> DomainListParseErrorKind {
        self.kind
    }

    /// Returns an offset into the original input, not the normalized domain.
    ///
    /// Unclosed comments/literals point to their opening delimiter. Other
    /// errors point to the offending byte, or EOF for missing input.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for DomainListParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            DomainListParseErrorKind::MissingAt => "expected '@' before domain route",
            DomainListParseErrorKind::InvalidDomain => "invalid route domain",
            DomainListParseErrorKind::IncompleteDomainLiteral => "unterminated domain literal",
            DomainListParseErrorKind::IncompleteComment => "unterminated comment",
            DomainListParseErrorKind::InvalidUtf8 => "invalid UTF-8 in route domain",
            DomainListParseErrorKind::TrailingInput => "unexpected input after domain route",
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for DomainListParseError {}

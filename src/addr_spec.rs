// Adapted from MimeKit's AddressParser.cs and MailboxAddress.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{borrow::Cow, error::Error, fmt, str::FromStr};

use crate::{
    charsets::Charset,
    idna::{self, IdnaError},
    parse_utils::{
        self, DomainParseErrorKind, is_ascii_atom, is_atom, skip_comments_and_whitespace,
    },
};

const SENTINELS: &[u8] = b",>;";

/// Compatibility policy for parsing address specifications.
///
/// These modes mirror MimeKit's address parser, not complete RFC/SMTP validation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AddressComplianceMode {
    /// Requires valid UTF-8 local-part words and rejects extra local-part dots.
    Strict,
    /// Allows Latin-1 recovery of malformed UTF-8 local-part words.
    #[default]
    Loose,
    /// Also allows repeated dots, a dot before `@`, and escaped `\@` atoms.
    Looser,
}

/// Options for parsing an [`AddrSpec`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddrSpecParseOptions {
    /// Compatibility mode; defaults to [`AddressComplianceMode::Loose`].
    pub compliance_mode: AddressComplianceMode,
    /// Whether a local part without `@domain` is accepted; defaults to true.
    pub allow_local_only: bool,
}

impl Default for AddrSpecParseOptions {
    fn default() -> Self {
        Self {
            compliance_mode: AddressComplianceMode::Loose,
            allow_local_only: true,
        }
    }
}

/// A normalized mailbox address specification, without a display name or route.
///
/// Comments and whitespace around atoms/dots are removed. Quotes, quoted-pair
/// escapes, case, and domain spelling are retained; parsing does not IDNA-decode
/// ACE labels. A tolerated trailing domain dot is stripped, as in the shared
/// message-ID/domain-route grammar. Quoted local-part whitespace is retained;
/// controls other than TAB are rejected even when escaped. Domain literals use MIME token
/// syntax, not IP-address validation.
///
/// This is a compatibility reading API, not a complete mailbox parser or an
/// authoring validator. It accepts obsolete quoted-word local parts, optionally
/// local-only addresses, and international local parts. It does not enforce
/// SMTP length limits, DNS validity, or deliverability. Names, angle brackets,
/// routes, and groups must be handled by an enclosing parser.
///
/// In loose modes, malformed UTF-8 local-part words are decoded as Latin-1.
/// [`Self::had_latin1_fallback`] explicitly reports this recovery. Domain atoms
/// always require UTF-8. Raw source bytes are not retained.
///
/// ```
/// use mimecraft::AddrSpec;
///
/// let address = AddrSpec::parse(b" user (note) . name @ Example.COM ")?;
/// assert_eq!(address.as_str(), "user.name@Example.COM");
/// assert_eq!(address.local_part(), "user.name");
/// assert_eq!(address.domain(), Some("Example.COM"));
/// let international: AddrSpec = "user@b\u{fc}cher.example".parse()?;
/// assert_eq!(international.with_ascii_domain()?, "user@xn--bcher-kva.example");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug)]
pub struct AddrSpec {
    value: String,
    at: Option<usize>,
    latin1_fallback: bool,
}

impl AddrSpec {
    /// Returns the normalized address, preserving its parsed domain spelling.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the local part, including any quotes and quoted-pair escapes.
    pub fn local_part(&self) -> &str {
        &self.value[..self.at.unwrap_or(self.value.len())]
    }

    /// Returns the domain, or `None` for a local-only address.
    pub fn domain(&self) -> Option<&str> {
        self.at.map(|at| &self.value[at + 1..])
    }

    /// Returns whether the stored address contains non-ASCII characters.
    ///
    /// ACE labels alone do not make an address international by this test.
    pub fn is_international(&self) -> bool {
        !self.value.is_ascii()
    }

    /// Reports whether at least one malformed UTF-8 word used Latin-1 recovery.
    ///
    /// The whole offending word is recovered, not just its invalid bytes.
    /// Strict mode returns an error instead. This flag remains part of the
    /// parse result even after requesting a converted domain.
    pub fn had_latin1_fallback(&self) -> bool {
        self.latin1_fallback
    }

    /// Returns an IDNA-normalized ASCII domain, leaving literals untouched.
    ///
    /// Returns `None` for local-only addresses. Conversion can fail with
    /// [`IdnaError`] even when the tolerant address grammar accepted the domain.
    pub fn ascii_domain(&self) -> Result<Option<Cow<'_, str>>, IdnaError> {
        self.convert_domain(true)
    }

    /// Returns an IDNA-normalized Unicode domain, leaving literals untouched.
    ///
    /// Returns `None` for local-only addresses. Invalid IDNA names return
    /// [`IdnaError`], not unchanged or partially decoded output.
    pub fn unicode_domain(&self) -> Result<Option<Cow<'_, str>>, IdnaError> {
        self.convert_domain(false)
    }

    /// Returns the address with its domain converted to ASCII.
    ///
    /// The local part is never modified: a non-ASCII local part stays non-ASCII,
    /// so this is not a downgrade to an ASCII-only SMTP address. Domain literals
    /// and local-only addresses are returned unchanged. Errors are [`IdnaError`].
    pub fn with_ascii_domain(&self) -> Result<Cow<'_, str>, IdnaError> {
        Ok(self.replace_domain(self.ascii_domain()?))
    }

    /// Returns the address with its domain converted to Unicode.
    ///
    /// The local part and domain literals remain untouched. Invalid IDNA names
    /// return [`IdnaError`]; the original stored address is never changed.
    pub fn with_unicode_domain(&self) -> Result<Cow<'_, str>, IdnaError> {
        Ok(self.replace_domain(self.unicode_domain()?))
    }

    fn convert_domain(&self, ascii: bool) -> Result<Option<Cow<'_, str>>, IdnaError> {
        let Some(domain) = self.domain() else {
            return Ok(None);
        };
        // A domain literal is not a domain name and must be preserved exactly,
        // even if it contains something that looks like an A-label.
        if domain.starts_with('[') {
            return Ok(Some(Cow::Borrowed(domain)));
        }
        if ascii {
            idna::to_ascii(domain).map(Some)
        } else {
            idna::to_unicode(domain).map(Some)
        }
    }

    fn replace_domain(&self, domain: Option<Cow<'_, str>>) -> Cow<'_, str> {
        match domain {
            Some(domain) if Some(domain.as_ref()) != self.domain() => {
                Cow::Owned(format!("{}@{domain}", self.local_part()))
            }
            _ => Cow::Borrowed(&self.value),
        }
    }

    /// Parses one complete address specification with default options.
    ///
    /// Leading/trailing comments and whitespace are accepted; all other trailing
    /// input, including a closing angle bracket, is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`AddrSpecParseError`] for malformed syntax or trailing input.
    pub fn parse(input: &[u8]) -> Result<Self, AddrSpecParseError> {
        Self::parse_with_options(input, &AddrSpecParseOptions::default())
    }

    /// Parses one complete address specification with explicit compatibility.
    ///
    /// # Errors
    ///
    /// Returns [`AddrSpecParseError`] for malformed syntax, trailing input,
    /// strict-mode UTF-8 errors, or disallowed local-only addresses.
    pub fn parse_with_options(
        input: &[u8],
        options: &AddrSpecParseOptions,
    ) -> Result<Self, AddrSpecParseError> {
        let (address, consumed) = Self::parse_prefix_with_options(input, options)?;
        if consumed != input.len() {
            return Err(AddrSpecParseError::new(
                AddrSpecParseErrorKind::TrailingInput,
                consumed,
            ));
        }
        Ok(address)
    }

    /// Parses a prefix with default options, returning address and bytes consumed.
    ///
    /// Leading/trailing comments and whitespace are consumed. Comma, `>`, and
    /// `;` delimiters are left untouched; other suffixes after a domain are also
    /// left for the enclosing parser to validate. This does not scan for an
    /// address inside display-name text or accept an opening `<`.
    ///
    /// Input must contain a complete token, not an incremental chunk.
    ///
    /// # Errors
    ///
    /// Returns [`AddrSpecParseError`] for malformed or incomplete syntax.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), AddrSpecParseError> {
        Self::parse_prefix_with_options(input, &AddrSpecParseOptions::default())
    }

    /// Parses a prefix using explicit options; see [`Self::parse_prefix`].
    ///
    /// # Errors
    ///
    /// Returns [`AddrSpecParseError`] for syntax errors, strict-mode UTF-8
    /// errors, or disallowed local-only addresses. No partial address is returned.
    pub fn parse_prefix_with_options(
        input: &[u8],
        options: &AddrSpecParseOptions,
    ) -> Result<(Self, usize), AddrSpecParseError> {
        let (address, mut index) = Self::parse_prefix_untrimmed(input, options)?;
        skip_cfws(input, &mut index)?;
        Ok((address, index))
    }

    // Leave trailing CFWS for mailbox parsing, which can use the first comment
    // as a display name. Public addr-spec parsing still consumes that CFWS.
    pub(crate) fn parse_prefix_untrimmed(
        input: &[u8],
        options: &AddrSpecParseOptions,
    ) -> Result<(Self, usize), AddrSpecParseError> {
        let mut index = 0;
        skip_cfws(input, &mut index)?;
        if index == input.len() {
            return Err(AddrSpecParseError::new(
                AddrSpecParseErrorKind::MissingLocalPart,
                index,
            ));
        }
        if let Some((end, at)) = contiguous_addrspec(input, index) {
            return Ok((
                Self {
                    value: std::str::from_utf8(&input[index..end])
                        .expect("the fast path scans only ASCII atoms and separators")
                        .to_owned(),
                    at: Some(at),
                    latin1_fallback: false,
                },
                end,
            ));
        }

        let mut latin1_fallback = false;
        let (mut value, local_end) = parse_local_part(
            input,
            &mut index,
            options.compliance_mode,
            &mut latin1_fallback,
        )?;
        let at = if index == input.len() || SENTINELS.contains(&input[index]) {
            if !options.allow_local_only {
                return Err(AddrSpecParseError::new(
                    AddrSpecParseErrorKind::MissingDomain,
                    index,
                ));
            }
            index = local_end;
            None
        } else {
            if input[index] != b'@' {
                return Err(AddrSpecParseError::new(
                    AddrSpecParseErrorKind::InvalidAddress,
                    index,
                ));
            }
            index += 1;
            skip_cfws(input, &mut index)?;
            if index == input.len() {
                return Err(AddrSpecParseError::new(
                    AddrSpecParseErrorKind::IncompleteAddress,
                    index,
                ));
            }
            let at = value.len();
            value.push('@');
            parse_utils::parse_domain(input, &mut index, SENTINELS, &mut value).map_err(
                |error| {
                    let kind = match error.kind {
                        DomainParseErrorKind::InvalidDomain => {
                            AddrSpecParseErrorKind::InvalidDomain
                        }
                        DomainParseErrorKind::IncompleteLiteral => {
                            AddrSpecParseErrorKind::IncompleteDomainLiteral
                        }
                        DomainParseErrorKind::IncompleteComment => {
                            AddrSpecParseErrorKind::IncompleteComment
                        }
                        DomainParseErrorKind::InvalidUtf8 => AddrSpecParseErrorKind::InvalidUtf8,
                    };
                    AddrSpecParseError::new(kind, error.offset)
                },
            )?;
            // The domain is deliberately not IDN-decoded: preserve its parsed
            // spelling. Explicit domain conversion supplies the Unicode form.
            Some(at)
        };
        Ok((
            Self {
                value,
                at,
                latin1_fallback,
            },
            index,
        ))
    }
}

fn skip_cfws(input: &[u8], index: &mut usize) -> Result<(), AddrSpecParseError> {
    skip_comments_and_whitespace(input, index).map_err(|offset| {
        AddrSpecParseError::new(AddrSpecParseErrorKind::IncompleteComment, offset)
    })
}

fn scan_ascii_dot_atom(input: &[u8], index: &mut usize) -> bool {
    loop {
        let start = *index;
        while input.get(*index).is_some_and(|&byte| is_ascii_atom(byte)) {
            *index += 1;
        }
        if *index == start {
            return false;
        }
        if input.get(*index) != Some(&b'.') {
            return true;
        }
        *index += 1;
    }
}

fn contiguous_addrspec(input: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    if !scan_ascii_dot_atom(input, &mut index) || input.get(index) != Some(&b'@') {
        return None;
    }
    let at = index - start;
    index += 1;
    if !scan_ascii_dot_atom(input, &mut index) {
        return None;
    }
    // CFWS may hide another domain label. Leave it and all repairs to the
    // full grammar rather than prematurely accepting a contiguous prefix.
    if index < input.len() && !SENTINELS.contains(&input[index]) {
        return None;
    }
    Some((index, at))
}

fn parse_local_part(
    input: &[u8],
    index: &mut usize,
    mode: AddressComplianceMode,
    latin1_fallback: &mut bool,
) -> Result<(String, usize), AddrSpecParseError> {
    let mut token = String::new();
    loop {
        let start = *index;
        let mut escaped_at = false;
        if input.get(*index) == Some(&b'"') {
            skip_local_quote(input, index)?;
        } else if input.get(*index).is_some_and(|&byte| is_atom(byte)) {
            while input.get(*index).is_some_and(|&byte| is_atom(byte)) {
                *index += 1;
            }
            if mode == AddressComplianceMode::Looser {
                // Allow local parts with escaped '@' symbols (MimeKit #1043).
                while input.get(*index) == Some(&b'\\') && input.get(*index + 1) == Some(&b'@') {
                    escaped_at = true;
                    *index += 2;
                    let atom = *index;
                    while input.get(*index).is_some_and(|&byte| is_atom(byte)) {
                        *index += 1;
                    }
                    if *index == atom {
                        break;
                    }
                }
            }
        } else {
            return Err(AddrSpecParseError::new(
                AddrSpecParseErrorKind::InvalidLocalPart,
                *index,
            ));
        }

        let word = match std::str::from_utf8(&input[start..*index]) {
            Ok(word) => Cow::Borrowed(word),
            Err(error) if mode == AddressComplianceMode::Strict => {
                return Err(AddrSpecParseError::new(
                    AddrSpecParseErrorKind::InvalidUtf8,
                    start + error.valid_up_to(),
                ));
            }
            Err(_) => {
                *latin1_fallback = true;
                Charset::LATIN1.decode_lossy(&input[start..*index]).0
            }
        };
        if escaped_at {
            token.push_str(&word.replace("\\@", "%40"));
        } else {
            token.push_str(&word);
        }

        let cfws = *index;
        skip_cfws(input, index)?;
        if input.get(*index) != Some(&b'.') {
            return Ok((token, cfws));
        }
        loop {
            token.push('.');
            *index += 1;
            skip_cfws(input, index)?;
            if *index == input.len() {
                return Err(AddrSpecParseError::new(
                    AddrSpecParseErrorKind::IncompleteLocalPart,
                    *index,
                ));
            }
            if mode != AddressComplianceMode::Looser || input[*index] != b'.' {
                break;
            }
        }
        if mode == AddressComplianceMode::Looser && input[*index] == b'@' {
            break;
        }
    }
    Ok((token, *index))
}

fn skip_local_quote(input: &[u8], index: &mut usize) -> Result<(), AddrSpecParseError> {
    // From RFC 5322:
    //
    // qcontent      = qtext / quoted-pair
    // qtext         = %d33 / %d35-91 / %d93-126 / obs-qtext
    //                 ; Printable US-ASCII except quote and backslash.
    // obs-qtext     = <any CHAR except quote, backslash, and CR,
    //                  including linear whitespace>
    // quoted-pair   = ("\" (VCHAR / WSP)) / obs-qp
    // VCHAR         = %x21-7E
    // WSP           = SP / HTAB
    // obs-qp        = "\" (%d0 / obs-NO-WS-CTL / LF / CR)
    // obs-NO-WS-CTL = %d1-8 / %d11 / %d12 / %d14-31 / %d127
    //
    // RFC 6532 extends qtext to UTF-8, and obs-qtext permits SP/TAB.
    // Although obs-qp permits escaped controls, they are too dangerous: reject
    // controls other than TAB, including escaped CR/LF and DEL.
    let start = *index;
    *index += 1;
    while let Some(&byte) = input.get(*index) {
        if byte == b'"' {
            *index += 1;
            return Ok(());
        }
        let byte = if byte == b'\\' {
            *index += 1;
            let Some(&escaped) = input.get(*index) else {
                break;
            };
            escaped
        } else {
            byte
        };
        if (byte < 32 && byte != b'\t') || byte == 127 {
            return Err(AddrSpecParseError::new(
                AddrSpecParseErrorKind::InvalidQuotedCharacter,
                *index,
            ));
        }
        *index += 1;
    }
    Err(AddrSpecParseError::new(
        AddrSpecParseErrorKind::IncompleteQuotedString,
        start,
    ))
}

impl FromStr for AddrSpec {
    type Err = AddrSpecParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl fmt::Display for AddrSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

/// The reason an address specification could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddrSpecParseErrorKind {
    /// No local part followed leading comments/whitespace.
    MissingLocalPart,
    /// A local-part word was missing or had invalid syntax.
    InvalidLocalPart,
    /// A local part ended after a dot.
    IncompleteLocalPart,
    /// A quoted local part contained a disallowed control character.
    InvalidQuotedCharacter,
    /// A quoted local-part word was not closed.
    IncompleteQuotedString,
    /// A local-only address was disabled by the options.
    MissingDomain,
    /// Unexpected syntax followed the local part instead of `@domain`.
    InvalidAddress,
    /// The input ended after `@` or its following comments/whitespace.
    IncompleteAddress,
    /// A domain atom or literal had invalid syntax.
    InvalidDomain,
    /// A domain literal was not closed.
    IncompleteDomainLiteral,
    /// A comment was not closed.
    IncompleteComment,
    /// A domain or strict-mode local-part word contained malformed UTF-8.
    InvalidUtf8,
    /// A complete-value parse found extra input after the address.
    TrailingInput,
}

/// An address parsing failure with a zero-based input byte offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddrSpecParseError {
    kind: AddrSpecParseErrorKind,
    offset: usize,
}

impl AddrSpecParseError {
    fn new(kind: AddrSpecParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the reason parsing failed.
    pub fn kind(self) -> AddrSpecParseErrorKind {
        self.kind
    }

    /// Returns the original input byte offset, before normalization/recovery.
    ///
    /// Unclosed comments, quotes, and literals point to the opening delimiter.
    /// Other failures point to the offending byte or EOF for missing input.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for AddrSpecParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            AddrSpecParseErrorKind::MissingLocalPart => "missing address local part",
            AddrSpecParseErrorKind::InvalidLocalPart => "invalid address local part",
            AddrSpecParseErrorKind::IncompleteLocalPart => "incomplete address local part",
            AddrSpecParseErrorKind::InvalidQuotedCharacter => {
                "invalid control in quoted local part"
            }
            AddrSpecParseErrorKind::IncompleteQuotedString => "unterminated quoted local part",
            AddrSpecParseErrorKind::MissingDomain => "address domain is required",
            AddrSpecParseErrorKind::InvalidAddress => "expected '@' after local part",
            AddrSpecParseErrorKind::IncompleteAddress => "incomplete address after '@'",
            AddrSpecParseErrorKind::InvalidDomain => "invalid address domain",
            AddrSpecParseErrorKind::IncompleteDomainLiteral => "unterminated domain literal",
            AddrSpecParseErrorKind::IncompleteComment => "unterminated comment",
            AddrSpecParseErrorKind::InvalidUtf8 => "invalid UTF-8 in address",
            AddrSpecParseErrorKind::TrailingInput => "unexpected input after address",
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for AddrSpecParseError {}

#[cfg(test)]
mod tests {
    use super::contiguous_addrspec;

    #[test]
    fn contiguous_ascii_tokens_take_the_fast_path() {
        assert_eq!(contiguous_addrspec(b"user@example.com", 0), Some((16, 4)));
        assert_eq!(contiguous_addrspec(b"  a.b@c.d,tail", 2), Some((9, 3)));
        assert_eq!(contiguous_addrspec(b"a@b>tail", 0), Some((3, 1)));
    }

    #[test]
    fn noncontiguous_or_repaired_tokens_require_the_full_grammar() {
        for input in [
            b"a@b (note).c".as_slice(),
            b"a@b ",
            b"a@b.",
            b"a..@b",
            b"a\\@b@c",
            b"\"a\"@b",
            b"a@[::1]",
            b"a",
            b"a@",
            "a@b\u{fc}cher".as_bytes(),
        ] {
            assert_eq!(contiguous_addrspec(input, 0), None, "{input:?}");
        }
    }
}

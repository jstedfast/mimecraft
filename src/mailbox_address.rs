// Adapted from MimeKit's AddressParser.cs, AddressParserInput.cs,
// MailboxAddress.cs and InternetAddress.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use crate::{
    AddrSpec, AddrSpecParseErrorKind, AddrSpecParseOptions, AddressComplianceMode, DomainList,
    DomainListParseErrorKind,
    parse_utils::{
        is_atom, is_whitespace, skip_comment, skip_comments_and_whitespace, skip_quoted,
        skip_whitespace,
    },
    rfc2047::{self, DecodeIssue, DecodeOptions, DecodedHeader},
    utils,
};

/// Compatibility and name-decoding options for a single [`MailboxAddress`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxParseOptions {
    /// Local-part compatibility and acceptance of bare local-only addresses.
    ///
    /// Angle-bracketed local-only addresses remain accepted even when
    /// `allow_local_only` is false, matching MimeKit's mailbox grammar.
    pub address: AddrSpecParseOptions,
    /// Accept unquoted commas in display names; defaults to true.
    ///
    /// This can make an unquoted name ambiguous with an address separator.
    /// Disable it when consuming prefixes from a list of local-only addresses.
    pub allow_unquoted_commas: bool,
    /// RFC 2047 boundary recognition and preferred fallback charset for names.
    ///
    /// Independent of the address compatibility mode.
    pub name_decoding: DecodeOptions,
}

impl Default for MailboxParseOptions {
    fn default() -> Self {
        Self {
            address: AddrSpecParseOptions::default(),
            allow_unquoted_commas: true,
            name_decoding: DecodeOptions::default(),
        }
    }
}

/// One mailbox: a display name, optional source route, and addr-spec.
///
/// Accepts bare addresses, name-angle forms, obsolete routes, and trailing
/// comment names. Loose modes recover unbalanced/excessive angle brackets,
/// an unclosed leading display-name quote, and an addr-spec used as an unquoted
/// name. Groups and lists are not accepted by complete parsing.
///
/// Parsing and editing do not provide header-authoring validation.
/// Decoded names can contain controls or line breaks from encoded words.
/// [`Display`](fmt::Display) produces unencoded display text, not safe header
/// output: names are quoted, but controls and line breaks are preserved.
/// [`Self::to_ascii`] separately validates and encodes ASCII value syntax;
/// place mailboxes in an [`crate::AddressList`] for complete-header folding.
/// Address spelling and Latin-1 recovery diagnostics are retained in [`AddrSpec`].
/// Original header bytes and source charset preferences are not retained.
///
/// Equality compares the stored display name and addr-spec text exactly,
/// including case and domain spelling, matching MimeKit. Source routes and
/// decoding diagnostics do not participate. Hashing uses the same fields,
/// not display output, so equal mailboxes always hash equally even if their
/// routes differ. This is not an SMTP recipient-equivalence comparison.
///
/// ```
/// use std::collections::HashSet;
/// use mimecraft::MailboxAddress;
///
/// let plain: MailboxAddress = "User <user@example.com>".parse()?;
/// let routed: MailboxAddress = "User <@relay:user@example.com>".parse()?;
/// assert_eq!(plain, routed);
/// assert_eq!(HashSet::from([plain, routed]).len(), 1);
/// # Ok::<(), mimecraft::MailboxParseError>(())
/// ```
///
/// ```
/// use mimecraft::MailboxAddress;
///
/// let mailbox: MailboxAddress =
///     "=?utf-8?q?Ren=C3=A9e?= <@relay.example:user@example.com>".parse()?;
/// assert_eq!(mailbox.display_name(), "Ren\u{e9}e");
/// assert_eq!(mailbox.address().as_str(), "user@example.com");
/// assert_eq!(mailbox.route().as_slice(), &["relay.example"]);
/// assert_eq!(mailbox.to_string(), "\"Ren\u{e9}e\" <@relay.example:user@example.com>");
/// # Ok::<(), mimecraft::MailboxParseError>(())
/// ```
#[derive(Clone, Debug)]
pub struct MailboxAddress {
    name: DecodedHeader,
    address: AddrSpec,
    route: DomainList,
}

impl MailboxAddress {
    /// Encodes an ASCII address value using UTF-8 encoded words for names.
    ///
    /// Domains and routes are validated and IDNA-normalized; an international
    /// local part is rejected rather than downgraded. Local-only addresses are
    /// enclosed in angle brackets. This returns value syntax only, with no
    /// field name, folding, or terminating newline. Name controls are encoded,
    /// not removed. No SMTP length or deliverability checks are performed.
    ///
    /// ```
    /// use mimecraft::MailboxAddress;
    ///
    /// let mailbox = MailboxAddress::new("Ren\u{e9}e", "user@b\u{fc}cher.example".parse()?);
    /// assert_eq!(mailbox.to_ascii()?, "=?utf-8?b?UmVuw6ll?= <user@xn--bcher-kva.example>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for invalid name options or charset
    /// conversion, an international local part, non-strict address syntax,
    /// or invalid domains/routes. No partial output is returned.
    pub fn to_ascii(&self) -> Result<String, crate::AddressEncodeError> {
        self.to_ascii_with_options(&rfc2047::EncodeOptions::default())
    }

    /// Encodes an ASCII value with explicit name charset and word-length options.
    ///
    /// See [`Self::to_ascii`]. The word limit is not a physical-line limit.
    /// Stored values and decoding diagnostics are never changed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] with an empty root path on failure.
    pub fn to_ascii_with_options(
        &self,
        options: &rfc2047::EncodeOptions,
    ) -> Result<String, crate::AddressEncodeError> {
        crate::address_encode::encode_mailbox(self, options)
    }

    /// Validates the stored addr-spec for non-SMTPUTF8 SMTP envelope use.
    ///
    /// Ignores the display name, source route, and recovery diagnostics.
    /// Validates the parsed address, not the original header text: parsing may
    /// already have normalized that text. Does not modify this mailbox or
    /// convert its domain to IDNA. Length limits are disabled by default.
    ///
    /// # Errors
    ///
    /// Returns [`crate::SmtpAddressError`] as described by
    /// [`utils::validate_smtp_address`].
    ///
    /// ```
    /// use mimecraft::{MailboxAddress, SmtpAddressError};
    ///
    /// let mailbox: MailboxAddress = "Ren\u{e9}e <user@example.com>".parse()?;
    /// assert_eq!(mailbox.validate_smtp(), Ok(()));
    /// let mailbox: MailboxAddress = "<user@b\u{fc}cher.example>".parse()?;
    /// assert_eq!(mailbox.validate_smtp(), Err(SmtpAddressError::NonAscii));
    /// # Ok::<(), mimecraft::MailboxParseError>(())
    /// ```
    pub fn validate_smtp(&self) -> Result<(), crate::SmtpAddressError> {
        utils::validate_smtp_address(self.address.as_str())
    }

    /// Validates the stored addr-spec with SMTP compatibility/length options.
    ///
    /// See [`Self::validate_smtp`]. This does not change header parsing policy;
    /// use [`utils::validate_smtp_address_with_options`] to validate original
    /// text, including legacy ISO-2022-JP escape sequences.
    ///
    /// # Errors
    ///
    /// Returns [`crate::SmtpAddressError`] as described by
    /// [`utils::validate_smtp_address_with_options`].
    pub fn validate_smtp_with_options(
        &self,
        options: &crate::SmtpAddressValidationOptions,
    ) -> Result<(), crate::SmtpAddressError> {
        utils::validate_smtp_address_with_options(self.address.as_str(), options)
    }

    /// Validates the stored addr-spec using MimeKit's SMTPUTF8 Unicode policy.
    ///
    /// Ignores the display name, route, and recovery diagnostics, without changing
    /// the mailbox. Checks the stored address, not the original header text.
    /// Domain A-labels are rejected by default; this is stricter than checking
    /// RFC 6531 syntax alone. See [`utils::validate_smtp_utf8_address`].
    ///
    /// # Errors
    ///
    /// Returns the first [`crate::SmtpUtf8AddressError`] in its checking order.
    pub fn validate_smtp_utf8(&self) -> Result<(), crate::SmtpUtf8AddressError> {
        utils::validate_smtp_utf8_address(self.address.as_str())
    }

    /// Validates SMTPUTF8 policy with IDN, script, and length options.
    ///
    /// See [`Self::validate_smtp_utf8`]. IDN decoding is temporary: even after a
    /// successful check, the stored address retains its original spelling.
    ///
    /// # Errors
    ///
    /// Returns errors as described by [`utils::validate_smtp_utf8_address_with_options`].
    ///
    /// ```
    /// use mimecraft::{MailboxAddress, SmtpUtf8AddressError, SmtpUtf8AddressValidationOptions};
    ///
    /// let mailbox: MailboxAddress = "<user@xn--gr-zia.example.com>".parse()?;
    /// assert_eq!(mailbox.validate_smtp_utf8(), Err(SmtpUtf8AddressError::ALabel));
    /// let options = SmtpUtf8AddressValidationOptions { decode_idn: true, ..Default::default() };
    /// assert_eq!(mailbox.validate_smtp_utf8_with_options(&options), Ok(()));
    /// assert_eq!(mailbox.address().as_str(), "user@xn--gr-zia.example.com");
    /// # Ok::<(), mimecraft::MailboxParseError>(())
    /// ```
    pub fn validate_smtp_utf8_with_options(
        &self,
        options: &crate::SmtpUtf8AddressValidationOptions,
    ) -> Result<(), crate::SmtpUtf8AddressError> {
        utils::validate_smtp_utf8_address_with_options(self.address.as_str(), options)
    }

    /// Creates a mailbox from a Unicode display name and a parsed address.
    ///
    /// The name is trimmed, but not unquoted, RFC 2047-decoded, or sanitized.
    /// Use an empty name for an unnamed mailbox. The route is initially empty;
    /// [`Self::route_mut`] can populate it. The address and its diagnostics are
    /// retained unchanged; unlike MimeKit's string constructor, this requires
    /// an [`AddrSpec`] and cannot represent an empty reverse-path address.
    ///
    /// ```
    /// use mimecraft::MailboxAddress;
    ///
    /// let mut mailbox = MailboxAddress::new(" Alice ", "alice@example.com".parse()?);
    /// mailbox.set_display_name("Alice Smith");
    /// mailbox.set_address("alice@example.net".parse()?);
    /// assert_eq!(mailbox.display_name(), "Alice Smith");
    /// assert_eq!(mailbox.address().as_str(), "alice@example.net");
    /// # Ok::<(), mimecraft::AddrSpecParseError>(())
    /// ```
    pub fn new(name: &str, address: AddrSpec) -> Self {
        Self {
            name: assigned_name(name),
            address,
            route: DomainList::new(),
        }
    }

    /// Returns the trimmed, decoded display name, or an empty string.
    pub fn display_name(&self) -> &str {
        &self.name.text
    }

    /// Replaces the name with trimmed Unicode text, without decoding or sanitizing.
    ///
    /// Clears previous name-decoding diagnostics, even when the text is unchanged.
    /// The address, its diagnostics, and the source route are unaffected.
    pub fn set_display_name(&mut self, name: &str) {
        self.name = assigned_name(name);
    }

    /// Returns charset recovery diagnostics for the display name.
    ///
    /// Ranges refer to the name bytes supplied to the RFC 2047 decoder, not
    /// the original mailbox: phrase quotes/escapes and CR/LF have already been
    /// removed and tabs converted to spaces. For comment names only tabs are
    /// converted. Ranges precede trimming of the decoded display name.
    pub fn display_name_issues(&self) -> &[DecodeIssue] {
        &self.name.issues
    }

    /// Returns the normalized addr-spec, without changing its domain spelling.
    pub fn address(&self) -> &AddrSpec {
        &self.address
    }

    /// Replaces the addr-spec and its diagnostics without changing the name or route.
    ///
    /// Parse a replacement before calling this method; a failed parse therefore
    /// leaves the mailbox unchanged. No IDNA conversion is performed.
    pub fn set_address(&mut self, address: AddrSpec) {
        self.address = address;
    }

    /// Returns the source route, empty when absent.
    pub fn route(&self) -> &DomainList {
        &self.route
    }

    /// Exposes source-route edits without domain validation or normalization.
    ///
    /// As with [`DomainList::as_mut_vec`], arbitrary edits are not safe header
    /// output. Name/address values and their diagnostics are unaffected.
    pub fn route_mut(&mut self) -> &mut DomainList {
        &mut self.route
    }

    /// Parses a complete mailbox with default compatibility options.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxParseError`] for malformed input, groups, or extra input.
    pub fn parse(input: &[u8]) -> Result<Self, MailboxParseError> {
        Self::parse_with_options(input, &MailboxParseOptions::default())
    }

    /// Parses a complete mailbox with explicit compatibility options.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxParseError`] with an original-input byte offset.
    pub fn parse_with_options(
        input: &[u8],
        options: &MailboxParseOptions,
    ) -> Result<Self, MailboxParseError> {
        let (mailbox, consumed) = Self::parse_prefix_with_options(input, options)?;
        if consumed != input.len() {
            return Err(MailboxParseError::new(
                MailboxParseErrorKind::TrailingInput,
                consumed,
            ));
        }
        Ok(mailbox)
    }

    /// Parses a prefix, returning the mailbox and consumed byte count.
    ///
    /// Consumes trailing comments/whitespace, leaving list delimiters and other
    /// suffixes for the enclosing parser. This requires a complete input token,
    /// not an incremental chunk. Unquoted commas in names are accepted by default.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxParseError`] without a partial mailbox on failure.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), MailboxParseError> {
        Self::parse_prefix_with_options(input, &MailboxParseOptions::default())
    }

    /// Parses a prefix with explicit options; see [`Self::parse_prefix`].
    ///
    /// # Errors
    ///
    /// Returns [`MailboxParseError`] for malformed or disallowed syntax.
    pub fn parse_prefix_with_options(
        input: &[u8],
        options: &MailboxParseOptions,
    ) -> Result<(Self, usize), MailboxParseError> {
        let mut parser = Parser {
            input,
            options,
            index: 0,
        };
        parser.cfws()?;
        if parser.index == input.len() {
            return Err(parser.error(MailboxParseErrorKind::MissingMailbox));
        }
        let phrase = parser.scan_phrase()?;
        let mailbox = parser.mailbox_from_phrase(phrase)?;
        parser.cfws()?;
        Ok((mailbox, parser.index))
    }
}

pub(crate) fn assigned_name(name: &str) -> DecodedHeader {
    // Leading/trailing whitespace may be folded or dropped beside encoded
    // words, so trim assigned names to keep them consistent across round trips.
    DecodedHeader {
        text: name.trim().to_owned(),
        issues: Vec::new(),
    }
}

// A provisional phrase range; its delimiter determines whether it is a name
// or an addr-spec.
pub(crate) struct Phrase {
    start: usize,
    end: usize,
    trim_leading_quote: bool,
}

pub(crate) struct Parser<'a> {
    pub(crate) input: &'a [u8],
    pub(crate) options: &'a MailboxParseOptions,
    pub(crate) index: usize,
}

impl Parser<'_> {
    pub(crate) fn mailbox_from_phrase(
        &mut self,
        phrase: Phrase,
    ) -> Result<MailboxAddress, MailboxParseError> {
        match self.input.get(self.index) {
            None | Some(b',' | b'>' | b';') => {
                let sentinel = self.input.get(self.index).copied().unwrap_or(b',');
                self.bare(phrase, Some(sentinel))
            }
            Some(b'@') => self.bare(phrase, None),
            Some(b'<') => self.angle(phrase),
            Some(b':') => Err(self.error(MailboxParseErrorKind::GroupNotAllowed)),
            _ => Err(self.error(MailboxParseErrorKind::InvalidMailbox)),
        }
    }

    pub(crate) fn strict(&self) -> bool {
        self.options.address.compliance_mode == AddressComplianceMode::Strict
    }

    fn error(&self, kind: MailboxParseErrorKind) -> MailboxParseError {
        MailboxParseError::new(kind, self.index)
    }

    pub(crate) fn cfws(&mut self) -> Result<(), MailboxParseError> {
        skip_comments_and_whitespace(self.input, &mut self.index).map_err(|offset| {
            MailboxParseError::new(MailboxParseErrorKind::IncompleteComment, offset)
        })
    }

    fn phrase_atom(&mut self) -> bool {
        let start = self.index;
        while self.input.get(self.index).is_some_and(|&byte| {
            // Allow square brackets in loose display names, plus ')' for
            // RFC 7103 unbalanced-parenthesis recovery.
            is_atom(byte) || (!self.strict() && matches!(byte, b'[' | b']' | b')'))
        }) {
            self.index += 1;
        }
        self.index != start
    }

    pub(crate) fn scan_phrase(&mut self) -> Result<Phrase, MailboxParseError> {
        let mut phrase = Phrase {
            start: self.index,
            end: self.index,
            trim_leading_quote: false,
        };
        while self.index < self.input.len() {
            let quoted = self.input[self.index] == b'"';
            if quoted {
                let opening = self.index;
                if skip_quoted(self.input, &mut self.index).is_err() {
                    if self.strict() {
                        return Err(MailboxParseError::new(
                            MailboxParseErrorKind::IncompleteQuotedString,
                            opening,
                        ));
                    }
                    self.index = opening + 1;
                    skip_whitespace(self.input, &mut self.index);
                    if !self.phrase_atom() {
                        return Err(MailboxParseError::new(
                            MailboxParseErrorKind::IncompleteQuotedString,
                            opening,
                        ));
                    }
                    phrase.trim_leading_quote = opening == phrase.start;
                }
            } else if !self.phrase_atom() {
                break;
            }
            phrase.end = self.index;
            loop {
                self.cfws()?;
                // Some clients don't quote dots in the name.
                if self.input.get(self.index) != Some(&b'.') {
                    break;
                }
                self.index += 1;
                phrase.end = self.index;
            }
            // Some clients don't quote commas in the name.
            if self.options.allow_unquoted_commas
                && !quoted
                && self.input.get(self.index) == Some(&b',')
            {
                self.index += 1;
                phrase.end = self.index;
                self.cfws()?;
            }
        }
        self.cfws()?;
        Ok(phrase)
    }

    pub(crate) fn name(&self, phrase: &Phrase) -> DecodedHeader {
        let start = phrase.start + usize::from(phrase.trim_leading_quote);
        // Unquote before decoding to preserve quotes and backslashes inside
        // encoded words.
        let unquoted = utils::unquote_bytes(&self.input[start..phrase.end], true);
        self.decode_name(&unquoted)
    }

    fn decode_name(&self, input: &[u8]) -> DecodedHeader {
        if input.is_ascii() && !input.contains(&b'=') {
            return DecodedHeader {
                text: std::str::from_utf8(input)
                    .expect("plain ASCII is valid UTF-8")
                    .trim()
                    .to_owned(),
                issues: Vec::new(),
            };
        }
        let mut name = rfc2047::decode_phrase_with_options(input, &self.options.name_decoding);
        name.text.truncate(name.text.trim_end().len());
        let start = name.text.len() - name.text.trim_start().len();
        name.text.drain(..start);
        name
    }

    fn addr_spec(&mut self, angle: bool) -> Result<AddrSpec, MailboxParseError> {
        let start = self.index;
        let mut options = self.options.address;
        if angle {
            options.allow_local_only = true;
        }
        let (address, consumed) = AddrSpec::parse_prefix_untrimmed(&self.input[start..], &options)
            .map_err(|error| {
                MailboxParseError::new(
                    MailboxParseErrorKind::AddrSpec(error.kind()),
                    start + error.offset(),
                )
            })?;
        self.index += consumed;
        Ok(address)
    }

    fn angle(&mut self, phrase: Phrase) -> Result<MailboxAddress, MailboxParseError> {
        let name = self.name(&phrase);
        self.index += 1;
        // RFC 7103 section 7.1.2: tolerate excessive angle brackets in loose mode.
        self.excess_angles(b'<')?;
        self.cfws()?;
        let route = if self.input.get(self.index) == Some(&b'@') {
            let start = self.index;
            let (route, consumed) =
                DomainList::parse_prefix(&self.input[start..]).map_err(|error| {
                    MailboxParseError::new(
                        MailboxParseErrorKind::Route(error.kind()),
                        start + error.offset(),
                    )
                })?;
            self.index += consumed;
            if self.input.get(self.index) != Some(&b':') {
                return Err(self.error(MailboxParseErrorKind::MissingRouteColon));
            }
            self.index += 1;
            self.cfws()?;
            route
        } else {
            DomainList::new()
        };
        // Only '>' is syntactically correct, but RFC 7103 section 7.1.5 has
        // <third@example.net, fourth@example.net>. The shared addr-spec parser
        // also stops at comma/semicolon for recovery in enclosing lists/groups.
        let address = self.addr_spec(true)?;
        self.cfws()?;
        if self.input.get(self.index) == Some(&b'>') {
            self.index += 1;
            self.excess_angles(b'>')?;
        } else if self.strict() {
            return Err(self.error(MailboxParseErrorKind::MissingClosingAngleBracket));
        }
        Ok(MailboxAddress {
            name,
            address,
            route,
        })
    }

    fn excess_angles(&mut self, bracket: u8) -> Result<(), MailboxParseError> {
        if self.input.get(self.index) == Some(&bracket) && self.strict() {
            return Err(self.error(MailboxParseErrorKind::ExcessiveAngleBracket));
        }
        while self.input.get(self.index) == Some(&bracket) {
            self.index += 1;
        }
        Ok(())
    }

    fn bare(
        &mut self,
        mut phrase: Phrase,
        local_sentinel: Option<u8>,
    ) -> Result<MailboxAddress, MailboxParseError> {
        // Rewind to the beginning of the local part after the provisional scan.
        self.index = phrase.start;
        let address = self.addr_spec(false)?;
        skip_whitespace(self.input, &mut self.index);
        let name = if self.input.get(self.index) == Some(&b'(') {
            let start = self.index + 1;
            skip_comment(self.input, &mut self.index).map_err(|offset| {
                MailboxParseError::new(MailboxParseErrorKind::IncompleteComment, offset)
            })?;
            let comment = &self.input[start..self.index - 1];
            // Trailing-comment names convert tabs to spaces, consistently with
            // phrase unquoting. Quotes and backslashes in comments are retained.
            let comment = if comment.contains(&b'\t') {
                Cow::Owned(
                    comment
                        .iter()
                        .map(|&byte| if byte == b'\t' { b' ' } else { byte })
                        .collect::<Vec<_>>(),
                )
            } else {
                Cow::Borrowed(comment)
            };
            self.decode_name(&comment)
        } else {
            DecodedHeader {
                text: String::new(),
                issues: Vec::new(),
            }
        };
        self.cfws()?;
        if local_sentinel.is_none() && self.input.get(self.index) == Some(&b'<') {
            // An addr-spec used as an unquoted name: user@example <user@example>.
            if self.strict() {
                return Err(self.error(MailboxParseErrorKind::InvalidMailbox));
            }
            phrase.end = self.index;
            while phrase.end > phrase.start && is_whitespace(self.input[phrase.end - 1]) {
                phrase.end -= 1;
            }
            return self.angle(phrase);
        }
        // RFC 7103 section 7.1.3: accept second@example.org> without an opener.
        if self.input.get(self.index) == Some(&b'>') {
            if self.strict() {
                return Err(self.error(MailboxParseErrorKind::UnexpectedClosingAngleBracket));
            }
            self.index += 1;
        }
        if let Some(sentinel) = local_sentinel
            && self
                .input
                .get(self.index)
                .is_some_and(|&byte| byte != sentinel)
        {
            return Err(self.error(MailboxParseErrorKind::InvalidMailbox));
        }
        Ok(MailboxAddress {
            name,
            address,
            route: DomainList::new(),
        })
    }
}

impl PartialEq for MailboxAddress {
    fn eq(&self, other: &Self) -> bool {
        self.display_name() == other.display_name()
            && self.address.as_str() == other.address.as_str()
    }
}

impl Eq for MailboxAddress {}

impl Hash for MailboxAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.display_name().hash(state);
        self.address.as_str().hash(state);
    }
}

impl FromStr for MailboxAddress {
    type Err = MailboxParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl fmt::Display for MailboxAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let has_route = self
            .route
            .as_slice()
            .iter()
            .any(|domain| !domain.trim().is_empty());
        if self.display_name().is_empty() && !has_route {
            return write!(f, "{}", self.address);
        }
        if !self.display_name().is_empty() {
            write!(f, "{} ", utils::quote(self.display_name()))?;
        }
        f.write_str("<")?;
        if has_route {
            write!(f, "{}:", self.route)?;
        }
        write!(f, "{}>", self.address)
    }
}

/// The reason a mailbox could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxParseErrorKind {
    /// No mailbox followed leading comments and whitespace.
    MissingMailbox,
    /// Unexpected syntax in a mailbox or its display-name phrase.
    InvalidMailbox,
    /// A group delimiter was encountered instead of a single mailbox.
    GroupNotAllowed,
    /// A display-name quoted string was not closed or recoverable.
    IncompleteQuotedString,
    /// A comment was not closed.
    IncompleteComment,
    /// Strict mode encountered repeated angle brackets.
    ExcessiveAngleBracket,
    /// Strict mode expected the closing angle bracket.
    MissingClosingAngleBracket,
    /// Strict mode encountered a closing angle bracket without an opener.
    UnexpectedClosingAngleBracket,
    /// A route was not followed by its separating colon.
    MissingRouteColon,
    /// The underlying addr-spec parser rejected the address.
    AddrSpec(AddrSpecParseErrorKind),
    /// The underlying domain-route parser rejected the route.
    Route(DomainListParseErrorKind),
    /// Extra input followed a complete mailbox.
    TrailingInput,
}

/// A mailbox syntax failure with an original-input byte offset.
///
/// Unclosed comments and quoted strings point to their opening delimiters.
/// Nested addr-spec/route errors are rebased to the complete mailbox input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MailboxParseError {
    kind: MailboxParseErrorKind,
    offset: usize,
}

impl MailboxParseError {
    fn new(kind: MailboxParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the failure category.
    pub fn kind(self) -> MailboxParseErrorKind {
        self.kind
    }

    /// Returns the zero-based original-input byte offset, or EOF.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for MailboxParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            MailboxParseErrorKind::MissingMailbox => "missing mailbox",
            MailboxParseErrorKind::InvalidMailbox => "invalid mailbox syntax",
            MailboxParseErrorKind::GroupNotAllowed => "expected a mailbox, not a group",
            MailboxParseErrorKind::IncompleteQuotedString => "unterminated display-name quote",
            MailboxParseErrorKind::IncompleteComment => "unterminated mailbox comment",
            MailboxParseErrorKind::ExcessiveAngleBracket => "excessive angle bracket",
            MailboxParseErrorKind::MissingClosingAngleBracket => "expected '>' after address",
            MailboxParseErrorKind::UnexpectedClosingAngleBracket => "unexpected '>' after address",
            MailboxParseErrorKind::MissingRouteColon => "expected ':' after route",
            MailboxParseErrorKind::AddrSpec(kind) => {
                return write!(
                    f,
                    "invalid mailbox addr-spec ({kind:?}) at byte {}",
                    self.offset
                );
            }
            MailboxParseErrorKind::Route(kind) => {
                return write!(
                    f,
                    "invalid mailbox route ({kind:?}) at byte {}",
                    self.offset
                );
            }
            MailboxParseErrorKind::TrailingInput => "unexpected input after mailbox",
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for MailboxParseError {}

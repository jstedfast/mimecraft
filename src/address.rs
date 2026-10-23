// Adapted from MimeKit's AddressParser.cs, GroupAddress.cs and InternetAddressList.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::{
    AddressList, GroupAddress, MailboxAddress, MailboxParseError, MailboxParseErrorKind,
    MailboxParseOptions, mailbox_address::Parser,
};

/// Options for addresses, groups, and address lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressParseOptions {
    /// Mailbox syntax and name decoding; also applies to group names.
    pub mailbox: MailboxParseOptions,
    /// Maximum number of enclosing groups, including the outermost; defaults to 1.
    ///
    /// Use `1` for strict RFC 5322 section 3.4 compliance: groups may contain
    /// mailboxes, but not other groups. Higher values allow non-standard nested
    /// groups for compatibility, independently of the mailbox compliance mode.
    /// Zero disables groups. The `u8` bound limits parsed trees to 255 levels,
    /// even if the caller relaxes the default. Parsing uses an explicit stack.
    pub max_group_depth: u8,
}

impl Default for AddressParseOptions {
    fn default() -> Self {
        Self {
            mailbox: MailboxParseOptions::default(),
            max_group_depth: 1,
        }
    }
}

/// A mailbox or named group of addresses.
///
/// This represents address data, not validated SMTP recipients or
/// safe header output. Groups may be empty or nested for compatibility.
/// [`Display`](fmt::Display) delegates to the variant's unencoded display
/// formatter, without transport encoding, validation, or folding.
///
/// Equality and hashing distinguish variants and otherwise follow the contained
/// mailbox/group. In particular, mailbox routes and decoding diagnostics are
/// ignored; names and addr-specs remain case-sensitive.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Address {
    /// One mailbox, optionally with a display name and source route.
    Mailbox(MailboxAddress),
    /// A named group whose members may themselves contain groups.
    Group(GroupAddress),
}

impl Address {
    /// Encodes this variant as an ASCII address value with default options.
    ///
    /// See [`MailboxAddress::to_ascii`] and [`GroupAddress::to_ascii`].
    /// This does not fold a complete field or append a newline.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for invalid names, addresses, or routes.
    pub fn to_ascii(&self) -> Result<String, crate::AddressEncodeError> {
        self.to_ascii_with_options(&crate::rfc2047::EncodeOptions::default())
    }

    /// Encodes this variant with explicit display-name encoding options.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`]; no partial value is returned.
    pub fn to_ascii_with_options(
        &self,
        options: &crate::rfc2047::EncodeOptions,
    ) -> Result<String, crate::AddressEncodeError> {
        match self {
            Self::Mailbox(mailbox) => mailbox.to_ascii_with_options(options),
            Self::Group(group) => group.to_ascii_with_options(options),
        }
    }

    /// Returns the decoded display name of either variant.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Mailbox(mailbox) => mailbox.display_name(),
            Self::Group(group) => group.display_name(),
        }
    }

    /// Replaces either variant's name with trimmed, undecoded Unicode text.
    ///
    /// Clears that name's decoding diagnostics without changing other fields.
    /// This does not sanitize the name for use in headers.
    pub fn set_display_name(&mut self, name: &str) {
        match self {
            Self::Mailbox(mailbox) => mailbox.set_display_name(name),
            Self::Group(group) => group.set_display_name(name),
        }
    }

    /// Parses one complete address with default compatibility options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for malformed input or trailing data.
    pub fn parse(input: &[u8]) -> Result<Self, AddressParseError> {
        Self::parse_with_options(input, &AddressParseOptions::default())
    }

    /// Parses one complete address with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`]; no partial address is returned.
    pub fn parse_with_options(
        input: &[u8],
        options: &AddressParseOptions,
    ) -> Result<Self, AddressParseError> {
        let (address, consumed) = Self::parse_prefix_with_options(input, options)?;
        require_complete(input, consumed)?;
        Ok(address)
    }

    /// Parses an address prefix with default options.
    ///
    /// Returns bytes consumed, including trailing comments/whitespace, but not
    /// an enclosing list's comma or group terminator. This is not incremental.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for malformed input or excessive nesting.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), AddressParseError> {
        Self::parse_prefix_with_options(input, &AddressParseOptions::default())
    }

    /// Parses an address prefix with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] with an original-input byte offset.
    pub fn parse_prefix_with_options(
        input: &[u8],
        options: &AddressParseOptions,
    ) -> Result<(Self, usize), AddressParseError> {
        parse_one(input, options, false)
    }
}

pub(crate) fn parse_one(
    input: &[u8],
    options: &AddressParseOptions,
    group_only: bool,
) -> Result<(Address, usize), AddressParseError> {
    let mut parser = Parser {
        input,
        options: &options.mailbox,
        index: 0,
    };
    let address = parser.address(options.max_group_depth, group_only)?;
    parser.cfws()?;
    Ok((address, parser.index))
}

pub(crate) fn parse_list(
    input: &[u8],
    options: &AddressParseOptions,
) -> Result<AddressList, AddressParseError> {
    let mut parser = Parser {
        input,
        options: &options.mailbox,
        index: 0,
    };
    parser.cfws()?;
    if parser.index == input.len() {
        return Err(parser.address_error(AddressParseErrorKind::MissingAddress));
    }
    let mut addresses = Vec::new();
    while parser.index < input.len() {
        addresses.push(parser.address(options.max_group_depth, false)?);
        parser.list_separator(false)?;
    }
    Ok(addresses.into())
}

pub(crate) fn require_complete(input: &[u8], consumed: usize) -> Result<(), AddressParseError> {
    if consumed != input.len() {
        return Err(AddressParseError::new(
            AddressParseErrorKind::TrailingInput,
            consumed,
        ));
    }
    Ok(())
}

impl Parser<'_> {
    fn address_error(&self, kind: AddressParseErrorKind) -> AddressParseError {
        AddressParseError::new(kind, self.index)
    }

    fn address(&mut self, max_depth: u8, group_only: bool) -> Result<Address, AddressParseError> {
        self.cfws()?;
        if self.index == self.input.len() {
            return Err(self.address_error(AddressParseErrorKind::MissingAddress));
        }
        // Bound nested groups to reject maliciously deep input. An explicit
        // stack replaces the source parser's recursive group/list calls.
        let mut groups: Vec<GroupAddress> = Vec::new();
        loop {
            let phrase = self.scan_phrase()?;
            let mut address = if self.input.get(self.index) == Some(&b':') {
                if groups.len() >= usize::from(max_depth) {
                    return Err(self.address_error(AddressParseErrorKind::GroupDepthExceeded));
                }
                let name = self.name(&phrase);
                self.index += 1;
                while self
                    .input
                    .get(self.index)
                    .is_some_and(|byte| matches!(byte, b':' | b' ' | b'\t'))
                {
                    self.index += 1;
                }
                self.cfws()?;
                if self.index == self.input.len() {
                    return Err(self.address_error(AddressParseErrorKind::MissingAddress));
                }
                groups.push(GroupAddress::from_parsed(name, AddressList::new()));
                if self.input[self.index] != b';' {
                    continue;
                }
                self.index += 1;
                Address::Group(groups.pop().expect("the empty group was just pushed"))
            } else {
                if group_only && groups.is_empty() {
                    return Err(self.address_error(AddressParseErrorKind::ExpectedGroup));
                }
                Address::Mailbox(self.mailbox_from_phrase(phrase)?)
            };
            loop {
                let Some(group) = groups.last_mut() else {
                    return Ok(address);
                };
                group.members_mut().as_mut_vec().push(address);
                self.list_separator(true)?;
                if self.input.get(self.index) == Some(&b';') {
                    self.index += 1;
                } else if self.index != self.input.len() {
                    break;
                } else if self.strict() {
                    return Err(self.address_error(AddressParseErrorKind::MissingGroupTerminator));
                }
                address = Address::Group(groups.pop().expect("a member was added to this group"));
            }
        }
    }

    fn list_separator(&mut self, is_group: bool) -> Result<(), AddressParseError> {
        // Loop in case there are extraneous commas. Leave the group terminator
        // for the group production, rather than interpreting it as a member.
        let mut skipped_comma = false;
        loop {
            self.cfws()?;
            let Some(&byte) = self.input.get(self.index) else {
                return Ok(());
            };
            if is_group && byte == b';' {
                return Ok(());
            }
            if byte != b',' {
                if !skipped_comma && self.strict() {
                    return Err(self.address_error(AddressParseErrorKind::MissingSeparator));
                }
                // In loose mode this may be the start of the next address.
                return Ok(());
            }
            skipped_comma = true;
            self.index += 1;
        }
    }
}

impl FromStr for Address {
    type Err = AddressParseError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mailbox(mailbox) => fmt::Display::fmt(mailbox, f),
            Self::Group(group) => fmt::Display::fmt(group, f),
        }
    }
}

impl From<MailboxAddress> for Address {
    fn from(mailbox: MailboxAddress) -> Self {
        Self::Mailbox(mailbox)
    }
}

impl From<GroupAddress> for Address {
    fn from(group: GroupAddress) -> Self {
        Self::Group(group)
    }
}

/// The reason an address, group, or list could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressParseErrorKind {
    /// No address followed whitespace/comments or a group's colon.
    MissingAddress,
    /// A group-only parser encountered a mailbox or other non-group syntax.
    ExpectedGroup,
    /// Another group would exceed the configured nesting limit.
    GroupDepthExceeded,
    /// Strict mode expected a group's closing semicolon.
    MissingGroupTerminator,
    /// Strict mode expected a comma between addresses.
    MissingSeparator,
    /// The shared mailbox/phrase grammar failed.
    Mailbox(MailboxParseErrorKind),
    /// A complete-value parse found extra input.
    TrailingInput,
}

/// A syntax failure with a zero-based byte offset in the entire supplied input.
///
/// Group members keep their original offsets, not offsets relative to a group.
/// Unterminated quotes/comments follow the mailbox parser's opening-offset rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressParseError {
    kind: AddressParseErrorKind,
    offset: usize,
}

impl AddressParseError {
    fn new(kind: AddressParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the failure category.
    pub fn kind(self) -> AddressParseErrorKind {
        self.kind
    }

    /// Returns the original input byte offset, or EOF.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl From<MailboxParseError> for AddressParseError {
    fn from(error: MailboxParseError) -> Self {
        Self::new(AddressParseErrorKind::Mailbox(error.kind()), error.offset())
    }
}

impl fmt::Display for AddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            AddressParseErrorKind::MissingAddress => "missing address",
            AddressParseErrorKind::ExpectedGroup => "expected an address group",
            AddressParseErrorKind::GroupDepthExceeded => "address group nesting limit exceeded",
            AddressParseErrorKind::MissingGroupTerminator => "expected ';' after group",
            AddressParseErrorKind::MissingSeparator => "expected ',' between addresses",
            AddressParseErrorKind::TrailingInput => "unexpected input after address",
            AddressParseErrorKind::Mailbox(kind) => {
                return write!(f, "invalid address ({kind:?}) at byte {}", self.offset);
            }
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for AddressParseError {}

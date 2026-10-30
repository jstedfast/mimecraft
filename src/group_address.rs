// Adapted from MimeKit's GroupAddress.cs and InternetAddress.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use crate::{
    Address, AddressList, AddressParseError, AddressParseOptions,
    address::{parse_one, require_complete},
    address_list::fmt_addresses,
    mailbox_address::assigned_name,
    rfc2047::{DecodeIssue, DecodedHeader},
};

/// A group name and ordered, editable member list.
///
/// Group names use the same unquoting, RFC 2047 decoding, and trimming as
/// mailbox phrase names. Nested groups require increasing
/// [`AddressParseOptions::max_group_depth`] above its default of 1; this
/// compatibility extension is available even in strict mailbox mode.
///
/// Loose modes tolerate a missing final semicolon after members. A colon
/// followed only by EOF/comments/whitespace is still an error; use `Name:;`
/// for an empty group. Repeated colons after the name are tolerated in all
/// modes. [`Display`](fmt::Display) writes the name verbatim, followed by
/// `: `, comma-separated members, and `;`, including for an empty group.
/// This matches MimeKit's unencoded display output, not safe header syntax:
/// punctuation in the name is not quoted and controls are not sanitized.
///
/// Equality and hashing use the exact stored name and ordered member list,
/// including nested group boundaries and duplicates. Name-decoding diagnostics
/// are ignored, as are member mailboxes' routes and decoding diagnostics.
///
/// ```
/// use mimecraft::GroupAddress;
///
/// let group: GroupAddress = "Friends: Alice <alice@example.com>, bob@example.com;".parse()?;
/// assert_eq!(group.display_name(), "Friends");
/// assert_eq!(group.members().len(), 2);
/// assert_eq!(group.to_string(), "Friends: \"Alice\" <alice@example.com>, bob@example.com;");
/// # Ok::<(), mimecraft::AddressParseError>(())
/// ```
#[derive(Clone, Debug)]
pub struct GroupAddress {
    name: DecodedHeader,
    members: AddressList,
}

impl GroupAddress {
    /// Encodes an ASCII group value with default name-encoding options.
    ///
    /// Group names are encoded or quoted, including `""` for an unnamed group.
    /// Member mailboxes follow [`crate::MailboxAddress::to_ascii`]. Nested
    /// groups use an explicit stack; encoding imposes no parse-time depth limit.
    /// Output has no field name, folding, or terminating newline.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for any invalid name/member, with
    /// indexes relative to this group's members. No partial output is returned.
    pub fn to_ascii(&self) -> Result<String, crate::AddressEncodeError> {
        self.to_ascii_with_options(&crate::rfc2047::EncodeOptions::default())
    }

    /// Encodes an ASCII group with explicit options for all display names.
    ///
    /// See [`Self::to_ascii`]. Stored names, addresses, routes, and diagnostics
    /// are unchanged. Encoded name controls are preserved, not sanitized.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for invalid options or any name/member.
    pub fn to_ascii_with_options(
        &self,
        options: &crate::rfc2047::EncodeOptions,
    ) -> Result<String, crate::AddressEncodeError> {
        crate::address_encode::encode_group(self, options)
    }

    /// Creates a group from a Unicode display name and an ordered member list.
    ///
    /// The name is trimmed of ASCII space/tab/CR/LF, but not unquoted,
    /// RFC 2047-decoded, or sanitized. Unicode whitespace is preserved.
    /// Members are moved without validation, sorting, or deduplication. Unlike
    /// parsing, construction and editing impose no nesting limit; callers must
    /// bound programmatically assembled trees themselves.
    ///
    /// ```
    /// use mimecraft::{GroupAddress, AddressList, MailboxAddress};
    ///
    /// let mut group = GroupAddress::new(" Friends ", AddressList::new());
    /// let mailbox = MailboxAddress::new("Alice", "alice@example.com".parse()?);
    /// group.members_mut().as_mut_vec().push(mailbox.into());
    /// assert_eq!(group.display_name(), "Friends");
    /// assert_eq!(group.members().mailboxes().count(), 1);
    /// # Ok::<(), mimecraft::AddrSpecParseError>(())
    /// ```
    pub fn new(name: &str, members: AddressList) -> Self {
        Self::from_parsed(assigned_name(name), members)
    }

    pub(crate) fn from_parsed(name: DecodedHeader, members: AddressList) -> Self {
        Self { name, members }
    }

    /// Returns the trimmed, decoded group name.
    pub fn display_name(&self) -> &str {
        &self.name.text
    }

    /// Replaces the name, trimming ASCII space/tab/CR/LF without decoding or sanitizing.
    ///
    /// Clears previous name-decoding diagnostics even if the text is unchanged.
    /// Members and their diagnostics are unaffected.
    pub fn set_display_name(&mut self, name: &str) {
        self.name = assigned_name(name);
    }

    /// Returns name charset diagnostics, relative to the preprocessed name
    /// bytes, as with [`crate::MailboxAddress::display_name_issues`].
    pub fn display_name_issues(&self) -> &[DecodeIssue] {
        &self.name.issues
    }

    /// Returns members in source order, including nested and empty groups.
    pub fn members(&self) -> &AddressList {
        &self.members
    }

    /// Exposes member edits without authoring validation or nesting limits.
    ///
    /// Edits preserve this group's name and name-decoding diagnostics.
    /// [`AddressParseOptions::max_group_depth`] applies only during parsing.
    pub fn members_mut(&mut self) -> &mut AddressList {
        &mut self.members
    }

    /// Parses a complete group with default options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for malformed input or a non-group address.
    pub fn parse(input: &[u8]) -> Result<Self, AddressParseError> {
        Self::parse_with_options(input, &AddressParseOptions::default())
    }

    /// Parses a complete group with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for syntax errors, nesting, or trailing data.
    pub fn parse_with_options(
        input: &[u8],
        options: &AddressParseOptions,
    ) -> Result<Self, AddressParseError> {
        let (group, consumed) = Self::parse_prefix_with_options(input, options)?;
        require_complete(input, consumed)?;
        Ok(group)
    }

    /// Parses one group prefix, including its closing semicolon when present.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] without returning partial members.
    pub fn parse_prefix(input: &[u8]) -> Result<(Self, usize), AddressParseError> {
        Self::parse_prefix_with_options(input, &AddressParseOptions::default())
    }

    /// Parses one group prefix with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for malformed or non-group input.
    pub fn parse_prefix_with_options(
        input: &[u8],
        options: &AddressParseOptions,
    ) -> Result<(Self, usize), AddressParseError> {
        let (address, consumed) = parse_one(input, options, true)?;
        let Address::Group(group) = address else {
            unreachable!("group-only parsing rejects mailboxes");
        };
        Ok((group, consumed))
    }
}

impl PartialEq for GroupAddress {
    fn eq(&self, other: &Self) -> bool {
        self.display_name() == other.display_name() && self.members == other.members
    }
}

impl Eq for GroupAddress {}

impl Hash for GroupAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.display_name().hash(state);
        self.members.hash(state);
    }
}

impl FromStr for GroupAddress {
    type Err = AddressParseError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl fmt::Display for GroupAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.display_name())?;
        fmt_addresses(self.members.as_slice(), f)?;
        f.write_str(";")
    }
}

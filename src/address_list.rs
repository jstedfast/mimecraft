// Adapted from MimeKit's InternetAddressList.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{fmt, iter::FusedIterator, str::FromStr};

use crate::{Address, AddressParseError, AddressParseOptions, MailboxAddress, address::parse_list};

/// An ordered, editable collection of addresses.
///
/// Parsing is fail-fast: malformed members are never silently discarded,
/// including inside groups. Strict mode requires commas between addresses;
/// loose modes tolerate missing separators. Repeated/trailing commas after
/// a valid address are accepted in all modes, but leading commas are errors.
/// Unquoted name commas retain [`crate::MailboxParseOptions`] semantics.
///
/// Parsing empty/whitespace-only input fails, though [`Self::new`] creates an
/// empty collection and parsed groups can have no members. [`Self::to_ascii`]
/// validates and encodes ASCII value syntax; [`Self::to_header_string`] adds
/// a field name and folding. Malformed-header recovery is not provided here.
///
/// [`Display`](fmt::Display) joins unencoded address display strings with `, `,
/// preserving groups, empty groups, and duplicates. An empty list displays as
/// an empty string. Nested groups are formatted with an explicit stack.
/// This output is not sanitized, encoded, or folded for use in headers.
///
/// Equality and hashing compare ordered addresses, not flattened mailboxes.
/// Order, duplicates, empty groups, and nested group boundaries matter;
/// each address uses its own name/address equality semantics.
///
/// ```
/// use mimecraft::AddressList;
///
/// let list: AddressList = "Empty:;, Friends: a@example.com, b@example.com;, c@example.com".parse()?;
/// assert_eq!(list.len(), 3);
/// let addresses: Vec<_> = list.mailboxes().map(|m| m.address().as_str()).collect();
/// assert_eq!(addresses, ["a@example.com", "b@example.com", "c@example.com"]);
/// assert_eq!(list.to_string(), "Empty: ;, Friends: a@example.com, b@example.com;, c@example.com");
/// # Ok::<(), mimecraft::AddressParseError>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct AddressList {
    addresses: Vec<Address>,
}

impl AddressList {
    /// Creates a complete ASCII [`crate::Header`] with default formatting options.
    ///
    /// Uses [`Self::to_header_string`]'s field validation, folding, and CRLF
    /// termination. The result can be added to or replace a field in a
    /// [`crate::HeaderList`]. Encoding completes before any collection edit.
    ///
    /// ```
    /// use mimecraft::{AddressList, HeaderList};
    ///
    /// let addresses: AddressList = "Alice <alice@example.com>".parse()?;
    /// let mut headers = HeaderList::new();
    /// headers.replace_all(addresses.to_header("To")?)?;
    /// assert_eq!(headers.get("to").unwrap().parse_addresses()?, addresses);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressHeaderFormatError`] without a partial record.
    pub fn to_header(&self, field: &str) -> Result<crate::Header, crate::AddressHeaderFormatError> {
        self.to_header_with_options(field, &crate::AddressHeaderFormatOptions::default())
    }

    /// Creates a header record with explicit address-formatting options.
    ///
    /// Preserves the formatter's field spelling, folds, and newline choice.
    /// Stored addresses and decoding diagnostics are not changed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressHeaderFormatError`] for invalid fields/options,
    /// encoding failures, or pieces that cannot fit the physical-line limit.
    pub fn to_header_with_options(
        &self,
        field: &str,
        options: &crate::AddressHeaderFormatOptions,
    ) -> Result<crate::Header, crate::AddressHeaderFormatError> {
        self.to_header_string_with_options(field, options)
            .map(crate::Header::from_formatted)
    }

    /// Serializes a complete ASCII address header using default formatting options.
    ///
    /// Includes the supplied field name and colon, folding as needed, and one
    /// terminating CRLF. An empty list produces `field:\r\n`. Field spelling is
    /// preserved; callers select an appropriate field and enforce its address
    /// count/group requirements. This does not validate field-specific semantics.
    ///
    /// ```
    /// use mimecraft::AddressList;
    ///
    /// let list: AddressList = "Alice <alice@example.com>, bob@example.com".parse()?;
    /// assert_eq!(list.to_header_string("To")?,
    ///     "To: Alice <alice@example.com>, bob@example.com\r\n");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressHeaderFormatError`] for invalid fields, addresses,
    /// or indivisible pieces that cannot fit the physical-line limit.
    pub fn to_header_string(&self, field: &str) -> Result<String, crate::AddressHeaderFormatError> {
        self.to_header_string_with_options(field, &crate::AddressHeaderFormatOptions::default())
    }

    /// Serializes a complete ASCII header with explicit folding and name options.
    ///
    /// Folds use a continuation tab, without splitting quoted strings,
    /// addr-specs, source routes, or attached punctuation. Long display names
    /// use bounded encoded words. Values and diagnostics remain unchanged.
    /// Name controls are encoded, not sanitized; SMTPUTF8 output is not supported.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressHeaderFormatError`] for invalid field names/options,
    /// encoding failures, or pieces that cannot fit. No partial header is returned.
    pub fn to_header_string_with_options(
        &self,
        field: &str,
        options: &crate::AddressHeaderFormatOptions,
    ) -> Result<String, crate::AddressHeaderFormatError> {
        crate::address_header_format::format_header(self, field, options)
    }

    /// Encodes a comma-separated ASCII value with default name-encoding options.
    ///
    /// Preserves address order, groups, and duplicates; an empty list produces
    /// an empty string. Each mailbox follows [`MailboxAddress::to_ascii`].
    /// This is not a complete folded header: no field name or newline is added.
    ///
    /// ```
    /// use mimecraft::AddressList;
    ///
    /// let addresses: AddressList = "Friends: Alice <alice@example.com>;, bob@example.com".parse()?;
    /// assert_eq!(addresses.to_ascii()?, "Friends: Alice <alice@example.com>;, bob@example.com");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for any invalid name/address,
    /// including its top-level and nested member indexes. No partial list is returned.
    pub fn to_ascii(&self) -> Result<String, crate::AddressEncodeError> {
        self.to_ascii_with_options(&crate::rfc2047::EncodeOptions::default())
    }

    /// Encodes an ASCII list with explicit options for all display names.
    ///
    /// See [`Self::to_ascii`]. Options are checked even for an empty list.
    /// Names may encode controls; this does not sanitize their decoded values.
    ///
    /// # Errors
    ///
    /// Returns [`crate::AddressEncodeError`] for invalid options or any name/address.
    pub fn to_ascii_with_options(
        &self,
        options: &crate::rfc2047::EncodeOptions,
    ) -> Result<String, crate::AddressEncodeError> {
        crate::address_encode::encode_list(self, options)
    }

    /// Creates an empty list without allocating.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of top-level addresses, counting each group once.
    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    /// Returns whether there are no top-level addresses.
    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }

    /// Returns addresses in order, preserving duplicates.
    pub fn as_slice(&self) -> &[Address] {
        &self.addresses
    }

    /// Exposes ordinary vector edits without authoring validation.
    pub fn as_mut_vec(&mut self) -> &mut Vec<Address> {
        &mut self.addresses
    }

    /// Returns the stored vector without cloning its addresses.
    pub fn into_vec(self) -> Vec<Address> {
        self.addresses
    }

    /// Traverses mailboxes depth-first in source order, skipping empty groups.
    ///
    /// Duplicates remain; names and group boundaries are not modified.
    pub fn mailboxes(&self) -> Mailboxes<'_> {
        Mailboxes {
            stack: vec![self.addresses.iter()],
        }
    }

    /// Parses a complete list with default options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for any malformed entry, not a partial list.
    pub fn parse(input: &[u8]) -> Result<Self, AddressParseError> {
        Self::parse_with_options(input, &AddressParseOptions::default())
    }

    /// Parses a complete list with explicit options.
    ///
    /// # Errors
    ///
    /// Returns [`AddressParseError`] for empty input, malformed members,
    /// missing strict-mode separators, or excessive nesting.
    pub fn parse_with_options(
        input: &[u8],
        options: &AddressParseOptions,
    ) -> Result<Self, AddressParseError> {
        parse_list(input, options)
    }
}

/// Depth-first borrowed mailbox traversal returned by [`AddressList::mailboxes`].
#[derive(Clone, Debug)]
#[must_use = "iterators are lazy"]
pub struct Mailboxes<'a> {
    stack: Vec<std::slice::Iter<'a, Address>>,
}

impl<'a> Iterator for Mailboxes<'a> {
    type Item = &'a MailboxAddress;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.stack.last_mut()?.next() {
                Some(Address::Mailbox(mailbox)) => return Some(mailbox),
                Some(Address::Group(group)) => self.stack.push(group.members().as_slice().iter()),
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}

impl FusedIterator for Mailboxes<'_> {}

impl fmt::Display for AddressList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_addresses(&self.addresses, f)
    }
}

pub(crate) fn fmt_addresses(addresses: &[Address], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let mut stack = vec![(addresses.iter(), false)];
    while let Some((iter, started)) = stack.last_mut() {
        let Some(address) = iter.next() else {
            stack.pop();
            if !stack.is_empty() {
                f.write_str(";")?;
            }
            continue;
        };
        if *started {
            f.write_str(", ")?;
        }
        *started = true;
        match address {
            Address::Mailbox(mailbox) => write!(f, "{mailbox}")?,
            Address::Group(group) => {
                write!(f, "{}: ", group.display_name())?;
                stack.push((group.members().as_slice().iter(), false));
            }
        }
    }
    Ok(())
}

impl FromStr for AddressList {
    type Err = AddressParseError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input.as_bytes())
    }
}

impl From<Vec<Address>> for AddressList {
    fn from(addresses: Vec<Address>) -> Self {
        Self { addresses }
    }
}

impl FromIterator<Address> for AddressList {
    fn from_iter<T: IntoIterator<Item = Address>>(iter: T) -> Self {
        Self {
            addresses: iter.into_iter().collect(),
        }
    }
}

impl IntoIterator for AddressList {
    type Item = Address;
    type IntoIter = std::vec::IntoIter<Address>;
    fn into_iter(self) -> Self::IntoIter {
        self.addresses.into_iter()
    }
}

impl<'a> IntoIterator for &'a AddressList {
    type Item = &'a Address;
    type IntoIter = std::slice::Iter<'a, Address>;
    fn into_iter(self) -> Self::IntoIter {
        self.addresses.iter()
    }
}

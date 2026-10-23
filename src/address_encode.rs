// Adapted from MimeKit's MailboxAddress.cs, GroupAddress.cs and InternetAddressList.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    AddrSpec, AddrSpecParseError, AddrSpecParseOptions, Address, AddressComplianceMode,
    AddressList, DomainListEncodeError, GroupAddress, MailboxAddress,
    idna::IdnaError,
    rfc2047::{self, EncodeOptions},
};

/// Why an address could not be encoded as an ASCII value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddressEncodeErrorKind {
    /// Invalid name-encoding options, unmappable text, or an encoded word too long.
    Name(rfc2047::EncodeError),
    /// A local part requires internationalized address support and cannot be downgraded.
    InternationalLocalPart,
    /// An address accepted by compatibility parsing failed strict syntax parsing.
    AddrSpec(AddrSpecParseError),
    /// An address domain failed IDNA conversion or MIME-safety validation.
    Domain(IdnaError),
    /// A stored route domain failed validation; the source includes its route index.
    Route(DomainListEncodeError),
}

/// An ASCII address-value encoding failure, with no partial output returned.
///
/// [`Self::path`] identifies the failing address by zero-based member indexes.
/// For a list, its first element indexes the top-level list. For a standalone
/// group, it starts with that group's members. An empty path identifies the
/// standalone address/group itself or invalid options checked before encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressEncodeError {
    path: Vec<usize>,
    kind: AddressEncodeErrorKind,
}

impl AddressEncodeError {
    /// Returns indexes leading to the failing address, or an empty root path.
    pub fn path(&self) -> &[usize] {
        &self.path
    }

    /// Returns the failure category and any underlying conversion error.
    pub fn kind(&self) -> &AddressEncodeErrorKind {
        &self.kind
    }

    fn at_root(kind: AddressEncodeErrorKind) -> Self {
        Self {
            path: Vec::new(),
            kind,
        }
    }
}

impl fmt::Display for AddressEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cannot encode ASCII address")?;
        if !self.path.is_empty() {
            write!(f, " at member path {:?}", self.path)?;
        }
        match &self.kind {
            AddressEncodeErrorKind::Name(error) => write!(f, ": display name: {error}"),
            AddressEncodeErrorKind::InternationalLocalPart => {
                f.write_str(": international local part cannot be downgraded")
            }
            AddressEncodeErrorKind::AddrSpec(error) => write!(f, ": {error}"),
            AddressEncodeErrorKind::Domain(error) => write!(f, ": domain: {error}"),
            AddressEncodeErrorKind::Route(error) => write!(f, ": {error}"),
        }
    }
}

impl Error for AddressEncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            AddressEncodeErrorKind::Name(error) => Some(error),
            AddressEncodeErrorKind::AddrSpec(error) => Some(error),
            AddressEncodeErrorKind::Domain(error) => Some(error),
            AddressEncodeErrorKind::Route(error) => Some(error),
            AddressEncodeErrorKind::InternationalLocalPart => None,
        }
    }
}

pub(crate) fn validate_options(options: &EncodeOptions) -> Result<(), AddressEncodeError> {
    if !(1..=75).contains(&options.max_word_length) {
        return Err(AddressEncodeError::at_root(AddressEncodeErrorKind::Name(
            rfc2047::EncodeError::InvalidWordLength {
                requested: options.max_word_length,
            },
        )));
    }
    Ok(())
}

pub(crate) fn encode_mailbox(
    mailbox: &MailboxAddress,
    options: &EncodeOptions,
) -> Result<String, AddressEncodeError> {
    validate_options(options)?;
    let mut output = String::new();
    append_mailbox(mailbox, options, &mut output).map_err(AddressEncodeError::at_root)?;
    Ok(output)
}

pub(crate) fn encode_group(
    group: &GroupAddress,
    options: &EncodeOptions,
) -> Result<String, AddressEncodeError> {
    validate_options(options)?;
    let mut output = String::new();
    append_group_name(group, options, &mut output).map_err(AddressEncodeError::at_root)?;
    append_members(group.members(), options, &mut output)?;
    output.push(';');
    Ok(output)
}

pub(crate) fn encode_list(
    list: &AddressList,
    options: &EncodeOptions,
) -> Result<String, AddressEncodeError> {
    validate_options(options)?;
    let mut output = String::new();
    append_members(list, options, &mut output)?;
    Ok(output)
}

fn append_mailbox(
    mailbox: &MailboxAddress,
    options: &EncodeOptions,
    output: &mut String,
) -> Result<(), AddressEncodeErrorKind> {
    let address = mailbox.address();
    if !address.local_part().is_ascii() {
        return Err(AddressEncodeErrorKind::InternationalLocalPart);
    }
    // Compatibility parsing can retain consecutive/trailing local-part dots.
    // Do not silently emit those malformed forms as authored address syntax.
    AddrSpec::parse_with_options(
        address.as_str().as_bytes(),
        &AddrSpecParseOptions {
            compliance_mode: AddressComplianceMode::Strict,
            allow_local_only: true,
        },
    )
    .map_err(AddressEncodeErrorKind::AddrSpec)?;
    let ascii = address
        .with_ascii_domain()
        .map_err(AddressEncodeErrorKind::Domain)?;
    let route = mailbox
        .route()
        .to_ascii()
        .map_err(AddressEncodeErrorKind::Route)?;
    let named = !mailbox.display_name().is_empty();
    if named {
        output.push_str(
            &rfc2047::encode_phrase_with_options(mailbox.display_name(), options)
                .map_err(AddressEncodeErrorKind::Name)?,
        );
        output.push(' ');
    }
    // Angle brackets keep local-only mailboxes parseable even when bare
    // addresses without a domain are disabled by the reader.
    let angled = named || !route.is_empty() || address.domain().is_none();
    if angled {
        output.push('<');
    }
    if !route.is_empty() {
        output.push_str(&route);
        output.push(':');
    }
    output.push_str(&ascii);
    if angled {
        output.push('>');
    }
    Ok(())
}

fn append_group_name(
    group: &GroupAddress,
    options: &EncodeOptions,
    output: &mut String,
) -> Result<(), AddressEncodeErrorKind> {
    // An empty quoted name is significant: consecutive bare colons would be
    // treated as compatibility noise rather than nested unnamed groups.
    output.push_str(
        &rfc2047::encode_phrase_with_options(group.display_name(), options)
            .map_err(AddressEncodeErrorKind::Name)?,
    );
    output.push_str(": ");
    Ok(())
}

fn append_members(
    list: &AddressList,
    options: &EncodeOptions,
    output: &mut String,
) -> Result<(), AddressEncodeError> {
    let mut stack = vec![list.as_slice().iter().enumerate()];
    let mut path = Vec::new();
    while let Some(iter) = stack.last_mut() {
        let Some((index, address)) = iter.next() else {
            stack.pop();
            if !stack.is_empty() {
                output.push(';');
                path.pop();
            }
            continue;
        };
        if index > 0 {
            output.push_str(", ");
        }
        path.push(index);
        let result = match address {
            Address::Mailbox(mailbox) => append_mailbox(mailbox, options, output),
            Address::Group(group) => append_group_name(group, options, output),
        };
        result.map_err(|kind| AddressEncodeError {
            path: path.clone(),
            kind,
        })?;
        if let Address::Group(group) = address {
            stack.push(group.members().as_slice().iter().enumerate());
        } else {
            path.pop();
        }
    }
    Ok(())
}

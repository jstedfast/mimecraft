// Adapted from MimeKit's Utils/SmtpAddressValidator.cs, SmtpAddressSyntax.cs,
// SmtpAddressValidationOptions.cs, SmtpAddressError.cs and SmtpUtf8AddressValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::parse_utils::{is_atom, skip_ipv4, skip_ipv6};

/// Options for validating non-SMTPUTF8 envelope addresses.
///
/// Defaults check syntax and ASCII without imposing length limits. Validation
/// never normalizes, decodes, or performs DNS or mailbox-existence checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SmtpAddressValidationOptions {
    /// Accept legacy ISO-2022-JP escape sequences inside a quoted local part.
    ///
    /// Defaults to false. This is a compatibility exception, not standard SMTP
    /// syntax: ESC is otherwise forbidden. Accepts ASCII (`ESC ( B`),
    /// JIS Roman (`ESC ( J`), and JIS X 0208 (`ESC $ @` or `ESC $ B`).
    /// The local part must return to ASCII before its closing quote.
    /// JIS X 0208 runs require complete pairs of bytes in `0x21..=0x7e`;
    /// quotes and backslashes still need quoted-pair escapes.
    ///
    /// Only escape syntax and byte pairing are checked, not assigned Japanese
    /// characters. This does not permit escape sequences in domains or
    /// unquoted local parts. Supply original text to
    /// [`crate::utils::validate_smtp_address_with_options`]; header parsing
    /// may reject or normalize text and is not part of this validation.
    pub allow_iso2022_jp_local_part: bool,
    /// Enforce local-part (64), label (63), domain (253), and address (254) byte limits.
    ///
    /// Defaults to false. Quotes, quoted-pair escapes, and ISO-2022-JP escape
    /// sequences count toward the local-part limit. The 254-byte address
    /// limit reserves two bytes for the SMTP path's angle brackets.
    /// Label and domain limits do not apply to address literals.
    pub enforce_length_limits: bool,
}

/// Why a non-SMTPUTF8 envelope address was rejected.
///
/// Checks run in the order listed here; only the first failure is returned.
/// Successful validation uses `Ok(())`, not an error variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpAddressError {
    /// Not a complete SMTP mailbox: local part, `@`, and domain/address literal.
    Syntax,
    /// Otherwise syntactically valid text contains non-ASCII characters.
    ///
    /// This is not full SMTPUTF8 validation. If only the domain is non-ASCII,
    /// explicitly convert it with [`crate::AddrSpec::with_ascii_domain`] and
    /// validate that result instead; local parts cannot be silently downgraded.
    NonAscii,
    /// Local part exceeds 64 bytes, with length enforcement enabled.
    LocalPartTooLong,
    /// Domain label exceeds 63 bytes, with length enforcement enabled.
    LabelTooLong,
    /// Nonliteral domain exceeds 253 bytes, with length enforcement enabled.
    DomainTooLong,
    /// Complete address exceeds 254 bytes, with length enforcement enabled.
    AddressTooLong,
}

impl fmt::Display for SmtpAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "invalid SMTP address syntax",
            Self::NonAscii => "SMTP address contains non-ASCII characters",
            Self::LocalPartTooLong => "SMTP local part exceeds 64 bytes",
            Self::LabelTooLong => "SMTP domain label exceeds 63 bytes",
            Self::DomainTooLong => "SMTP domain exceeds 253 bytes",
            Self::AddressTooLong => "SMTP address exceeds 254 bytes",
        })
    }
}

impl Error for SmtpAddressError {}

/// Validates an address for non-SMTPUTF8 SMTP envelope use.
///
/// Accepts an addr-spec such as `user@example.com`, not a display name, angle
/// brackets, comments, surrounding whitespace, or a null reverse path.
/// This checks the supplied text exactly, without IDNA conversion or repairs.
/// ASCII A-label spelling is accepted without checking its IDNA validity.
/// Use [`validate_smtp_address_with_options`] to enforce length limits or allow
/// legacy ISO-2022-JP quoted local parts; both are disabled by default.
///
/// This is independent of header parsing, header formatting, and compliance
/// logging. A valid envelope address says nothing about header text or transport
/// capabilities. For example, raw UTF-8 headers can still require SMTPUTF8.
///
/// # Errors
///
/// Returns [`SmtpAddressError::Syntax`] before [`SmtpAddressError::NonAscii`]
/// for malformed international text. Does not establish deliverability.
///
/// ```
/// use mimecraft::{SmtpAddressError, utils::validate_smtp_address};
///
/// assert_eq!(validate_smtp_address("user@example.com"), Ok(()));
/// assert_eq!(validate_smtp_address("caf\u{e9}@example.com"),
///     Err(SmtpAddressError::NonAscii));
/// assert_eq!(validate_smtp_address("User <user@example.com>"),
///     Err(SmtpAddressError::Syntax));
/// ```
pub fn validate_smtp_address(address: &str) -> Result<(), SmtpAddressError> {
    validate_smtp_address_with_options(address, &SmtpAddressValidationOptions::default())
}

/// Validates an SMTP envelope address with explicit compatibility/length options.
///
/// See [`validate_smtp_address`]. Address literals support IPv4, `IPv6:`, and
/// general `tag:content` syntax. IPv4 allows decimal leading zeros; SMTP IPv6
/// compression must replace at least two groups. General tags are checked
/// syntactically, without consulting a registry.
///
/// # Errors
///
/// Returns the first [`SmtpAddressError`] in syntax, ASCII, local-part length,
/// label length, domain length, then total address length order.
///
/// ```
/// use mimecraft::{SmtpAddressError, SmtpAddressValidationOptions, utils};
///
/// let options = SmtpAddressValidationOptions {
///     enforce_length_limits: true,
///     ..Default::default()
/// };
/// let address = format!("{}@example.com", "a".repeat(65));
/// assert_eq!(utils::validate_smtp_address(&address), Ok(()));
/// assert_eq!(utils::validate_smtp_address_with_options(&address, &options),
///     Err(SmtpAddressError::LocalPartTooLong));
/// ```
pub fn validate_smtp_address_with_options(
    address: &str,
    options: &SmtpAddressValidationOptions,
) -> Result<(), SmtpAddressError> {
    // The syntax permits UTF8-non-ascii (as extended by RFC 6531) so that an
    // address invalid only because of non-ASCII text can be reported separately.
    let domain_index = validate_syntax(address, options.allow_iso2022_jp_local_part)
        .ok_or(SmtpAddressError::Syntax)?;
    if !address.is_ascii() {
        return Err(SmtpAddressError::NonAscii);
    }
    if options.enforce_length_limits {
        // ASCII label lengths need no IDNA conversion.
        validate_lengths(address, domain_index, |label| Ok(label.len())).map_err(|error| {
            match error {
                LengthError::LocalPart => SmtpAddressError::LocalPartTooLong,
                LengthError::Label => SmtpAddressError::LabelTooLong,
                LengthError::Domain => SmtpAddressError::DomainTooLong,
                LengthError::Address => SmtpAddressError::AddressTooLong,
            }
        })?;
    }
    Ok(())
}

// Mailbox = Local-part "@" ( Domain / address-literal )
pub(crate) fn validate_syntax(address: &str, allow_iso2022_jp: bool) -> Option<usize> {
    let input = address.as_bytes();
    let mut index = 0;
    let valid = if input.first() == Some(&b'"') {
        skip_quoted_local(input, &mut index, allow_iso2022_jp)
    } else {
        skip_dot_string(input, &mut index, false)
    };
    if !valid || input.get(index) != Some(&b'@') {
        return None;
    }
    index += 1;
    let domain_index = index;
    if input.get(index) == Some(&b'[') {
        return valid_literal(&address[index..]).then_some(domain_index);
    }
    let valid = skip_dot_string(input, &mut index, true);
    (valid && index == input.len()).then_some(domain_index)
}

pub(crate) enum LengthError {
    LocalPart,
    Label,
    Domain,
    Address,
}

pub(crate) fn validate_lengths(
    address: &str,
    domain_index: usize,
    label_length: impl Fn(&str) -> Result<usize, LengthError>,
) -> Result<(), LengthError> {
    // RFC 6531 measures UTF-8 octets, not characters. domain_index - 1 is '@'.
    // RFC 5321 limits the local part to 64 octets, including quotes/escapes.
    if domain_index - 1 > 64 {
        return Err(LengthError::LocalPart);
    }
    let domain = &address[domain_index..];
    if !domain.starts_with('[') {
        let mut domain_length: usize = 0;
        for (index, label) in domain.split('.').enumerate() {
            let length = label_length(label)?;
            // DNS labels are limited to 63 octets in A-label form.
            if length > 63 {
                return Err(LengthError::Label);
            }
            domain_length = domain_length.saturating_add(length + usize::from(index != 0));
        }
        // DNS limits the text form (without the trailing dot) to 253 octets.
        if domain_length > 253 {
            return Err(LengthError::Domain);
        }
    }
    // An SMTP path permits 256 octets, leaving 254 after the angle brackets.
    if address.len() > 254 {
        return Err(LengthError::Address);
    }
    Ok(())
}

// Dot-string = Atom *("." Atom), Atom = 1*atext.
// Domain labels instead allow letters, digits, and interior hyphens.
// RFC 6531 extends atext and domain labels to allow UTF8-non-ascii.
fn skip_dot_string(input: &[u8], index: &mut usize, domain: bool) -> bool {
    loop {
        let start = *index;
        while let Some(&byte) = input.get(*index) {
            let valid = if domain {
                byte >= 128 || byte == b'-' || byte.is_ascii_alphanumeric()
            } else {
                is_atom(byte)
            };
            if !valid {
                break;
            }
            *index += 1;
        }
        if *index == start || (domain && (input[start] == b'-' || input[*index - 1] == b'-')) {
            return false;
        }
        if input.get(*index) != Some(&b'.') {
            return true;
        }
        *index += 1;
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Iso2022JpMode {
    Ascii,
    JisRoman,
    JisX0208,
}

// Quoted-string = DQUOTE *QcontentSMTP DQUOTE
// QcontentSMTP = qtextSMTP / quoted-pairSMTP
// quoted-pairSMTP = %d92 %d32-126
// qtextSMTP = %d32-33 / %d35-91 / %d93-126 / UTF8-non-ascii
fn skip_quoted_local(input: &[u8], index: &mut usize, allow_iso2022_jp: bool) -> bool {
    let mut mode = Iso2022JpMode::Ascii;
    let mut pending = false;
    *index += 1;
    while let Some(&byte) = input.get(*index) {
        *index += 1;
        if byte == b'"' {
            return mode == Iso2022JpMode::Ascii;
        }
        if allow_iso2022_jp && byte == 0x1b {
            // Legacy Japanese clients use ESC even though SMTP forbids it.
            // A JIS X 0208 character is two bytes: a run cannot end mid-pair.
            if pending {
                return false;
            }
            // RFC 1468 starts/ends in ASCII. ESC ( B selects ASCII; ESC ( J
            // selects JIS Roman; ESC $ @ / ESC $ B select JIS X 0208.
            mode = match (input.get(*index), input.get(*index + 1)) {
                (Some(b'('), Some(b'B')) => Iso2022JpMode::Ascii,
                (Some(b'('), Some(b'J')) => Iso2022JpMode::JisRoman,
                (Some(b'$'), Some(b'@' | b'B')) => Iso2022JpMode::JisX0208,
                _ => return false,
            };
            *index += 2;
            continue;
        }
        let value = if byte == b'\\' {
            let Some(&escaped) = input
                .get(*index)
                .filter(|&&value| (32..=126).contains(&value))
            else {
                return false;
            };
            *index += 1;
            escaped
        } else if byte < 32 || byte == 127 {
            return false;
        } else {
            byte
        };
        // Quotes/backslashes still require quoted pairs inside Japanese text.
        // Only byte pairing is checked; unassigned JIS characters are not decoded.
        if mode == Iso2022JpMode::JisX0208 {
            if !(0x21..=0x7e).contains(&value) {
                return false;
            }
            pending = !pending;
        } else if mode == Iso2022JpMode::JisRoman && value >= 128 {
            return false;
        }
    }
    false
}

// address-literal = "[" ( IPv4 / "IPv6:" IPv6 / General-address-literal ) "]"
fn valid_literal(domain: &str) -> bool {
    let Some(end) = domain.find(']') else {
        return false;
    };
    if end != domain.len() - 1 {
        return false;
    }
    let literal = &domain[1..end];
    let mut index = 0;
    if literal
        .as_bytes()
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"IPv6:"))
    {
        index = 5;
        skip_ipv6(literal, &mut index, true) && index == literal.len()
    } else if let Some((tag, content)) = literal.split_once(':') {
        // Standardized-tag = Ldh-str; dcontent = %d33-90 / %d94-126.
        // Unlike a domain label, Ldh-str permits a leading hyphen.
        !tag.is_empty()
            && !tag.ends_with('-')
            && tag
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_alphanumeric())
            && !content.is_empty()
            && content
                .bytes()
                .all(|byte| (33..=126).contains(&byte) && !matches!(byte, b'[' | b'\\'))
    } else {
        skip_ipv4(literal, &mut index, true) && index == literal.len()
    }
}

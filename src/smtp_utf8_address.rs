// Adapted from MimeKit's Utils/SmtpUtf8AddressValidator.cs,
// SmtpUtf8AddressValidationOptions.cs and SmtpUtf8AddressError.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    smtp_address::{LengthError, validate_lengths, validate_syntax},
    smtp_utf8_data::*,
    unicode_normalization::{FIRST_NORMALIZATION_CODE_POINT, is_nfc, lookup},
};

/// Optional policies for MimeKit-compatible SMTPUTF8 address validation.
///
/// All options default to false. This is a Unicode address policy based on
/// `draft-ietf-mailmaint-smtputf8-syntax`, stricter than RFC 6531 syntax alone.
/// It is neither a complete spoofing detector nor proof of deliverability.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SmtpUtf8AddressValidationOptions {
    /// Decode domain A-labels before applying Unicode policy; defaults to false.
    ///
    /// Uses raw RFC 3492 Punycode, not UTS #46 mapping. A-labels must be ASCII,
    /// at most 63 bytes, canonically encoded (ignoring ASCII case), and decode
    /// to at least one non-ASCII scalar. Decoded labels cannot start/end with
    /// hyphens or contain hyphens in both the third and fourth positions.
    /// Decoded code points still undergo PRECIS, NFC, context, and script checks.
    /// Local-part atoms beginning with `xn--` remain rejected.
    ///
    /// Validation does not modify the input or return a rewritten address.
    pub decode_idn: bool,
    /// Count ASCII letters as Latin within the local part and each domain label.
    ///
    /// Defaults to false. Detects additional mixed-script lookalikes but can
    /// reject legitimate mixed-script labels. Each segment is checked separately
    /// so a Cyrillic local part with an ASCII domain remains acceptable.
    /// Non-ASCII scripts are still checked across the entire address; this
    /// option only rejects additional addresses, never relaxes the default.
    pub ascii_is_latin: bool,
    /// Enforce byte lengths before A-label and Unicode policy checks.
    ///
    /// Defaults to false. Limits are 64 UTF-8 bytes for the local part, 63 bytes
    /// per domain label and 253 for the domain in raw Punycode A-label form,
    /// and 254 UTF-8 bytes for the original address. Quotes and escapes count.
    /// Label/domain limits exclude literals. Decoding IDNs does not change
    /// which spelling is used for these length checks.
    pub enforce_length_limits: bool,
}

/// First failure of SMTPUTF8 validation, in the checking order listed here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpUtf8AddressError {
    /// Invalid bare SMTP mailbox syntax.
    Syntax,
    /// Local part exceeds 64 UTF-8 bytes with length enforcement enabled.
    LocalPartTooLong,
    /// Domain label exceeds 63 bytes in A-label form with length enforcement enabled.
    LabelTooLong,
    /// Domain exceeds 253 bytes in A-label form with length enforcement enabled.
    DomainTooLong,
    /// Original address exceeds 254 UTF-8 bytes with length enforcement enabled.
    AddressTooLong,
    /// A local atom/domain label begins with `xn--`, or an opt-in decode fails.
    ALabel,
    /// A code point is outside PRECIS IdentifierClass, other than permitted SPACE.
    DisallowedCodePoint,
    /// The address (after optional domain decoding) is not already NFC.
    NotNormalized,
    /// A CONTEXTJ or CONTEXTO rule is not satisfied.
    ContextRule,
    /// Incompatible scripts are mixed, accounting for the permitted CJK families.
    MixedScripts,
}

impl fmt::Display for SmtpUtf8AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Syntax => "invalid SMTPUTF8 address syntax",
            Self::LocalPartTooLong => "SMTPUTF8 local part exceeds 64 UTF-8 bytes",
            Self::LabelTooLong => "SMTPUTF8 domain label exceeds 63 A-label bytes",
            Self::DomainTooLong => "SMTPUTF8 domain exceeds 253 A-label bytes",
            Self::AddressTooLong => "SMTPUTF8 address exceeds 254 UTF-8 bytes",
            Self::ALabel => "SMTPUTF8 address contains a forbidden or invalid A-label",
            Self::DisallowedCodePoint => "SMTPUTF8 address contains a disallowed code point",
            Self::NotNormalized => "SMTPUTF8 address is not NFC-normalized",
            Self::ContextRule => "SMTPUTF8 address violates a Unicode context rule",
            Self::MixedScripts => "SMTPUTF8 address mixes incompatible scripts",
        })
    }
}

impl Error for SmtpUtf8AddressError {}

/// Validates a bare address using MimeKit's SMTPUTF8 Unicode policy.
///
/// Requires SMTP mailbox syntax, no ACE-prefixed local atoms/domain labels,
/// PRECIS IdentifierClass code points (plus SPACE), NFC, valid contextual
/// characters, and compatible non-ASCII scripts. Common/Inherited scripts are
/// neutral. Han/Hiragana/Katakana, Han/Hangul, and Han/Bopomofo are permitted
/// families; they cannot be freely combined with one another.
///
/// Uses the bundled [`crate::SMTP_UTF8_UNICODE_VERSION`] snapshot, not operating
/// system Unicode tables. ASCII is ignored for script counting by default.
/// Length checking and domain A-label decoding are opt-in through
/// [`validate_smtp_utf8_address_with_options`].
///
/// This implements MimeKit's draft-based policy, not all possible RFC 6531
/// mailboxes. It does not negotiate SMTPUTF8, validate headers, establish
/// deliverability, or detect every confusable address. It never normalizes
/// or repairs text. Rust strings cannot contain invalid UTF-8 or lone surrogates.
///
/// # Errors
///
/// Returns the first [`SmtpUtf8AddressError`] in its documented checking order.
///
/// ```
/// use mimecraft::{SmtpUtf8AddressError, utils::validate_smtp_utf8_address};
///
/// assert_eq!(validate_smtp_utf8_address("caf\u{e9}@example.com"), Ok(()));
/// assert_eq!(validate_smtp_utf8_address("cafe\u{301}@example.com"),
///     Err(SmtpUtf8AddressError::NotNormalized));
/// assert_eq!(validate_smtp_utf8_address("user@xn--gr-zia.example.com"),
///     Err(SmtpUtf8AddressError::ALabel));
/// ```
pub fn validate_smtp_utf8_address(address: &str) -> Result<(), SmtpUtf8AddressError> {
    validate_smtp_utf8_address_with_options(address, &SmtpUtf8AddressValidationOptions::default())
}

/// Validates SMTPUTF8 policy with explicit IDN, script, and length options.
///
/// See [`validate_smtp_utf8_address`]. When decoding, all A-labels are checked
/// before applying Unicode policy to the decoded address. Unlike general IDNA
/// conversion, this does not case-fold, normalize, or apply UTS #46 mappings.
///
/// # Errors
///
/// Syntax precedes optional length errors, A-label errors, disallowed code
/// points, NFC errors, contextual errors, then mixed scripts.
///
/// ```
/// use mimecraft::{SmtpUtf8AddressValidationOptions, utils};
///
/// let options = SmtpUtf8AddressValidationOptions {
///     decode_idn: true,
///     ascii_is_latin: true,
///     enforce_length_limits: true,
/// };
/// assert_eq!(utils::validate_smtp_utf8_address_with_options(
///     "user@xn--gr-zia.example.com", &options), Ok(()));
/// ```
pub fn validate_smtp_utf8_address_with_options(
    address: &str,
    options: &SmtpUtf8AddressValidationOptions,
) -> Result<(), SmtpUtf8AddressError> {
    let domain_index = validate_syntax(address, false).ok_or(SmtpUtf8AddressError::Syntax)?;
    if options.enforce_length_limits {
        validate_lengths(address, domain_index, a_label_length).map_err(|error| match error {
            LengthError::LocalPart => SmtpUtf8AddressError::LocalPartTooLong,
            LengthError::Label => SmtpUtf8AddressError::LabelTooLong,
            LengthError::Domain => SmtpUtf8AddressError::DomainTooLong,
            LengthError::Address => SmtpUtf8AddressError::AddressTooLong,
        })?;
    }
    let local = &address[..domain_index - 1];
    if !local.starts_with('"') && local.split('.').any(has_ace_prefix) {
        return Err(SmtpUtf8AddressError::ALabel);
    }
    let domain = &address[domain_index..];
    if !domain.starts_with('[') && domain.split('.').any(has_ace_prefix) {
        if !options.decode_idn {
            return Err(SmtpUtf8AddressError::ALabel);
        }
        // Decode only A-labels, preserving the local part and all other labels.
        let mut decoded = String::from(&address[..domain_index]);
        for (index, label) in domain.split('.').enumerate() {
            if index != 0 {
                decoded.push('.');
            }
            if has_ace_prefix(label) {
                decoded.push_str(&decode_a_label(label)?);
            } else {
                decoded.push_str(label);
            }
        }
        validate_code_points(&decoded, domain_index, options.ascii_is_latin)
    } else {
        validate_code_points(address, domain_index, options.ascii_is_latin)
    }
}

// An A-label starts with the case-insensitive ACE prefix (RFC 5890, 2.3.1).
fn has_ace_prefix(label: &str) -> bool {
    label
        .as_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"xn--"))
}

fn a_label_length(label: &str) -> Result<usize, LengthError> {
    if label.is_ascii() {
        return Ok(label.len());
    }
    // Each code point produces at least one octet. Bound the quadratic encoder
    // before invoking it; labels exceeding this lower bound cannot fit.
    let mut total = 0;
    let mut basic = false;
    for ch in label.chars() {
        total += 1;
        basic |= ch.is_ascii();
        if 4 + total + usize::from(basic) > 63 {
            return Err(LengthError::Label);
        }
    }
    ::idna::punycode::encode_str(label)
        .map(|encoded| 4 + encoded.len())
        .ok_or(LengthError::Label)
}

fn decode_a_label(label: &str) -> Result<String, SmtpUtf8AddressError> {
    // Use raw RFC 3492, not platform IDNA or UTS #46 mapping: those may reject,
    // normalize, or map characters differently from this Unicode policy.
    // A-labels must fit 63 octets and decode to valid scalars, include non-ASCII,
    // obey hyphen restrictions, and round-trip canonically ignoring ASCII case.
    if label.len() > 63 || !label.is_ascii() {
        return Err(SmtpUtf8AddressError::ALabel);
    }
    let payload = &label[4..];
    let decoded =
        ::idna::punycode::decode_to_string(payload).ok_or(SmtpUtf8AddressError::ALabel)?;
    let mut chars = decoded.chars();
    let third = chars.nth(2);
    let fourth = chars.next();
    if decoded.is_ascii()
        || decoded.starts_with('-')
        || decoded.ends_with('-')
        || (third == Some('-') && fourth == Some('-'))
    {
        return Err(SmtpUtf8AddressError::ALabel);
    }
    let encoded = ::idna::punycode::encode_str(&decoded).ok_or(SmtpUtf8AddressError::ALabel)?;
    if !encoded.eq_ignore_ascii_case(payload) {
        return Err(SmtpUtf8AddressError::ALabel);
    }
    Ok(decoded)
}

fn script(cp: u32) -> u32 {
    (lookup(CODE_POINTS, 11, cp) >> 2) & 0x1ff
}
// Joining types: NonJoining=0, Left=1, Dual=2, Right=3, Transparent=4, JoinCausing=5.
fn joining(cp: u32) -> u32 {
    lookup(JOINING, 4, cp) & 7
}
fn virama(cp: u32) -> bool {
    lookup(JOINING, 4, cp) & 8 != 0
}

fn cjk(script: u32) -> u8 {
    match script {
        HAN => 1,
        HIRAGANA => 2,
        KATAKANA => 4,
        HANGUL => 8,
        BOPOMOFO => 16,
        _ => 0,
    }
}

fn validate_code_points(
    address: &str,
    domain_index: usize,
    ascii_is_latin: bool,
) -> Result<(), SmtpUtf8AddressError> {
    let mut segment_latin = false;
    let mut segment_non_latin = false;
    let mut first_script = UNKNOWN;
    let mut cjk_scripts = 0;
    let mut japanese = false;
    let mut arabic_digits = false;
    let mut extended_arabic_digits = false;
    let mut contextual = false;
    let mut normalization = false;
    let mut mixed = false;
    for (index, ch) in address.char_indices() {
        // Count ASCII Latin separately for the local part and each domain label.
        // Non-ASCII scripts still count across the entire address, so this
        // option can only reject more addresses, never relax the default.
        if ascii_is_latin && index >= domain_index && (index == domain_index || ch == '.') {
            segment_latin = false;
            segment_non_latin = false;
        }
        let cp = u32::from(ch);
        let current_script = if ch.is_ascii() {
            // Syntax permits only printable ASCII: PVALID except SPACE, which
            // the draft explicitly permits. Ignore ASCII scripts by default.
            if !ascii_is_latin || !ch.is_ascii_alphabetic() {
                continue;
            }
            LATIN
        } else {
            let entry = lookup(CODE_POINTS, 11, cp);
            // PRECIS classes: Disallowed=0, PValid=1, ContextJ=2, ContextO=3.
            let precis = entry & 3;
            if precis == 0 {
                return Err(SmtpUtf8AddressError::DisallowedCodePoint);
            }
            contextual |= precis != 1;
            normalization |= cp >= FIRST_NORMALIZATION_CODE_POINT;
            arabic_digits |= (0x0660..=0x0669).contains(&cp);
            extended_arabic_digits |= (0x06f0..=0x06f9).contains(&cp);
            (entry >> 2) & 0x1ff
        };
        if matches!(current_script, COMMON | INHERITED) {
            continue;
        }
        japanese |= matches!(current_script, HAN | HIRAGANA | KATAKANA);
        if !ch.is_ascii() {
            let family = cjk(current_script);
            if family != 0 {
                cjk_scripts |= family;
            } else if first_script == UNKNOWN {
                first_script = current_script;
            } else if current_script != first_script {
                mixed = true;
            }
        }
        if ascii_is_latin {
            segment_latin |= current_script == LATIN;
            segment_non_latin |= current_script != LATIN;
            mixed |= segment_latin && segment_non_latin;
        }
    }
    // The draft counts Han/Hiragana/Katakana as one script for Japanese,
    // Han/Hangul for Korean, and Han/Bopomofo for Chinese.
    if cjk_scripts != 0
        && (first_script != UNKNOWN || [7u8, 9, 17].iter().all(|family| cjk_scripts & !family != 0))
    {
        mixed = true;
    }
    // Individual NFC_Quick_Check=No characters have already failed PRECIS,
    // but sequences of permitted characters can still be non-normalized.
    if normalization && !is_nfc(address) {
        return Err(SmtpUtf8AddressError::NotNormalized);
    }
    if contextual {
        // RFC 5892 A.8/A.9 forbid mixing the two Arabic-Indic digit sets.
        if arabic_digits && extended_arabic_digits {
            return Err(SmtpUtf8AddressError::ContextRule);
        }
        for (index, ch) in address.char_indices() {
            // All CONTEXTJ/CONTEXTO code points except the Arabic-Indic digit
            // sets, which were checked together above (RFC 5892, A.8/A.9).
            if matches!(
                ch,
                '\u{200c}' | '\u{200d}' | '\u{b7}' | '\u{375}' | '\u{5f3}' | '\u{5f4}' | '\u{30fb}'
            ) && !context_satisfied(address, index, ch, japanese)
            {
                return Err(SmtpUtf8AddressError::ContextRule);
            }
        }
    }
    if mixed {
        Err(SmtpUtf8AddressError::MixedScripts)
    } else {
        Ok(())
    }
}

fn context_satisfied(text: &str, index: usize, ch: char, japanese: bool) -> bool {
    let before_text = &text[..index];
    let after_text = &text[index + ch.len_utf8()..];
    let before = before_text.chars().next_back();
    let after = after_text.chars().next();
    match ch {
        '\u{200c}' => {
            // A.1: a preceding virama, or joining types (L|D) T* ZWNJ T* (R|D).
            if before.is_some_and(|ch| virama(u32::from(ch))) {
                return true;
            }
            let left = before_text
                .chars()
                .rev()
                .map(|ch| joining(u32::from(ch)))
                .find(|&t| t != 4);
            let right = after_text
                .chars()
                .map(|ch| joining(u32::from(ch)))
                .find(|&t| t != 4);
            matches!(left, Some(1 | 2)) && matches!(right, Some(2 | 3))
        }
        // A.2: ZWJ must follow a virama.
        '\u{200d}' => before.is_some_and(|ch| virama(u32::from(ch))),
        // A.3: middle dot between l's. Unlike DNS, email is not case-folded;
        // also accept L on either side, as recommended by the draft's author.
        '\u{b7}' => matches!(before, Some('l' | 'L')) && matches!(after, Some('l' | 'L')),
        // A.4: Greek lower numeral sign must precede Greek.
        '\u{375}' => after.is_some_and(|ch| script(u32::from(ch)) == GREEK),
        // A.5/A.6: Hebrew punctuation must follow Hebrew.
        '\u{5f3}' | '\u{5f4}' => before.is_some_and(|ch| script(u32::from(ch)) == HEBREW),
        // A.7: Katakana middle dot needs Han, Hiragana or Katakana somewhere.
        _ => japanese,
    }
}

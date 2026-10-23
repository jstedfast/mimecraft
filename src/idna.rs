// Adapted from MimeKit's Encodings/Punycode.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! International domain conversion with MIME-safe ASCII output.
//!
//! These helpers adapt MimeKit's IDNA wrapper, not just the Punycode algorithm.
//! They use the `idna` crate's nontransitional UTS #46 processing instead of
//! platform-dependent .NET `IdnMapping`. Case/normalization and Unicode-version
//! behavior can therefore differ from MimeKit.
//!
//! Both directions validate the ASCII form: labels must be nonempty and at most
//! 63 bytes; the name is at most 253 bytes excluding an optional final root dot.
//! Hyphen, bidirectional-text, and joiner checks are enabled. STD3's restriction
//! to letters/digits/hyphens is not enabled: MIME atom characters such as `_`
//! remain allowed, but whitespace, controls, and MIME delimiters are rejected.
//!
//! Inputs are whole domains, not mailboxes, URLs, or bracketed domain literals.
//! Use string slices instead of MimeKit's index/count overloads. Conversion
//! errors never fall back to the original input or replacement-character output.
//! This is not a DNS lookup, mailbox validator, or Unicode spoofing detector.

use std::{borrow::Cow, error::Error, fmt};

use ::idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

use crate::parse_utils::is_ascii_atom;

/// Converts a domain to normalized lowercase ASCII, using ACE (`xn--`) labels.
///
/// A trailing root dot is preserved. The result can borrow unchanged input.
///
/// ```
/// use mimecraft::idna;
///
/// assert_eq!(idna::to_ascii("b\u{fc}cher.example")?, "xn--bcher-kva.example");
/// assert!(idna::to_ascii("example\u{ff20}evil.com").is_err());
/// # Ok::<(), idna::IdnaError>(())
/// ```
///
/// # Errors
///
/// Returns [`IdnaError`] for invalid IDNA syntax/lengths or output that would
/// introduce ASCII whitespace, controls, or MIME delimiters into a domain.
pub fn to_ascii(domain: &str) -> Result<Cow<'_, str>, IdnaError> {
    let ascii = Uts46::new()
        .to_ascii(
            domain.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Check,
            DnsLength::VerifyAllowRootDot,
        )
        .map_err(|_| IdnaError::InvalidDomain)?;

    // Some Unicode characters (e.g. THIN SPACE or FULLWIDTH COMMERCIAL AT)
    // map to ASCII whitespace or specials, producing an unparsable domain.
    if !ascii
        .bytes()
        .all(|byte| byte == b'.' || is_ascii_atom(byte))
    {
        return Err(IdnaError::UnsafeAscii);
    }
    Ok(ascii)
}

/// Converts an ASCII or Unicode domain to normalized Unicode.
///
/// This performs the same length and MIME-safety checks as [`to_ascii`], even
/// though UTS #46's ToUnicode operation alone does not check DNS lengths.
/// All eligible ACE labels are decoded; no confusable-character UI policy is
/// applied. Prefer ASCII when a display must not obscure the encoded spelling.
/// Unchanged input can be borrowed.
///
/// ```
/// use mimecraft::idna;
///
/// assert_eq!(idna::to_unicode("xn--bcher-kva.example")?, "b\u{fc}cher.example");
/// # Ok::<(), idna::IdnaError>(())
/// ```
///
/// # Errors
///
/// Returns [`IdnaError`] rather than partially decoded or replacement output
/// if the domain fails IDNA, length, or MIME-safety checks.
pub fn to_unicode(domain: &str) -> Result<Cow<'_, str>, IdnaError> {
    to_ascii(domain)?;
    let (unicode, status) =
        Uts46::new().to_unicode(domain.as_bytes(), AsciiDenyList::EMPTY, Hyphens::Check);
    status.map_err(|_| IdnaError::InvalidDomain)?;
    Ok(unicode)
}

/// An international domain conversion failure.
///
/// The backend does not supply source offsets or individual IDNA rule failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdnaError {
    /// The domain failed IDNA validity checks or DNS length limits.
    InvalidDomain,
    /// Mapping produced ASCII whitespace, controls, or MIME domain delimiters.
    UnsafeAscii,
}

impl fmt::Display for IdnaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDomain => "invalid international domain or DNS length",
            Self::UnsafeAscii => "domain conversion produced unsafe ASCII syntax",
        })
    }
}

impl Error for IdnaError {}

// Ported from MimeKit's ContentEncoding.cs, Utils/MimeUtils.cs, and MimePart.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

/// A MIME content transfer encoding, matching MimeKit's `ContentEncoding`.
///
/// This identifies an encoding; it does not encode, decode, or validate bytes.
/// yEnc and parameter percent escaping have no separate variants, matching
/// MimeKit. Their incremental codecs report [`Unspecified`](Self::Unspecified).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContentEncoding {
    /// No content transfer encoding is specified.
    ///
    /// Setting this on a MIME part removes its Content-Transfer-Encoding
    /// headers. For stored content, this means the bytes have not been
    /// transfer-encoded. Encoder and decoder filters pass them through unchanged.
    #[default]
    Unspecified,
    /// Unencoded text restricted to the US-ASCII range.
    SevenBit,
    /// Unencoded text containing non-ASCII bytes, requiring a suitable transport.
    EightBit,
    /// Unencoded binary data, requiring a suitable transport.
    Binary,
    /// Base64, typically used for binary or predominantly non-ASCII content.
    Base64,
    /// Quoted-printable, typically used for mostly ASCII text.
    QuotedPrintable,
    /// The legacy Unix-to-Unix encoding.
    UuEncode,
}

impl ContentEncoding {
    /// Parses a Content-Transfer-Encoding value using MimeKit's tolerant rules.
    ///
    /// Skips leading Unicode whitespace and recognizes the first token,
    /// case-insensitively, up to whitespace or a semicolon. Remaining text is
    /// ignored. In addition to the standard names, `x-uuencode`, `uuencode`,
    /// and `x-uue` all select [`UuEncode`](Self::UuEncode).
    ///
    /// This is not strict header validation: for example, `base64; junk` and
    /// `base64 other text` are accepted, but `(comment) base64` is not.
    /// No encoded-word decoding or charset conversion is performed.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownContentEncoding`] for empty or unknown tokens.
    /// [`Unspecified`](Self::Unspecified) is never returned as a successful parse.
    ///
    /// ```
    /// use mimecraft::ContentEncoding;
    ///
    /// assert_eq!(" BASE64;\r\n".parse(), Ok(ContentEncoding::Base64));
    /// assert_eq!(ContentEncoding::parse("x-uue"), Ok(ContentEncoding::UuEncode));
    /// assert!(ContentEncoding::parse("unknown").is_err());
    /// ```
    pub fn parse(value: &str) -> Result<Self, UnknownContentEncoding> {
        let value = value.trim_start_matches(char::is_whitespace);
        // Google Docs appends a semicolon to some encoding values. As in
        // MimeKit, stop at it rather than including it in the encoding token.
        let end = value
            .find(|ch: char| ch == ';' || ch.is_whitespace())
            .unwrap_or(value.len());
        let token = &value[..end];

        [
            ("7bit", Self::SevenBit),
            ("8bit", Self::EightBit),
            ("binary", Self::Binary),
            ("base64", Self::Base64),
            ("quoted-printable", Self::QuotedPrintable),
            ("x-uuencode", Self::UuEncode),
            ("uuencode", Self::UuEncode),
            ("x-uue", Self::UuEncode),
        ]
        .into_iter()
        .find_map(|(name, encoding)| token.eq_ignore_ascii_case(name).then_some(encoding))
        .ok_or(UnknownContentEncoding)
    }

    /// Returns the canonical header token, or `None` for an unspecified encoding.
    ///
    /// UU encoding is written as `x-uuencode`, matching MimeKit. The other
    /// tokens are `7bit`, `8bit`, `binary`, `base64`, and `quoted-printable`.
    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Unspecified => None,
            Self::SevenBit => Some("7bit"),
            Self::EightBit => Some("8bit"),
            Self::Binary => Some("binary"),
            Self::Base64 => Some("base64"),
            Self::QuotedPrintable => Some("quoted-printable"),
            Self::UuEncode => Some("x-uuencode"),
        }
    }
}

impl FromStr for ContentEncoding {
    type Err = UnknownContentEncoding;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// A Content-Transfer-Encoding value with an empty or unrecognized token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownContentEncoding;

impl fmt::Display for UnknownContentEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unrecognized content transfer encoding")
    }
}

impl Error for UnknownContentEncoding {}

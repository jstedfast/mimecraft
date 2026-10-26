// Alias and fallback behavior adapted from MimeKit's Utils/CharsetUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! MIME charset lookup and complete-buffer character conversion.
//!
//! ASCII and ISO-8859-1 retain their MIME meanings, rather than being treated as
//! Windows-1252. Legacy codecs use `encoding_rs` mappings, which are not exact
//! equivalents of every .NET code page. Unsupported charsets are explicit errors.
//! These functions do not parse encoded words or MIME parameter syntax.
//! For incremental conversion with replacement diagnostics, see
//! [`crate::filters::CharsetFilter`].

use std::{borrow::Cow, error::Error, fmt, str::FromStr};

use encoding_rs::{self as rs, Encoding};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Codec {
    Ascii,
    Latin1,
    Encoding(&'static Encoding),
}

/// A supported MIME character set, resolved independently of transfer encoding.
///
/// This is a small, copyable value. Alias spellings do not affect equality.
/// Explicit decoding does not sniff, strip, or switch encodings based on BOMs.
///
/// ```
/// use mimecraft::charsets::Charset;
///
/// let latin1 = Charset::from_label("ISO_8859_1")?;
/// assert_eq!(latin1, Charset::LATIN1);
/// assert_eq!(latin1.decode(b"Ren\xe9e")?, "Ren\u{e9}e");
/// assert_eq!(latin1.encode("Ren\u{e9}e")?.as_ref(), b"Ren\xe9e");
/// # Ok::<(), mimecraft::charsets::CharsetError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Charset(Codec);

impl Charset {
    pub(crate) fn ends_in_iso2022_ascii_reset(self, bytes: &[u8]) -> bool {
        // Independently encoded MIME chunks may end in ESC ( B and start with
        // another designation. encoding_rs rejects adjacent designations when
        // concatenated. At this explicit ASCII reset, character decoding can
        // restart at the MIME boundary without losing shift or character state.
        self.name() == "iso-2022-jp" && bytes.ends_with(b"\x1b(B")
    }

    pub(crate) fn codec(self) -> Codec {
        self.0
    }

    /// Strict US-ASCII, not Windows-1252.
    pub const ASCII: Self = Self(Codec::Ascii);
    /// ISO-8859-1: every byte maps to the same Unicode scalar value.
    pub const LATIN1: Self = Self(Codec::Latin1);
    /// UTF-8.
    pub const UTF8: Self = Self(Codec::Encoding(rs::UTF_8));

    /// Resolves an ASCII charset label case-insensitively.
    ///
    /// Accepts supported MimeKit aliases and numeric forms such as `cp1252`,
    /// `windows-cp1252`, and `iso_8859_1`. Like MimeKit, `unicode` means UTF-8
    /// and `iso-10646` means UTF-16BE. Surrounding whitespace is not trimmed.
    ///
    /// Other labels use `encoding_rs` lookup, except browser-only replacement
    /// and user-defined encodings. ISO-8859-9/11 and their aliases are rejected
    /// rather than silently substituted with Windows supersets. UTF-7, UTF-32,
    /// ISO-2022-KR, HZ, and unsupported DOS/code-page encodings are also rejected.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::UnsupportedLabel`] when the label is unsupported.
    pub fn from_label(label: &str) -> Result<Self, CharsetError> {
        let unsupported = || CharsetError::UnsupportedLabel(label.to_owned());
        if label.is_empty() || !label.is_ascii() || label.bytes().any(|b| b.is_ascii_whitespace()) {
            return Err(unsupported());
        }
        let lower = label.to_ascii_lowercase();
        let name = lower.as_str();
        let charset = match name {
            // The historical ANSI_X3.4 labels denote US-ASCII, not a Windows
            // "ANSI" code page.
            "ascii" | "us-ascii" | "us" | "ansi_x3.4-1968" | "ansi_x3.4-1986" | "iso-ir-6"
            | "iso646-us" | "iso_646.irv:1991" | "csascii" => Self::ASCII,
            // MimeKit treats the occasional ANSI_X3.110-1983 label as Latin-1,
            // its closest supported approximation rather than an exact alias.
            "latin1" | "latin-1" | "l1" | "iso-ir-100" | "csisolatin1" | "iso88591" | "ibm819"
            | "cp819" | "iso_8859-1:1987" | "ansi_x3.110-1983" => Self::LATIN1,
            "unicode" | "utf8" | "utf-8" => Self::UTF8,
            "utf-16" | "utf-16le" => Self(Codec::Encoding(rs::UTF_16LE)),
            "unicodefffe" | "utf-16be" => Self(Codec::Encoding(rs::UTF_16BE)),
            // Upgrade these Korean labels to the EUC-KR superset, as MimeKit
            // does, rather than restricting input to the older character set.
            "ksc-5601-1987" | "ksc-5601_1987" | "ksc-5601" | "5601" | "ks_c_5861-1992"
            | "ksc-5861-1992" | "ksc-5861_1992" | "euckr-0" => Self(Codec::Encoding(rs::EUC_KR)),
            "big5-0" | "big5.eten-0" | "big5hkscs-0" => Self(Codec::Encoding(rs::BIG5)),
            "gb-2312" | "gb2312-0" | "gb2312-80" | "gb2312.1980-0" | "euc-cn" | "gbk-0"
            | "x-gbk" => Self(Codec::Encoding(rs::GBK)),
            "gb18030-0" => Self(Codec::Encoding(rs::GB18030)),
            "eucjp-0" | "ujis-0" | "ujis" => Self(Codec::Encoding(rs::EUC_JP)),
            "jisx0208.1983-0" | "jisx0212.1990-0" | "pck" => Self(Codec::Encoding(rs::SHIFT_JIS)),
            _ => {
                if let Some(codepage) = parse_codepage(name) {
                    return from_codepage(codepage).ok_or_else(unsupported);
                }
                let encoding =
                    Encoding::for_label_no_replacement(name.as_bytes()).ok_or_else(unsupported)?;
                // These browser label upgrades are not equivalent MIME charsets.
                if encoding == rs::X_USER_DEFINED
                    || (encoding == rs::WINDOWS_1252
                        && !matches!(name, "windows-1252" | "x-cp1252"))
                    || (encoding == rs::WINDOWS_1254
                        && !matches!(name, "windows-1254" | "x-cp1254"))
                    || (encoding == rs::WINDOWS_874 && !matches!(name, "windows-874" | "dos-874"))
                {
                    return Err(unsupported());
                }
                Self(Codec::Encoding(encoding))
            }
        };
        Ok(charset)
    }

    /// Returns a canonical lowercase MIME label for the actual codec.
    ///
    /// For example, a GB2312 alias resolves to `gbk`, reflecting the backend's
    /// superset mapping. UTF-16LE uses `utf-16`, as in MimeKit.
    pub fn name(self) -> &'static str {
        match self.0 {
            Codec::Ascii => "us-ascii",
            Codec::Latin1 => "iso-8859-1",
            Codec::Encoding(encoding) => match encoding.name() {
                "UTF-8" => "utf-8",
                "UTF-16LE" => "utf-16",
                "UTF-16BE" => "utf-16be",
                "Big5" => "big5",
                "EUC-JP" => "euc-jp",
                "EUC-KR" => "euc-kr",
                "GBK" => "gbk",
                "IBM866" => "ibm866",
                "ISO-2022-JP" => "iso-2022-jp",
                "ISO-8859-2" => "iso-8859-2",
                "ISO-8859-3" => "iso-8859-3",
                "ISO-8859-4" => "iso-8859-4",
                "ISO-8859-5" => "iso-8859-5",
                "ISO-8859-6" => "iso-8859-6",
                "ISO-8859-7" => "iso-8859-7",
                "ISO-8859-8" => "iso-8859-8",
                "ISO-8859-8-I" => "iso-8859-8-i",
                "ISO-8859-10" => "iso-8859-10",
                "ISO-8859-13" => "iso-8859-13",
                "ISO-8859-14" => "iso-8859-14",
                "ISO-8859-15" => "iso-8859-15",
                "ISO-8859-16" => "iso-8859-16",
                "KOI8-R" => "koi8-r",
                "KOI8-U" => "koi8-u",
                "Shift_JIS" => "shift_jis",
                name => name,
            },
        }
    }

    /// Decodes a complete buffer, rejecting malformed input without replacement.
    ///
    /// The result borrows the input when the codec can return it unchanged.
    /// Initial BOMs are ordinary characters, not a charset-selection mechanism.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::InvalidBytes`] for bytes invalid in this charset.
    pub fn decode(self, input: &[u8]) -> Result<Cow<'_, str>, CharsetError> {
        let decoded = match self.0 {
            Codec::Ascii => input
                .is_ascii()
                .then(|| Cow::Borrowed(std::str::from_utf8(input).expect("ASCII is valid UTF-8"))),
            Codec::Latin1 => Some(decode_latin1(input)),
            Codec::Encoding(encoding) => {
                encoding.decode_without_bom_handling_and_without_replacement(input)
            }
        };
        decoded.ok_or(CharsetError::InvalidBytes(self))
    }

    /// Decodes complete input, replacing malformed sequences with U+FFFD.
    ///
    /// Returns the text and a flag indicating whether replacement occurred.
    /// This explicit replacement policy is uniform, rather than .NET's
    /// charset-dependent default replacements. BOMs are preserved.
    pub fn decode_lossy(self, input: &[u8]) -> (Cow<'_, str>, bool) {
        match self.0 {
            Codec::Ascii if !input.is_ascii() => (
                Cow::Owned(
                    input
                        .iter()
                        .map(|&b| if b < 128 { char::from(b) } else { '\u{fffd}' })
                        .collect(),
                ),
                true,
            ),
            Codec::Ascii => (
                Cow::Borrowed(std::str::from_utf8(input).expect("ASCII is valid UTF-8")),
                false,
            ),
            Codec::Latin1 => (decode_latin1(input), false),
            Codec::Encoding(encoding) => encoding.decode_without_bom_handling(input),
        }
    }

    /// Encodes a complete string without inserting a BOM or replacement text.
    ///
    /// UTF-16 is encoded explicitly, avoiding the backend's browser-oriented
    /// UTF-8 output substitution. Unmappable legacy characters cause an error;
    /// the backend's HTML numeric character references are never returned.
    ///
    /// # Errors
    ///
    /// Returns [`CharsetError::UnmappableText`] if the charset cannot encode
    /// the input. This does not promise byte-for-byte reversibility for every
    /// legacy codec's duplicate or compatibility character mappings.
    pub fn encode(self, input: &str) -> Result<Cow<'_, [u8]>, CharsetError> {
        let unmappable = || CharsetError::UnmappableText(self);
        match self.0 {
            Codec::Ascii | Codec::Latin1 if input.is_ascii() => Ok(Cow::Borrowed(input.as_bytes())),
            Codec::Ascii => Err(unmappable()),
            Codec::Latin1 => input
                .chars()
                .map(|ch| u8::try_from(u32::from(ch)).map_err(|_| unmappable()))
                .collect::<Result<Vec<_>, _>>()
                .map(Cow::Owned),
            Codec::Encoding(encoding) if encoding == rs::UTF_16LE || encoding == rs::UTF_16BE => {
                let bytes = input
                    .encode_utf16()
                    .flat_map(|unit| {
                        if encoding == rs::UTF_16LE {
                            unit.to_le_bytes()
                        } else {
                            unit.to_be_bytes()
                        }
                    })
                    .collect();
                Ok(Cow::Owned(bytes))
            }
            Codec::Encoding(encoding) => {
                let (output, _, had_errors) = encoding.encode(input);
                if had_errors {
                    Err(unmappable())
                } else {
                    Ok(output)
                }
            }
        }
    }
}

impl FromStr for Charset {
    type Err = CharsetError;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        Self::from_label(label)
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A charset lookup or conversion failure. Conversion errors do not carry offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CharsetError {
    /// The original label is unknown or denotes an unsupported encoding.
    UnsupportedLabel(String),
    /// The input contains an invalid byte sequence for this charset.
    InvalidBytes(Charset),
    /// The input contains text this charset cannot encode.
    UnmappableText(Charset),
}

impl fmt::Display for CharsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedLabel(label) => write!(f, "unsupported charset: {label}"),
            Self::InvalidBytes(charset) => write!(f, "invalid byte sequence for {charset}"),
            Self::UnmappableText(charset) => write!(f, "text cannot be encoded as {charset}"),
        }
    }
}

impl Error for CharsetError {}

/// Decodes unlabelled text using MimeKit's UTF-8, preferred-charset, Latin-1 order.
///
/// Valid UTF-8 always wins, even when a preferred charset is supplied. Otherwise
/// the preferred charset is used only if it decodes without errors; Latin-1 is
/// the final, lossless byte-to-scalar fallback. Returns both text and the charset
/// selected, making fallback visible. This is not general charset detection.
///
/// ```
/// use mimecraft::charsets::{Charset, decode_best};
///
/// let (text, charset) = decode_best(b"Ren\xe9e", None);
/// assert_eq!(text, "Ren\u{e9}e");
/// assert_eq!(charset, Charset::LATIN1);
/// ```
pub fn decode_best(input: &[u8], preferred: Option<Charset>) -> (Cow<'_, str>, Charset) {
    // Error-free UTF-8 wins even if another charset could decode these bytes.
    if let Ok(text) = std::str::from_utf8(input) {
        return (Cow::Borrowed(text), Charset::UTF8);
    }
    // Latin-1 has zero invalid sequences, so MimeKit's minimum-error selection
    // can choose the earlier preferred codec only when it is also error-free.
    if let Some(charset) = preferred
        && charset != Charset::UTF8
        && charset != Charset::LATIN1
        && let Ok(text) = charset.decode(input)
    {
        return (text, charset);
    }
    (decode_latin1(input), Charset::LATIN1)
}

fn decode_latin1(input: &[u8]) -> Cow<'_, str> {
    if input.is_ascii() {
        Cow::Borrowed(std::str::from_utf8(input).expect("ASCII is valid UTF-8"))
    } else {
        Cow::Owned(input.iter().map(|&byte| char::from(byte)).collect())
    }
}

fn parse_codepage(name: &str) -> Option<u32> {
    fn number(text: &str) -> Option<u32> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse::<i32>().ok().map(|value| value as u32)
    }
    fn strip_separator(text: &str) -> &str {
        text.strip_prefix(['-', '_']).unwrap_or(text)
    }

    if let Some(rest) = name.strip_prefix("windows") {
        let rest = strip_separator(rest);
        number(rest.strip_prefix("cp").unwrap_or(rest))
    } else if let Some(rest) = name.strip_prefix("ibm").or_else(|| name.strip_prefix("cp")) {
        number(strip_separator(rest))
    } else if let Some(rest) = name.strip_prefix("iso") {
        let rest = strip_separator(rest);
        let (family, suffix) = rest.split_once(['-', '_']).unwrap_or((rest, ""));
        match number(family)? {
            10646 => Some(1201),
            8859 => {
                let part = number(suffix)?;
                // Include the backend's additional ISO-8859 variants as well.
                (1..=16).contains(&part).then_some(28590 + part)
            }
            2022 if suffix == "jp" => Some(50220),
            2022 if suffix == "kr" => Some(50225),
            _ => None,
        }
    } else {
        None
    }
}

pub(crate) fn from_codepage(codepage: u32) -> Option<Charset> {
    let encoding = match codepage {
        20127 => return Some(Charset::ASCII),
        28591 | 819 => return Some(Charset::LATIN1),
        65001 => rs::UTF_8,
        1200 => rs::UTF_16LE,
        1201 => rs::UTF_16BE,
        866 => rs::IBM866,
        874 => rs::WINDOWS_874,
        932 => rs::SHIFT_JIS,
        936 | 51936 => rs::GBK,
        949 | 51949 => rs::EUC_KR,
        950 => rs::BIG5,
        1250 => rs::WINDOWS_1250,
        1251 => rs::WINDOWS_1251,
        1252 => rs::WINDOWS_1252,
        1253 => rs::WINDOWS_1253,
        1254 => rs::WINDOWS_1254,
        1255 => rs::WINDOWS_1255,
        1256 => rs::WINDOWS_1256,
        1257 => rs::WINDOWS_1257,
        1258 => rs::WINDOWS_1258,
        10000 => rs::MACINTOSH,
        10007 => rs::X_MAC_CYRILLIC,
        20866 => rs::KOI8_R,
        21866 => rs::KOI8_U,
        28592 => rs::ISO_8859_2,
        28593 => rs::ISO_8859_3,
        28594 => rs::ISO_8859_4,
        28595 => rs::ISO_8859_5,
        28596 => rs::ISO_8859_6,
        28597 => rs::ISO_8859_7,
        28598 => rs::ISO_8859_8,
        28600 => rs::ISO_8859_10,
        28603 => rs::ISO_8859_13,
        28604 => rs::ISO_8859_14,
        28605 => rs::ISO_8859_15,
        28606 => rs::ISO_8859_16,
        38598 => rs::ISO_8859_8_I,
        50220 => rs::ISO_2022_JP,
        51932 => rs::EUC_JP,
        54936 => rs::GB18030,
        _ => return None,
    };
    Some(Charset(Codec::Encoding(encoding)))
}

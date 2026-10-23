// Adapted from MimeKit's Utils/MimeUtils.cs and Utils/ParseUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::parse_utils::skip_comments_and_whitespace;

/// A parsed version value, such as the value of a MIME-Version header.
///
/// Like MimeKit's version utility, this accepts two through four nonnegative
/// decimal components, each at most `i32::MAX`. MIME normally uses `1.0`;
/// accepting other values does not claim support for other MIME versions.
/// Three- and four-component values are retained for source compatibility,
/// and `1.0`, `1.0.0`, and `1.0.0.0` remain distinct values.
///
/// Parsing accepts nested comments and space, tab, CR, or LF around components
/// and dots. Formatting produces plain decimal components without comments,
/// whitespace, or leading zeros. It does not preserve the original header;
/// use [`crate::Header::raw`] for lossless output.
///
/// ```
/// use mimecraft::MimeVersion;
///
/// let version = MimeVersion::parse(b" 1 (comment) .\t0\r\n")?;
/// assert_eq!(version, MimeVersion::V1_0);
/// assert_eq!(version.components(), &[1, 0]);
/// assert_eq!(version.to_string(), "1.0");
/// # Ok::<(), mimecraft::MimeVersionParseError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MimeVersion {
    components: [u32; 4],
    count: usize,
}

impl MimeVersion {
    /// MIME version 1.0.
    pub const V1_0: Self = Self {
        components: [1, 0, 0, 0],
        count: 2,
    };

    /// Parses a complete byte slice containing a version value.
    ///
    /// Number tokens contain ASCII digits only. Comment content is treated as
    /// opaque bytes, with escaped parentheses handled as quoted pairs.
    ///
    /// For compatibility with MimeKit, a final dot at the exact end of input
    /// is accepted after two through four components (`1.0.` becomes `1.0`).
    /// A dot followed only by whitespace or comments is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`MimeVersionParseError`] for malformed syntax, an unterminated
    /// comment, an overflowing number, or a component count outside two to four.
    /// Error offsets are byte positions relative to this slice.
    pub fn parse(input: &[u8]) -> Result<Self, MimeVersionParseError> {
        let mut version = Self {
            components: [0; 4],
            count: 0,
        };
        let mut index = 0;
        loop {
            skip_comments_and_whitespace(input, &mut index).map_err(|offset| {
                MimeVersionParseError::new(MimeVersionParseErrorKind::IncompleteComment, offset)
            })?;

            let start = index;
            let mut value = 0u32;
            while let Some(&byte) = input.get(index).filter(|byte| byte.is_ascii_digit()) {
                let digit = u32::from(byte - b'0');
                // Match MimeKit's signed 32-bit component bound without ever
                // overflowing during accumulation, even on arbitrarily long input.
                if value > (i32::MAX as u32 - digit) / 10 {
                    return Err(MimeVersionParseError::new(
                        MimeVersionParseErrorKind::ComponentOverflow,
                        index,
                    ));
                }
                value = value * 10 + digit;
                index += 1;
            }
            if index == start {
                return Err(MimeVersionParseError::new(
                    MimeVersionParseErrorKind::InvalidComponent,
                    index,
                ));
            }
            if version.count == version.components.len() {
                return Err(MimeVersionParseError::new(
                    MimeVersionParseErrorKind::InvalidComponentCount,
                    start,
                ));
            }
            version.components[version.count] = value;
            version.count += 1;

            skip_comments_and_whitespace(input, &mut index).map_err(|offset| {
                MimeVersionParseError::new(MimeVersionParseErrorKind::IncompleteComment, offset)
            })?;
            if index == input.len() {
                break;
            }
            if input[index] != b'.' {
                return Err(MimeVersionParseError::new(
                    MimeVersionParseErrorKind::ExpectedDot,
                    index,
                ));
            }
            index += 1;
            // MimeKit ends its loop when a trailing dot consumes the last byte.
            if index == input.len() {
                break;
            }
        }

        if version.count < 2 {
            return Err(MimeVersionParseError::new(
                MimeVersionParseErrorKind::InvalidComponentCount,
                index,
            ));
        }
        Ok(version)
    }

    /// Returns all two, three, or four version components in order.
    pub fn components(&self) -> &[u32] {
        &self.components[..self.count]
    }

    /// Returns the major version component.
    pub const fn major(&self) -> u32 {
        self.components[0]
    }

    /// Returns the minor version component.
    pub const fn minor(&self) -> u32 {
        self.components[1]
    }

    /// Returns the optional third component, called the build number in MimeKit.
    pub fn build(&self) -> Option<u32> {
        self.components().get(2).copied()
    }

    /// Returns the optional fourth component, called the revision in MimeKit.
    pub fn revision(&self) -> Option<u32> {
        self.components().get(3).copied()
    }
}

impl FromStr for MimeVersion {
    type Err = MimeVersionParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

impl fmt::Display for MimeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, component) in self.components().iter().enumerate() {
            if index != 0 {
                f.write_str(".")?;
            }
            write!(f, "{component}")?;
        }
        Ok(())
    }
}

/// The reason a version value could not be parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MimeVersionParseErrorKind {
    /// An ASCII decimal component was expected but no digit was present.
    InvalidComponent,
    /// A decimal component exceeded `i32::MAX`.
    ComponentOverflow,
    /// A dot was expected between components.
    ExpectedDot,
    /// Fewer than two or more than four components were supplied.
    InvalidComponentCount,
    /// A comment reached the end of input without a closing parenthesis.
    IncompleteComment,
}

/// A version parsing failure and its byte offset in the supplied value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MimeVersionParseError {
    kind: MimeVersionParseErrorKind,
    offset: usize,
}

impl MimeVersionParseError {
    fn new(kind: MimeVersionParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }

    /// Returns the reason parsing failed.
    pub fn kind(self) -> MimeVersionParseErrorKind {
        self.kind
    }

    /// Returns the zero-based byte offset of the failure.
    ///
    /// For incomplete comments this points to the opening parenthesis; for
    /// overflow, the digit that exceeds the limit. It may equal the input length
    /// when a required component is missing or too few components were present.
    pub fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for MimeVersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            MimeVersionParseErrorKind::InvalidComponent => "expected a decimal version component",
            MimeVersionParseErrorKind::ComponentOverflow => "version component exceeds i32::MAX",
            MimeVersionParseErrorKind::ExpectedDot => "expected '.' between version components",
            MimeVersionParseErrorKind::InvalidComponentCount => {
                "version must contain two through four components"
            }
            MimeVersionParseErrorKind::IncompleteComment => "unterminated comment",
        };
        write!(f, "{reason} at byte {}", self.offset)
    }
}

impl Error for MimeVersionParseError {}

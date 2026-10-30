// Adapted from MimeKit's MessageImportance.cs, MessagePriority.cs,
// XMessagePriority.cs, MimeMessage.cs, and Utils/ParseUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::{
    Header,
    parse_utils::{skip_whitespace, trim_whitespace},
};

/// An Importance header value, independent of Priority and X-Priority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessageImportance {
    /// Low importance.
    Low,
    /// Normal importance, also used when the header is absent.
    #[default]
    Normal,
    /// High importance.
    High,
}

impl MessageImportance {
    /// Parses `low`, `normal`, or `high`, ignoring ASCII case and surrounding
    /// ASCII space/tab/CR/LF. Does not decode encoded words or remove comments.
    ///
    /// # Errors
    ///
    /// Empty or unknown values return [`MessagePriorityParseError::Importance`]
    /// rather than MimeKit's implicit Normal fallback.
    pub fn parse(value: &str) -> Result<Self, MessagePriorityParseError> {
        let value = trim_whitespace(value);
        [Self::Low, Self::Normal, Self::High]
            .into_iter()
            .find(|importance| value.eq_ignore_ascii_case(importance.as_str()))
            .ok_or(MessagePriorityParseError::Importance)
    }

    /// Returns the canonical lowercase header token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    /// Creates a canonical CRLF-terminated Importance header.
    pub fn to_header(self) -> Header {
        Header::from_formatted(format!("Importance: {}\r\n", self.as_str()))
    }
}

impl FromStr for MessageImportance {
    type Err = MessagePriorityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for MessageImportance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A Priority header value, independent of Importance and X-Priority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MessagePriority {
    /// Non-urgent priority.
    NonUrgent,
    /// Normal priority, also used when the header is absent.
    #[default]
    Normal,
    /// Urgent priority.
    Urgent,
}

impl MessagePriority {
    /// Parses `non-urgent`, `normal`, or `urgent`, ignoring ASCII case and
    /// surrounding ASCII space/tab/CR/LF. Does not decode words or remove comments.
    ///
    /// # Errors
    ///
    /// Empty or unknown values return [`MessagePriorityParseError::Priority`]
    /// rather than MimeKit's implicit Normal fallback.
    pub fn parse(value: &str) -> Result<Self, MessagePriorityParseError> {
        let value = trim_whitespace(value);
        [Self::NonUrgent, Self::Normal, Self::Urgent]
            .into_iter()
            .find(|priority| value.eq_ignore_ascii_case(priority.as_str()))
            .ok_or(MessagePriorityParseError::Priority)
    }

    /// Returns the canonical lowercase header token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonUrgent => "non-urgent",
            Self::Normal => "normal",
            Self::Urgent => "urgent",
        }
    }

    /// Creates a canonical CRLF-terminated Priority header.
    pub fn to_header(self) -> Header {
        Header::from_formatted(format!("Priority: {}\r\n", self.as_str()))
    }
}

impl FromStr for MessagePriority {
    type Err = MessagePriorityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for MessagePriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An X-Priority header value, with one the highest and five the lowest.
///
/// Independent of Importance and Priority; no conversion or synchronization
/// between the three headers is implied.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum XMessagePriority {
    /// Highest priority (1).
    Highest = 1,
    /// High priority (2).
    High = 2,
    /// Normal priority (3), also used when the header is absent.
    #[default]
    Normal = 3,
    /// Low priority (4).
    Low = 4,
    /// Lowest priority (5).
    Lowest = 5,
}

impl XMessagePriority {
    /// Parses a raw X-Priority value using MimeKit's numeric-prefix rules.
    ///
    /// Skips leading space, tab, CR, and LF, then reads unsigned ASCII decimal
    /// digits through the first non-digit. The integer must fit in `i32`, then
    /// is clamped to 1 through 5. Thus `0` is Highest and `6` is Lowest.
    /// Trailing bytes are ignored, including comments, garbage, and non-UTF-8.
    /// Leading comments, signs, and encoded words are not interpreted.
    ///
    /// # Errors
    ///
    /// Missing digits or integer overflow return
    /// [`MessagePriorityParseError::XPriority`] rather than defaulting to Normal.
    pub fn parse(value: &[u8]) -> Result<Self, MessagePriorityParseError> {
        let mut index = 0;
        skip_whitespace(value, &mut index);
        let start = index;
        let mut number = 0i32;
        while let Some(&byte) = value.get(index).filter(|byte| byte.is_ascii_digit()) {
            // Check integer overflow before clamping: saturating early would
            // accept digit runs that MimeKit's TryParseInt32 rejects.
            number = number
                .checked_mul(10)
                .and_then(|number| number.checked_add(i32::from(byte - b'0')))
                .ok_or(MessagePriorityParseError::XPriority)?;
            index += 1;
        }
        if index == start {
            return Err(MessagePriorityParseError::XPriority);
        }
        Ok(match number {
            0 | 1 => Self::Highest,
            2 => Self::High,
            3 => Self::Normal,
            4 => Self::Low,
            _ => Self::Lowest,
        })
    }

    /// Returns the numeric value, from one (highest) to five (lowest).
    pub const fn number(self) -> u8 {
        self as u8
    }

    /// Returns MimeKit's canonical numeric value and descriptive comment.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Highest => "1 (Highest)",
            Self::High => "2 (High)",
            Self::Normal => "3 (Normal)",
            Self::Low => "4 (Low)",
            Self::Lowest => "5 (Lowest)",
        }
    }

    /// Creates a canonical CRLF-terminated X-Priority header.
    pub fn to_header(self) -> Header {
        Header::from_formatted(format!("X-Priority: {}\r\n", self.as_str()))
    }
}

impl FromStr for XMessagePriority {
    type Err = MessagePriorityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

impl fmt::Display for XMessagePriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Invalid Importance, Priority, or X-Priority header metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagePriorityParseError {
    /// An Importance value is empty or does not name a supported level.
    Importance,
    /// A Priority value is empty or does not name a supported level.
    Priority,
    /// An X-Priority value has no unsigned decimal prefix or exceeds `i32::MAX`.
    XPriority,
}

impl fmt::Display for MessagePriorityParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Importance => "unrecognized message importance",
            Self::Priority => "unrecognized message priority",
            Self::XPriority => "X-Priority requires a nonnegative 32-bit decimal integer prefix",
        })
    }
}

impl Error for MessagePriorityParseError {}

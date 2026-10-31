// Adapted from MimeKit's Tnef attribute, options, and compliance types.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Synchronous, forward-only TNEF attribute reading.
//!
//! Raw attribute values, checksums, and framing diagnostics are supported.
//! MAPI properties and recipient rows support forward-only, bounded value reads
//! and owned snapshots and ordered sets with explicit typed conversions.
//! Attribute dates and MAPI date conversion retain explicit UTC/unspecified kinds.
//! Embedded readers share limits and diagnostics; MIME conversion is not yet implemented.
//! Malformed framing follows MimeKit's recovery rules;
//! supply a fallible logger to stop at the first reported issue.

use std::{error::Error, fmt, io};

mod date_time;
mod property_set;
mod property_tags;
mod property_types;
mod reader;
mod value_conversion;
pub use date_time::{TnefDateTime, TnefDateTimeError, TnefDateTimeKind};
pub use property_set::{TnefProperty, TnefPropertySet};
pub use property_types::{
    TnefName, TnefNameId, TnefPropertyInfo, TnefPropertyTag, TnefPropertyType, TnefValue,
};
pub use reader::{TnefEmbeddedReader, TnefEmbeddedSource};
pub use reader::{TnefPropertyReader, TnefPropertyValueReader, TnefReader, TnefValueReader};
pub use value_conversion::TnefValueConversionError;

/// The object to which an attribute belongs, preserving unrecognized bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefAttributeLevel {
    /// Message metadata.
    Message,
    /// Attachment metadata or content.
    Attachment,
    /// An unrecognized wire value.
    Unknown(u8),
}

impl From<u8> for TnefAttributeLevel {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Message,
            2 => Self::Attachment,
            value => Self::Unknown(value),
        }
    }
}

/// The high word of an attribute tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefAttributeType {
    /// Recipient or sender structures.
    Triples,
    /// NUL-terminated 8-bit characters.
    String,
    /// A block of 8-bit text.
    Text,
    /// A fourteen-byte date structure.
    Date,
    /// A signed sixteen-bit integer.
    Short,
    /// A signed thirty-two-bit integer.
    Long,
    /// Binary data.
    Byte,
    /// An unsigned sixteen-bit integer.
    Word,
    /// An unsigned thirty-two-bit integer.
    DWord,
    /// An unrecognized high word.
    Unknown(u16),
}

/// A complete wire tag, including its value type; unknown tags are preserved.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TnefAttributeTag(pub u32);

macro_rules! tags {
    ($($name:ident = $value:expr, $doc:literal;)+) => {
        impl TnefAttributeTag {
            $(#[doc = $doc] pub const $name: Self = Self($value);)+

            /// Whether this is one of MimeKit's recognized attribute tags.
            pub fn is_known(self) -> bool {
                matches!(self, $(Self::$name)|+)
            }
        }
    };
}

tags! {
    NULL = 0x00000000, "The null attribute, allowed at either level.";
    OWNER = 0x00060000, "Owner information.";
    SENT_FOR = 0x00060001, "Represented sender information.";
    DELEGATE = 0x00060002, "Delegate information.";
    ORIGINAL_MESSAGE_CLASS = 0x00070006, "The original message class.";
    DATE_START = 0x00030006, "The start date.";
    DATE_END = 0x00030007, "The end date.";
    AID_OWNER = 0x00050008, "Owner appointment identifier.";
    REQUEST_RESPONSE = 0x00040009, "Whether a response is requested.";
    FROM = 0x00008000, "Sender information.";
    SUBJECT = 0x00018004, "The subject.";
    DATE_SENT = 0x00038005, "The sent date.";
    DATE_RECEIVED = 0x00038006, "The received date.";
    MESSAGE_STATUS = 0x00068007, "Message status flags.";
    MESSAGE_CLASS = 0x00078008, "The message class, a string despite its Word type.";
    MESSAGE_ID = 0x00018009, "The message identifier.";
    PARENT_ID = 0x0001800a, "The parent message identifier.";
    CONVERSATION_ID = 0x0001800b, "The conversation identifier.";
    BODY = 0x0002800c, "The text body.";
    PRIORITY = 0x0004800d, "Message priority.";
    ATTACH_DATA = 0x0006800f, "Attachment content.";
    ATTACH_TITLE = 0x00018010, "Attachment title.";
    ATTACH_META_FILE = 0x00068011, "Attachment metafile.";
    ATTACH_CREATE_DATE = 0x00038012, "Attachment creation date.";
    ATTACH_MODIFY_DATE = 0x00038013, "Attachment modification date.";
    DATE_MODIFIED = 0x00038020, "Message modification date.";
    ATTACH_TRANSPORT_FILENAME = 0x00069001, "Attachment transport filename.";
    ATTACH_RENDER_DATA = 0x00069002, "Rendering data marking the start of an attachment.";
    MAPI_PROPERTIES = 0x00069003, "Message MAPI properties.";
    RECIPIENT_TABLE = 0x00069004, "Recipient MAPI rows.";
    ATTACHMENT = 0x00069005, "Attachment MAPI properties.";
    TNEF_VERSION = 0x00089006, "The TNEF version.";
    OEM_CODEPAGE = 0x00069007, "The codepage used for subsequent 8-bit strings.";
}

impl TnefAttributeTag {
    /// Decodes the high word without discarding unknown types.
    pub fn attribute_type(self) -> TnefAttributeType {
        match (self.0 >> 16) as u16 {
            0 => TnefAttributeType::Triples,
            1 => TnefAttributeType::String,
            2 => TnefAttributeType::Text,
            3 => TnefAttributeType::Date,
            4 => TnefAttributeType::Short,
            5 => TnefAttributeType::Long,
            6 => TnefAttributeType::Byte,
            7 => TnefAttributeType::Word,
            8 => TnefAttributeType::DWord,
            value => TnefAttributeType::Unknown(value),
        }
    }

    pub(super) fn is_attachment(self) -> bool {
        matches!(
            self,
            Self::ATTACH_CREATE_DATE
                | Self::ATTACH_DATA
                | Self::ATTACHMENT
                | Self::ATTACH_META_FILE
                | Self::ATTACH_MODIFY_DATE
                | Self::ATTACH_RENDER_DATA
                | Self::ATTACH_TITLE
                | Self::ATTACH_TRANSPORT_FILENAME
        )
    }
}

/// Metadata for the current attribute. Value bytes are not allocated here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TnefAttribute {
    /// Message/attachment level, including unknown wire values.
    pub level: TnefAttributeLevel,
    /// The full wire tag.
    pub tag: TnefAttributeTag,
    /// Declared value length, excluding the two-byte checksum.
    pub length: u32,
    /// Header offset relative to the start of the outermost TNEF stream.
    pub offset: u64,
}

/// Copied configuration for the synchronous attribute reader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TnefReaderOptions {
    /// Initial codepage; zero selects Windows-1252.
    ///
    /// Unsupported explicit codepages are construction errors, rather than
    /// silently falling back. OEM attributes may update this while reading.
    pub default_codepage: u32,
    /// Maximum bytes collected by a single owned-value read; default 32 MiB.
    /// Also applies to collected property names; does not limit streaming or
    /// skipping value payloads.
    pub max_value_length: usize,
    /// Total collected value bytes across the reader tree; default 64 MiB. Fixed-width reads,
    /// raw streaming, and skipped values do not consume this budget. Collected
    /// property names do, even when their values are skipped.
    pub max_total_data_bytes: u64,
    /// Maximum embedded-message depth; root is zero, default is 32.
    /// Exceeding this limit returns an error without claiming the value.
    pub max_nesting_depth: usize,
    /// Maximum delivered reports of each violation; zero means unlimited.
    /// Calls without a logger discard issues without consuming this budget.
    pub max_issues_per_violation: usize,
}

impl Default for TnefReaderOptions {
    fn default() -> Self {
        Self {
            default_codepage: 0,
            max_value_length: 32 * 1024 * 1024,
            max_total_data_bytes: 64 * 1024 * 1024,
            max_nesting_depth: 32,
            max_issues_per_violation: 0,
        }
    }
}

/// Attribute/property diagnostics. Numeric values retain MimeKit's identifiers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum TnefComplianceViolation {
    /// The stream does not start with the TNEF signature.
    InvalidSignature = 1,
    /// A version other than 0x00010000 was declared.
    UnsupportedVersion = 2,
    /// An OEM codepage could not be resolved; the current charset is retained.
    InvalidMessageCodepage = 3,
    /// A message class was empty, over 255 bytes, or not printable ASCII.
    InvalidMessageClass = 4,
    /// The level was neither message nor attachment.
    InvalidAttributeLevel = 5,
    /// A message-level attribute followed an attachment-level attribute.
    MessageAttributeAfterAttachment = 6,
    /// A known attribute appeared at the wrong level.
    AttributeLevelMismatch = 7,
    /// The tag is unknown; its raw value remains readable.
    UnknownAttribute = 8,
    /// The declared length exceeded i32::MAX; framing cannot continue.
    InvalidAttributeLength = 9,
    /// The checksum differed from the sum of the value bytes modulo 65536.
    AttributeChecksumMismatch = 10,
    /// A known value was too short for its interpretation.
    InvalidAttributeValue = 11,
    /// A date has invalid components or lies outside the supported range.
    InvalidDate = 12,
    /// The source ended inside a required header, value, or checksum.
    TruncatedStream = 13,
    /// A property count was negative or larger than the containing attribute permits.
    InvalidPropertyCount = 14,
    /// A recipient-row count was negative or impossible.
    InvalidRowCount = 15,
    /// A property value count was invalid.
    InvalidValueCount = 16,
    /// A property structure or value could not fit within its attribute.
    InvalidPropertyLength = 17,
    /// An unknown type makes subsequent property boundaries unknowable.
    UnsupportedPropertyType = 18,
    /// A named property's kind was neither numeric nor textual.
    InvalidNamedPropertyKind = 19,
    /// An embedded message would exceed the configured nesting limit.
    NestingTooDeep = 20,
    /// An owned-value read exceeded a configured byte budget.
    DataSizeLimitExceeded = 22,
}

/// A framing diagnostic with its original stream position and attribute tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TnefComplianceIssue {
    /// The condition that was detected.
    pub violation: TnefComplianceViolation,
    /// Offset relative to the beginning of the outermost supplied stream.
    pub stream_offset: u64,
    /// Embedded-message depth; zero for the outermost reader.
    pub depth: usize,
    /// Current attribute tag, or `None` outside an attribute.
    pub attribute_tag: Option<TnefAttributeTag>,
    /// Current property tag, or `None` for attribute/row/count diagnostics.
    pub property_tag: Option<TnefPropertyTag>,
}

/// Optional diagnostic sink. An error aborts parsing and faults the reader.
///
/// Closures returning `io::Result<()>` implement this trait. The original I/O
/// error is retained in [`TnefReadError::Io`]. Retained reports should also be
/// bounded by the sink, since an unlimited logger may receive many issues.
pub trait TnefComplianceLogger {
    /// Records an issue or returns the caller's reason to stop.
    fn log(&mut self, issue: TnefComplianceIssue) -> io::Result<()>;
}

impl<F: FnMut(TnefComplianceIssue) -> io::Result<()>> TnefComplianceLogger for F {
    fn log(&mut self, issue: TnefComplianceIssue) -> io::Result<()> {
        self(issue)
    }
}

/// API, resource-limit, or source/logger failures, distinct from diagnostics.
#[derive(Debug)]
pub enum TnefReadError {
    /// Source or logger error.
    Io(io::Error),
    /// An explicit initial codepage is unsupported.
    UnsupportedCodepage(u32),
    /// A value was requested without a current attribute.
    NoAttribute,
    /// The current value was already claimed by another read.
    ValueAlreadyRead,
    /// The tag cannot be interpreted as the requested scalar/string type.
    InvalidValueType(TnefAttributeTag),
    /// A property value was requested without a current value.
    NoPropertyValue,
    /// A property snapshot was requested without a current property.
    NoProperty,
    /// The current property has the wrong type for the requested operation.
    InvalidPropertyType(TnefPropertyTag),
    /// Date interpretation failed; raw MAPI values remain available.
    /// This error does not fault the reader.
    InvalidDate(TnefDateTimeError),
    /// An owned-value read exceeded a configured byte budget.
    ///
    /// No value bytes were consumed or claimed; streaming or skipping remains
    /// possible. Unlike MimeKit this is not returned as a successful empty value.
    ValueLimitExceeded,
    /// The current value is not an embedded message.
    NotEmbeddedMessage,
    /// Opening a child would exceed the nesting limit. The value is unclaimed.
    NestingLimitExceeded,
    /// Position arithmetic would overflow.
    PositionOverflow,
    /// An earlier source/logger/arithmetic error made further reads unsafe.
    Faulted,
}

impl fmt::Display for TnefReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::UnsupportedCodepage(page) => write!(f, "unsupported TNEF codepage: {page}"),
            Self::NoAttribute => f.write_str("the TNEF reader is not on an attribute"),
            Self::ValueAlreadyRead => f.write_str("the TNEF value has already been claimed"),
            Self::InvalidValueType(tag) => write!(f, "invalid TNEF value type: {tag:?}"),
            Self::NoPropertyValue => f.write_str("the TNEF property reader is not on a value"),
            Self::NoProperty => f.write_str("the TNEF property reader is not on a property"),
            Self::InvalidPropertyType(tag) => {
                write!(f, "invalid TNEF property value type: {tag:?}")
            }
            Self::ValueLimitExceeded => f.write_str("TNEF owned-value byte limit exceeded"),
            Self::NotEmbeddedMessage => f.write_str("the TNEF value is not an embedded message"),
            Self::NestingLimitExceeded => f.write_str("TNEF nesting depth limit exceeded"),
            Self::InvalidDate(error) => error.fmt(f),
            Self::PositionOverflow => f.write_str("TNEF stream position overflow"),
            Self::Faulted => f.write_str("the TNEF reader is faulted"),
        }
    }
}

impl Error for TnefReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidDate(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TnefReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

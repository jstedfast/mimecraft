// Adapted from MimeKit's TnefPropertyType, TnefPropertyTag, and TnefNameId.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

/// A MAPI value type, preserving unknown bits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TnefPropertyType(pub u16);

macro_rules! types {
    ($($name:ident = $value:expr, $doc:literal;)+) => {
        impl TnefPropertyType {
            $(#[doc = $doc] pub const $name: Self = Self($value);)+
        }
    };
}

types! {
    UNSPECIFIED = 0, "Unspecified; cannot be read from the wire.";
    NULL = 1, "A null value occupying no bytes.";
    I2 = 2, "A signed sixteen-bit integer padded to four bytes.";
    LONG = 3, "A signed thirty-two-bit integer.";
    R4 = 4, "A thirty-two-bit floating-point value.";
    DOUBLE = 5, "A sixty-four-bit floating-point value.";
    CURRENCY = 6, "A signed integer in units of 1/10000.";
    APP_TIME = 7, "An OLE Automation date encoded as a double.";
    ERROR = 10, "A thirty-two-bit error code.";
    BOOLEAN = 11, "A sixteen-bit boolean padded to four bytes.";
    OBJECT = 13, "Object bytes, including any leading interface GUID.";
    I8 = 20, "A signed sixty-four-bit integer.";
    STRING8 = 30, "An eight-bit string in the message codepage.";
    UNICODE = 31, "A UTF-16LE string.";
    SYS_TIME = 64, "A FILETIME: 100-nanosecond intervals since January 1, 1601 UTC.";
    CLASS_ID = 72, "A sixteen-byte GUID in Windows wire order.";
    BINARY = 258, "Binary data.";
    MULTI_VALUED = 4096, "The multivalue flag, not a standalone value type.";
}

pub(super) enum Width {
    Fixed(usize),
    Variable,
    Unsupported,
}

impl TnefPropertyType {
    pub(super) fn width(self) -> Width {
        match self {
            Self::NULL => Width::Fixed(0),
            // MS-OXTNEF pads sixteen-bit values out to four bytes.
            Self::I2 | Self::LONG | Self::R4 | Self::ERROR | Self::BOOLEAN => Width::Fixed(4),
            Self::DOUBLE | Self::CURRENCY | Self::APP_TIME | Self::SYS_TIME | Self::I8 => {
                Width::Fixed(8)
            }
            Self::CLASS_ID => Width::Fixed(16),
            Self::UNICODE | Self::STRING8 | Self::BINARY | Self::OBJECT => Width::Variable,
            _ => Width::Unsupported,
        }
    }
}

/// A full MAPI tag: property identifier in the high word, type in the low word.
/// No catalog lookup is needed; unrecognized identifiers remain usable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TnefPropertyTag(pub u32);

impl TnefPropertyTag {
    /// Combines an identifier and wire type (which may contain the multivalue flag).
    pub const fn new(id: u16, kind: TnefPropertyType) -> Self {
        Self(((id as u32) << 16) | kind.0 as u32)
    }

    /// Returns the property identifier.
    pub const fn id(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// Returns the value type without the multivalue flag.
    pub const fn value_type(self) -> TnefPropertyType {
        TnefPropertyType((self.0 as u16) & !0x1000)
    }

    /// Returns whether a value count is present for a multivalued property.
    pub const fn is_multi_valued(self) -> bool {
        self.0 & 0x1000 != 0
    }

    /// Returns whether a GUID/name descriptor follows this tag.
    pub const fn is_named(self) -> bool {
        self.id() >= 0x8000 && self.id() <= 0xfffe
    }
}

/// The identifier portion of a named MAPI property.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TnefName {
    /// A numeric name.
    Id(i32),
    /// A UTF-16LE name decoded with replacement.
    String(String),
    /// An invalid kind, reported but retained rather than fabricated as ID zero.
    Unknown(u32),
}

/// A named-property descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TnefNameId {
    /// Property-set GUID in Windows wire order (first three fields little-endian).
    pub guid: [u8; 16],
    /// Numeric or textual identifier.
    pub name: TnefName,
}

/// Current property metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TnefPropertyInfo {
    /// Full property tag.
    pub tag: TnefPropertyTag,
    /// Named-property descriptor, when present.
    pub name: Option<TnefNameId>,
    /// Offset of the property's wire tag in the outermost TNEF stream.
    pub offset: u64,
}

/// A native property value without lossy cross-type coercion.
///
/// Currency retains its exact scaled integer. Date values retain their wire
/// numbers; [`to_datetime`](Self::to_datetime) performs checked calendar conversion.
/// GUIDs retain Windows byte order, not network-order UUID bytes.
#[derive(Clone, Debug, PartialEq)]
pub enum TnefValue {
    /// Null.
    Null,
    /// Signed sixteen-bit integer.
    I16(i16),
    /// Signed thirty-two-bit integer.
    I32(i32),
    /// Signed sixty-four-bit integer.
    I64(i64),
    /// Floating-point value.
    F32(f32),
    /// Floating-point value.
    F64(f64),
    /// Integer in units of 1/10000.
    Currency(i64),
    /// Raw OLE Automation date.
    AppTime(f64),
    /// Raw FILETIME.
    SysTime(i64),
    /// Error code with its original bits.
    Error(u32),
    /// Boolean using the low sixteen bits only.
    Bool(bool),
    /// GUID in Windows wire order.
    Guid([u8; 16]),
    /// Decoded String8 or Unicode value.
    Text(String),
    /// Binary value.
    Binary(Vec<u8>),
    /// Object value, retaining any interface GUID prefix.
    Object(Vec<u8>),
}

impl TnefValue {
    /// Converts AppTime or SysTime to a checked date without changing the raw value.
    /// AppTime uses .NET-compatible millisecond rounding and an unspecified zone;
    /// FILETIME retains 100-nanosecond precision and is always UTC.
    ///
    /// # Errors
    ///
    /// Returns `NotDate` for other native types, or `OutOfRange` for invalid dates.
    /// This standalone conversion has no logger; property navigation reports
    /// invalid dates when loading their fixed-width values.
    pub fn to_datetime(&self) -> Result<super::TnefDateTime, super::TnefDateTimeError> {
        match self {
            Self::AppTime(value) => super::TnefDateTime::from_oa_date(*value),
            Self::SysTime(value) => super::TnefDateTime::from_file_time(*value),
            _ => Err(super::TnefDateTimeError::NotDate),
        }
    }
}

// Adapted from MimeKit's TnefProperty.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use super::{TnefDateTimeError, TnefValue};

/// An explicit failure to interpret a native value or scalar property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefValueConversionError {
    /// The native type does not support the requested conversion.
    InvalidType,
    /// A floating-point value cannot be represented as a signed 64-bit integer.
    OutOfRange,
    /// The property has no value.
    NoValue,
    /// A scalar accessor was used on a multivalued property.
    MultiValued,
    /// Date conversion failed without changing the original wire value.
    InvalidDate(TnefDateTimeError),
}

impl fmt::Display for TnefValueConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidType => f.write_str("the TNEF value does not support this conversion"),
            Self::OutOfRange => {
                f.write_str("the TNEF numeric value is outside the conversion range")
            }
            Self::NoValue => f.write_str("the TNEF property has no value"),
            Self::MultiValued => f.write_str("the TNEF property is multivalued"),
            Self::InvalidDate(error) => error.fmt(f),
        }
    }
}

impl Error for TnefValueConversionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDate(error) => Some(error),
            _ => None,
        }
    }
}

impl TnefValue {
    /// Converts a boolean, integer, error code, or currency value to a boolean.
    /// Nonzero currency fractions are true, even when integer conversion is zero.
    ///
    /// # Errors
    ///
    /// Returns `InvalidType` for other types, including floating-point and dates.
    pub fn to_bool(&self) -> Result<bool, TnefValueConversionError> {
        match self {
            Self::Bool(value) => Ok(*value),
            Self::I16(value) => Ok(*value != 0),
            Self::I32(value) => Ok(*value != 0),
            Self::I64(value) | Self::Currency(value) => Ok(*value != 0),
            Self::Error(value) => Ok(*value != 0),
            _ => Err(TnefValueConversionError::InvalidType),
        }
    }

    /// Converts a numeric/boolean value to i64, truncating fractions toward zero.
    /// Error codes are interpreted as signed 32-bit integers.
    ///
    /// # Errors
    ///
    /// Returns `InvalidType` for nonnumeric values (including dates).
    /// Unlike unchecked runtime-dependent casts, nonfinite or out-of-range
    /// floating-point values return `OutOfRange`, never a saturated/sentinel value.
    pub fn to_i64(&self) -> Result<i64, TnefValueConversionError> {
        let value = match self {
            Self::Bool(value) => return Ok(i64::from(*value)),
            Self::I16(value) => return Ok(i64::from(*value)),
            Self::I32(value) => return Ok(i64::from(*value)),
            Self::I64(value) => return Ok(*value),
            Self::Error(value) => return Ok(i64::from(*value as i32)),
            // A PtypCurrency value always fits in i64 once scaled by 1/10000.
            Self::Currency(value) => return Ok(*value / 10000),
            Self::F32(value) => f64::from(*value),
            Self::F64(value) => *value,
            _ => return Err(TnefValueConversionError::InvalidType),
        };
        // i64::MAX rounds up to 2^63 in f64, so the upper bound is exclusive.
        if !(-9223372036854775808.0..9223372036854775808.0).contains(&value) {
            return Err(TnefValueConversionError::OutOfRange);
        }
        Ok(value as i64)
    }

    /// Converts through i64, retaining the low 32 bits as MimeKit does.
    /// Values that do not fit in i32 are truncated, not clamped.
    ///
    /// # Errors
    ///
    /// Shares [`to_i64`](Self::to_i64) errors.
    pub fn to_i32(&self) -> Result<i32, TnefValueConversionError> {
        self.to_i64().map(|value| value as i32)
    }

    /// Converts through i64, retaining the low 16 bits as MimeKit does.
    /// Values that do not fit in i16 are truncated, not clamped.
    ///
    /// # Errors
    ///
    /// Shares [`to_i64`](Self::to_i64) errors.
    pub fn to_i16(&self) -> Result<i16, TnefValueConversionError> {
        self.to_i64().map(|value| value as i16)
    }

    /// Converts a numeric/boolean value to f64. Currency retains its fraction.
    /// Native floating-point NaNs and infinities remain floating-point values.
    ///
    /// # Errors
    ///
    /// Returns `InvalidType` for nonnumeric values, including dates.
    pub fn to_f64(&self) -> Result<f64, TnefValueConversionError> {
        match self {
            Self::F64(value) => Ok(*value),
            Self::F32(value) => Ok(f64::from(*value)),
            Self::Currency(value) => Ok(*value as f64 / 10000.0),
            _ => self.to_i64().map(|value| value as f64),
        }
    }

    /// Converts through f64 to f32, with normal floating-point rounding and
    /// overflow to infinity, matching MimeKit's floating-point conversion.
    ///
    /// # Errors
    ///
    /// Shares [`to_f64`](Self::to_f64) errors.
    pub fn to_f32(&self) -> Result<f32, TnefValueConversionError> {
        self.to_f64().map(|value| value as f32)
    }

    /// Borrows Binary, Object, or GUID bytes, without copying or removing an IID.
    /// GUID bytes retain Windows wire order.
    ///
    /// # Errors
    ///
    /// Returns `InvalidType` for other native types, including strings.
    pub fn as_bytes(&self) -> Result<&[u8], TnefValueConversionError> {
        match self {
            Self::Binary(value) | Self::Object(value) => Ok(value),
            Self::Guid(value) => Ok(value),
            _ => Err(TnefValueConversionError::InvalidType),
        }
    }

    /// Returns a native GUID in Windows wire order, without parsing binary/text.
    ///
    /// # Errors
    ///
    /// Returns `InvalidType` unless this is a GUID value.
    pub fn to_guid(&self) -> Result<[u8; 16], TnefValueConversionError> {
        match self {
            Self::Guid(value) => Ok(*value),
            _ => Err(TnefValueConversionError::InvalidType),
        }
    }
}

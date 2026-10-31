// Adapted from MimeKit's TnefReader.cs and TnefPropertyReader.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt};

use crate::{
    DateTimeOffset,
    date_time_offset::{days_before_year, days_in_month},
};

const TICKS_PER_SECOND: u64 = 10_000_000;
const TICKS_PER_DAY: u64 = TICKS_PER_SECOND * 86400;
const MAX_TICKS: u64 = days_before_year(10000) as u64 * TICKS_PER_DAY - 1;
const FILE_TIME_OFFSET: u64 = days_before_year(1601) as u64 * TICKS_PER_DAY;
const OA_DAYS: i64 = days_before_year(1900) - 2;
const MILLIS_PER_DAY: i64 = 86_400_000;

/// Whether a decoded date denotes UTC or has no specified time zone.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TnefDateTimeKind {
    /// Legacy attribute dates and OLE Automation dates have no time zone.
    Unspecified,
    /// FILETIME values denote UTC, independent of the machine's time zone.
    Utc,
}

/// A Gregorian date in years 1 through 9999, with 100-nanosecond precision.
///
/// This is not a local-time-zone or date arithmetic API. Unspecified dates
/// are not implicitly interpreted as UTC. Equality includes the time kind.
/// Property wire numbers remain available separately in [`super::TnefValue`].
///
/// ```
/// use mimecraft::tnef::{TnefDateTime, TnefDateTimeKind};
/// let date = TnefDateTime::from_file_time(1)?;
/// assert_eq!((date.year(), date.month(), date.day()), (1601, 1, 1));
/// assert_eq!(date.nanosecond(), 100);
/// assert_eq!(date.kind(), TnefDateTimeKind::Utc);
/// # Ok::<(), mimecraft::tnef::TnefDateTimeError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TnefDateTime {
    ticks: u64,
    kind: TnefDateTimeKind,
}

impl TnefDateTime {
    /// Constructs a whole-second date with an explicit time kind.
    ///
    /// # Errors
    ///
    /// Rejects invalid Gregorian components, including leap seconds.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        kind: TnefDateTimeKind,
    ) -> Result<Self, TnefDateTimeError> {
        // Reuse the checked calendar implementation. Offset zero is used only
        // for component arithmetic; the caller's time kind is kept unchanged.
        let calendar = DateTimeOffset::new(year, month, day, hour, minute, second, 0)
            .map_err(|_| TnefDateTimeError::InvalidComponents)?;
        let seconds = calendar.unix_timestamp() + days_before_year(1970) * 86400;
        Ok(Self {
            ticks: seconds as u64 * TICKS_PER_SECOND,
            kind,
        })
    }

    /// Constructs a date from 100-nanosecond ticks since 0001-01-01 midnight.
    /// Ticks for an unspecified date are calendar coordinates, not a UTC instant.
    ///
    /// # Errors
    ///
    /// Rejects ticks after 9999-12-31 23:59:59.9999999.
    pub fn from_ticks(ticks: u64, kind: TnefDateTimeKind) -> Result<Self, TnefDateTimeError> {
        if ticks > MAX_TICKS {
            return Err(TnefDateTimeError::OutOfRange);
        }
        Ok(Self { ticks, kind })
    }

    /// Decodes a FILETIME without losing its 100-nanosecond precision.
    ///
    /// # Errors
    ///
    /// Rejects negative values and dates beyond year 9999, as MimeKit does.
    pub fn from_file_time(value: i64) -> Result<Self, TnefDateTimeError> {
        // MS-OXCDATA defines PtypTime as a FILETIME: the number of
        // 100-nanosecond intervals since January 1, 1601 UTC.
        if value < 0 || value as u64 > MAX_TICKS - FILE_TIME_OFFSET {
            return Err(TnefDateTimeError::OutOfRange);
        }
        Self::from_ticks(value as u64 + FILE_TIME_OFFSET, TnefDateTimeKind::Utc)
    }

    /// Decodes an OLE Automation date, with an unspecified time zone.
    ///
    /// Matches MimeKit's .NET millisecond rounding, including negative dates'
    /// absolute fractional time of day. The original double is not modified.
    ///
    /// # Errors
    ///
    /// Rejects nonfinite and out-of-range values, including rounded overflow.
    pub fn from_oa_date(value: f64) -> Result<Self, TnefDateTimeError> {
        if !(value > -657435.0 && value < 2958466.0) {
            return Err(TnefDateTimeError::OutOfRange);
        }
        let mut millis =
            (value * MILLIS_PER_DAY as f64 + if value >= 0.0 { 0.5 } else { -0.5 }) as i64;
        // Before the epoch, the fractional part is a positive time of day:
        // -1.25 means the previous day at 06:00, not 18:00.
        if millis < 0 {
            millis -= 2 * (millis % MILLIS_PER_DAY);
        }
        millis += OA_DAYS * MILLIS_PER_DAY;
        if millis < 0 || millis > (MAX_TICKS / 10_000) as i64 {
            return Err(TnefDateTimeError::OutOfRange);
        }
        Self::from_ticks(millis as u64 * 10_000, TnefDateTimeKind::Unspecified)
    }

    pub(super) fn from_attribute(bytes: &[u8; 14]) -> Result<Self, TnefDateTimeError> {
        // The TNEF date structure is seven 16-bit values: year, month, day,
        // hour, minute, second, and day-of-week. As in MimeKit, ignore the
        // weekday instead of rejecting dates with an incorrect weekday.
        let word = |i: usize| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
        let small = |i| u8::try_from(word(i)).map_err(|_| TnefDateTimeError::InvalidComponents);
        Self::new(
            word(0),
            small(1)?,
            small(2)?,
            small(3)?,
            small(4)?,
            small(5)?,
            TnefDateTimeKind::Unspecified,
        )
    }

    /// Returns 100-nanosecond ticks since 0001-01-01 midnight, in this date's kind.
    pub const fn ticks(self) -> u64 {
        self.ticks
    }

    /// Returns whether the date is UTC or has no specified zone.
    pub const fn kind(self) -> TnefDateTimeKind {
        self.kind
    }

    /// Returns the calendar year.
    pub fn year(self) -> u16 {
        self.date_parts().0
    }

    /// Returns the month, from 1 through 12.
    pub fn month(self) -> u8 {
        self.date_parts().1
    }

    /// Returns the day of the month, starting at 1.
    pub fn day(self) -> u8 {
        self.date_parts().2
    }

    /// Returns the hour, from 0 through 23.
    pub const fn hour(self) -> u8 {
        (self.ticks / (TICKS_PER_SECOND * 3600) % 24) as u8
    }

    /// Returns the minute, from 0 through 59.
    pub const fn minute(self) -> u8 {
        (self.ticks / (TICKS_PER_SECOND * 60) % 60) as u8
    }

    /// Returns the second, from 0 through 59.
    pub const fn second(self) -> u8 {
        (self.ticks / TICKS_PER_SECOND % 60) as u8
    }

    /// Returns the fractional second in nanoseconds, always a multiple of 100.
    pub const fn nanosecond(self) -> u32 {
        (self.ticks % TICKS_PER_SECOND) as u32 * 100
    }

    fn date_parts(self) -> (u16, u8, u8) {
        let mut days = self.ticks / TICKS_PER_DAY;
        let centuries400 = days / 146097;
        days %= 146097;
        // The final day of a cycle belongs to its last year, not the next
        // century/year; the fourth century is the only leap century.
        let centuries = (days / 36524).min(3);
        days -= centuries * 36524;
        let years4 = days / 1461;
        days %= 1461;
        let years = (days / 365).min(3);
        days -= years * 365;
        let year = (1 + centuries400 * 400 + centuries * 100 + years4 * 4 + years) as u16;
        let mut month = 1;
        while days >= u64::from(days_in_month(year, month)) {
            days -= u64::from(days_in_month(year, month));
            month += 1;
        }
        (year, month, days as u8 + 1)
    }
}

/// A failed date interpretation, never a fabricated minimum/zero date.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefDateTimeError {
    /// Invalid year, month, day, or time component.
    InvalidComponents,
    /// Incomplete fourteen-byte attribute date.
    IncompleteValue,
    /// Nonfinite or out-of-range ticks/OLE Automation date.
    OutOfRange,
    /// The native value is neither AppTime nor SysTime.
    NotDate,
}

impl fmt::Display for TnefDateTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidComponents => "invalid TNEF calendar date or time",
            Self::IncompleteValue => "incomplete TNEF date value",
            Self::OutOfRange => "TNEF date is outside the supported range",
            Self::NotDate => "the TNEF value is not a date",
        })
    }
}

impl Error for TnefDateTimeError {}

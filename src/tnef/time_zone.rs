// Adapted from MimeKit's Tnef/TnefTimeZone.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use super::{
    TnefDateTime, TnefDateTimeError, TnefDateTimeKind,
    calendar::{CalendarWriter, format_date_time, format_utc_offset},
};
use crate::date_time_offset::days_in_month;

/// A raw SYSTEMTIME transition from an Outlook time-zone rule.
///
/// With year zero, `day` is the occurrence of `day_of_week` in the month,
/// with 5 meaning the last occurrence. Otherwise `day` is a day of the month.
/// Raw fields, including invalid values, are preserved. Transition calculation
/// uses the requested year and ignores milliseconds, matching MimeKit.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TnefSystemTime {
    /// Zero for a relative transition; nonzero for an absolute transition.
    pub year: u16,
    /// Calendar month, or zero when the transition is absent.
    pub month: u16,
    /// Sunday-based weekday, from 0 through 6 for relative transitions.
    pub day_of_week: u16,
    /// Week occurrence (relative) or day of the month (absolute).
    pub day: u16,
    /// Hour of the local transition.
    pub hour: u16,
    /// Minute of the local transition.
    pub minute: u16,
    /// Second of the local transition.
    pub second: u16,
    /// Preserved wire milliseconds; not used in transition calculation.
    pub milliseconds: u16,
}

impl TnefSystemTime {
    /// Reads the first sixteen little-endian bytes without validating the fields.
    ///
    /// # Errors
    ///
    /// Returns `IncompleteValue` if fewer than sixteen bytes are available.
    pub fn read(bytes: &[u8]) -> Result<Self, TnefTimeZoneError> {
        if bytes.len() < 16 {
            return Err(TnefTimeZoneError::IncompleteValue);
        }
        Ok(Self {
            year: word(bytes, 0),
            month: word(bytes, 2),
            day_of_week: word(bytes, 4),
            day: word(bytes, 6),
            hour: word(bytes, 8),
            minute: word(bytes, 10),
            second: word(bytes, 12),
            milliseconds: word(bytes, 14),
        })
    }

    /// Returns whether this uses the day-in-month format.
    pub const fn is_relative(self) -> bool {
        self.year == 0
    }

    /// Checks the transition fields used by MimeKit, excluding milliseconds.
    ///
    /// Absolute dates allow days through 31, which are clamped to the last day
    /// of the requested month. Their stored year and weekday are not validated.
    pub fn is_valid_transition(self) -> bool {
        (1..=12).contains(&self.month)
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
            && if self.is_relative() {
                self.day_of_week <= 6 && (1..=5).contains(&self.day)
            } else {
                (1..=31).contains(&self.day)
            }
    }

    /// Calculates the local, unspecified transition date in `year`.
    ///
    /// # Errors
    ///
    /// Rejects invalid transition fields or years outside 1 through 9999.
    pub fn get_transition(self, year: u16) -> Result<TnefDateTime, TnefDateTimeError> {
        if !self.is_valid_transition() || !(1..=9999).contains(&year) {
            return Err(TnefDateTimeError::InvalidComponents);
        }
        let days = u16::from(days_in_month(year, self.month as u8));
        // A transition either uses the day-in-month format (year 0, day 5
        // meaning the last weekday) or an absolute day of the month.
        let day = if self.is_relative() {
            let first = TnefDateTime::new(
                year,
                self.month as u8,
                1,
                0,
                0,
                0,
                TnefDateTimeKind::Unspecified,
            )?;
            // 0001-01-01 was a Monday; SYSTEMTIME numbers Sunday as zero.
            let weekday = (first.ticks() / 864_000_000_000 + 1) % 7;
            let offset = (u64::from(self.day_of_week) + 7 - weekday) % 7;
            let mut day = 1 + offset as u16 + (self.day - 1) * 7;
            while day > days {
                day -= 7;
            }
            day
        } else {
            self.day.min(days)
        };
        TnefDateTime::new(
            year,
            self.month as u8,
            day as u8,
            self.hour as u8,
            self.minute as u8,
            self.second as u8,
            TnefDateTimeKind::Unspecified,
        )
    }
}

/// An Outlook embedded time zone, not an IANA or host time-zone database entry.
///
/// Biases are minutes in the Windows convention: UTC = local time + bias.
/// Each bias is limited to +/-1440; combined offsets can reach +/-2880.
/// Rules repeat every year using MimeKit's transition and ambiguity policies.
/// The ID is descriptive; it is never resolved through the operating system.
///
/// Conversions interpret the supplied calendar coordinates as UTC or local
/// according to the method, irrespective of their input kind. Results retain
/// 100-nanosecond precision and reject date-range overflow.
///
/// ```
/// use mimecraft::tnef::{TnefDateTime, TnefDateTimeKind, TnefSystemTime, TnefTimeZone};
/// let zone = TnefTimeZone::new(
///     "India", -330, 0, 0, TnefSystemTime::default(), TnefSystemTime::default(),
/// )?;
/// let utc = TnefDateTime::new(2024, 7, 1, 12, 0, 0, TnefDateTimeKind::Utc)?;
/// let local = zone.to_local_time(utc)?;
/// assert_eq!((local.hour(), local.minute()), (17, 30));
/// assert_eq!(zone.to_universal_time(local)?, utc);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TnefTimeZone {
    id: String,
    bias: i32,
    standard_bias: i32,
    daylight_bias: i32,
    standard_date: TnefSystemTime,
    daylight_date: TnefSystemTime,
}

impl TnefTimeZone {
    /// Creates a checked zone. The ID is retained verbatim, including whitespace.
    ///
    /// If either transition's month is zero, both transitions are discarded
    /// and the zone uses only its standard offset.
    ///
    /// # Errors
    ///
    /// Rejects out-of-range biases and invalid paired transitions.
    pub fn new(
        id: impl Into<String>,
        bias: i32,
        standard_bias: i32,
        daylight_bias: i32,
        standard_date: TnefSystemTime,
        daylight_date: TnefSystemTime,
    ) -> Result<Self, TnefTimeZoneError> {
        // Comparing both bounds avoids absolute-value overflow for i32::MIN.
        if [bias, standard_bias, daylight_bias]
            .iter()
            .any(|v| !(-1440..=1440).contains(v))
        {
            return Err(TnefTimeZoneError::InvalidBias);
        }
        // A zone without daylight saving time has month zero in both dates.
        // Treat a zone with only one transition the same way: the missing
        // transition cannot be inferred.
        let (standard_date, daylight_date) = if standard_date.month != 0 && daylight_date.month != 0
        {
            if !standard_date.is_valid_transition() || !daylight_date.is_valid_transition() {
                return Err(TnefTimeZoneError::InvalidTransition);
            }
            (standard_date, daylight_date)
        } else {
            (TnefSystemTime::default(), TnefSystemTime::default())
        };
        Ok(Self {
            id: id.into(),
            bias,
            standard_bias,
            daylight_bias,
            standard_date,
            daylight_date,
        })
    }

    /// Parses a PidLidTimeZoneStruct, retaining the supplied ID verbatim.
    ///
    /// # Errors
    ///
    /// Rejects fewer than 48 bytes, invalid biases, or invalid paired transitions.
    pub fn from_time_zone_struct(
        bytes: &[u8],
        id: impl Into<String>,
    ) -> Result<Self, TnefTimeZoneError> {
        if bytes.len() < 48 {
            return Err(TnefTimeZoneError::IncompleteValue);
        }
        // lBias, lStandardBias, lDaylightBias, wStandardYear, stStandardDate,
        // wDaylightYear, stDaylightDate. The SYSTEMTIMEs start at 14 and 32.
        Self::new(
            id,
            integer(bytes, 0),
            integer(bytes, 4),
            integer(bytes, 8),
            TnefSystemTime::read(&bytes[14..])?,
            TnefSystemTime::read(&bytes[32..])?,
        )
    }

    /// Parses a TZDefinition using its effective TZRule.
    ///
    /// With no effective rule, the last complete declared rule is used, even
    /// if later rules are truncated. The UTF-16 key replaces unpaired
    /// surrogates, removes trailing NULs, and trims whitespace, as in MimeKit.
    /// An unavailable/blank key uses a nonblank trimmed fallback ID, then a
    /// generated ID such as `UTC-05:00`. Extra bytes are ignored.
    ///
    /// # Errors
    ///
    /// Rejects incomplete/invalid headers, unsupported major versions, missing
    /// complete rules, invalid biases, and invalid paired transitions.
    pub fn from_time_zone_definition(
        bytes: &[u8],
        fallback_id: Option<&str>,
    ) -> Result<Self, TnefTimeZoneError> {
        // Major, Minor, cbHeader, Reserved, cchKeyName, KeyName, cRules, TZRules.
        if bytes.len() < 6 {
            return Err(TnefTimeZoneError::IncompleteValue);
        }
        if bytes[0] != 2 {
            return Err(TnefTimeZoneError::InvalidVersion);
        }
        let header_size = usize::from(word(bytes, 2));
        let rules_index = 4 + header_size;
        if header_size < 6 || rules_index > bytes.len() {
            return Err(TnefTimeZoneError::InvalidHeader);
        }
        let rule_count = usize::from(word(bytes, rules_index - 2));
        let mut selected = None;
        for rule in bytes[rules_index..]
            .as_chunks::<66>()
            .0
            .iter()
            .take(rule_count)
        {
            // TZRULE_FLAG_EFFECTIVE_TZREG marks the effective rule. Otherwise
            // fall back to the last complete rule, not a partial trailing rule.
            selected = Some(rule);
            if word(rule, 4) & 0x0002 != 0 {
                break;
            }
        }
        let rule = selected.ok_or(TnefTimeZoneError::NoRule)?;
        // TZRule: Major, Minor, Reserved, Flags, wYear, X (14 bytes),
        // lBias, lStandardBias, lDaylightBias, stStandardDate, stDaylightDate.
        let mut zone = Self::new(
            String::new(),
            integer(rule, 22),
            integer(rule, 26),
            integer(rule, 30),
            TnefSystemTime::read(&rule[34..])?,
            TnefSystemTime::read(&rule[50..])?,
        )?;
        let key_end = 8 + usize::from(word(bytes, 6)) * 2;
        let key = if key_end <= rules_index - 2 {
            char::decode_utf16(
                bytes[8..key_end]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c)),
            )
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
        } else {
            String::new()
        };
        let key = key.trim_end_matches('\0').trim();
        zone.id = if !key.is_empty() {
            key.to_owned()
        } else if let Some(id) = fallback_id.map(str::trim).filter(|id| !id.is_empty()) {
            id.to_owned()
        } else {
            let offset = -i64::from(zone.bias + zone.standard_bias);
            let sign = if offset < 0 { '-' } else { '+' };
            format!("UTC{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60)
        };
        Ok(zone)
    }

    /// Returns the descriptive identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Replaces only the descriptive identifier, retaining it verbatim.
    pub fn set_id(&mut self, id: impl Into<String>) {
        self.id = id.into();
    }

    /// Returns the base Windows bias in minutes.
    pub const fn bias(&self) -> i32 {
        self.bias
    }

    /// Returns the additional standard-time bias in minutes.
    pub const fn standard_bias(&self) -> i32 {
        self.standard_bias
    }

    /// Returns the additional daylight-time bias in minutes.
    pub const fn daylight_bias(&self) -> i32 {
        self.daylight_bias
    }

    /// Returns the standard transition, or zeroed fields for a fixed-offset zone.
    pub const fn standard_date(&self) -> TnefSystemTime {
        self.standard_date
    }

    /// Returns the daylight transition, or zeroed fields for a fixed-offset zone.
    pub const fn daylight_date(&self) -> TnefSystemTime {
        self.daylight_date
    }

    /// Returns whether both daylight-saving transitions are present.
    pub const fn has_daylight_time(&self) -> bool {
        self.standard_date.month != 0
    }

    /// Returns the standard offset in minutes: local = UTC + offset.
    pub const fn standard_offset(&self) -> i32 {
        -(self.bias + self.standard_bias)
    }

    /// Returns the daylight offset, or the standard offset for a fixed zone.
    pub const fn daylight_offset(&self) -> i32 {
        if self.has_daylight_time() {
            -(self.bias + self.daylight_bias)
        } else {
            self.standard_offset()
        }
    }

    /// Compares rules irrespective of ID, ignoring unused daylight fields.
    pub fn has_same_rules(&self, other: &Self) -> bool {
        self.bias == other.bias
            && self.standard_bias == other.standard_bias
            && self.has_daylight_time() == other.has_daylight_time()
            && (!self.has_daylight_time()
                || (self.daylight_bias == other.daylight_bias
                    && self.standard_date == other.standard_date
                    && self.daylight_date == other.daylight_date))
    }

    /// Determines whether UTC coordinates fall in daylight saving time.
    ///
    /// Transition years are taken from local standard time and clamped to
    /// 2 through 9998, matching MimeKit's boundary-year policy.
    ///
    /// # Errors
    ///
    /// Rejects overflow while obtaining the local standard date.
    pub fn is_daylight_time(&self, utc: TnefDateTime) -> Result<bool, TnefDateTimeError> {
        if !self.has_daylight_time() {
            return Ok(false);
        }
        let year = utc
            .checked_add_minutes(self.standard_offset())?
            .year()
            .clamp(2, 9998);
        // DST starts in local standard time and ends in local daylight time.
        let start = self
            .daylight_date
            .get_transition(year)?
            .checked_add_minutes(-self.standard_offset())?;
        let end = self
            .standard_date
            .get_transition(year)?
            .checked_add_minutes(-self.daylight_offset())?;
        if start.ticks() < end.ticks() {
            Ok(utc.ticks() >= start.ticks() && utc.ticks() < end.ticks())
        } else {
            // Southern hemisphere: daylight saving time spans the year end.
            Ok(utc.ticks() >= start.ticks() || utc.ticks() < end.ticks())
        }
    }

    /// Gets the UTC-to-local offset in minutes.
    ///
    /// # Errors
    ///
    /// Shares [`Self::is_daylight_time`] date-range errors.
    pub fn get_offset(&self, utc: TnefDateTime) -> Result<i32, TnefDateTimeError> {
        Ok(if self.is_daylight_time(utc)? {
            self.daylight_offset()
        } else {
            self.standard_offset()
        })
    }

    /// Converts UTC coordinates into local coordinates with unspecified kind.
    ///
    /// # Errors
    ///
    /// Rejects date-range overflow in offset selection or conversion.
    pub fn to_local_time(&self, utc: TnefDateTime) -> Result<TnefDateTime, TnefDateTimeError> {
        Ok(utc
            .checked_add_minutes(self.get_offset(utc)?)?
            .with_kind(TnefDateTimeKind::Unspecified))
    }

    /// Converts local coordinates into UTC, using MimeKit's ambiguity policy.
    ///
    /// Repeated local times prefer daylight time; skipped local times use
    /// standard time. This is not a strict gap/fold validation API.
    ///
    /// # Errors
    ///
    /// Rejects date-range overflow, including a daylight candidate that would
    /// overflow even if the standard-time candidate would fit.
    pub fn to_universal_time(
        &self,
        local: TnefDateTime,
    ) -> Result<TnefDateTime, TnefDateTimeError> {
        // A repeated local time at DST end is interpreted as daylight time.
        // A skipped local time at DST start is interpreted as standard time.
        if self.has_daylight_time() {
            let daylight = local.checked_add_minutes(-self.daylight_offset())?;
            if self.is_daylight_time(daylight)? {
                return Ok(daylight.with_kind(TnefDateTimeKind::Utc));
            }
        }
        Ok(local
            .checked_add_minutes(-self.standard_offset())?
            .with_kind(TnefDateTimeKind::Utc))
    }

    fn write_observance<W: Write>(
        writer: &mut CalendarWriter<W>,
        name: &str,
        date: TnefSystemTime,
        offset_from: i32,
        offset_to: i32,
    ) -> io::Result<()> {
        writer.begin_component(name)?;
        // MS-OXCICAL 2.1.3.1.1.19: DTSTART is the local transition in 1601;
        // RRULE describes the yearly transition.
        let transition = date
            .get_transition(1601)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        writer.write_property("DTSTART", &format_date_time(transition, false))?;
        let rule = if date.is_relative() {
            let week = if date.day == 5 {
                -1
            } else {
                i32::from(date.day)
            };
            let weekday = super::calendar::WEEK_DAYS[usize::from(date.day_of_week)];
            format!("FREQ=YEARLY;BYDAY={week}{weekday};BYMONTH={}", date.month)
        } else {
            format!("FREQ=YEARLY;BYMONTHDAY={};BYMONTH={}", date.day, date.month)
        };
        writer.write_property("RRULE", &rule)?;
        writer.write_property("TZOFFSETFROM", &format_utc_offset(offset_from)?)?;
        writer.write_property("TZOFFSETTO", &format_utc_offset(offset_to)?)?;
        writer.end_component(name)
    }

    /// Writes a VTIMEZONE component without finishing or flushing the writer.
    ///
    /// # Errors
    ///
    /// Propagates calendar writer state and sink errors. Partial sink failures
    /// leave the writer terminally faulted; no rollback is attempted.
    pub fn write_to<W: Write>(&self, writer: &mut CalendarWriter<W>) -> io::Result<()> {
        writer.begin_component("VTIMEZONE")?;
        writer.write_text_property("TZID", &self.id)?;
        if self.has_daylight_time() {
            Self::write_observance(
                writer,
                "STANDARD",
                self.standard_date,
                self.daylight_offset(),
                self.standard_offset(),
            )?;
            Self::write_observance(
                writer,
                "DAYLIGHT",
                self.daylight_date,
                self.standard_offset(),
                self.daylight_offset(),
            )?;
        } else {
            let offset = format_utc_offset(self.standard_offset())?;
            writer.begin_component("STANDARD")?;
            writer.write_property("DTSTART", "16010101T000000")?;
            writer.write_property("TZOFFSETFROM", &offset)?;
            writer.write_property("TZOFFSETTO", &offset)?;
            writer.end_component("STANDARD")?;
        }
        writer.end_component("VTIMEZONE")
    }
}

fn word(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([bytes[index], bytes[index + 1]])
}

fn integer(bytes: &[u8], index: usize) -> i32 {
    i32::from_le_bytes([
        bytes[index],
        bytes[index + 1],
        bytes[index + 2],
        bytes[index + 3],
    ])
}

/// A rejected embedded time-zone structure or rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefTimeZoneError {
    /// The fixed-size structure or definition prefix is incomplete.
    IncompleteValue,
    /// The TZDefinition major version is not 2.
    InvalidVersion,
    /// The definition header is too short or extends beyond the input.
    InvalidHeader,
    /// No complete declared TZRule is available.
    NoRule,
    /// A bias lies outside -1440 through 1440 minutes.
    InvalidBias,
    /// A paired transition has invalid calendar or time fields.
    InvalidTransition,
}

impl fmt::Display for TnefTimeZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::IncompleteValue => "incomplete TNEF time-zone value",
            Self::InvalidVersion => "unsupported TNEF time-zone definition version",
            Self::InvalidHeader => "invalid TNEF time-zone definition header",
            Self::NoRule => "no complete TNEF time-zone rule",
            Self::InvalidBias => "TNEF time-zone bias is outside the supported range",
            Self::InvalidTransition => "invalid TNEF time-zone transition",
        })
    }
}

impl Error for TnefTimeZoneError {}

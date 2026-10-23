// Adapted from MimeKit's Utils/DateUtils.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{error::Error, fmt, str::FromStr};

use crate::parse_utils::{is_whitespace, skip_comments_and_whitespace};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A calendar date and time with a fixed numeric offset, for mail headers.
///
/// [`Self::parse`] follows MimeKit's tolerant date reader, not strict RFC
/// validation. [`Self::new`] instead checks supplied components without recovery.
/// [`fmt::Display`] writes a canonical English date with seconds and a numeric
/// zone, recalculating the weekday. No comments or original spelling are retained.
///
/// This is not a timezone database or a general-purpose date arithmetic API.
/// Offsets are minutes east of UTC. Equality and hashing include the local
/// components and offset; compare [`Self::unix_timestamp`] for instant equality.
///
/// ```
/// use mimecraft::DateTimeOffset;
///
/// let date = DateTimeOffset::parse(b"Tue, 9 Jun 92 03:45:24 JST")?;
/// assert_eq!(date.year(), 1992);
/// assert_eq!(date.offset_minutes(), 540);
/// assert_eq!(date.to_string(), "Tue, 09 Jun 1992 03:45:24 +0900");
/// # Ok::<(), mimecraft::DateTimeOffsetError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DateTimeOffset {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    offset_minutes: i16,
}

impl DateTimeOffset {
    /// Constructs a checked date without parsing or normalizing its components.
    ///
    /// Years range from 1 through 9999 in the proleptic Gregorian calendar,
    /// hours from 0 through 23, and minutes and seconds from 0 through 59.
    /// Offsets range from -840 through 840 minutes (14 hours).
    /// Both local and UTC dates must fit in years 1 through 9999.
    ///
    /// Unlike this constructor, the compatibility parser only recognizes years
    /// from 1969 onward and offsets from -12 through +14 hours. Consequently,
    /// some constructible dates do not round-trip through that parser.
    ///
    /// # Errors
    ///
    /// Returns the corresponding [`DateTimeOffsetError`] for invalid calendar or time
    /// components, an invalid offset, or an out-of-range UTC date.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        offset_minutes: i16,
    ) -> Result<Self, DateTimeOffsetError> {
        if !(1..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
        {
            return Err(DateTimeOffsetError::InvalidDate);
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(DateTimeOffsetError::InvalidTime);
        }
        if !(-840..=840).contains(&offset_minutes) {
            return Err(DateTimeOffsetError::InvalidOffset);
        }
        let date = Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            offset_minutes,
        };
        if !(0..days_before_year(10000) * 86400).contains(&date.utc_seconds()) {
            return Err(DateTimeOffsetError::UtcOutOfRange);
        }
        Ok(date)
    }

    /// Parses a complete byte slice containing a date header's value.
    ///
    /// Tries the standard field order first, then MimeKit's token-based recovery
    /// for reordered fields, numeric dates, missing times, and other legacy
    /// forms. Month/weekday names are case-insensitive; named zones and AM/PM
    /// markers are uppercase only, matching the source token classification.
    ///
    /// Compatibility behavior includes ignoring incorrect weekdays and extra
    /// tokens, treating absent/unrecognized/out-of-range zones as UTC, and
    /// defaulting missing or invalid times to midnight in the recovery parser.
    /// A leap second at `23:59:60` becomes `23:59:59`. `-0000` becomes `+0000`.
    /// An unterminated comment stops tokenization; preceding tokens can still
    /// produce a date. Comment and ignored-token bytes need not be UTF-8.
    ///
    /// # Errors
    ///
    /// Returns [`DateTimeOffsetError::InvalidSyntax`] if neither interpretation yields
    /// a valid date. This heuristic parser does not assign diagnostic offsets.
    pub fn parse(input: &[u8]) -> Result<Self, DateTimeOffsetError> {
        let tokens = tokenize(input);
        parse_standard(&tokens)
            .or_else(|| parse_unknown(&tokens))
            .ok_or(DateTimeOffsetError::InvalidSyntax)
    }

    /// Returns the calendar year.
    pub const fn year(&self) -> u16 {
        self.year
    }

    /// Returns the month, from 1 (January) through 12.
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// Returns the day of the month, starting at 1.
    pub const fn day(&self) -> u8 {
        self.day
    }

    /// Returns the hour, from 0 through 23.
    pub const fn hour(&self) -> u8 {
        self.hour
    }

    /// Returns the minute, from 0 through 59.
    pub const fn minute(&self) -> u8 {
        self.minute
    }

    /// Returns the second, from 0 through 59.
    pub const fn second(&self) -> u8 {
        self.second
    }

    /// Returns the fixed offset in minutes east of UTC.
    pub const fn offset_minutes(&self) -> i16 {
        self.offset_minutes
    }

    /// Returns whole seconds since 1970-01-01 00:00:00 UTC, excluding leap seconds.
    ///
    /// ```
    /// use mimecraft::DateTimeOffset;
    ///
    /// let date = DateTimeOffset::new(1970, 1, 1, 1, 0, 0, 60)?;
    /// assert_eq!(date.unix_timestamp(), 0);
    /// # Ok::<(), mimecraft::DateTimeOffsetError>(())
    /// ```
    pub fn unix_timestamp(&self) -> i64 {
        self.utc_seconds() - days_before_year(1970) * 86400
    }

    fn ordinal(&self) -> i64 {
        let mut days = days_before_year(i64::from(self.year));
        for month in 1..self.month {
            days += i64::from(days_in_month(self.year, month));
        }
        days + i64::from(self.day) - 1
    }

    fn utc_seconds(&self) -> i64 {
        self.ordinal() * 86400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
            - i64::from(self.offset_minutes) * 60
    }
}

impl FromStr for DateTimeOffset {
    type Err = DateTimeOffsetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.as_bytes())
    }
}

impl fmt::Display for DateTimeOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Gregorian 0001-01-01 was a Monday; the name table starts on Sunday.
        let weekday = ((self.ordinal() + 1) % 7) as usize;
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let offset = self.offset_minutes.unsigned_abs();
        write!(
            f,
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} {}{:02}{:02}",
            WEEKDAYS[weekday],
            self.day,
            MONTHS[usize::from(self.month - 1)],
            self.year,
            self.hour,
            self.minute,
            self.second,
            sign,
            offset / 60,
            offset % 60,
        )
    }
}

/// An invalid date value or an unsuccessful date-header interpretation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateTimeOffsetError {
    /// The year, month, or day is outside the supported Gregorian calendar.
    InvalidDate,
    /// The hour, minute, or second is out of range.
    InvalidTime,
    /// The fixed offset exceeds 14 hours in either direction.
    InvalidOffset,
    /// Applying the offset would put the UTC date outside years 1 through 9999.
    UtcOutOfRange,
    /// Neither the standard nor the recovery parser found a valid date.
    InvalidSyntax,
}

impl fmt::Display for DateTimeOffsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDate => "invalid calendar date",
            Self::InvalidTime => "invalid time of day",
            Self::InvalidOffset => "date offset exceeds 14 hours",
            Self::UtcOutOfRange => "UTC date is outside years 1 through 9999",
            Self::InvalidSyntax => "could not interpret a valid mail date",
        })
    }
}

impl Error for DateTimeOffsetError {}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

const fn days_before_year(year: i64) -> i64 {
    let y = year - 1;
    y * 365 + y / 4 - y / 100 + y / 400
}

const NON_NUMERIC: u8 = 1;
const NON_WEEKDAY: u8 = 2;
const NON_MONTH: u8 = 4;
const NON_TIME: u8 = 8;
const NON_ALPHA_ZONE: u8 = 16;
const NON_NUMERIC_ZONE: u8 = 32;
const HAS_COLON: u8 = 64;
const HAS_SIGN: u8 = 128;

const TOKEN_FLAGS: [u8; 256] = {
    const fn contains(bytes: &[u8], byte: u8) -> bool {
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == byte {
                return true;
            }
            index += 1;
        }
        false
    }

    let mut table = [0; 256];
    let mut index = 0;
    while index < table.len() {
        let byte = index as u8;
        let mut flags = 0;
        if !byte.is_ascii_digit() {
            flags |= NON_NUMERIC;
        }
        if !contains(
            b"sundaymondaytuesdaywednesdaythursdayfridaysaturday",
            byte.to_ascii_lowercase(),
        ) {
            flags |= NON_WEEKDAY;
        }
        if !contains(
            b"januaryfebruarymarchaprilmayjunejulyaugustseptemberoctobernovemberdecember",
            byte.to_ascii_lowercase(),
        ) {
            flags |= NON_MONTH;
        }
        if !byte.is_ascii_digit() && byte != b':' {
            flags |= NON_TIME;
        }
        if !byte.is_ascii_uppercase() {
            flags |= NON_ALPHA_ZONE;
        }
        if !byte.is_ascii_digit() && byte != b'+' && byte != b'-' {
            flags |= NON_NUMERIC_ZONE;
        }
        if byte == b':' {
            flags |= HAS_COLON;
        }
        if byte == b'+' || byte == b'-' {
            flags |= HAS_SIGN;
        }
        table[index] = flags;
        index += 1;
    }
    table
};

#[derive(Clone, Copy)]
struct Token<'a> {
    bytes: &'a [u8],
    flags: u8,
}

impl Token<'_> {
    fn numeric(self) -> bool {
        self.flags & NON_NUMERIC == 0
    }

    fn alpha_zone(self) -> bool {
        self.flags & NON_ALPHA_ZONE == 0
    }

    fn numeric_zone(self) -> bool {
        self.flags & (NON_NUMERIC_ZONE | HAS_SIGN) == HAS_SIGN
    }

    fn weekday(self) -> bool {
        self.flags & NON_WEEKDAY == 0 && self.name_index(&WEEKDAYS).is_some()
    }

    fn month(self) -> Option<i32> {
        if self.flags & NON_MONTH != 0 {
            return None;
        }
        self.name_index(&MONTHS).map(|index| index as i32 + 1)
    }

    fn name_index(self, names: &[&str]) -> Option<usize> {
        let prefix = self.bytes.get(..3)?;
        names
            .iter()
            .position(|name| prefix.eq_ignore_ascii_case(name.as_bytes()))
    }

    fn day(self) -> Option<i32> {
        let day = number(self.bytes)?;
        (1..=31).contains(&day).then_some(day)
    }

    fn year(self) -> Option<i32> {
        let mut year = number(self.bytes)?;
        if year < 100 {
            year += if year < 70 { 2000 } else { 1900 };
        }
        (year >= 1969).then_some(year)
    }

    fn time(self) -> Option<(i32, i32, i32)> {
        if self.flags & (NON_TIME | HAS_COLON) != HAS_COLON {
            return None;
        }
        let mut parts = self.bytes.split(|&byte| byte == b':');
        let hour = number(parts.next()?)?;
        let minute = number(parts.next()?)?;
        // Accept legacy hh:mm values without requiring a seconds field.
        let mut second = match parts.next() {
            Some(part) => number(part)?,
            None => 0,
        };
        if parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        if second == 60 {
            if hour != 23 || minute != 59 {
                return None;
            }
            // Like the source's DateTime, this representation cannot store a
            // leap second. Clamp only the valid end-of-day spelling.
            second = 59;
        }
        Some((hour, minute, second))
    }

    fn am_pm(self, hour: &mut i32) -> bool {
        if !self.alpha_zone() {
            return false;
        }
        match self.bytes {
            b"AM" => {
                if *hour == 12 {
                    *hour = 0;
                }
                true
            }
            b"PM" => {
                if *hour < 12 {
                    *hour += 12;
                }
                true
            }
            _ => false,
        }
    }

    fn zone(self) -> Option<i32> {
        let zone = if self.numeric_zone() {
            let sign = match self.bytes[0] {
                b'-' => -1,
                b'+' => 1,
                _ => return None,
            };
            sign * number(&self.bytes[1..])?
        } else if self.alpha_zone() {
            match self.bytes {
                b"UT" | b"UTC" | b"GMT" | b"Z" => 0,
                b"EDT" => -400,
                b"EST" | b"CDT" => -500,
                b"CST" | b"MDT" => -600,
                b"MST" | b"PDT" => -700,
                b"PST" => -800,
                b"JST" | b"KST" => 900,
                // RFC 822 reversed the military-zone signs; some senders still
                // use those mistaken values. Match MimeKit's corrected signs.
                [byte @ b'A'..=b'I'] => i32::from(byte - b'A' + 1) * 100,
                [byte @ b'K'..=b'M'] => i32::from(byte - b'K' + 10) * 100,
                [byte @ b'N'..=b'Y'] => -i32::from(byte - b'N' + 1) * 100,
                _ => return None,
            }
        } else if self.numeric() {
            number(self.bytes)?
        } else {
            return None;
        };
        // Validate the minute digits before converting HHMM to minutes:
        // otherwise an invalid -1192 would silently normalize to -12:32.
        if !(-1200..=1400).contains(&zone) || (zone % 100).abs() >= 60 {
            return None;
        }
        Some(zone / 100 * 60 + zone % 100)
    }
}

fn number(bytes: &[u8]) -> Option<i32> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0i32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(i32::from(byte - b'0'))?;
    }
    Some(value)
}

fn tokenize(input: &[u8]) -> Vec<Token<'_>> {
    let mut tokens = Vec::with_capacity(8);
    let mut index = 0;
    while index < input.len() {
        if skip_comments_and_whitespace(input, &mut index).is_err() || index == input.len() {
            break;
        }
        let start = index;
        let mut flags = TOKEN_FLAGS[usize::from(input[index])];
        index += 1;
        // A delimiter at the start is part of the token, notably a zone's sign.
        while index < input.len()
            && !is_whitespace(input[index])
            && !matches!(input[index], b'-' | b'/' | b',')
        {
            flags |= TOKEN_FLAGS[usize::from(input[index])];
            index += 1;
        }
        tokens.push(Token {
            bytes: &input[start..index],
            flags,
        });
        if index < input.len() {
            index += 1;
        }
    }
    tokens
}

fn parsed_date(
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    zone: i32,
) -> Option<DateTimeOffset> {
    DateTimeOffset::new(
        year.try_into().ok()?,
        month.try_into().ok()?,
        day.try_into().ok()?,
        hour.try_into().ok()?,
        minute.try_into().ok()?,
        second.try_into().ok()?,
        zone.try_into().ok()?,
    )
    .ok()
}

fn parse_standard(tokens: &[Token<'_>]) -> Option<DateTimeOffset> {
    // Day, month, year, time, and zone require five tokens; an optional
    // weekday adds a sixth but need not agree with the actual calendar date.
    if tokens.len() < 5 {
        return None;
    }
    let index = usize::from(tokens[0].weekday());
    if tokens.len() < index + 5 {
        return None;
    }
    let day = tokens[index].day()?;
    let month = tokens[index + 1].month()?;
    let year = tokens[index + 2].year()?;
    let (mut hour, minute, second) = tokens[index + 3].time()?;
    let mut zone_index = index + 4;
    if tokens[zone_index].am_pm(&mut hour) {
        zone_index += 1;
    }
    let zone = tokens
        .get(zone_index)
        .and_then(|token| token.zone())
        .unwrap_or(0);
    parsed_date(year, month, day, hour, minute, second, zone)
}

fn parse_unknown(tokens: &[Token<'_>]) -> Option<DateTimeOffset> {
    let (mut day, mut month, mut year, mut zone) = (None, None, None, None);
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    let mut numeric_month = false;
    let mut have_weekday = false;
    let mut have_time = false;
    let mut have_am_pm = false;

    for &token in tokens {
        if !have_weekday && token.weekday() {
            have_weekday = true;
            continue;
        }
        if (month.is_none() || numeric_month)
            && let Some(value) = token.month()
        {
            if numeric_month {
                // A named month disambiguates an earlier numeric guess:
                // that number was the day, not the month.
                numeric_month = false;
                day = month;
            }
            month = Some(value);
            continue;
        }
        if !have_time && let Some(time) = token.time() {
            (hour, minute, second) = time;
            have_time = true;
            continue;
        }
        if have_time && zone.is_none() && !have_am_pm && token.am_pm(&mut hour) {
            have_am_pm = true;
            continue;
        }
        // Leave bare numbers for the date-field heuristics below. Only signed
        // numeric offsets and named zones are unambiguous at this point.
        if zone.is_none()
            && (token.alpha_zone() || token.numeric_zone())
            && let Some(value) = token.zone()
        {
            zone = Some(value);
            continue;
        }
        if !token.numeric() {
            continue;
        }
        if token.bytes.len() == 4 {
            if year.is_none() {
                year = token.year();
            } else if zone.is_none() {
                zone = token.zone();
            }
            continue;
        }
        if token.bytes.len() > 2 {
            continue;
        }
        // Assume YYYY[-/]MM[-/]DD or MM[-/]DD[-/]YY for numeric dates.
        let value = number(token.bytes)?;
        if month.is_none() && (1..=12).contains(&value) {
            numeric_month = true;
            month = Some(value);
        } else if day.is_none() && (1..=31).contains(&value) {
            day = Some(value);
        } else if year.is_none() && value >= 69 {
            year = Some(1900 + value);
        }
    }
    parsed_date(year?, month?, day?, hour, minute, second, zone.unwrap_or(0))
}

// Adapted from MimeKit's Tnef/TnefCalendarBuilder.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::HashSet,
    error::Error,
    fmt::{self, Write},
};

use super::{
    TnefDateTime, TnefDateTimeError, TnefRecurrence as Recurrence, TnefTimeZone,
    calendar::{WEEK_DAYS, format_date, format_date_time},
};

/// A recurrence that cannot be represented by the calendar rule generator.
///
/// Unsupported calendars are distinguished from invalid patterns and date
/// overflow so the calendar builder can report the appropriate loss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecurrenceRuleError {
    /// The pattern/calendar combination needs non-Gregorian recurrence support.
    UnsupportedCalendar {
        /// Raw pattern type, including Hijri variants.
        pattern_type: u16,
        /// Raw calendar type.
        calendar_type: u16,
    },
    /// A daily period is zero or is not a whole number of days.
    InvalidDailyPeriod,
    /// A weekly period is zero or its mask has no recognized weekdays.
    InvalidWeeklyPattern,
    /// A monthly period is zero.
    InvalidMonthlyPeriod,
    /// A monthly day, weekday mask, or occurrence is invalid.
    InvalidMonthDay,
    /// An unknown pattern layout.
    UnknownPattern(u16),
    /// UNTIL arithmetic or time-zone conversion exceeds the supported range.
    InvalidDate(TnefDateTimeError),
}

impl fmt::Display for RecurrenceRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCalendar {
                pattern_type,
                calendar_type,
            } => write!(
                f,
                "recurrence pattern 0x{pattern_type:04X} with calendar 0x{calendar_type:04X} cannot be represented by a Gregorian RRULE"
            ),
            Self::InvalidDailyPeriod => {
                f.write_str("the daily period is not a whole number of days")
            }
            Self::InvalidWeeklyPattern => {
                f.write_str("the weekly pattern has a zero period or no days")
            }
            Self::InvalidMonthlyPeriod => f.write_str("the monthly period is zero"),
            Self::InvalidMonthDay => f.write_str("the day of the month is not valid"),
            Self::UnknownPattern(pattern) => {
                write!(f, "the pattern type 0x{pattern:04X} is not known")
            }
            Self::InvalidDate(error) => write!(f, "the recurrence end date is not valid: {error}"),
        }
    }
}

impl Error for RecurrenceRuleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidDate(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TnefDateTimeError> for RecurrenceRuleError {
    fn from(error: TnefDateTimeError) -> Self {
        Self::InvalidDate(error)
    }
}

fn is_gregorian(calendar: u16) -> bool {
    // MS-OXOCAL 2.2.1.44.1: CAL_DEFAULT, CAL_GREGORIAN and its localized
    // variants, plus Japanese, Taiwan, Korean and Thai calendars, which use
    // Gregorian months and days.
    matches!(calendar, 0x00..=0x05 | 0x07 | 0x09..=0x0c)
}

fn append_days(rule: &mut String, mask: u32) {
    let mut first = true;
    for (index, day) in WEEK_DAYS.iter().enumerate() {
        if mask & (1 << index) == 0 {
            continue;
        }
        if !first {
            rule.push(',');
        }
        rule.push_str(day);
        first = false;
    }
}

/// Builds an RRULE value (without `RRULE:`) using MimeKit's MS-OXCICAL mapping.
///
/// Daily periods use minutes, weekly periods weeks, and monthly periods
/// months. Multiples of twelve months become yearly rules using StartDate's
/// month. Day 31 and month-end patterns mean the last day of the month.
///
/// COUNT is retained even when zero. Unknown end types omit COUNT/UNTIL.
/// All-day UNTIL is a date and ignores the start-time offset; timed UNTIL
/// adds that unsigned offset and uses the supplied zone's local-to-UTC policy.
/// The zone is unused for other termination types.
///
/// A malformed exception tail does not prevent generating the master rule.
/// Callers must still inspect [`Recurrence::exception_error`] and
/// [`Recurrence::had_decode_errors`]. No occurrences or exception events are
/// generated, and the parsed model is not modified.
///
/// # Errors
///
/// Reports unsupported calendars, invalid periods/days, and date overflow.
/// No partial string or silent single-occurrence fallback is returned.
///
/// ```
/// use mimecraft::{charsets::Charset, tnef::{TnefRecurrence, TnefTimeZone,
///     calendar::{CalendarWriter, recurrence_rule}}};
/// // A daily master with no end and an empty exception section.
/// let mut bytes = vec![0; 72];
/// bytes[..2].copy_from_slice(&0x3004u16.to_le_bytes());
/// bytes[14..18].copy_from_slice(&1440u32.to_le_bytes());
/// bytes[22..26].copy_from_slice(&0x2023u32.to_le_bytes());
/// let recurrence = TnefRecurrence::parse(&bytes, Charset::ASCII)?;
/// assert!(recurrence.exception_error().is_none());
/// let utc = TnefTimeZone::new("UTC", 0, 0, 0,
///     Default::default(), Default::default())?;
/// let rule = recurrence_rule(&recurrence, &utc, false)?;
/// let mut writer = CalendarWriter::new(Vec::new());
/// writer.write_property("RRULE", &rule)?;
/// writer.finish()?;
/// assert_eq!(writer.into_inner(), b"RRULE:FREQ=DAILY\r\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn recurrence_rule(
    recurrence: &Recurrence,
    zone: &TnefTimeZone,
    all_day: bool,
) -> Result<String, RecurrenceRuleError> {
    let mut pattern = recurrence.pattern_type();
    let calendar = recurrence.calendar_type();
    if !is_gregorian(calendar) || (pattern >= Recurrence::PATTERN_HJ_MONTH && calendar == 0) {
        return Err(RecurrenceRuleError::UnsupportedCalendar {
            pattern_type: pattern,
            calendar_type: calendar,
        });
    }
    if pattern >= Recurrence::PATTERN_HJ_MONTH {
        // MS-OXCICAL 2.1.3.2.1: Hijri patterns need X-MICROSOFT-RRULE unless
        // the calendar is explicitly Gregorian.
        pattern = pattern - Recurrence::PATTERN_HJ_MONTH + Recurrence::PATTERN_MONTH;
    }

    let mut rule = String::from("FREQ=");
    let period = recurrence.period();
    let interval = match pattern {
        Recurrence::PATTERN_DAY => {
            // The period of a daily recurrence is in minutes.
            if period == 0 || !period.is_multiple_of(1440) {
                return Err(RecurrenceRuleError::InvalidDailyPeriod);
            }
            rule.push_str("DAILY");
            period / 1440
        }
        Recurrence::PATTERN_WEEK => {
            if period == 0 || recurrence.day_mask() & 0x7f == 0 {
                return Err(RecurrenceRuleError::InvalidWeeklyPattern);
            }
            rule.push_str("WEEKLY;BYDAY=");
            append_days(&mut rule, recurrence.day_mask());
            period
        }
        Recurrence::PATTERN_MONTH
        | Recurrence::PATTERN_MONTH_END
        | Recurrence::PATTERN_MONTH_NTH => {
            if period == 0 {
                return Err(RecurrenceRuleError::InvalidMonthlyPeriod);
            }
            let yearly = period.is_multiple_of(12);
            rule.push_str(if yearly { "YEARLY" } else { "MONTHLY" });
            if pattern == Recurrence::PATTERN_MONTH_NTH {
                if recurrence.day_mask() & 0x7f == 0
                    || !(1..=5).contains(&recurrence.week_of_month())
                {
                    return Err(RecurrenceRuleError::InvalidMonthDay);
                }
                rule.push_str(";BYDAY=");
                append_days(&mut rule, recurrence.day_mask());
            } else {
                let day = recurrence.day_of_month();
                rule.push_str(";BYMONTHDAY=");
                if pattern == Recurrence::PATTERN_MONTH_END || day == 31 {
                    rule.push_str("-1");
                } else if (1..=30).contains(&day) {
                    write!(rule, "{day}").expect("writing to a String cannot fail");
                } else {
                    return Err(RecurrenceRuleError::InvalidMonthDay);
                }
            }
            if yearly {
                let month = TnefDateTime::from_recurrence_minutes(recurrence.start_date()).month();
                write!(rule, ";BYMONTH={month}").expect("writing to a String cannot fail");
            }
            if pattern == Recurrence::PATTERN_MONTH_NTH {
                let week = recurrence.week_of_month();
                if week == 5 {
                    rule.push_str(";BYSETPOS=-1");
                } else {
                    write!(rule, ";BYSETPOS={week}").expect("writing to a String cannot fail");
                }
            }
            if yearly { period / 12 } else { period }
        }
        other => return Err(RecurrenceRuleError::UnknownPattern(other)),
    };
    if interval != 1 {
        write!(rule, ";INTERVAL={interval}").expect("writing to a String cannot fail");
    }
    match recurrence.end_type() {
        Recurrence::END_AFTER_OCCURRENCES => {
            write!(rule, ";COUNT={}", recurrence.occurrence_count())
                .expect("writing to a String cannot fail");
        }
        Recurrence::END_AFTER_DATE => {
            // UNTIL is the start of the last instance, in UTC unless all-day.
            let until = TnefDateTime::from_recurrence_minutes(recurrence.end_date());
            rule.push_str(";UNTIL=");
            if all_day {
                rule.push_str(&format_date(until));
            } else {
                let local = until.checked_add_unsigned_minutes(recurrence.start_time_offset())?;
                rule.push_str(&format_date_time(zone.to_universal_time(local)?, true));
            }
        }
        _ => {}
    }
    if pattern == Recurrence::PATTERN_WEEK && period > 1 && recurrence.first_day_of_week() < 7 {
        rule.push_str(";WKST=");
        rule.push_str(WEEK_DAYS[recurrence.first_day_of_week() as usize]);
    }
    Ok(rule)
}

/// Selects local EXDATE start times, retaining deleted-date order and duplicates.
///
/// Modified days come from successfully parsed exceptions' original starts,
/// not the ModifiedInstanceDates array. Both sides are compared as dates before
/// adding StartTimeOffset. Even all-day callers must add that offset before
/// formatting the resulting calendar date, matching MimeKit.
///
/// The results have unspecified kind. Use the series TZID for timed local
/// EXDATEs, or format them as DATE values for all-day events. This function
/// neither validates the master RRULE nor consumes exception diagnostics:
/// generate a valid rule first and inspect [`Recurrence::exception_error`].
/// After malformed-tail recovery, discarded exceptions no longer suppress
/// deleted days. Exception-export limits must not change this selection.
///
/// # Errors
///
/// Returns date overflow without exposing a partial result.
pub fn recurrence_exdates(recurrence: &Recurrence) -> Result<Vec<TnefDateTime>, TnefDateTimeError> {
    // MS-OXCICAL 2.1.3.1.1.20.11: deleted instances that are not modified
    // instances become EXDATEs. Compare local calendar days, not UTC instants.
    let modified: HashSet<_> = recurrence
        .exceptions()
        .iter()
        .map(|ex| ex.original_start_date() / 1440)
        .collect();
    recurrence
        .deleted_instance_dates()
        .iter()
        .filter(|&&minutes| !modified.contains(&(minutes / 1440)))
        .map(|&minutes| {
            TnefDateTime::from_recurrence_minutes(minutes / 1440 * 1440)
                .checked_add_unsigned_minutes(recurrence.start_time_offset())
        })
        .collect()
}

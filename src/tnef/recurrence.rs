// Adapted from MimeKit's Tnef/TnefRecurrence.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use crate::charsets::Charset;
use std::{error::Error, fmt};

/// An ExceptionInfo merged with its matching ExtendedException (MS-OXOCAL
/// 2.2.1.44.2 and 2.2.1.44.3).
///
/// All dates are local minutes since 1601-01-01. Use
/// [`super::TnefDateTime::from_recurrence_minutes`] to obtain calendar values.
/// Fields without an override retain zero/false/None; inspect the flags before
/// treating them as replacements. Unknown flag bits are retained.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TnefRecurrenceException {
    start_date_time: u32,
    end_date_time: u32,
    original_start_date: u32,
    override_flags: u16,
    subject: Option<String>,
    location: Option<String>,
    reminder_delta: i32,
    reminder_set: bool,
    busy_status: i32,
    is_all_day: bool,
}

impl TnefRecurrenceException {
    /// Subject override.
    pub const SUBJECT_FLAG: u16 = 0x0001;
    /// Meeting-type override; its payload is skipped by this parser.
    pub const MEETING_TYPE_FLAG: u16 = 0x0002;
    /// Reminder delta override.
    pub const REMINDER_DELTA_FLAG: u16 = 0x0004;
    /// Reminder enabled/disabled override.
    pub const REMINDER_FLAG: u16 = 0x0008;
    /// Location override.
    pub const LOCATION_FLAG: u16 = 0x0010;
    /// Busy-status override.
    pub const BUSY_STATUS_FLAG: u16 = 0x0020;
    /// Attachment override; its payload is skipped by this parser.
    pub const ATTACHMENT_FLAG: u16 = 0x0040;
    /// All-day subtype override.
    pub const SUB_TYPE_FLAG: u16 = 0x0080;
    /// Appointment-color override; its payload is skipped by this parser.
    pub const APPOINTMENT_COLOR_FLAG: u16 = 0x0100;
    /// Exceptional body flag; no inline payload is consumed.
    pub const EXCEPTIONAL_BODY_FLAG: u16 = 0x0200;

    /// Returns whether any bit in `flag` is present.
    pub const fn has_override(&self, flag: u16) -> bool {
        self.override_flags & flag != 0
    }
    /// Returns the modified start in local recurrence minutes.
    pub const fn start_date_time(&self) -> u32 {
        self.start_date_time
    }
    /// Returns the modified end in local recurrence minutes.
    pub const fn end_date_time(&self) -> u32 {
        self.end_date_time
    }
    /// Returns the original start in local recurrence minutes.
    pub const fn original_start_date(&self) -> u32 {
        self.original_start_date
    }
    /// Returns the unmodified override bitmask.
    pub const fn override_flags(&self) -> u16 {
        self.override_flags
    }
    /// Returns the subject, preferring the extended Unicode value even if empty.
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }
    /// Returns the location, preferring the extended Unicode value even if empty.
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }
    /// Returns the signed reminder delta in minutes.
    pub const fn reminder_delta(&self) -> i32 {
        self.reminder_delta
    }
    /// Returns the reminder override (any nonzero wire value means true).
    pub const fn reminder_set(&self) -> bool {
        self.reminder_set
    }
    /// Returns the signed raw busy-status override.
    pub const fn busy_status(&self) -> i32 {
        self.busy_status
    }
    /// Returns the all-day override (any nonzero wire value means true).
    pub const fn is_all_day(&self) -> bool {
        self.is_all_day
    }
}

/// A decoded AppointmentRecurrencePattern from PidLidAppointmentRecur.
///
/// This is a structural parser, not an occurrence generator or RRULE builder.
/// Calendar IDs, periods, masks, dates, and end types retain their raw values;
/// semantic validation belongs to the calendar consumer.
///
/// Malformed exception data does not invalidate the master recurrence:
/// all exceptions are discarded and [`Self::exception_error`] records the
/// failure. Callers must inspect that diagnostic rather than assume that an
/// empty exception list means none were present.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "inspect exception_error() for recovered malformed exception data"]
pub struct TnefRecurrence {
    pattern_type: u16,
    calendar_type: u16,
    first_date_time: u32,
    period: u32,
    day_mask: u32,
    day_of_month: u32,
    week_of_month: u32,
    end_type: u32,
    occurrence_count: u32,
    first_day_of_week: u32,
    deleted_instance_dates: Vec<u32>,
    modified_instance_dates: Vec<u32>,
    start_date: u32,
    end_date: u32,
    start_time_offset: u32,
    end_time_offset: u32,
    exceptions: Vec<TnefRecurrenceException>,
    exception_error: Option<TnefRecurrenceError>,
    had_decode_errors: bool,
}

impl TnefRecurrence {
    // MS-OXOCAL 2.2.1.44.1: RecurrencePattern.PatternType values.
    /// Daily pattern.
    pub const PATTERN_DAY: u16 = 0x0000;
    /// Weekly pattern.
    pub const PATTERN_WEEK: u16 = 0x0001;
    /// Monthly day-of-month pattern.
    pub const PATTERN_MONTH: u16 = 0x0002;
    /// Monthly weekday occurrence pattern.
    pub const PATTERN_MONTH_NTH: u16 = 0x0003;
    /// Monthly end-of-month pattern.
    pub const PATTERN_MONTH_END: u16 = 0x0004;
    /// Hijri monthly day-of-month pattern.
    pub const PATTERN_HJ_MONTH: u16 = 0x000a;
    /// Hijri monthly weekday occurrence pattern.
    pub const PATTERN_HJ_MONTH_NTH: u16 = 0x000b;
    /// Hijri monthly end-of-month pattern.
    pub const PATTERN_HJ_MONTH_END: u16 = 0x000c;

    // MS-OXOCAL 2.2.1.44.1: RecurrencePattern.EndType values.
    /// End after a date.
    pub const END_AFTER_DATE: u32 = 0x2021;
    /// End after a number of occurrences.
    pub const END_AFTER_OCCURRENCES: u32 = 0x2022;

    /// Parses a complete byte slice with an explicit charset for 8-bit strings.
    ///
    /// Strings strip trailing NULs, preserve BOMs and whitespace, and prefer
    /// extended UTF-16LE subject/location values. Malformed text uses the
    /// existing [`Charset::decode_lossy`] U+FFFD policy, with replacements
    /// reported by [`Self::had_decode_errors`]. Unlike .NET's ASCII defaults,
    /// malformed ASCII therefore uses U+FFFD rather than `?`.
    ///
    /// Counts and block sizes are checked against remaining input before
    /// allocation or skipping. No arbitrary occurrence/exception limit is
    /// imposed. ReservedBlock2 and other trailing bytes are ignored, as in
    /// MimeKit; neither array consistency nor calendar semantics are checked.
    ///
    /// # Errors
    ///
    /// Returns errors for a truncated master, unsupported reader version, or
    /// unknown pattern type. Exception-tail errors instead appear in the
    /// returned model's `exception_error`.
    ///
    /// ```
    /// use mimecraft::{charsets::Charset, tnef::{TnefRecurrence, TnefRecurrenceError}};
    /// // A daily master without its exception tail remains available.
    /// let mut bytes = vec![0; 66];
    /// bytes[..2].copy_from_slice(&0x3004u16.to_le_bytes());
    /// bytes[14..18].copy_from_slice(&1440u32.to_le_bytes());
    /// let recurrence = TnefRecurrence::parse(&bytes, Charset::ASCII)?;
    /// assert_eq!(recurrence.period(), 1440);
    /// assert_eq!(recurrence.exception_error(),
    ///     Some(&TnefRecurrenceError::UnexpectedEnd { offset: 66 }));
    /// # Ok::<(), TnefRecurrenceError>(())
    /// ```
    pub fn parse(bytes: &[u8], charset: Charset) -> Result<Self, TnefRecurrenceError> {
        let mut reader = BlobReader { bytes, index: 0 };
        // RecurrencePattern (MS-OXOCAL 2.2.1.44.1).
        let version = reader.u16()?;
        if version != 0x3004 {
            return Err(TnefRecurrenceError::UnsupportedVersion(version));
        }
        reader.u16()?; // WriterVersion
        reader.u16()?; // RecurFrequency
        let pattern_type = reader.u16()?;
        let calendar_type = reader.u16()?;
        let first_date_time = reader.u32()?;
        let period = reader.u32()?;
        reader.u32()?; // SlidingFlag
        // PatternTypeSpecific: weekday bits (Week/MonthNth), day of month
        // (Month/MonthEnd), and week of month (MonthNth).
        let (mut day_mask, mut day_of_month, mut week_of_month) = (0, 0, 0);
        match pattern_type {
            Self::PATTERN_DAY => {}
            Self::PATTERN_WEEK => day_mask = reader.u32()?,
            Self::PATTERN_MONTH
            | Self::PATTERN_MONTH_END
            | Self::PATTERN_HJ_MONTH
            | Self::PATTERN_HJ_MONTH_END => day_of_month = reader.u32()?,
            Self::PATTERN_MONTH_NTH | Self::PATTERN_HJ_MONTH_NTH => {
                day_mask = reader.u32()?;
                week_of_month = reader.u32()?;
            }
            value => return Err(TnefRecurrenceError::UnknownPattern(value)),
        }
        let end_type = reader.u32()?;
        let occurrence_count = reader.u32()?;
        let first_day_of_week = reader.u32()?;
        let deleted_count = reader.u32()?;
        let deleted_instance_dates = reader.array(deleted_count)?;
        let modified_count = reader.u32()?;
        let modified_instance_dates = reader.array(modified_count)?;
        let start_date = reader.u32()?;
        let end_date = reader.u32()?;
        // The remainder of AppointmentRecurrencePattern.
        reader.u32()?; // ReaderVersion2
        let writer_version2 = reader.u32()?;
        let start_time_offset = reader.u32()?;
        let end_time_offset = reader.u32()?;
        let mut result = Self {
            pattern_type,
            calendar_type,
            first_date_time,
            period,
            day_mask,
            day_of_month,
            week_of_month,
            end_type,
            occurrence_count,
            first_day_of_week,
            deleted_instance_dates,
            modified_instance_dates,
            start_date,
            end_date,
            start_time_offset,
            end_time_offset,
            exceptions: Vec::new(),
            exception_error: None,
            had_decode_errors: false,
        };
        // Exceptions are optional for conversion: a malformed tail still
        // permits exporting the master. Discard every exception already read,
        // since the input evidently was not laid out as expected.
        if let Err(error) = result.read_exceptions(&mut reader, writer_version2, charset) {
            result.exceptions.clear();
            result.exception_error = Some(error);
        }
        Ok(result)
    }

    fn read_exceptions(
        &mut self,
        reader: &mut BlobReader<'_>,
        version: u32,
        charset: Charset,
    ) -> Result<(), TnefRecurrenceError> {
        let count = reader.u16()?;
        // ExceptionInfo (MS-OXOCAL 2.2.1.44.2). Grow only as complete entries
        // are read, never preallocate based on an unchecked count.
        for _ in 0..count {
            let mut exception = TnefRecurrenceException {
                start_date_time: reader.u32()?,
                end_date_time: reader.u32()?,
                original_start_date: reader.u32()?,
                override_flags: reader.u16()?,
                ..TnefRecurrenceException::default()
            };
            use TnefRecurrenceException as Ex;
            if exception.has_override(Ex::SUBJECT_FLAG) {
                reader.u16()?; // SubjectLength
                let count = reader.u16()?;
                exception.subject =
                    Some(reader.string(u64::from(count), charset, &mut self.had_decode_errors)?);
            }
            if exception.has_override(Ex::MEETING_TYPE_FLAG) {
                reader.u32()?;
            }
            if exception.has_override(Ex::REMINDER_DELTA_FLAG) {
                exception.reminder_delta = reader.u32()? as i32;
            }
            if exception.has_override(Ex::REMINDER_FLAG) {
                exception.reminder_set = reader.u32()? != 0;
            }
            if exception.has_override(Ex::LOCATION_FLAG) {
                reader.u16()?; // LocationLength
                let count = reader.u16()?;
                exception.location =
                    Some(reader.string(u64::from(count), charset, &mut self.had_decode_errors)?);
            }
            if exception.has_override(Ex::BUSY_STATUS_FLAG) {
                exception.busy_status = reader.u32()? as i32;
            }
            if exception.has_override(Ex::ATTACHMENT_FLAG) {
                reader.u32()?;
            }
            if exception.has_override(Ex::SUB_TYPE_FLAG) {
                exception.is_all_day = reader.u32()? != 0;
            }
            if exception.has_override(Ex::APPOINTMENT_COLOR_FLAG) {
                reader.u32()?;
            }
            self.exceptions.push(exception);
        }
        reader.block()?; // ReservedBlock1
        let unicode = crate::charsets::from_codepage(1200).expect("UTF-16LE is supported");
        // ExtendedException (MS-OXOCAL 2.2.1.44.3): one per ExceptionInfo.
        // Unicode subject/location values replace the corresponding 8-bit
        // values, including when the Unicode value is empty.
        for exception in &mut self.exceptions {
            if version >= 0x3009 {
                reader.block()?;
            } // ChangeHighlight
            reader.block()?; // ReservedBlockEE1
            let subject = exception.has_override(TnefRecurrenceException::SUBJECT_FLAG);
            let location = exception.has_override(TnefRecurrenceException::LOCATION_FLAG);
            if !subject && !location {
                continue;
            }
            reader.u32()?; // StartDateTime
            reader.u32()?; // EndDateTime
            reader.u32()?; // OriginalStartDate
            if subject {
                let count = reader.u16()?;
                exception.subject = Some(reader.string(
                    u64::from(count) * 2,
                    unicode,
                    &mut self.had_decode_errors,
                )?);
            }
            if location {
                let count = reader.u16()?;
                exception.location = Some(reader.string(
                    u64::from(count) * 2,
                    unicode,
                    &mut self.had_decode_errors,
                )?);
            }
            reader.block()?; // ReservedBlockEE2
        }
        Ok(())
    }

    /// Returns the recognized raw pattern type.
    pub const fn pattern_type(&self) -> u16 {
        self.pattern_type
    }
    /// Returns the unvalidated calendar identifier.
    pub const fn calendar_type(&self) -> u16 {
        self.calendar_type
    }
    /// Returns raw FirstDateTime, whose interpretation depends on the pattern.
    pub const fn first_date_time(&self) -> u32 {
        self.first_date_time
    }
    /// Returns the unvalidated pattern-specific period.
    pub const fn period(&self) -> u32 {
        self.period
    }
    /// Returns the weekday bitmask; Sunday is bit zero.
    pub const fn day_mask(&self) -> u32 {
        self.day_mask
    }
    /// Returns the day-of-month field, or zero for other pattern layouts.
    pub const fn day_of_month(&self) -> u32 {
        self.day_of_month
    }
    /// Returns the week-of-month field, or zero for other pattern layouts.
    pub const fn week_of_month(&self) -> u32 {
        self.week_of_month
    }
    /// Returns the raw termination type.
    pub const fn end_type(&self) -> u32 {
        self.end_type
    }
    /// Returns the raw occurrence count.
    pub const fn occurrence_count(&self) -> u32 {
        self.occurrence_count
    }
    /// Returns the unvalidated first-day-of-week field.
    pub const fn first_day_of_week(&self) -> u32 {
        self.first_day_of_week
    }
    /// Returns deleted instance dates in local recurrence minutes, in wire order.
    pub fn deleted_instance_dates(&self) -> &[u32] {
        &self.deleted_instance_dates
    }
    /// Returns modified instance dates in local recurrence minutes, in wire order.
    pub fn modified_instance_dates(&self) -> &[u32] {
        &self.modified_instance_dates
    }
    /// Returns the start date in local recurrence minutes.
    pub const fn start_date(&self) -> u32 {
        self.start_date
    }
    /// Returns the raw end date in local recurrence minutes.
    pub const fn end_date(&self) -> u32 {
        self.end_date
    }
    /// Returns the unvalidated start-time offset in minutes.
    pub const fn start_time_offset(&self) -> u32 {
        self.start_time_offset
    }
    /// Returns the unvalidated end-time offset in minutes.
    pub const fn end_time_offset(&self) -> u32 {
        self.end_time_offset
    }
    /// Returns complete merged exceptions, or an empty slice on tail failure.
    pub fn exceptions(&self) -> &[TnefRecurrenceException] {
        &self.exceptions
    }
    /// Returns the explicit error that caused all exceptions to be discarded.
    pub const fn exception_error(&self) -> Option<&TnefRecurrenceError> {
        self.exception_error.as_ref()
    }
    /// Returns whether any decoded string needed replacement, including an
    /// 8-bit value later overwritten by Unicode or a discarded exception.
    pub const fn had_decode_errors(&self) -> bool {
        self.had_decode_errors
    }
}

/// A structural recurrence error. Exception-tail errors are retained on the
/// parsed master rather than returned as a fatal parse failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TnefRecurrenceError {
    /// A requested value/block extends beyond the input.
    UnexpectedEnd {
        /// Absolute byte offset of the unavailable value or block.
        offset: usize,
    },
    /// ReaderVersion is not 0x3004.
    UnsupportedVersion(u16),
    /// The pattern has an unknown binary layout.
    UnknownPattern(u16),
}

impl fmt::Display for TnefRecurrenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd { offset } => {
                write!(f, "unexpected end of recurrence data at byte {offset}")
            }
            Self::UnsupportedVersion(value) => {
                write!(f, "unsupported recurrence pattern version 0x{value:04X}")
            }
            Self::UnknownPattern(value) => {
                write!(f, "unknown recurrence pattern type 0x{value:04X}")
            }
        }
    }
}
impl Error for TnefRecurrenceError {}

/// A bounds-checked cursor over a MAPI binary structure.
struct BlobReader<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> BlobReader<'a> {
    fn take(&mut self, count: u64) -> Result<&'a [u8], TnefRecurrenceError> {
        if count > (self.bytes.len() - self.index) as u64 {
            return Err(TnefRecurrenceError::UnexpectedEnd { offset: self.index });
        }
        let start = self.index;
        self.index += count as usize;
        Ok(&self.bytes[start..self.index])
    }
    fn u16(&mut self) -> Result<u16, TnefRecurrenceError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn u32(&mut self) -> Result<u32, TnefRecurrenceError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn block(&mut self) -> Result<(), TnefRecurrenceError> {
        let count = self.u32()?;
        self.take(u64::from(count))?;
        Ok(())
    }
    fn array(&mut self, count: u32) -> Result<Vec<u32>, TnefRecurrenceError> {
        let bytes = self.take(u64::from(count) * 4)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect())
    }
    fn string(
        &mut self,
        count: u64,
        charset: Charset,
        errors: &mut bool,
    ) -> Result<String, TnefRecurrenceError> {
        let bytes = self.take(count)?;
        let (text, replaced) = charset.decode_lossy(bytes);
        *errors |= replaced;
        Ok(text.trim_end_matches('\0').to_owned())
    }
}

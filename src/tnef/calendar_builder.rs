// Adapted from MimeKit's Tnef/TnefCalendarBuilder.cs and calendar conversion options.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

// Specification map: MS-OXCMAIL 2.1.3.3.8 describes a text/calendar
// alternative for appointments and meeting messages. This module exports the
// TNEF-carried properties under MS-OXCICAL 2.1.3.1.1; MIME restructuring is
// left to the caller. Section suffixes below are relative to that section.
//
// iCalendar             MAPI source                                  Section
// METHOD                MessageClass                                 .16
// VTIMEZONE             AppointmentTimeZoneDefinition*, TimeZoneStruct .19
// ATTENDEE / ORGANIZER  recipient table, RecipientFlags               .20.2/.18
// CATEGORIES            Keywords                                     .20.3
// CLASS                 Sensitivity, Private                         .20.4
// COMMENT / DESCRIPTION Body                                         .20.5/.7
// DTSTART / DTEND       AppointmentStartWhole / EndWhole              .20.8/.10
// DTSTAMP               OwnerCriticalChange / AttendeeCriticalChange .20.9
// EXDATE / RRULE        AppointmentRecur                             .20.11/.21
// LOCATION              Location                                     .20.15
// PRIORITY              Importance                                   .20.19
// RECURRENCE-ID         ExceptionReplaceTime, ExceptionInfo          .20.20
// SEQUENCE              AppointmentSequence                          .20.22
// SUMMARY               NormalizedSubject                            .20.24
// TRANSP                BusyStatus                                   .20.25
// UID                   CleanGlobalObjectId / GlobalObjectId         .20.26
// X-MICROSOFT-CDO-*     busy/importance/intended status, owner ID    .20.31-35
// VALARM                ReminderSet / ReminderDelta                  .20.62
//
// Recurrence exceptions become additional VEVENTs from ExceptionInfo blocks,
// overridden by the exception's embedded Calendar object when present.

use std::{
    borrow::Cow,
    io::{self, Write},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use super::{
    TnefDateTime as Date, TnefDateTimeError, TnefDateTimeKind as Kind, TnefMessage,
    TnefNameId as Name, TnefProperty, TnefPropertySet, TnefPropertyTag as Tag, TnefRecipient,
    TnefRecipientType, TnefRecurrence, TnefRecurrenceException as Exception, TnefSystemTime,
    TnefTimeZone, TnefValue, TnefValueConversionError,
    calendar::{
        CalendarWriter, RecurrenceRuleError, append_text, format_date, format_date_time,
        recurrence_exdates, recurrence_rule,
    },
};
use crate::{MailboxAddress, charsets::Charset, io::MeasuringStream};

const PRODUCT_ID: &str = "-//mimecraft//EN";
// [MS-OXOCAL] 2.2.1.10: asfMeeting.
const APPOINTMENT_STATE_MEETING: i32 = 1;
// [MS-OXOCAL] 2.2.4.10.1: recipOrganizer and recipExceptionalDeleted.
const RECIPIENT_ORGANIZER: i32 = 2;
const RECIPIENT_EXCEPTIONAL_DELETED: i32 = 0x20;
// [MS-OXCMSG] 2.2.2.18: afException, not the similarly named AttachFlags.
const ATTACHMENT_FLAG_EXCEPTION: i32 = 2;
// [MS-OXOCAL] 2.2.1.3: "not set" means the default of 15 minutes.
const REMINDER_DEFAULT: i32 = 0x5ae980e1;
// [MS-OXOCAL] 2.2.1.27: signature of an imported iCalendar UID.
const VCAL_UID: &[u8] = b"vCal-Uid\x01\0\0\0";
const BUSY_NAMES: [&str; 5] = ["FREE", "TENTATIVE", "BUSY", "OOF", "WORKINGELSEWHERE"];

/// Limits and policy for exporting a TNEF appointment as iCalendar.
#[derive(Clone, Debug)]
pub struct CalendarOptions {
    /// Disable generation without reading appointment properties.
    pub generate_calendar: bool,
    /// Charset of legacy recurrence strings and bodies without a known charset.
    pub fallback_charset: Charset,
    /// Maximum modified occurrences, excluding the master (default 1024).
    /// Zero exports the master only; omitted exceptions are reported.
    pub max_exceptions: usize,
    /// Maximum combined serialized exception VEVENT bytes (default 16 MiB).
    /// Includes UTF-8 encoding, escaping, folding and CRLF. The master and
    /// VTIMEZONEs are not counted. Stops at the first exception that cannot fit.
    pub max_exceptions_size: u64,
    /// Replaces the current UTC time only when no source DTSTAMP exists.
    /// Coordinates are interpreted as UTC. Useful for reproducible exports.
    pub fallback_stamp: Option<Date>,
}

impl Default for CalendarOptions {
    fn default() -> Self {
        Self {
            generate_calendar: true,
            fallback_charset: Charset::from_label("windows-1252").expect("built-in charset"),
            max_exceptions: 1024,
            max_exceptions_size: 16 * 1024 * 1024,
            fallback_stamp: None,
        }
    }
}

/// Category of information lost while exporting a calendar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarLossKind {
    /// Missing, malformed, undecodable, or out-of-range calendar data.
    InvalidCalendarData,
    /// Recurrence uses an unsupported calendar or pattern.
    UnsupportedCalendarData,
    /// The count or serialized-byte budget omitted modified occurrences.
    CalendarExceptionLimitExceeded,
}

/// An explicit recovery diagnostic. No diagnostic is silently discarded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarLoss {
    /// Machine-readable recovery category.
    pub kind: CalendarLossKind,
    /// Explanation of the omitted or substituted information.
    pub description: String,
    /// Original attachment index when the diagnostic concerns an embedded object.
    pub attachment_index: Option<usize>,
}

/// Calendar export plus diagnostics, including when no calendar could be built.
///
/// Non-calendar messages and disabled generation produce `None` without losses.
/// Malformed calendar data is reported here, not mistaken for a non-calendar
/// message. The original message and all attachment content remain unchanged.
#[derive(Clone, Debug)]
#[must_use = "inspect calendar conversion losses before using the output"]
pub struct CalendarBuildResult {
    /// Complete calendar, or no calendar when generation was skipped or failed.
    pub calendar: Option<Calendar>,
    /// Recoveries in encounter order.
    pub losses: Vec<CalendarLoss>,
}

/// An owned, repeatably writable VCALENDAR exported from a TNEF message.
///
/// Implements the export mappings in MS-OXCICAL 2.1.3.1.1: METHOD,
/// VTIMEZONE, attendees/organizer, categories/class/body, dates/stamp,
/// recurrence rules/exclusions/overrides, priority/sequence/summary/UID,
/// availability, Microsoft extensions, and display alarms.
///
/// This is an export snapshot, not an editable general-purpose calendar model.
/// Exception attachment indices refer to the original message's attachment
/// order; only successfully emitted overrides consume attachments. A MIME
/// converter must retain every other attachment.
#[derive(Clone, Debug)]
pub struct Calendar {
    method: &'static str,
    zones: Vec<RegisteredZone>,
    events: Vec<Event>,
    exception_attachments: Vec<usize>,
    exceptions_size: u64,
}

impl Calendar {
    /// METHOD, also suitable for the text/calendar Content-Type method parameter.
    pub fn method(&self) -> &str {
        self.method
    }

    /// Number of VEVENTs, including the master.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Attachment indices consumed by exported exceptions, in occurrence order.
    pub fn exception_attachment_indices(&self) -> &[usize] {
        &self.exception_attachments
    }

    /// Exact serialized byte total of exception VEVENTs, excluding the master.
    pub fn exceptions_size(&self) -> u64 {
        self.exceptions_size
    }

    /// Creates an owned UTF-8 `text/calendar` part with the METHOD parameter.
    ///
    /// This does not restructure a MIME message or remove TNEF attachments.
    /// Use [`Self::exception_attachment_indices`] when integrating the part.
    ///
    /// # Errors
    ///
    /// Returns serialization, content-type, or header-format errors.
    pub fn to_text_part(&self) -> Result<crate::TextPart, crate::TextPartError> {
        use crate::{ContentType, MimeContent, MimePart, TextPart, TextPartError};
        let mut bytes = Vec::new();
        self.write_to(&mut bytes).map_err(TextPartError::Io)?;
        let mut content_type =
            ContentType::new("text", "calendar").map_err(TextPartError::ContentType)?;
        content_type.set_charset(Some("utf-8"));
        content_type
            .parameters_mut()
            .set("method", self.method)
            .expect("method is a valid parameter name");
        let part = MimePart::with_content_type(
            &content_type,
            MimeContent::new(std::io::Cursor::new(bytes)),
        )
        .map_err(TextPartError::HeaderFormat)?;
        TextPart::from_part(part)
    }

    /// Writes UTF-8 with RFC 5545 folding and CRLF, then flushes the sink.
    ///
    /// # Errors
    ///
    /// Propagates all sink errors, including final flush errors. A failed sink
    /// may contain a prefix; the immutable calendar can be written again.
    pub fn write_to<W: Write>(&self, sink: W) -> io::Result<()> {
        let mut writer = CalendarWriter::new(sink);
        writer.begin_component("VCALENDAR")?;
        writer.write_property("METHOD", self.method)?;
        writer.write_text_property("PRODID", PRODUCT_ID)?;
        writer.write_property("VERSION", "2.0")?;
        writer.write_property("X-MS-OLK-FORCEINSPECTOROPEN", "TRUE")?;
        for zone in &self.zones {
            zone.zone.write_to(&mut writer)?;
        }
        for event in &self.events {
            self.write_event(&mut writer, event)?;
        }
        writer.end_component("VCALENDAR")?;
        writer.finish()
    }

    fn is_reply(&self) -> bool {
        matches!(self.method, "REPLY" | "COUNTER")
    }

    fn register(&mut self, zone: &TnefTimeZone) -> usize {
        if let Some(index) = self
            .zones
            .iter()
            .position(|z| z.original_id == zone.id() && z.zone.has_same_rules(zone))
        {
            return index;
        }
        // Two different zones with the same name need different TZIDs.
        let mut registered = zone.clone();
        let mut suffix = 2;
        while self.zones.iter().any(|z| z.zone.id() == registered.id()) {
            registered.set_id(format!("{} ({suffix})", zone.id()));
            suffix += 1;
        }
        self.zones.push(RegisteredZone {
            original_id: zone.id().to_owned(),
            zone: registered,
        });
        self.zones.len() - 1
    }

    fn utc_time(
        &mut self,
        utc: Date,
        zone: Option<&TnefTimeZone>,
        all_day: bool,
    ) -> Result<CalendarTime, TnefDateTimeError> {
        if all_day {
            // An all-day appointment is midnight in the organizer's time zone.
            // Without a zone, the nearest midnight is the best estimate.
            let local = match zone {
                Some(zone) => zone.to_local_time(utc)?,
                None => utc.checked_add_minutes(720)?,
            };
            return Ok(CalendarTime {
                value: midnight(local),
                zone: None,
                is_date: true,
            });
        }
        match zone {
            Some(zone) => Ok(CalendarTime {
                value: zone.to_local_time(utc)?,
                zone: Some(self.register(zone)),
                is_date: false,
            }),
            None => Ok(CalendarTime {
                value: utc.with_kind(Kind::Utc),
                zone: None,
                is_date: false,
            }),
        }
    }

    fn local_time(&mut self, local: Date, zone: &TnefTimeZone, all_day: bool) -> CalendarTime {
        CalendarTime {
            value: if all_day {
                midnight(local)
            } else {
                local.with_kind(Kind::Unspecified)
            },
            zone: if all_day {
                None
            } else {
                Some(self.register(zone))
            },
            is_date: all_day,
        }
    }

    fn write_time<W: Write>(
        &self,
        writer: &mut CalendarWriter<W>,
        name: &str,
        time: CalendarTime,
    ) -> io::Result<()> {
        writer.begin_property(name)?;
        if time.is_date {
            writer.write_parameter("VALUE", "DATE")?;
            writer.write_value(&format_date(time.value))
        } else if let Some(index) = time.zone {
            writer.write_parameter("TZID", self.zones[index].zone.id())?;
            writer.write_value(&format_date_time(time.value, false))
        } else {
            writer.write_value(&format_date_time(time.value, true))
        }
    }

    fn write_event<W: Write>(&self, w: &mut CalendarWriter<W>, e: &Event) -> io::Result<()> {
        w.begin_component("VEVENT")?;
        for address in e.attendees.iter() {
            write_address(w, "ATTENDEE", address)?;
        }
        let mut categories = String::new();
        for category in e.categories.iter().filter(|s| !s.is_empty()) {
            if !categories.is_empty() {
                categories.push(',');
            }
            append_text(&mut categories, category);
        }
        if !categories.is_empty() {
            w.write_property("CATEGORIES", &categories)?;
        }
        if let Some(class) = e.class {
            w.write_property("CLASS", class)?;
        }
        // MS-OXCICAL 2.1.3.1.1.20.5/.7: a response body is COMMENT,
        // otherwise DESCRIPTION.
        if self.is_reply()
            && let Some(text) = &e.description
        {
            w.write_text_property("COMMENT", text)?;
        }
        if let Some(time) = e.created {
            w.write_property("CREATED", &format_date_time(time, true))?;
        }
        if !self.is_reply()
            && let Some(text) = &e.description
        {
            w.write_text_property("DESCRIPTION", text)?;
        }
        if let Some(time) = e.end {
            self.write_time(w, "DTEND", time)?;
        }
        w.write_property("DTSTAMP", &format_date_time(e.stamp, true))?;
        self.write_time(w, "DTSTART", e.start)?;
        // Each EXDATE has its own line: values on one line must share a TZID.
        for &time in &e.exdates {
            self.write_time(w, "EXDATE", time)?;
        }
        if let Some(time) = e.last_modified {
            w.write_property("LAST-MODIFIED", &format_date_time(time, true))?;
        }
        if let Some(text) = &e.location
            && !text.is_empty()
        {
            w.write_text_property("LOCATION", text)?;
        }
        if let Some(address) = &e.organizer {
            write_address(w, "ORGANIZER", address)?;
        }
        if let Some(priority) = e.priority {
            w.write_property("PRIORITY", &priority.to_string())?;
        }
        if let Some(time) = e.recurrence_id {
            self.write_time(w, "RECURRENCE-ID", time)?;
        }
        if let Some(rule) = &e.rule {
            w.write_property("RRULE", rule)?;
        }
        w.write_property("SEQUENCE", &e.sequence.to_string())?;
        if self.method == "CANCEL" {
            w.write_property("STATUS", "CANCELLED")?;
        }
        w.write_text_property("SUMMARY", &e.summary)?;
        // MS-OXCICAL 2.1.3.1.1.20.25: only a free appointment is transparent.
        if let Some(busy) = e.busy {
            w.write_property("TRANSP", if busy == 0 { "TRANSPARENT" } else { "OPAQUE" })?;
        }
        w.write_text_property("UID", &e.uid)?;
        if let Some(status) = e.busy.filter(|s| (0..5).contains(s)) {
            w.write_property("X-MICROSOFT-CDO-BUSYSTATUS", BUSY_NAMES[status as usize])?;
        }
        if let Some(importance) = e.importance {
            w.write_property("X-MICROSOFT-CDO-IMPORTANCE", &importance.to_string())?;
        }
        if let Some(status) = e.intended_busy.filter(|s| (0..5).contains(s)) {
            w.write_property(
                "X-MICROSOFT-CDO-INTENDEDSTATUS",
                BUSY_NAMES[status as usize],
            )?;
        }
        if let Some(owner) = e.owner {
            w.write_property("X-MICROSOFT-CDO-OWNERAPPTID", &owner.to_string())?;
        }
        if let Some(value) = e.disallow_counter {
            w.write_property(
                "X-MICROSOFT-DISALLOW-COUNTER",
                if value { "TRUE" } else { "FALSE" },
            )?;
        }
        if let Some(time) = e.original_end {
            self.write_time(w, "X-MS-OLK-ORIGINALEND", time)?;
        }
        if let Some(time) = e.original_start {
            self.write_time(w, "X-MS-OLK-ORIGINALSTART", time)?;
        }
        if let Some(minutes) = e.reminder {
            w.begin_component("VALARM")?;
            w.write_property("ACTION", "DISPLAY")?;
            w.write_property("DESCRIPTION", "Reminder")?;
            w.write_property(
                "TRIGGER",
                &if minutes == 0 {
                    "PT0M".into()
                } else {
                    format!("-PT{minutes}M")
                },
            )?;
            w.end_component("VALARM")?;
        }
        w.end_component("VEVENT")
    }
}

#[derive(Clone, Copy, Debug)]
struct CalendarTime {
    value: Date,
    zone: Option<usize>,
    is_date: bool,
}

#[derive(Clone, Debug)]
struct RegisteredZone {
    original_id: String,
    zone: TnefTimeZone,
}

#[derive(Clone, Debug, Default)]
struct Address {
    address: Arc<str>,
    name: Option<Arc<str>>,
    cu_type: Option<&'static str>,
    role: Option<&'static str>,
    part_stat: Option<&'static str>,
    rsvp: Option<bool>,
}

#[derive(Clone, Debug)]
struct Event {
    // Exceptions share immutable master text/recipients instead of multiplying
    // a large description in memory before the serialized-size check.
    attendees: Arc<[Address]>,
    organizer: Option<Address>,
    categories: Arc<[Arc<str>]>,
    class: Option<&'static str>,
    description: Option<Arc<str>>,
    created: Option<Date>,
    end: Option<CalendarTime>,
    stamp: Date,
    start: CalendarTime,
    exdates: Vec<CalendarTime>,
    last_modified: Option<Date>,
    location: Option<Arc<str>>,
    priority: Option<i32>,
    recurrence_id: Option<CalendarTime>,
    rule: Option<String>,
    sequence: i32,
    summary: Arc<str>,
    uid: Arc<str>,
    busy: Option<i32>,
    importance: Option<i32>,
    intended_busy: Option<i32>,
    owner: Option<i32>,
    disallow_counter: Option<bool>,
    original_start: Option<CalendarTime>,
    original_end: Option<CalendarTime>,
    reminder: Option<i32>,
}

impl Event {
    fn clone_for_exception(&self) -> Self {
        // Do not clone an arbitrarily large EXDATE list only to discard it.
        Self {
            attendees: self.attendees.clone(),
            organizer: self.organizer.clone(),
            categories: self.categories.clone(),
            class: self.class,
            description: self.description.clone(),
            created: self.created,
            end: self.end,
            stamp: self.stamp,
            start: self.start,
            exdates: Vec::new(),
            last_modified: self.last_modified,
            location: self.location.clone(),
            priority: self.priority,
            recurrence_id: self.recurrence_id,
            rule: None,
            sequence: self.sequence,
            summary: self.summary.clone(),
            uid: self.uid.clone(),
            busy: self.busy,
            importance: self.importance,
            intended_busy: self.intended_busy,
            owner: self.owner,
            disallow_counter: self.disallow_counter,
            original_start: None,
            original_end: None,
            reminder: self.reminder,
        }
    }
}

#[derive(Default)]
struct Losses {
    values: Vec<CalendarLoss>,
    attachment: Option<usize>,
}
impl Losses {
    fn add(&mut self, kind: CalendarLossKind, description: impl Into<String>) {
        self.values.push(CalendarLoss {
            kind,
            description: description.into(),
            attachment_index: self.attachment,
        });
    }
    fn invalid(&mut self, description: impl Into<String>) {
        self.add(CalendarLossKind::InvalidCalendarData, description);
    }
}

enum Key {
    Tag(Tag),
    Name(Name),
}
impl From<Tag> for Key {
    fn from(tag: Tag) -> Self {
        Self::Tag(tag)
    }
}
impl From<Name> for Key {
    fn from(name: Name) -> Self {
        Self::Name(name)
    }
}

struct Properties<'a, 'b> {
    set: &'a TnefPropertySet,
    losses: &'b mut Losses,
}
impl<'a> Properties<'a, '_> {
    fn read<T>(
        &mut self,
        key: impl Into<Key>,
        convert: impl FnOnce(&'a TnefProperty) -> Result<T, TnefValueConversionError>,
    ) -> Option<T> {
        let property = match key.into() {
            Key::Tag(tag) => self.set.get(tag),
            Key::Name(name) => self.set.get_named(&name),
        }?;
        match convert(property) {
            Ok(value) => Some(value),
            Err(error) => {
                self.losses.invalid(format!(
                    "Calendar property {:?} ({:?}) was ignored: {error}",
                    property.tag(),
                    property.name()
                ));
                None
            }
        }
    }
    fn boolean(&mut self, key: impl Into<Key>) -> Option<bool> {
        self.read(key, TnefProperty::to_bool)
    }
    fn integer(&mut self, key: impl Into<Key>) -> Option<i32> {
        self.read(key, TnefProperty::to_i32)
    }
    fn date(&mut self, key: impl Into<Key>) -> Option<Date> {
        self.read(key, TnefProperty::to_datetime)
            .map(|d| d.with_kind(Kind::Utc))
    }
    fn text(&mut self, key: impl Into<Key>) -> Option<Arc<str>> {
        self.read(key, TnefProperty::to_text)
            .map(|s| Arc::from(s.trim_end_matches('\0')))
    }
    fn bytes(&mut self, key: impl Into<Key>) -> Option<&'a [u8]> {
        self.read(key, TnefProperty::as_bytes)
            .filter(|s| !s.is_empty())
    }
}

/// Exports all supported calendar data, including bounded modified occurrences.
///
/// Malformed source data is reported in the result's losses. An unrepresentable
/// master date prevents calendar creation; no partially built calendar or
/// consumed attachment indices escape. Invalid recurrence patterns instead
/// preserve the single master event.
///
/// ```
/// use mimecraft::tnef::{TnefMessage, calendar::{build_calendar, CalendarOptions}};
///
/// # fn export(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
/// let message = TnefMessage::load(bytes)?;
/// let result = build_calendar(&message, &CalendarOptions::default())?;
/// for loss in &result.losses {
///     eprintln!("{:?}: {}", loss.kind, loss.description);
/// }
/// if let Some(calendar) = result.calendar {
///     let mut output = Vec::new();
///     calendar.write_to(&mut output)?;
///     let calendar_part = calendar.to_text_part()?;
///     // Only these original attachments are represented by exported overrides.
///     let consumed = calendar.exception_attachment_indices();
/// #   let _ = (calendar_part, consumed);
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns measurement I/O or system-clock errors. Source calendar errors are
/// recoveries in [`CalendarBuildResult::losses`].
pub fn build_calendar(
    message: &TnefMessage,
    options: &CalendarOptions,
) -> io::Result<CalendarBuildResult> {
    build_calendar_with_check(message, options, || Ok(()))
}

/// Exports a calendar with cooperative cancellation.
///
/// `check` runs before building and before each embedded attachment and modified
/// occurrence. Return an error to abort without publishing a partial result.
/// Serialization can independently be cancelled by a fallible output sink.
///
/// # Errors
///
/// Propagates cancellation and [`build_calendar`] errors unchanged.
pub fn build_calendar_with_check(
    message: &TnefMessage,
    options: &CalendarOptions,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<CalendarBuildResult> {
    check()?;
    let mut losses = Losses::default();
    if !options.generate_calendar {
        return Ok(CalendarBuildResult {
            calendar: None,
            losses: losses.values,
        });
    }
    let mut properties = Properties {
        set: message.properties(),
        losses: &mut losses,
    };
    let class = properties.text(Tag::MESSAGE_CLASS_W);
    let method = class
        .as_deref()
        .and_then(|class| method(class.trim(), &mut properties));
    let Some(method) = method else {
        return Ok(CalendarBuildResult {
            calendar: None,
            losses: losses.values,
        });
    };
    let Some(start) = properties.date(Name::APPOINTMENT_START_WHOLE) else {
        losses.invalid("The calendar item has no valid start time; no calendar was generated.");
        return Ok(CalendarBuildResult {
            calendar: None,
            losses: losses.values,
        });
    };
    let mut calendar = Calendar {
        method,
        zones: Vec::new(),
        events: Vec::new(),
        exception_attachments: Vec::new(),
        exceptions_size: 0,
    };
    let built = build_master(&mut calendar, message, options, start, &mut losses)?;
    match built {
        Ok((mut master, recurrence)) => {
            if let Some((recurrence, zone, all_day)) = recurrence
                && let Err(error) = add_recurrence(
                    &mut calendar,
                    &mut master,
                    message,
                    &recurrence,
                    &zone,
                    all_day,
                    options,
                    &mut losses,
                    &mut check,
                )?
            {
                losses.invalid(format!(
                    "Calendar date conversion failed; no calendar was generated: {error}"
                ));
                return Ok(CalendarBuildResult {
                    calendar: None,
                    losses: losses.values,
                });
            }
            calendar.events.insert(0, master);
            Ok(CalendarBuildResult {
                calendar: Some(calendar),
                losses: losses.values,
            })
        }
        Err(error) => {
            losses.invalid(format!(
                "Calendar date conversion failed; no calendar was generated: {error}"
            ));
            Ok(CalendarBuildResult {
                calendar: None,
                losses: losses.values,
            })
        }
    }
}

fn is_class(class: &str, expected: &str) -> bool {
    class
        .get(..expected.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(expected))
        && (class.len() == expected.len() || class.as_bytes().get(expected.len()) == Some(&b'.'))
}

fn method(class: &str, p: &mut Properties<'_, '_>) -> Option<&'static str> {
    // MS-OXCICAL 2.1.3.1.1.16: METHOD depends on the message class.
    for (prefix, method) in [
        ("IPM.Appointment", "PUBLISH"),
        ("IPM.Schedule.Meeting.Request", "REQUEST"),
        ("IPM.Schedule.Meeting.Canceled", "CANCEL"),
        ("IPM.Schedule.Meeting.Resp.Pos", "REPLY"),
        ("IPM.Schedule.Meeting.Resp.Neg", "REPLY"),
    ] {
        if is_class(class, prefix) {
            return Some(method);
        }
    }
    if is_class(class, "IPM.Schedule.Meeting.Resp.Tent") {
        // A tentative response proposing a new time is a counter proposal.
        return Some(
            if p.boolean(Name::APPOINTMENT_COUNTER_PROPOSAL) == Some(true) {
                "COUNTER"
            } else {
                "REPLY"
            },
        );
    }
    None
}

fn midnight(date: Date) -> Date {
    Date::from_ticks(
        date.ticks() / 864_000_000_000 * 864_000_000_000,
        date.kind(),
    )
    .expect("rounding down a valid date preserves its range")
}

fn subject(message: &TnefMessage, losses: &mut Losses) -> Option<Arc<str>> {
    // MS-OXCICAL 2.1.3.1.1.20.24: prefer NormalizedSubject to Subject.
    let mut p = Properties {
        set: message.properties(),
        losses,
    };
    p.text(Tag::NORMALIZED_SUBJECT_W)
        .or_else(|| p.text(Tag::SUBJECT_W))
}

fn body(message: &TnefMessage, options: &CalendarOptions, losses: &mut Losses) -> Option<Arc<str>> {
    let body = message.text_body()?;
    let charset = body.charset().unwrap_or(options.fallback_charset);
    let bytes = body.as_bytes();
    // StreamReader removes the selected encoding's preamble even with BOM
    // detection disabled. Do not switch encodings based on a different BOM.
    let bytes = if crate::charsets::bom_charset(bytes) == Some(charset) {
        &bytes[if charset == Charset::UTF8 { 3 } else { 2 }..]
    } else {
        bytes
    };
    let (text, replaced) = charset.decode_lossy(bytes);
    if replaced {
        losses.invalid("Invalid calendar body text was decoded with replacement characters.");
    }
    let text = text.trim_end_matches(['\0', '\r', '\n', ' ', '\t']);
    (!text.is_empty()).then(|| Arc::from(text))
}

fn recurrence_zone(p: &mut Properties<'_, '_>) -> Option<TnefTimeZone> {
    let description = p.text(Name::TIME_ZONE_DESCRIPTION);
    let description = description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // MS-OXCICAL 2.1.3.1.1.19: prefer the recurrence TZDefinition, falling
    // back to TimeZoneStruct and its description.
    if let Some(bytes) = p.bytes(Name::APPOINTMENT_TIME_ZONE_DEFINITION_RECUR) {
        match TnefTimeZone::from_time_zone_definition(bytes, description) {
            Ok(zone) => return Some(zone),
            Err(error) => p.losses.invalid(format!(
                "Invalid recurrence time-zone definition; trying the legacy structure: {error}"
            )),
        }
    }
    if let Some(bytes) = p.bytes(Name::TIME_ZONE_STRUCT) {
        match TnefTimeZone::from_time_zone_struct(bytes, description.unwrap_or("")) {
            Ok(mut zone) => {
                if description.is_none() {
                    let offset = zone.standard_offset();
                    zone.set_id(format!(
                        "UTC{}{:02}:{:02}",
                        if offset < 0 { '-' } else { '+' },
                        offset.abs() / 60,
                        offset.abs() % 60
                    ));
                }
                return Some(zone);
            }
            Err(error) => p
                .losses
                .invalid(format!("Invalid legacy recurrence time zone: {error}")),
        }
    }
    None
}

fn display_zone(p: &mut Properties<'_, '_>, name: Name) -> Option<TnefTimeZone> {
    let bytes = p.bytes(name)?;
    match TnefTimeZone::from_time_zone_definition(bytes, None) {
        Ok(zone) => Some(zone),
        Err(error) => {
            p.losses
                .invalid(format!("Invalid display time zone was ignored: {error}"));
            None
        }
    }
}

fn normalize_reminder(minutes: i32) -> i32 {
    if minutes == REMINDER_DEFAULT {
        15
    } else {
        minutes.max(0)
    }
}
fn reminder(p: &mut Properties<'_, '_>) -> Option<i32> {
    // MS-OXCICAL 2.1.3.1.1.20.62: export VALARM only when ReminderSet is true.
    (p.boolean(Name::REMINDER_SET) == Some(true))
        .then(|| normalize_reminder(p.integer(Name::REMINDER_DELTA).unwrap_or(REMINDER_DEFAULT)))
}

fn uid(p: &mut Properties<'_, '_>) -> Option<Arc<str>> {
    let id = p
        .bytes(Name::CLEAN_GLOBAL_OBJECT_ID)
        .or_else(|| p.bytes(Name::GLOBAL_OBJECT_ID))?;
    // Byte Array ID (16), YH/YL/M/D (4), Creation Time (8), X (8), Size (4), Data.
    if id.len() < 40 {
        return None;
    }
    let size = i32::from_le_bytes(id[36..40].try_into().expect("checked header"));
    if let Ok(size) = usize::try_from(size)
        && size >= VCAL_UID.len()
        && size <= id.len() - 40
        && id[40..].starts_with(VCAL_UID)
    {
        // MS-OXCICAL 2.1.3.1.1.20.26: preserve an imported iCalendar UID.
        let text = String::from_utf8_lossy(&id[40 + VCAL_UID.len()..40 + size]);
        if matches!(text, Cow::Owned(_)) {
            p.losses
                .invalid("Invalid UTF-8 in the imported calendar UID was replaced.");
        }
        let text = text.trim_end_matches('\0');
        if !text.is_empty() {
            return Some(Arc::from(text));
        }
    }
    // Otherwise use the hex GlobalObjectId with the instance date cleared.
    let mut text = String::with_capacity(id.len() * 2);
    const HEX: &[u8] = b"0123456789ABCDEF";
    for (index, &byte) in id.iter().enumerate() {
        let byte = if (16..20).contains(&index) { 0 } else { byte };
        text.push(HEX[usize::from(byte >> 4)] as char);
        text.push(HEX[usize::from(byte & 15)] as char);
    }
    Some(text.into())
}

fn create_uid(
    message: &TnefMessage,
    start: Date,
    end: Option<Date>,
    losses: &mut Losses,
) -> Arc<str> {
    // Derive a stable UID from identifying properties so clients match updates.
    let subject = subject(message, losses);
    let mut p = Properties {
        set: message.properties(),
        losses,
    };
    let created = p.date(Tag::CREATION_TIME);
    let sender = p
        .text(Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W)
        .or_else(|| p.text(Tag::SENDER_EMAIL_ADDRESS_W));
    let seed = format!(
        "{}\n{}\n{}\n{}\n{}",
        subject.as_deref().unwrap_or(""),
        start.ticks(),
        end.map(|d| d.ticks().to_string()).unwrap_or_default(),
        created.map(|d| d.ticks().to_string()).unwrap_or_default(),
        sender.as_deref().unwrap_or("")
    );
    let mut hash = Sha256::digest(seed.as_bytes());
    // Mark a name-based (version-5-style) UUID. MimeKit uses SHA-256, not the
    // RFC's SHA-1, and .NET Guid's first three fields use Windows byte order.
    hash[7] = (hash[7] & 15) | 0x50;
    hash[8] = (hash[8] & 63) | 0x80;
    format!("{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        hash[3], hash[2], hash[1], hash[0], hash[5], hash[4], hash[7], hash[6],
        hash[8], hash[9], hash[10], hash[11], hash[12], hash[13], hash[14], hash[15]).into()
}

fn now() -> io::Result<Date> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    let ticks = elapsed.as_nanos() / 100 + 621_355_968_000_000_000;
    let ticks = u64::try_from(ticks).map_err(io::Error::other)?;
    Date::from_ticks(ticks, Kind::Utc).map_err(io::Error::other)
}

type Master = (Event, Option<(TnefRecurrence, TnefTimeZone, bool)>);
fn build_master(
    calendar: &mut Calendar,
    message: &TnefMessage,
    options: &CalendarOptions,
    mut start: Date,
    losses: &mut Losses,
) -> io::Result<Result<Master, TnefDateTimeError>> {
    let mut p = Properties {
        set: message.properties(),
        losses,
    };
    let mut end = p.date(Name::APPOINTMENT_END_WHOLE);
    if end.is_none()
        && let Some(duration) = p.integer(Name::APPOINTMENT_DURATION).filter(|&d| d >= 0)
    {
        match start.checked_add_minutes(duration) {
            Ok(date) => end = Some(date),
            Err(e) => return Ok(Err(e)),
        }
    }
    let all_day = p.boolean(Name::APPOINTMENT_SUB_TYPE) == Some(true);
    let mut recurrence = None;
    if let Some(bytes) = p.bytes(Name::APPOINTMENT_RECUR)
        && p.boolean(Name::RECURRING) != Some(false)
    {
        match TnefRecurrence::parse(bytes, options.fallback_charset) {
            Ok(parsed) => {
                if parsed.had_decode_errors() { p.losses.invalid("Invalid recurrence text was decoded with replacement characters."); }
                recurrence = Some(parsed);
            }
            Err(error) => p.losses.invalid(format!("The recurrence pattern could not be parsed; a single occurrence was exported: {error}")),
        }
    }
    let id = uid(&mut p).unwrap_or_else(|| create_uid(message, start, end, p.losses));
    let summary = subject(message, p.losses).unwrap_or_else(|| Arc::from(""));
    let description = body(message, options, p.losses);
    // MS-OXCICAL 2.1.3.1.1.20.9: the sender's last critical change.
    let stamp = p
        .date(if calendar.is_reply() {
            Name::ATTENDEE_CRITICAL_CHANGE
        } else {
            Name::OWNER_CRITICAL_CHANGE
        })
        .or_else(|| p.date(Tag::CLIENT_SUBMIT_TIME))
        .or_else(|| p.date(Tag::LAST_MODIFICATION_TIME));
    let stamp = match stamp.or(options.fallback_stamp) {
        Some(stamp) => stamp,
        None => now()?,
    };
    let mut event = Event {
        uid: id,
        summary,
        description,
        location: p.text(Name::LOCATION),
        sequence: p.integer(Name::APPOINTMENT_SEQUENCE).unwrap_or(0),
        created: p.date(Tag::CREATION_TIME),
        last_modified: p.date(Tag::LAST_MODIFICATION_TIME),
        stamp,
        busy: p.integer(Name::BUSY_STATUS),
        owner: p.integer(Tag::OWNER_APPT_ID),
        disallow_counter: p.boolean(Name::APPOINTMENT_NOT_ALLOW_PROPOSE),
        intended_busy: if calendar.method == "REQUEST" {
            p.integer(Name::INTENDED_BUSY_STATUS)
        } else {
            None
        },
        reminder: reminder(&mut p),
        attendees: Arc::from([]),
        organizer: None,
        categories: Arc::from([]),
        class: None,
        end: None,
        start: CalendarTime {
            value: start,
            zone: None,
            is_date: false,
        },
        exdates: Vec::new(),
        priority: None,
        recurrence_id: None,
        rule: None,
        importance: None,
        original_start: None,
        original_end: None,
    };
    if let Some(property) = p.set.get_named(&Name::keywords()) {
        let mut categories = Vec::new();
        for value in property.values() {
            if let TnefValue::Text(text) = value {
                categories.push(Arc::from(text.as_str()));
            } else {
                p.losses
                    .invalid("Non-text calendar categories were ignored.");
                categories.clear();
                break;
            }
        }
        event.categories = categories.into();
    }
    // MS-OXCICAL 2.1.3.1.1.20.4: Sensitivity, or Private when absent.
    event.class = match p.integer(Tag::SENSITIVITY) {
        Some(0) => Some("PUBLIC"),
        Some(1) => Some("X-PERSONAL"),
        Some(2) => Some("PRIVATE"),
        Some(3) => Some("CONFIDENTIAL"),
        None if p.boolean(Name::PRIVATE) == Some(true) => Some("PRIVATE"),
        _ => None,
    };
    // MS-OXCICAL 2.1.3.1.1.20.19: PRIORITY from Importance.
    event.importance = p.integer(Tag::IMPORTANCE).filter(|i| (0..=2).contains(i));
    event.priority = event.importance.map(|i| [9, 5, 1][i as usize]);
    let dates = (|| {
        let recurrence = if let Some(recurrence) = recurrence {
            let zone = recurrence_zone(&mut p).unwrap_or_else(|| {
                // A local recurrence cannot be interpreted without a time zone.
                p.losses
                    .invalid("The recurring appointment has no valid time zone; UTC was assumed.");
                TnefTimeZone::new(
                    "UTC",
                    0,
                    0,
                    0,
                    TnefSystemTime::default(),
                    TnefSystemTime::default(),
                )
                .expect("UTC")
            });
            event.start = calendar.utc_time(start, Some(&zone), all_day)?;
            event.end = end
                .map(|d| calendar.utc_time(d, Some(&zone), all_day))
                .transpose()?;
            Some((recurrence, zone, all_day))
        } else {
            // A single appointment uses the time zones in which it was created.
            let start_zone =
                display_zone(&mut p, Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY);
            let end_zone = display_zone(&mut p, Name::APPOINTMENT_TIME_ZONE_DEFINITION_END_DISPLAY);
            let end_zone = end_zone.as_ref().or(start_zone.as_ref());
            if calendar.method == "COUNTER" {
                // Proposed times become DTSTART/DTEND; preserve originals in
                // X-MS-OLK-ORIGINALSTART/END (MS-OXCICAL .20.8/.20.10).
                let proposed_start = p.date(Name::APPOINTMENT_PROPOSED_START_WHOLE);
                let proposed_end = p.date(Name::APPOINTMENT_PROPOSED_END_WHOLE);
                if let Some(proposed) = proposed_start {
                    event.original_start =
                        Some(calendar.utc_time(start, start_zone.as_ref(), all_day)?);
                    event.original_end = end
                        .map(|d| calendar.utc_time(d, end_zone, all_day))
                        .transpose()?;
                    start = proposed;
                    end = proposed_end.or(end);
                }
            }
            event.start = calendar.utc_time(start, start_zone.as_ref(), all_day)?;
            event.end = end
                .map(|d| calendar.utc_time(d, end_zone, all_day))
                .transpose()?;
            event.recurrence_id = recurrence_id(calendar, &mut p, start_zone.as_ref(), all_day)?;
            None
        };
        Ok(recurrence)
    })();
    let recurrence = match dates {
        Ok(value) => value,
        Err(error) => return Ok(Err(error)),
    };
    add_attendees(calendar, &mut event, message, losses);
    Ok(Ok((event, recurrence)))
}

fn recurrence_id(
    calendar: &mut Calendar,
    p: &mut Properties<'_, '_>,
    start_zone: Option<&TnefTimeZone>,
    all_day: bool,
) -> Result<Option<CalendarTime>, TnefDateTimeError> {
    // An individual meeting instance identifies its original start in the
    // series' time zone (MS-OXCICAL 2.1.3.1.1.20.20).
    let replace = p.date(Name::EXCEPTION_REPLACE_TIME);
    if replace.is_none() && p.boolean(Name::IS_EXCEPTION) != Some(true) {
        return Ok(None);
    }
    let zone = recurrence_zone(p);
    let zone = zone.as_ref().or(start_zone);
    if let Some(replace) = replace {
        return calendar.utc_time(replace, zone, all_day).map(Some);
    }
    // Fall back to the GlobalObjectId instance date and StartRecurrenceTime.
    let Some(id) = p.bytes(Name::GLOBAL_OBJECT_ID) else {
        return Ok(None);
    };
    if id.len() < 20 {
        p.losses
            .invalid("GlobalObjectId is too short for RECURRENCE-ID.");
        return Ok(None);
    }
    let year = u16::from_be_bytes([id[16], id[17]]);
    let date = Date::new(year, id[18], id[19], 0, 0, 0, Kind::Unspecified);
    let mut date = match date {
        Ok(date) if year >= 1601 => date,
        _ => {
            p.losses
                .invalid("Invalid GlobalObjectId instance date; RECURRENCE-ID was omitted.");
            return Ok(None);
        }
    };
    if all_day {
        return Ok(Some(CalendarTime {
            value: date,
            zone: None,
            is_date: true,
        }));
    }
    if let Some(time) = p.integer(Name::START_RECURRENCE_TIME) {
        let (hour, minute, second) = ((time >> 12) & 31, (time >> 6) & 63, time & 63);
        if hour < 24 && minute < 60 && second < 60 {
            date = Date::new(
                year,
                id[18],
                id[19],
                hour as u8,
                minute as u8,
                second as u8,
                Kind::Unspecified,
            )?;
        } else {
            p.losses
                .invalid("Invalid packed recurrence time; midnight was used.");
        }
    }
    Ok(Some(match zone {
        Some(zone) => calendar.local_time(date, zone, false),
        None => CalendarTime {
            value: date.with_kind(Kind::Utc),
            zone: None,
            is_date: false,
        },
    }))
}

fn resolve_address(
    smtp: Option<&str>,
    email: Option<&str>,
    kind: Option<&str>,
    key: Option<&str>,
) -> Option<Arc<str>> {
    let is_smtp = kind.is_none_or(|s| s.is_empty() || s.eq_ignore_ascii_case("SMTP"));
    // Same sources as MIME To/Cc/From. Non-SMTP recipients remain nomail.
    let address = if is_smtp && email.is_some_and(|s| !s.is_empty()) {
        email
    } else if smtp.is_some_and(|s| !s.is_empty()) {
        smtp
    } else if is_smtp {
        key.and_then(|s| {
            s.get(..5)
                .filter(|v| v.eq_ignore_ascii_case("SMTP:"))
                .map(|_| &s[5..])
        })
    } else {
        None
    }?;
    MailboxAddress::parse(address.as_bytes())
        .ok()
        .map(|m| Arc::from(m.address().as_str()))
}

fn create_address(address: Option<Arc<str>>, name: Option<Arc<str>>) -> Address {
    // MS-OXCICAL .20.2: attendees without an SMTP address use invalid:nomail.
    Address {
        address: address
            .map(|s| Arc::from(format!("mailto:{s}")))
            .unwrap_or_else(|| Arc::from("invalid:nomail")),
        name: name.filter(|s| !s.trim().is_empty()),
        ..Address::default()
    }
}

fn recipient_address(recipient: &TnefRecipient, losses: &mut Losses) -> Address {
    let mut p = Properties {
        set: recipient.properties(),
        losses,
    };
    let smtp = p.text(Tag::SMTP_ADDRESS_W);
    let email = p.text(Tag::EMAIL_ADDRESS_W);
    let kind = p.text(Tag::ADDRTYPE_W);
    let key = p.text(Tag::SEARCH_KEY);
    let name = p
        .text(Tag::RECIPIENT_DISPLAY_NAME_W)
        .or_else(|| p.text(Tag::TRANSMITABLE_DISPLAY_NAME_W))
        .or_else(|| p.text(Tag::DISPLAY_NAME_W));
    create_address(
        resolve_address(
            smtp.as_deref(),
            email.as_deref(),
            kind.as_deref(),
            key.as_deref(),
        ),
        name,
    )
}

fn sent_representing(p: &mut Properties<'_, '_>) -> Option<Address> {
    let mut name = p.text(Tag::SENT_REPRESENTING_NAME_W);
    let email = p.text(Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W);
    let kind = p.text(Tag::SENT_REPRESENTING_ADDRTYPE_W);
    let key = p.text(Tag::SENT_REPRESENTING_SEARCH_KEY);
    let mut address = resolve_address(None, email.as_deref(), kind.as_deref(), key.as_deref());
    if address.is_none() {
        name = name.or_else(|| p.text(Tag::SENDER_NAME_W));
        let smtp = p.text(Tag::SENDER_SMTP_ADDRESS_W);
        let email = p.text(Tag::SENDER_EMAIL_ADDRESS_W);
        let kind = p.text(Tag::SENDER_ADDRTYPE_W);
        let key = p.text(Tag::SENDER_SEARCH_KEY);
        address = resolve_address(
            smtp.as_deref(),
            email.as_deref(),
            kind.as_deref(),
            key.as_deref(),
        );
    }
    if address.is_none() && name.as_ref().is_none_or(|s| s.trim().is_empty()) {
        None
    } else {
        Some(create_address(address, name))
    }
}

fn add_attendees(calendar: &Calendar, e: &mut Event, message: &TnefMessage, losses: &mut Losses) {
    let mut p = Properties {
        set: message.properties(),
        losses,
    };
    if calendar.is_reply() {
        // A response is sent by the attendee to the organizer, its To recipient.
        for recipient in message.recipients() {
            let mut row = Properties {
                set: recipient.properties(),
                losses: p.losses,
            };
            if row.integer(Tag::RECIPIENT_TYPE).unwrap_or(1) & 0x0fff_ffff == 1 {
                e.organizer = Some(recipient_address(recipient, p.losses));
                break;
            }
        }
        if let Some(mut attendee) = sent_representing(&mut p) {
            let class = p.text(Tag::MESSAGE_CLASS_W);
            attendee.part_stat = class.as_deref().and_then(|class| {
                [
                    ("Pos", "ACCEPTED"),
                    ("Tent", "TENTATIVE"),
                    ("Neg", "DECLINED"),
                ]
                .into_iter()
                .find(|(suffix, _)| {
                    is_class(class.trim(), &format!("IPM.Schedule.Meeting.Resp.{suffix}"))
                })
                .map(|(_, status)| status)
            });
            e.attendees = Arc::from([attendee]);
        }
        return;
    }
    // A published appointment only has attendees when it is a meeting.
    if calendar.method == "PUBLISH"
        && p.integer(Name::APPOINTMENT_STATE_FLAGS).unwrap_or(0) & APPOINTMENT_STATE_MEETING == 0
    {
        return;
    }
    let rsvp = p.boolean(Tag::RESPONSE_REQUESTED);
    let mut attendees = Vec::new();
    for recipient in message.recipients() {
        let mut row = Properties {
            set: recipient.properties(),
            losses: p.losses,
        };
        let flags = row.integer(Tag::RECIPIENT_FLAGS).unwrap_or(0);
        if flags & RECIPIENT_EXCEPTIONAL_DELETED != 0 {
            continue;
        }
        let kind =
            TnefRecipientType::from(row.integer(Tag::RECIPIENT_TYPE).unwrap_or(1) & 0x0fff_ffff);
        if flags & RECIPIENT_ORGANIZER != 0 || kind == TnefRecipientType::Originator {
            if e.organizer.is_none() {
                e.organizer = Some(recipient_address(recipient, row.losses));
            }
            continue;
        }
        let mut attendee = recipient_address(recipient, row.losses);
        match kind {
            TnefRecipientType::Cc => attendee.role = Some("OPT-PARTICIPANT"),
            TnefRecipientType::Bcc => {
                // Rooms/equipment are Bcc recipients of a meeting request.
                attendee.cu_type = Some("RESOURCE");
                attendee.role = Some("NON-PARTICIPANT");
            }
            _ => attendee.role = Some("REQ-PARTICIPANT"),
        }
        attendee.part_stat = if calendar.method == "PUBLISH" {
            // MS-OXCICAL .20.2.4: use RecipientTrackStatus for published items.
            match row.integer(Tag::RECIPIENT_TRACK_STATUS) {
                Some(2) => Some("TENTATIVE"),
                Some(3) => Some("ACCEPTED"),
                Some(4) => Some("DECLINED"),
                _ => None,
            }
        } else {
            Some("NEEDS-ACTION")
        };
        attendee.rsvp = rsvp;
        attendees.push(attendee);
    }
    e.attendees = attendees.into();
    if e.organizer.is_none() {
        e.organizer = sent_representing(&mut p);
    }
}

fn write_address<W: Write>(
    w: &mut CalendarWriter<W>,
    name: &str,
    address: &Address,
) -> io::Result<()> {
    w.begin_property(name)?;
    for (name, value) in [
        ("CUTYPE", address.cu_type),
        ("ROLE", address.role),
        ("PARTSTAT", address.part_stat),
    ] {
        if let Some(value) = value {
            w.write_parameter(name, value)?;
        }
    }
    if let Some(rsvp) = address.rsvp {
        w.write_parameter("RSVP", if rsvp { "TRUE" } else { "FALSE" })?;
    }
    if let Some(name) = &address.name {
        w.write_parameter("CN", name)?;
    }
    w.write_value(&address.address)
}

struct ExceptionAttachment {
    index: usize,
    message: TnefMessage,
    replace: Option<Date>,
    has_replace: bool,
    start: Option<Date>,
    used: bool,
}

fn load_attachments(
    message: &TnefMessage,
    losses: &mut Losses,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<Vec<ExceptionAttachment>> {
    let mut attachments = Vec::new();
    for (index, attachment) in message.attachments().iter().enumerate() {
        check()?;
        losses.attachment = Some(index);
        let mut p = Properties {
            set: attachment.properties(),
            losses,
        };
        let flags = p.integer(Tag::ATTACHMENT_FLAGS).unwrap_or(0);
        if flags & ATTACHMENT_FLAG_EXCEPTION == 0 || !attachment.is_embedded_message() {
            continue;
        }
        let start = p.date(Tag::EXCEPTION_START_TIME);
        // Detached readers retain their original depth/byte limits. Surface
        // permissive framing recovery as well as fatal embedded-load errors.
        let loaded = attachment.load_embedded_message_with_logger(&mut |diagnostic| {
            losses.invalid(format!("Exception attachment framing: {diagnostic:?}"));
            Ok(())
        });
        match loaded {
            Ok(message) => {
                let mut p = Properties {
                    set: message.properties(),
                    losses,
                };
                let has_replace = p.set.get_named(&Name::EXCEPTION_REPLACE_TIME).is_some();
                let replace = p.date(Name::EXCEPTION_REPLACE_TIME);
                attachments.push(ExceptionAttachment {
                    index,
                    message,
                    replace,
                    has_replace,
                    start,
                    used: false,
                });
            }
            Err(error) => losses.invalid(format!(
                "An exception attachment could not be loaded: {error}"
            )),
        }
    }
    losses.attachment = None;
    Ok(attachments)
}

fn find_attachment(
    attachments: &[ExceptionAttachment],
    original_utc: Date,
    start_local: Date,
) -> Option<usize> {
    // MS-OXOCAL 2.2.10.1.3: ExceptionReplaceTime is the original UTC start.
    attachments
        .iter()
        .position(|a| {
            !a.used
                && a.replace
                    .is_some_and(|d| d.ticks().abs_diff(original_utc.ticks()) < 600_000_000)
        })
        // Otherwise ExceptionStartTime on the attachment is the new local
        // start. A present but invalid ReplaceTime must not use this fallback.
        .or_else(|| {
            attachments.iter().position(|a| {
                !a.used
                    && !a.has_replace
                    && a.start
                        .is_some_and(|d| d.ticks().abs_diff(start_local.ticks()) < 600_000_000)
            })
        })
}

#[allow(clippy::too_many_arguments)]
fn add_recurrence(
    calendar: &mut Calendar,
    master: &mut Event,
    message: &TnefMessage,
    recurrence: &TnefRecurrence,
    zone: &TnefTimeZone,
    all_day: bool,
    options: &CalendarOptions,
    losses: &mut Losses,
    check: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<Result<(), TnefDateTimeError>> {
    match recurrence_rule(recurrence, zone, all_day) {
        Ok(rule) => master.rule = Some(rule),
        Err(RecurrenceRuleError::InvalidDate(error)) => return Ok(Err(error)),
        Err(error) => {
            let kind = if matches!(
                error,
                RecurrenceRuleError::UnsupportedCalendar { .. }
                    | RecurrenceRuleError::UnknownPattern(_)
            ) {
                CalendarLossKind::UnsupportedCalendarData
            } else {
                CalendarLossKind::InvalidCalendarData
            };
            losses.add(kind, format!("The recurrence rule was omitted: {error}"));
            return Ok(Ok(()));
        }
    }
    if let Some(error) = recurrence.exception_error() {
        losses.invalid(format!(
            "The recurring appointment's exceptions could not be parsed: {error}"
        ));
    }
    // Deleted instances that are not modified instances become EXDATEs.
    // Membership includes all parsed originals, even those beyond export limits.
    let dates = match recurrence_exdates(recurrence) {
        Ok(dates) => dates,
        Err(e) => return Ok(Err(e)),
    };
    master.exdates = dates
        .into_iter()
        .map(|d| calendar.local_time(d, zone, all_day))
        .collect();
    let count = recurrence.exceptions().len().min(options.max_exceptions);
    if count < recurrence.exceptions().len() {
        losses.add(
            CalendarLossKind::CalendarExceptionLimitExceeded,
            format!(
                "Only the first {count} of {} modified occurrences were eligible for export (max_exceptions).",
                recurrence.exceptions().len()
            ),
        );
    }
    if count == 0 {
        return Ok(Ok(()));
    }
    let mut attachments = load_attachments(message, losses, check)?;
    for (index, exception) in recurrence.exceptions().iter().take(count).enumerate() {
        check()?;
        let original = Date::from_recurrence_minutes(exception.original_start_date());
        let start = Date::from_recurrence_minutes(exception.start_date_time());
        let end = Date::from_recurrence_minutes(exception.end_date_time());
        let original_utc = match zone.to_universal_time(original) {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        let matched = find_attachment(&attachments, original_utc, start);
        let mut event = master.clone_for_exception();
        let mut exception_all_day = if exception.has_override(Exception::SUB_TYPE_FLAG) {
            exception.is_all_day()
        } else {
            all_day
        };
        // RECURRENCE-ID retains the series' type even if the instance changes
        // between an all-day and timed event (MS-OXCICAL .20.20).
        event.recurrence_id = Some(calendar.local_time(original, zone, all_day));
        event.start = calendar.local_time(start, zone, exception_all_day);
        event.end = Some(calendar.local_time(end, zone, exception_all_day));
        if exception.has_override(Exception::SUBJECT_FLAG) {
            event.summary = Arc::from(exception.subject().unwrap_or(""));
        }
        if exception.has_override(Exception::LOCATION_FLAG) {
            event.location = exception.location().map(Arc::from);
        }
        if exception.has_override(Exception::BUSY_STATUS_FLAG) {
            event.busy = Some(exception.busy_status());
        }
        let enabled = if exception.has_override(Exception::REMINDER_FLAG) {
            exception.reminder_set()
        } else {
            master.reminder.is_some()
        };
        event.reminder = if !enabled {
            None
        } else if exception.has_override(Exception::REMINDER_DELTA_FLAG) {
            Some(normalize_reminder(exception.reminder_delta()))
        } else {
            Some(master.reminder.unwrap_or(15))
        };
        // Remaining overrides come from the exception's embedded Calendar object.
        if let Some(matched) = matched {
            let attachment = &attachments[matched];
            losses.attachment = Some(attachment.index);
            if let Some(subject) = subject(&attachment.message, losses) {
                event.summary = subject;
            }
            if let Some(body) = body(&attachment.message, options, losses) {
                event.description = Some(body);
            }
            let mut p = Properties {
                set: attachment.message.properties(),
                losses,
            };
            if let Some(location) = p.text(Name::LOCATION) {
                event.location = Some(location);
            }
            if let Some(busy) = p.integer(Name::BUSY_STATUS) {
                event.busy = Some(busy);
            }
            if let Some(set) = p.boolean(Name::REMINDER_SET) {
                event.reminder = if set { reminder(&mut p) } else { None };
            }
            if let Some(subtype) = p.boolean(Name::APPOINTMENT_SUB_TYPE) {
                exception_all_day = subtype;
            }
            if let Some(start) = p.date(Name::APPOINTMENT_START_WHOLE) {
                match calendar.utc_time(start, Some(zone), exception_all_day) {
                    Ok(time) => event.start = time,
                    Err(e) => return Ok(Err(e)),
                }
            }
            if let Some(end) = p.date(Name::APPOINTMENT_END_WHOLE) {
                match calendar.utc_time(end, Some(zone), exception_all_day) {
                    Ok(time) => event.end = Some(time),
                    Err(e) => return Ok(Err(e)),
                }
            }
            losses.attachment = None;
        }
        // An exception repeats most of the master: a tiny recurrence blob can
        // expand to a huge VEVENT. Measure before committing to it.
        let mut measure = CalendarWriter::new(MeasuringStream::new());
        calendar.write_event(&mut measure, &event)?;
        measure.finish()?;
        let size = measure.get_ref().len();
        if size > options.max_exceptions_size - calendar.exceptions_size {
            losses.add(CalendarLossKind::CalendarExceptionLimitExceeded,
                format!("Modified occurrences exceed {} bytes; only the first {index} of {} were exported (max_exceptions_size).",
                    options.max_exceptions_size, recurrence.exceptions().len()));
            break;
        }
        calendar.exceptions_size += size;
        if let Some(matched) = matched {
            attachments[matched].used = true;
            calendar
                .exception_attachments
                .push(attachments[matched].index);
        }
        calendar.events.push(event);
    }
    Ok(Ok(()))
}

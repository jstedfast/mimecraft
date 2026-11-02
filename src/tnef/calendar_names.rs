// Adapted from MimeKit's TnefNameId.cs and TnefPropertySetGuid.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::{TnefName, TnefNameId};

const APPOINTMENT: [u8; 16] = [2, 32, 6, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70];
const COMMON: [u8; 16] = [8, 32, 6, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70];
const MEETING: [u8; 16] = [
    144, 218, 216, 110, 11, 69, 27, 16, 152, 218, 0, 170, 0, 63, 19, 5,
];

macro_rules! names {
    ($($name:ident, $guid:ident, $id:expr, $doc:literal;)+) => {
        impl TnefNameId {
            $(#[doc = $doc]
            pub const $name: Self = Self { guid: $guid, name: TnefName::Id($id) };)+
        }
    };
}

names! {
    APPOINTMENT_COUNTER_PROPOSAL, APPOINTMENT, 0x8257, "Whether a tentative reply proposes new times.";
    APPOINTMENT_DURATION, APPOINTMENT, 0x8213, "Appointment duration in minutes.";
    APPOINTMENT_END_WHOLE, APPOINTMENT, 0x820e, "Appointment end in UTC.";
    APPOINTMENT_NOT_ALLOW_PROPOSE, APPOINTMENT, 0x825a, "Whether counter proposals are disallowed.";
    APPOINTMENT_PROPOSED_END_WHOLE, APPOINTMENT, 0x8251, "Proposed appointment end in UTC.";
    APPOINTMENT_PROPOSED_START_WHOLE, APPOINTMENT, 0x8250, "Proposed appointment start in UTC.";
    APPOINTMENT_RECUR, APPOINTMENT, 0x8216, "AppointmentRecurrencePattern binary data.";
    APPOINTMENT_SEQUENCE, APPOINTMENT, 0x8201, "Appointment update sequence.";
    APPOINTMENT_START_WHOLE, APPOINTMENT, 0x820d, "Appointment start in UTC.";
    APPOINTMENT_STATE_FLAGS, APPOINTMENT, 0x8217, "Appointment state flags, including asfMeeting.";
    APPOINTMENT_SUB_TYPE, APPOINTMENT, 0x8215, "Whether the appointment is all-day.";
    APPOINTMENT_TIME_ZONE_DEFINITION_END_DISPLAY, APPOINTMENT, 0x825f, "Display time zone for the end.";
    APPOINTMENT_TIME_ZONE_DEFINITION_RECUR, APPOINTMENT, 0x8260, "Time zone of the recurring series.";
    APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY, APPOINTMENT, 0x825e, "Display time zone for the start.";
    BUSY_STATUS, APPOINTMENT, 0x8205, "Appointment availability.";
    EXCEPTION_REPLACE_TIME, APPOINTMENT, 0x8228, "Original UTC start of a modified occurrence.";
    INTENDED_BUSY_STATUS, APPOINTMENT, 0x8224, "Availability intended by the organizer.";
    LOCATION, APPOINTMENT, 0x8208, "Appointment location.";
    RECURRING, APPOINTMENT, 0x8223, "Whether recurrence is enabled.";
    TIME_ZONE_DESCRIPTION, APPOINTMENT, 0x8234, "Name of the legacy recurrence time zone.";
    TIME_ZONE_STRUCT, APPOINTMENT, 0x8233, "Legacy TimeZoneStruct binary data.";
    PRIVATE, COMMON, 0x8506, "Whether the item is private.";
    REMINDER_DELTA, COMMON, 0x8501, "Reminder lead time in minutes.";
    REMINDER_SET, COMMON, 0x8503, "Whether a reminder is enabled.";
    ATTENDEE_CRITICAL_CHANGE, MEETING, 0x0001, "Last critical change made by an attendee.";
    CLEAN_GLOBAL_OBJECT_ID, MEETING, 0x0023, "Series GlobalObjectId without an instance date.";
    GLOBAL_OBJECT_ID, MEETING, 0x0003, "GlobalObjectId binary data.";
    IS_EXCEPTION, MEETING, 0x000a, "Whether this is an individual recurrence instance.";
    OWNER_CRITICAL_CHANGE, MEETING, 0x001a, "Last critical change made by the organizer.";
    START_RECURRENCE_TIME, MEETING, 0x000e, "Packed local time of a recurrence instance.";
}

impl TnefNameId {
    /// Returns the case-sensitive PidNameKeywords descriptor in PS_PUBLIC_STRINGS.
    pub fn keywords() -> Self {
        Self {
            guid: [41, 3, 2, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70],
            name: TnefName::String("Keywords".into()),
        }
    }
}

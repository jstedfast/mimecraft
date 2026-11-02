// Original workloads based on MimeKit's calendar export tests.
// MimeKit has no corresponding calendar-builder benchmark.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::hint::black_box;

use mimecraft::tnef::{
    TnefMessage, TnefNameId as Name, TnefPropertyTag as Tag, TnefValue as Value,
    calendar::{CalendarBuildResult, CalendarLossKind, CalendarOptions, build_calendar},
};

use super::{calendar_fixtures as fixtures, recurrence_fixtures};
use fixtures::{Fixture, date, eastern, global_id, recipient, stamp, structure};

pub const CASES: [&str; 10] = [
    "Appointment",
    "Meeting",
    "Counter",
    "TimeZone",
    "LegacyZone",
    "Recurrence",
    "Embedded",
    "CountLimit",
    "SizeLimit",
    "InvalidRecurrence",
];

pub struct Workload {
    pub name: &'static str,
    /// Complete build/write/observe/drop operations per batch; loading is excluded.
    pub operations: usize,
    message: TnefMessage,
    options: CalendarOptions,
    method: &'static str,
    events: usize,
    zones: usize,
    loss: Option<CalendarLossKind>,
    consumed: &'static [usize],
    expected: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub events: usize,
    pub losses: usize,
    pub consumed: usize,
    pub checksum: u64,
}

impl Summary {
    fn observe(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len();
        for &byte in bytes {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let recurring = matches!(name, "Recurrence" | "Embedded" | "CountLimit" | "SizeLimit");
    let mut fixture = if recurring {
        Fixture::recurring(recurrence_fixtures::many(8))
    } else {
        Fixture::appointment(if name == "Meeting" {
            "IPM.Schedule.Meeting.Request"
        } else if name == "Counter" {
            "IPM.Schedule.Meeting.Resp.Tent"
        } else {
            "IPM.Appointment"
        })
    };
    let mut case = Workload {
        name,
        operations: 8,
        message: Fixture::empty("IPM.Note").load(),
        options: CalendarOptions {
            fallback_stamp: Some(stamp()),
            ..CalendarOptions::default()
        },
        method: if recurring || name == "Meeting" {
            "REQUEST"
        } else if name == "Counter" {
            "COUNTER"
        } else {
            "PUBLISH"
        },
        events: if recurring { 9 } else { 1 },
        zones: usize::from(recurring || matches!(name, "TimeZone" | "LegacyZone")),
        loss: None,
        consumed: &[],
        expected: vec![
            "DTSTAMP:20240102T030405Z",
            "SUMMARY:Team sync",
            "UID:benchmark@example.com",
        ],
    };
    if name == "Appointment" {
        case.expected[2] = "UID:F41DB2D6-86C2-570B-9997-4AF21122A0D0";
    } else {
        fixture.properties.named(
            Name::GLOBAL_OBJECT_ID,
            Value::Binary(global_id(
                b"vCal-Uid\x01\0\0\0benchmark@example.com",
                0,
                0,
                0,
            )),
        );
    }
    match name {
        "Appointment" => case.expected.push("DTSTART:20240301T150000Z"),
        "Meeting" => {
            fixture.recipients = vec![
                recipient(1, "Alice", "alice@example.com", Some(3)),
                recipient(1, "Bob", "bob@example.com", Some(1)),
            ];
            fixture.properties.categories(&["Planning", "Quarterly"]);
            fixture.properties.tag(
                Tag::BODY_W,
                Value::Text("Agenda\r\nDiscuss next steps".into()),
            );
            case.expected.extend([
                "ORGANIZER;CN=Alice:mailto:alice@example.com",
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;CN=Bob:mailto:bob@example.com",
                "CATEGORIES:Planning,Quarterly",
                "DESCRIPTION:Agenda\\nDiscuss next steps",
            ]);
        }
        "Counter" => {
            fixture
                .properties
                .named(Name::APPOINTMENT_COUNTER_PROPOSAL, Value::Bool(true));
            fixture.properties.named(
                Name::APPOINTMENT_PROPOSED_START_WHOLE,
                Value::DateTime(date(2024, 3, 2, 15, 0)),
            );
            case.expected.extend([
                "DTSTART:20240302T150000Z",
                "X-MS-OLK-ORIGINALSTART:20240301T150000Z",
            ]);
        }
        "TimeZone" => {
            fixture.properties.named(
                Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
                Value::Binary(eastern("Eastern Standard Time", 300)),
            );
            case.expected
                .push("DTSTART;TZID=Eastern Standard Time:20240301T100000");
        }
        "LegacyZone" => {
            fixture.properties.named(
                Name::APPOINTMENT_RECUR,
                Value::Binary(recurrence_fixtures::pattern(0, 1440, &[])),
            );
            fixture
                .properties
                .named(Name::TIME_ZONE_STRUCT, Value::Binary(structure()));
            fixture
                .properties
                .named(Name::TIME_ZONE_DESCRIPTION, Value::Text("Eastern".into()));
            case.expected.extend([
                "DTSTART;TZID=Eastern:20240301T100000",
                "RRULE:FREQ=DAILY;COUNT=10",
            ]);
        }
        "Embedded" => {
            let mut embedded = Fixture::empty("IPM.Appointment");
            embedded.properties.named(
                Name::EXCEPTION_REPLACE_TIME,
                Value::DateTime(date(2024, 7, 1, 14, 0)),
            );
            embedded
                .properties
                .tag(Tag::SUBJECT_W, Value::Text("Moved by attachment".into()));
            fixture.attach(2, embedded.bytes(), None);
            case.consumed = &[0];
            case.expected.push("SUMMARY:Moved by attachment");
        }
        "CountLimit" => {
            case.options.max_exceptions = 2;
            case.events = 3;
            case.loss = Some(CalendarLossKind::CalendarExceptionLimitExceeded);
        }
        "SizeLimit" => {
            case.options.max_exceptions_size = 0;
            case.events = 1;
            case.loss = Some(CalendarLossKind::CalendarExceptionLimitExceeded);
        }
        "InvalidRecurrence" => {
            fixture
                .properties
                .named(Name::APPOINTMENT_RECUR, Value::Binary(vec![0xff; 5]));
            case.loss = Some(CalendarLossKind::InvalidCalendarData);
        }
        "Recurrence" => {}
        _ => unreachable!(),
    }
    if recurring {
        case.expected
            .push("RRULE:FREQ=WEEKLY;BYDAY=SU,MO,TU,WE,TH,FR,SA;COUNT=18");
        if case.events > 1 {
            case.expected.extend([
                "RECURRENCE-ID;TZID=Eastern Standard Time:20240701T100000",
                "DTSTART;TZID=Eastern Standard Time:20240701T130000",
            ]);
        }
    }
    case.message = fixture.load();
    case
}

impl Workload {
    pub fn check(&self) {
        let result = build_calendar(&self.message, &self.options).unwrap();
        assert_eq!(result.losses.len(), usize::from(self.loss.is_some()));
        if let Some(kind) = self.loss {
            assert_eq!(result.losses[0].kind, kind);
            assert!(!result.losses[0].description.is_empty());
        }
        let calendar = result.calendar.unwrap();
        assert_eq!(calendar.method(), self.method);
        assert_eq!(calendar.event_count(), self.events);
        assert_eq!(calendar.exception_attachment_indices(), self.consumed);
        let mut bytes = Vec::new();
        calendar.write_to(&mut bytes).unwrap();
        let raw = String::from_utf8(bytes).unwrap();
        assert!(raw.split("\r\n").all(|line| line.len() <= 75));
        let unfolded = raw.replace("\r\n ", "");
        let lines: Vec<_> = unfolded.split("\r\n").collect();
        assert_eq!(lines[0], "BEGIN:VCALENDAR");
        assert!(unfolded.ends_with("END:VCALENDAR\r\n"));
        assert_eq!(
            lines.iter().filter(|&&s| s == "BEGIN:VEVENT").count(),
            self.events
        );
        assert_eq!(
            lines.iter().filter(|&&s| s == "END:VEVENT").count(),
            self.events
        );
        assert_eq!(
            lines.iter().filter(|&&s| s == "BEGIN:VTIMEZONE").count(),
            self.zones
        );
        for expected in &self.expected {
            assert!(lines.contains(expected), "{} missing {expected}", self.name);
        }
        // Measure physical exception bytes independently, including folding/CRLF.
        let exceptions: usize = raw
            .split("BEGIN:VEVENT\r\n")
            .skip(2)
            .map(|event| {
                "BEGIN:VEVENT\r\n".len()
                    + event.find("END:VEVENT\r\n").unwrap()
                    + "END:VEVENT\r\n".len()
            })
            .sum();
        assert_eq!(calendar.exceptions_size(), exceptions as u64);
        let summary = run(self);
        assert_eq!(summary.events, self.events * self.operations);
        assert_eq!(
            summary.losses,
            usize::from(self.loss.is_some()) * self.operations
        );
        assert_eq!(summary.consumed, self.consumed.len() * self.operations);
    }
}

pub fn run(case: &Workload) -> Summary {
    let mut summary = Summary::default();
    for _ in 0..case.operations {
        let CalendarBuildResult { calendar, losses } =
            build_calendar(black_box(&case.message), black_box(&case.options)).unwrap();
        let calendar = calendar.unwrap();
        let mut bytes = Vec::new();
        calendar.write_to(&mut bytes).unwrap();
        summary.observe(&bytes);
        summary.events += calendar.event_count();
        summary.losses += losses.len();
        summary.consumed += calendar.exception_attachment_indices().len();
        summary.checksum = summary.checksum.wrapping_add(calendar.exceptions_size());
        for &index in calendar.exception_attachment_indices() {
            summary.checksum = summary.checksum.wrapping_mul(31).wrapping_add(index as u64);
        }
        for loss in losses {
            summary.observe(loss.description.as_bytes());
            summary.checksum = summary
                .checksum
                .wrapping_add(match loss.kind {
                    CalendarLossKind::InvalidCalendarData => 1,
                    CalendarLossKind::UnsupportedCalendarData => 2,
                    CalendarLossKind::CalendarExceptionLimitExceeded => 3,
                })
                .wrapping_add(loss.attachment_index.map_or(0, |i| i as u64 + 1));
        }
    }
    summary
}

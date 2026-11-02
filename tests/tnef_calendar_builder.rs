// Adapted from MimeKit's TnefCalendarTests and TnefCalendarRobustnessTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefDateTime as Date, TnefDateTimeKind as Kind, TnefNameId as Name, TnefPropertyTag as Tag,
    TnefValue as Value,
    calendar::{
        Calendar, CalendarBuildResult, CalendarLossKind as Loss, CalendarOptions, build_calendar,
        build_calendar_with_check,
    },
};
use std::io::{self, Write};
#[path = "support/tnef_calendar.rs"]
mod fixtures;
#[path = "support/tnef_recurrence.rs"]
mod recurrence;
use fixtures::*;

fn options() -> CalendarOptions {
    CalendarOptions {
        fallback_stamp: Some(stamp()),
        ..CalendarOptions::default()
    }
}
fn convert(fixture: &Fixture) -> CalendarBuildResult {
    build_calendar(&fixture.load(), &options()).unwrap()
}
fn raw(calendar: &Calendar) -> String {
    let mut bytes = Vec::new();
    calendar.write_to(&mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}
fn lines(calendar: &Calendar) -> Vec<String> {
    let text = raw(calendar);
    assert!(text.ends_with("\r\n"));
    for line in text.split("\r\n") {
        assert!(line.len() <= 75, "{line}");
        assert!(!line.contains(['\r', '\n']));
    }
    text.replace("\r\n ", "")
        .trim_end_matches("\r\n")
        .split("\r\n")
        .map(str::to_owned)
        .collect()
}
fn event(lines: &[String], index: usize) -> &[String] {
    let start = lines
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == "BEGIN:VEVENT")
        .nth(index)
        .unwrap()
        .0;
    let end = start
        + lines[start..]
            .iter()
            .position(|s| s == "END:VEVENT")
            .unwrap();
    &lines[start..=end]
}
fn has(lines: &[String], expected: &str) {
    assert!(
        lines.iter().any(|s| s == expected),
        "missing {expected:?} in {lines:#?}"
    );
}
fn field<'a>(lines: &'a [String], prefix: &str) -> Option<&'a str> {
    lines
        .iter()
        .find(|s| s.starts_with(prefix))
        .map(String::as_str)
}
fn clean(f: &Fixture) -> Calendar {
    let result = convert(f);
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    result.calendar.unwrap()
}

#[test]
fn meeting_request_exact_upstream_output_and_mime_part() {
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Request");
    let p = &mut f.properties;
    p.tag(
        Tag::BODY_W,
        Value::Text("Agenda;\r\nItem 1, Item 2\r\n".into()),
    );
    p.tag(Tag::RESPONSE_REQUESTED, Value::I32(1));
    p.named(Name::LOCATION, Value::Text("Room 1".into()));
    p.named(Name::BUSY_STATUS, Value::I32(2));
    p.named(Name::APPOINTMENT_SEQUENCE, Value::I32(3));
    p.named(Name::REMINDER_SET, Value::Bool(true));
    p.named(Name::REMINDER_DELTA, Value::I32(10));
    p.named(
        Name::GLOBAL_OBJECT_ID,
        Value::Binary(global_id(&[], 2024, 3, 1)),
    );
    f.recipients = vec![
        recipient(1, "Alice", "alice@example.com", Some(3)),
        recipient(1, "Bob", "bob@example.com", Some(1)),
        recipient(2, "Carol", "carol@example.com", None),
        recipient(3, "Room 1", "room1@example.com", None),
        recipient(1, "Dave", "dave@example.com", Some(0x21)),
    ];
    let calendar = clean(&f);
    let expected = [
        "BEGIN:VCALENDAR",
        "METHOD:REQUEST",
        "PRODID:-//mimecraft//EN",
        "VERSION:2.0",
        "X-MS-OLK-FORCEINSPECTOROPEN:TRUE",
        "BEGIN:VEVENT",
        "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Bob:mailto:bob@example.com",
        "ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Carol:mailto:carol@example.com",
        "ATTENDEE;CUTYPE=RESOURCE;ROLE=NON-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Room 1:mailto:room1@example.com",
        "DESCRIPTION:Agenda\\;\\nItem 1\\, Item 2",
        "DTEND:20240301T160000Z",
        "DTSTAMP:20240102T030405Z",
        "DTSTART:20240301T150000Z",
        "LOCATION:Room 1",
        "ORGANIZER;CN=Alice:mailto:alice@example.com",
        "SEQUENCE:3",
        "SUMMARY:Team sync",
        "TRANSP:OPAQUE",
        "UID:040000008200E00074C5B7101A82E008000000001112131415161718000000000000000000000000",
        "X-MICROSOFT-CDO-BUSYSTATUS:BUSY",
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "DESCRIPTION:Reminder",
        "TRIGGER:-PT10M",
        "END:VALARM",
        "END:VEVENT",
        "END:VCALENDAR",
    ];
    assert_eq!(lines(&calendar), expected);
    let mut part = calendar.to_text_part().unwrap();
    let content_type = part.part().content_type().unwrap().unwrap();
    assert_eq!(content_type.charset(), Some("utf-8"));
    assert_eq!(content_type.parameters().get("method"), Some("REQUEST"));
    assert_eq!(part.text().unwrap(), raw(&calendar));
    let mut serialized = Vec::new();
    part.part_mut().write_to(&mut serialized).unwrap();
    assert!(
        serialized
            .windows(b"text/calendar".len())
            .any(|w| w == b"text/calendar")
    );
}

#[test]
fn detection_disabled_missing_start_and_non_calendar_classes() {
    for class in [
        "IPM.Note",
        "IPM.AppointmentX",
        "IPM.Schedule.Meeting.Notification.Forward",
        "",
    ] {
        let result = convert(&Fixture::appointment(class));
        assert!(result.calendar.is_none());
        assert!(result.losses.is_empty());
    }
    let f = Fixture::appointment("iPm.ApPoInTmEnT.Custom");
    let calendar = clean(&f);
    assert_eq!(calendar.method(), "PUBLISH");
    assert!(field(&lines(&calendar), "ATTENDEE").is_none());
    let result = build_calendar(
        &f.load(),
        &CalendarOptions {
            generate_calendar: false,
            ..options()
        },
    )
    .unwrap();
    assert!(result.calendar.is_none());
    assert!(result.losses.is_empty());
    let result = convert(&Fixture::empty("IPM.Appointment"));
    assert!(result.calendar.is_none());
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidCalendarData);
}

#[test]
fn stable_uid_imported_uid_clean_id_and_hex_fallback() {
    let f = Fixture::appointment("IPM.Appointment");
    let first = lines(&clean(&f));
    let uid = field(&first, "UID:").unwrap();
    // Independent SHA-256/.NET Guid reference, including Windows byte order.
    assert_eq!(uid, "UID:F41DB2D6-86C2-570B-9997-4AF21122A0D0");
    assert_eq!(uid.len(), 40);
    assert_eq!(uid.as_bytes()[18], b'5');
    assert_eq!(field(&lines(&clean(&f)), "UID:"), Some(uid));
    let mut other = f.clone();
    other
        .properties
        .tag(Tag::NORMALIZED_SUBJECT_W, Value::Text("Other".into()));
    assert_ne!(field(&lines(&clean(&other)), "UID:"), Some(uid));
    for (data, expected) in [
        (
            b"vCal-Uid\x01\0\0\0abc-123@example.com\0".as_slice(),
            "UID:abc-123@example.com",
        ),
        (
            b"vCal-Uid\x01\0\0\0\0".as_slice(),
            "UID:040000008200E00074C5B7101A82E008",
        ),
    ] {
        let mut f = f.clone();
        f.properties.named(
            Name::GLOBAL_OBJECT_ID,
            Value::Binary(global_id(data, 2024, 7, 10)),
        );
        assert!(
            field(&lines(&clean(&f)), "UID:")
                .unwrap()
                .starts_with(expected)
        );
    }
    let mut f = f;
    f.properties.named(
        Name::GLOBAL_OBJECT_ID,
        Value::Binary(global_id(b"vCal-Uid\x01\0\0\0wrong", 0, 0, 0)),
    );
    f.properties.named(
        Name::CLEAN_GLOBAL_OBJECT_ID,
        Value::Binary(global_id(b"vCal-Uid\x01\0\0\0clean", 0, 0, 0)),
    );
    has(&lines(&clean(&f)), "UID:clean");
}

#[test]
fn responses_counter_proposals_and_cancellation() {
    for (suffix, status) in [
        ("Pos", "ACCEPTED"),
        ("Tent", "TENTATIVE"),
        ("Neg", "DECLINED"),
    ] {
        let mut f = Fixture::appointment(&format!(" IPM.Schedule.Meeting.Resp.{suffix}.Custom "));
        f.properties
            .tag(Tag::BODY_W, Value::Text("See you there".into()));
        f.properties
            .tag(Tag::SENT_REPRESENTING_NAME_W, Value::Text("Bob".into()));
        f.properties.tag(
            Tag::SENT_REPRESENTING_ADDRTYPE_W,
            Value::Text("SMTP".into()),
        );
        f.properties.tag(
            Tag::SENT_REPRESENTING_EMAIL_ADDRESS_W,
            Value::Text("bob@example.com".into()),
        );
        f.recipients
            .push(recipient(1, "Alice", "alice@example.com", None));
        let l = lines(&clean(&f));
        has(&l, "METHOD:REPLY");
        has(&l, "COMMENT:See you there");
        has(
            &l,
            &format!("ATTENDEE;PARTSTAT={status};CN=Bob:mailto:bob@example.com"),
        );
        has(&l, "ORGANIZER;CN=Alice:mailto:alice@example.com");
        assert!(field(&l, "DESCRIPTION:").is_none());
    }
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Resp.Tent");
    f.properties
        .named(Name::APPOINTMENT_COUNTER_PROPOSAL, Value::Bool(true));
    f.properties.named(
        Name::APPOINTMENT_PROPOSED_START_WHOLE,
        Value::DateTime(date(2024, 3, 2, 15, 0)),
    );
    f.properties.named(
        Name::APPOINTMENT_PROPOSED_END_WHOLE,
        Value::DateTime(date(2024, 3, 2, 16, 30)),
    );
    let l = lines(&clean(&f));
    for s in [
        "METHOD:COUNTER",
        "DTSTART:20240302T150000Z",
        "DTEND:20240302T163000Z",
        "X-MS-OLK-ORIGINALSTART:20240301T150000Z",
        "X-MS-OLK-ORIGINALEND:20240301T160000Z",
    ] {
        has(&l, s);
    }
    let l = lines(&clean(&Fixture::appointment(
        "IPM.Schedule.Meeting.Canceled",
    )));
    has(&l, "METHOD:CANCEL");
    has(&l, "STATUS:CANCELLED");
}

#[test]
fn display_zones_deduplicate_rename_and_reuse_renamed_zone() {
    for (end_bias, zone_count, end_id) in [
        (300, 1, "Eastern Standard Time"),
        (360, 2, "Eastern Standard Time (2)"),
    ] {
        let mut f = Fixture::appointment("IPM.Schedule.Meeting.Resp.Tent");
        f.properties
            .named(Name::APPOINTMENT_COUNTER_PROPOSAL, Value::Bool(true));
        f.properties.named(
            Name::APPOINTMENT_PROPOSED_START_WHOLE,
            Value::DateTime(date(2024, 7, 1, 14, 0)),
        );
        f.properties.named(
            Name::APPOINTMENT_PROPOSED_END_WHOLE,
            Value::DateTime(date(2024, 12, 2, 15, 0)),
        );
        f.properties.named(
            Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
            Value::Binary(eastern("Eastern Standard Time", 300)),
        );
        f.properties.named(
            Name::APPOINTMENT_TIME_ZONE_DEFINITION_END_DISPLAY,
            Value::Binary(eastern("Eastern Standard Time", end_bias)),
        );
        let l = lines(&clean(&f));
        assert_eq!(
            l.iter().filter(|s| *s == "BEGIN:VTIMEZONE").count(),
            zone_count
        );
        has(&l, "DTSTART;TZID=Eastern Standard Time:20240701T100000");
        assert!(
            field(&l, "DTEND;")
                .unwrap()
                .starts_with(&format!("DTEND;TZID={end_id}:"))
        );
        assert!(
            field(&l, "X-MS-OLK-ORIGINALEND")
                .unwrap()
                .starts_with(&format!("X-MS-OLK-ORIGINALEND;TZID={end_id}:"))
        );
        assert!(!l.iter().any(|s| s.contains("(3)")));
    }
}

#[test]
fn all_day_with_and_without_time_zone() {
    for zone in [true, false] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties
            .named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(true));
        if zone {
            f.properties.named(
                Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
                Value::Binary(eastern("Eastern", 300)),
            );
        }
        let l = lines(&clean(&f));
        has(
            &l,
            if zone {
                "DTSTART;VALUE=DATE:20240301"
            } else {
                "DTSTART;VALUE=DATE:20240302"
            },
        );
        assert!(!l.iter().any(|s| s == "BEGIN:VTIMEZONE"));
    }
}

#[test]
fn duration_sensitivity_importance_and_reminders() {
    for (duration, expected) in [
        (Some(90), Some("DTEND:20240301T163000Z")),
        (Some(0), Some("DTEND:20240301T150000Z")),
        (Some(-1), None),
        (None, None),
    ] {
        let mut f = Fixture::empty("IPM.Appointment");
        f.properties.named(
            Name::APPOINTMENT_START_WHOLE,
            Value::DateTime(date(2024, 3, 1, 15, 0)),
        );
        if let Some(d) = duration {
            f.properties
                .named(Name::APPOINTMENT_DURATION, Value::I32(d));
        }
        assert_eq!(field(&lines(&clean(&f)), "DTEND"), expected);
    }
    for (sensitivity, private, expected) in [
        (Some(0), None, Some("CLASS:PUBLIC")),
        (Some(1), None, Some("CLASS:X-PERSONAL")),
        (Some(2), None, Some("CLASS:PRIVATE")),
        (Some(3), None, Some("CLASS:CONFIDENTIAL")),
        (Some(4), Some(true), None),
        (None, Some(true), Some("CLASS:PRIVATE")),
        (None, Some(false), None),
        (Some(0), Some(true), Some("CLASS:PUBLIC")),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        if let Some(s) = sensitivity {
            f.properties.tag(Tag::SENSITIVITY, Value::I32(s));
        }
        if let Some(p) = private {
            f.properties.named(Name::PRIVATE, Value::Bool(p));
        }
        assert_eq!(field(&lines(&clean(&f)), "CLASS:"), expected);
    }
    for (importance, expected) in [
        (0, Some("PRIORITY:9")),
        (1, Some("PRIORITY:5")),
        (2, Some("PRIORITY:1")),
        (3, None),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.tag(Tag::IMPORTANCE, Value::I32(importance));
        assert_eq!(field(&lines(&clean(&f)), "PRIORITY:"), expected);
    }
    for (minutes, expected) in [
        (None, "-PT15M"),
        (Some(0x5ae980e1), "-PT15M"),
        (Some(-1), "PT0M"),
        (Some(0), "PT0M"),
        (Some(i32::MAX), "-PT2147483647M"),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.named(Name::REMINDER_SET, Value::Bool(true));
        if let Some(minutes) = minutes {
            f.properties
                .named(Name::REMINDER_DELTA, Value::I32(minutes));
        }
        has(&lines(&clean(&f)), &format!("TRIGGER:{expected}"));
    }
}

#[test]
fn categories_extensions_normalized_subject_and_timestamps() {
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Request");
    f.properties.tag(
        Tag::NORMALIZED_SUBJECT_W,
        Value::Text("Normalized\0".into()),
    );
    f.properties
        .tag(Tag::CREATION_TIME, Value::DateTime(date(2023, 1, 1, 0, 0)));
    f.properties.tag(
        Tag::LAST_MODIFICATION_TIME,
        Value::DateTime(date(2023, 2, 1, 0, 0)),
    );
    f.properties.tag(Tag::IMPORTANCE, Value::I32(2));
    f.properties.tag(Tag::OWNER_APPT_ID, Value::I32(42));
    f.properties.named(
        Name::OWNER_CRITICAL_CHANGE,
        Value::DateTime(date(2024, 2, 1, 0, 0)),
    );
    f.properties.named(
        Name::ATTENDEE_CRITICAL_CHANGE,
        Value::DateTime(date(2024, 2, 2, 0, 0)),
    );
    f.properties
        .named(Name::INTENDED_BUSY_STATUS, Value::I32(0));
    f.properties.named(Name::BUSY_STATUS, Value::I32(4));
    f.properties
        .named(Name::APPOINTMENT_NOT_ALLOW_PROPOSE, Value::Bool(false));
    f.properties.categories(&["Work", "", "Urgent, important"]);
    let l = lines(&clean(&f));
    for s in [
        "SUMMARY:Normalized",
        "CREATED:20230101T000000Z",
        "LAST-MODIFIED:20230201T000000Z",
        "DTSTAMP:20240201T000000Z",
        "CATEGORIES:Work,Urgent\\, important",
        "X-MICROSOFT-CDO-IMPORTANCE:2",
        "X-MICROSOFT-CDO-OWNERAPPTID:42",
        "X-MICROSOFT-CDO-INTENDEDSTATUS:FREE",
        "X-MICROSOFT-CDO-BUSYSTATUS:WORKINGELSEWHERE",
        "X-MICROSOFT-DISALLOW-COUNTER:FALSE",
    ] {
        has(&l, s);
    }
    f.class = "IPM.Schedule.Meeting.Resp.Pos".into();
    let l = lines(&clean(&f));
    has(&l, "DTSTAMP:20240202T000000Z");
    assert!(field(&l, "X-MICROSOFT-CDO-INTENDEDSTATUS").is_none());
    let mut f = Fixture::appointment("IPM.Appointment");
    f.properties.categories(&["", ""]);
    assert!(field(&lines(&clean(&f)), "CATEGORIES").is_none());
}

#[test]
fn published_attendee_tracking_and_reply_without_organizer() {
    let mut f = Fixture::appointment("IPM.Appointment");
    f.properties
        .named(Name::APPOINTMENT_STATE_FLAGS, Value::I32(1));
    for (name, status) in [("Bob", 0), ("Carol", 2), ("Dave", 3), ("Erin", 4)] {
        let mut row = recipient(
            1,
            name,
            &format!("{}@example.com", name.to_lowercase()),
            None,
        );
        row.tag(Tag::RECIPIENT_TRACK_STATUS, Value::I32(status));
        f.recipients.push(row);
    }
    let l = lines(&clean(&f));
    has(
        &l,
        "ATTENDEE;ROLE=REQ-PARTICIPANT;CN=Bob:mailto:bob@example.com",
    );
    for (name, status) in [
        ("Carol", "TENTATIVE"),
        ("Dave", "ACCEPTED"),
        ("Erin", "DECLINED"),
    ] {
        has(
            &l,
            &format!(
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT={status};CN={name}:mailto:{}@example.com",
                name.to_lowercase()
            ),
        );
    }
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Resp.Neg");
    f.properties
        .tag(Tag::SENT_REPRESENTING_NAME_W, Value::Text("Bob".into()));
    f.recipients
        .push(recipient(2, "Alice", "alice@example.com", None));
    let l = lines(&clean(&f));
    assert!(field(&l, "ORGANIZER").is_none());
    has(&l, "ATTENDEE;PARTSTAT=DECLINED;CN=Bob:invalid:nomail");
}

#[test]
fn attendee_address_sources_and_sender_fallback() {
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Request");
    let mut exchange = Properties::default();
    exchange.tag(Tag::DISPLAY_NAME_W, Value::Text("Eve".into()));
    exchange.tag(Tag::ADDRTYPE_W, Value::Text("EX".into()));
    exchange.tag(
        Tag::EMAIL_ADDRESS_W,
        Value::Text("/o=Example/cn=Recipients/cn=eve".into()),
    );
    exchange.tag(Tag::SMTP_ADDRESS_W, Value::Text("eve@example.com".into()));
    let mut key = Properties::default();
    key.tag(Tag::DISPLAY_NAME_W, Value::Text("Frank".into()));
    key.tag(
        Tag::SEARCH_KEY,
        Value::Binary(b"SMTP:FRANK@EXAMPLE.COM\0".to_vec()),
    );
    let mut unknown = Properties::default();
    unknown.tag(Tag::DISPLAY_NAME_W, Value::Text("Grace".into()));
    unknown.tag(Tag::ADDRTYPE_W, Value::Text("EX".into()));
    unknown.tag(Tag::SEARCH_KEY, Value::Binary(b"EX:/O=EXAMPLE\0".to_vec()));
    f.recipients = vec![exchange, key, unknown];
    f.properties
        .tag(Tag::SENDER_NAME_W, Value::Text("Sender".into()));
    f.properties.tag(
        Tag::SENDER_EMAIL_ADDRESS_W,
        Value::Text("Sender <sender@example.com>".into()),
    );
    let l = lines(&clean(&f));
    for (name, address) in [
        ("Eve", "mailto:eve@example.com"),
        ("Frank", "mailto:FRANK@EXAMPLE.COM"),
        ("Grace", "invalid:nomail"),
    ] {
        has(
            &l,
            &format!("ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;CN={name}:{address}"),
        );
    }
    has(&l, "ORGANIZER;CN=Sender:mailto:sender@example.com");
}

#[test]
fn recurrence_master_and_embedded_exception_exact_mapping() {
    let mut f = Fixture::recurring(recurrence::weekly());
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded.properties.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 10, 14, 0)),
    );
    embedded
        .properties
        .tag(Tag::BODY_W, Value::Text("Exception body".into()));
    f.attach(2, embedded.bytes(), None);
    let original = f.load();
    let original_bytes = original.attachments()[0].as_bytes().unwrap().to_vec();
    let result = build_calendar(&original, &options()).unwrap();
    assert!(result.losses.is_empty(), "{:?}", result.losses);
    let c = result.calendar.unwrap();
    assert_eq!(c.event_count(), 2);
    assert_eq!(c.exception_attachment_indices(), [0]);
    assert_eq!(
        original.attachments()[0].as_bytes().unwrap(),
        original_bytes
    );
    let l = lines(&c);
    let master = event(&l, 0);
    let exception = event(&l, 1);
    for s in [
        "DTSTART;TZID=Eastern Standard Time:20240701T100000",
        "DTEND;TZID=Eastern Standard Time:20240701T110000",
        "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10",
        "EXDATE;TZID=Eastern Standard Time:20240708T100000",
        "SUMMARY:Team sync",
        "LOCATION:Room 1",
    ] {
        has(master, s);
    }
    assert!(field(master, "RECURRENCE-ID").is_none());
    for s in [
        "RECURRENCE-ID;TZID=Eastern Standard Time:20240710T100000",
        "DTSTART;TZID=Eastern Standard Time:20240710T130000",
        "DTEND;TZID=Eastern Standard Time:20240710T140000",
        "SUMMARY:Moved s\u{fd}nc",
        "LOCATION:Room 2",
        "DESCRIPTION:Exception body",
    ] {
        has(exception, s);
    }
    assert_eq!(field(master, "UID:"), field(exception, "UID:"));
    assert!(field(exception, "RRULE:").is_none());
    assert!(field(exception, "EXDATE").is_none());
}

#[test]
fn recurrence_failures_disable_flag_and_recovery_diagnostics() {
    for bytes in [vec![4, 48, 4, 48, 11], recurrence::weekly()[..112].to_vec()] {
        let result = convert(&Fixture::recurring(bytes.clone()));
        assert_eq!(result.losses.len(), 1, "{:?}", result.losses);
        assert_eq!(result.losses[0].kind, Loss::InvalidCalendarData);
        let l = lines(&result.calendar.unwrap());
        assert_eq!(field(event(&l, 0), "RRULE:").is_some(), bytes.len() == 112);
    }
    let mut f = Fixture::recurring(recurrence::weekly());
    f.properties.named(Name::RECURRING, Value::Bool(false));
    let c = clean(&f);
    assert_eq!(c.event_count(), 1);
    has(&lines(&c), "DTSTART:20240701T140000Z");
    let mut unsupported = recurrence::pattern(10, 1, &[10]);
    unsupported[8..10].copy_from_slice(&6u16.to_le_bytes());
    let result = convert(&Fixture::recurring(unsupported));
    assert_eq!(result.losses[0].kind, Loss::UnsupportedCalendarData);
    assert_eq!(result.calendar.unwrap().event_count(), 1);
    for bytes in [
        recurrence::pattern(0, 0, &[]),
        recurrence::pattern(1, 0, &[2]),
        recurrence::pattern(2, 1, &[32]),
    ] {
        let result = convert(&Fixture::recurring(bytes));
        assert_eq!(result.losses[0].kind, Loss::InvalidCalendarData);
        assert!(field(event(&lines(&result.calendar.unwrap()), 0), "RRULE:").is_none());
    }
}

#[test]
fn recurrence_zone_legacy_fallback_missing_zone_and_all_day() {
    for (description, expected) in [
        (None, "UTC-05:00"),
        (Some(" "), "UTC-05:00"),
        (Some("Eastern Time "), "Eastern Time"),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.named(
            Name::APPOINTMENT_RECUR,
            Value::Binary(recurrence::pattern(0, 1440, &[])),
        );
        f.properties
            .named(Name::TIME_ZONE_STRUCT, Value::Binary(structure()));
        if let Some(description) = description {
            f.properties
                .named(Name::TIME_ZONE_DESCRIPTION, Value::Text(description.into()));
        }
        has(&lines(&clean(&f)), &format!("TZID:{expected}"));
    }
    let mut f = Fixture::appointment("IPM.Appointment");
    f.properties.named(
        Name::APPOINTMENT_RECUR,
        Value::Binary(recurrence::pattern(0, 1440, &[])),
    );
    let result = convert(&f);
    assert_eq!(result.losses.len(), 1);
    has(
        &lines(&result.calendar.unwrap()),
        "DTSTART;TZID=UTC:20240301T150000",
    );
    let mut f = Fixture::recurring(recurrence::weekly());
    f.properties
        .named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(true));
    let c = clean(&f);
    let l = lines(&c);
    has(event(&l, 0), "DTSTART;VALUE=DATE:20240701");
    has(event(&l, 0), "EXDATE;VALUE=DATE:20240708");
    has(event(&l, 1), "RECURRENCE-ID;VALUE=DATE:20240710");
    assert!(!l.iter().any(|s| s == "BEGIN:VTIMEZONE"));
}

#[test]
fn recurrence_id_replace_time_global_id_and_packed_time() {
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Request");
    f.properties.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 10, 14, 0)),
    );
    f.properties.named(
        Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
        Value::Binary(eastern("Eastern", 300)),
    );
    has(
        &lines(&clean(&f)),
        "RECURRENCE-ID;TZID=Eastern:20240710T100000",
    );
    for (zone, all_day, time, expected) in [
        (
            true,
            false,
            Some((10 << 12) | (30 << 6) | 15),
            "RECURRENCE-ID;TZID=Eastern:20240710T103015",
        ),
        (
            false,
            false,
            Some((10 << 12) | (30 << 6) | 15),
            "RECURRENCE-ID:20240710T103015Z",
        ),
        (false, false, None, "RECURRENCE-ID:20240710T000000Z"),
        (
            true,
            true,
            Some(10 << 12),
            "RECURRENCE-ID;VALUE=DATE:20240710",
        ),
        (
            false,
            false,
            Some(24 << 12),
            "RECURRENCE-ID:20240710T000000Z",
        ),
        (
            false,
            false,
            Some(60 << 6),
            "RECURRENCE-ID:20240710T000000Z",
        ),
        (false, false, Some(60), "RECURRENCE-ID:20240710T000000Z"),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.named(Name::IS_EXCEPTION, Value::Bool(true));
        f.properties.named(
            Name::GLOBAL_OBJECT_ID,
            Value::Binary(global_id(&[], 2024, 7, 10)),
        );
        f.properties
            .named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(all_day));
        if zone {
            f.properties.named(
                Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
                Value::Binary(eastern("Eastern", 300)),
            );
        }
        if let Some(time) = time {
            f.properties
                .named(Name::START_RECURRENCE_TIME, Value::I32(time));
        }
        has(&lines(&convert(&f).calendar.unwrap()), expected);
    }
    for (year, month, day) in [
        (0, 7, 10),
        (1600, 7, 10),
        (10000, 7, 10),
        (2024, 0, 10),
        (2024, 13, 10),
        (2024, 7, 0),
        (2023, 2, 29),
    ] {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.named(Name::IS_EXCEPTION, Value::Bool(true));
        f.properties.named(
            Name::GLOBAL_OBJECT_ID,
            Value::Binary(global_id(&[], year, month, day)),
        );
        let result = convert(&f);
        assert!(!result.losses.is_empty());
        assert!(field(&lines(&result.calendar.unwrap()), "RECURRENCE-ID").is_none());
    }
}

fn overrides() -> Vec<u8> {
    let mut bytes = recurrence::master(1, 1, &[0x0a]);
    recurrence::put16(&mut bytes, 3);
    for (day, flags, values) in [(2, 0x2c, vec![5, 1, 0]), (7, 8, vec![0]), (9, 8, vec![1])] {
        let start = 222737760 + day * 1440 + 600;
        for value in [start, start + 60, start] {
            recurrence::put32(&mut bytes, value);
        }
        recurrence::put16(&mut bytes, flags);
        for value in values {
            recurrence::put32(&mut bytes, value);
        }
    }
    recurrence::put32(&mut bytes, 0);
    for _ in 0..3 {
        recurrence::put32(&mut bytes, 0);
        recurrence::put32(&mut bytes, 0);
    }
    bytes
}

#[test]
fn exception_reminder_and_busy_overrides() {
    let c = clean(&Fixture::recurring(overrides()));
    assert_eq!(c.event_count(), 4);
    let l = lines(&c);
    has(event(&l, 1), "TRANSP:TRANSPARENT");
    has(event(&l, 1), "TRIGGER:-PT5M");
    assert!(field(event(&l, 2), "TRIGGER:").is_none());
    has(event(&l, 3), "TRIGGER:-PT15M");
}

#[test]
fn exception_attachments_overlay_fallback_matching_and_invalid_content() {
    let mut f = Fixture::recurring(overrides());
    f.attach(0, Fixture::empty("IPM.Appointment").bytes(), None);
    let mut embedded = Fixture::empty("IPM.Appointment");
    let p = &mut embedded.properties;
    p.tag(Tag::SUBJECT_W, Value::Text("Attachment subject".into()));
    p.tag(Tag::BODY_W, Value::Text("Attachment body".into()));
    p.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 3, 14, 0)),
    );
    p.named(Name::LOCATION, Value::Text("Room 9".into()));
    p.named(Name::BUSY_STATUS, Value::I32(3));
    p.named(Name::REMINDER_SET, Value::Bool(true));
    p.named(Name::REMINDER_DELTA, Value::I32(20));
    p.named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(false));
    p.named(
        Name::APPOINTMENT_START_WHOLE,
        Value::DateTime(date(2024, 7, 3, 15, 0)),
    );
    p.named(
        Name::APPOINTMENT_END_WHOLE,
        Value::DateTime(date(2024, 7, 3, 16, 30)),
    );
    f.attach(2, embedded.bytes(), None);
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded
        .properties
        .tag(Tag::BODY_W, Value::Text("Second body".into()));
    embedded
        .properties
        .named(Name::REMINDER_SET, Value::Bool(false));
    f.attach(2, embedded.bytes(), Some(date(2024, 7, 8, 10, 0)));
    f.attach(2, vec![1, 2, 3, 4, 5, 6], None);
    let result = convert(&f);
    assert!(result.losses.iter().any(|l| l.attachment_index == Some(3) && l.description.contains("could not be loaded")));
    let c = result.calendar.unwrap();
    assert_eq!(c.exception_attachment_indices(), [1, 2]);
    let l = lines(&c);
    for s in [
        "SUMMARY:Attachment subject",
        "DESCRIPTION:Attachment body",
        "LOCATION:Room 9",
        "X-MICROSOFT-CDO-BUSYSTATUS:OOF",
        "TRIGGER:-PT20M",
        "DTSTART;TZID=Eastern Standard Time:20240703T110000",
        "DTEND;TZID=Eastern Standard Time:20240703T123000",
    ] {
        has(event(&l, 1), s);
    }
    has(event(&l, 2), "DESCRIPTION:Second body");
    has(event(&l, 2), "SUMMARY:Team sync");
    assert!(field(event(&l, 2), "TRIGGER:").is_none());
}

#[test]
fn attachment_matching_strict_minute_boundary_present_invalid_and_no_reuse() {
    for (delta, matched) in [(599_999_999, true), (600_000_000, false)] {
        let mut f = Fixture::recurring(recurrence::weekly());
        let mut embedded = Fixture::empty("IPM.Appointment");
        let replace =
            Date::from_ticks(date(2024, 7, 10, 14, 0).ticks() + delta, Kind::Utc).unwrap();
        embedded
            .properties
            .named(Name::EXCEPTION_REPLACE_TIME, Value::DateTime(replace));
        f.attach(2, embedded.bytes(), Some(date(2024, 7, 10, 13, 0)));
        assert_eq!(
            clean(&f).exception_attachment_indices().len(),
            usize::from(matched)
        );
    }
    let mut f = Fixture::recurring(recurrence::weekly());
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded
        .properties
        .named(Name::EXCEPTION_REPLACE_TIME, Value::Text("invalid".into()));
    f.attach(2, embedded.bytes(), Some(date(2024, 7, 10, 13, 0)));
    let result = convert(&f);
    assert!(!result.losses.is_empty());
    assert!(
        result
            .calendar
            .unwrap()
            .exception_attachment_indices()
            .is_empty()
    );
    let mut bytes = recurrence::many(2);
    // Give both occurrences the same original UTC date; one attachment is not reusable.
    let offset = 70 + 2 * 8 + 2;
    let first = bytes[offset + 8..offset + 12].to_vec();
    bytes[offset + 14 + 8..offset + 14 + 12].copy_from_slice(&first);
    let mut f = Fixture::recurring(bytes);
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded.properties.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 1, 14, 0)),
    );
    embedded
        .properties
        .tag(Tag::SUBJECT_W, Value::Text("Used once".into()));
    f.attach(2, embedded.bytes(), None);
    let c = clean(&f);
    assert_eq!(c.exception_attachment_indices(), [0]);
    assert_eq!(
        lines(&c)
            .iter()
            .filter(|s| *s == "SUMMARY:Used once")
            .count(),
        1
    );
}

#[test]
fn count_and_exact_serialized_byte_limits_preserve_unconsumed_attachments() {
    let mut f = Fixture::recurring(recurrence::weekly());
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded.properties.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 10, 14, 0)),
    );
    embedded.properties.tag(
        Tag::BODY_W,
        Value::Text("Unicode \u{1f600}, folding; ".repeat(90)),
    );
    f.attach(2, embedded.bytes(), None);
    let full = clean(&f);
    let bytes = raw(&full);
    let starts: Vec<usize> = bytes
        .match_indices("BEGIN:VEVENT\r\n")
        .map(|(i, _)| i)
        .collect();
    let end =
        starts[1] + bytes[starts[1]..].find("END:VEVENT\r\n").unwrap() + "END:VEVENT\r\n".len();
    let size = (end - starts[1]) as u64;
    assert_eq!(full.exceptions_size(), size);
    for (budget, expected) in [(0, 1), (10, 1), (size - 1, 1), (size, 2), (u64::MAX, 2)] {
        let result = build_calendar(
            &f.load(),
            &CalendarOptions {
                max_exceptions_size: budget,
                ..options()
            },
        )
        .unwrap();
        let c = result.calendar.unwrap();
        assert_eq!(c.event_count(), expected);
        assert_eq!(c.exception_attachment_indices().len(), expected - 1);
        assert_eq!(result.losses.is_empty(), expected == 2);
        assert!(c.exceptions_size() <= budget);
    }
    let result = build_calendar(
        &f.load(),
        &CalendarOptions {
            max_exceptions: 0,
            ..options()
        },
    )
    .unwrap();
    let c = result.calendar.unwrap();
    assert_eq!(c.event_count(), 1);
    assert!(c.exception_attachment_indices().is_empty());
    assert_eq!(result.losses[0].kind, Loss::CalendarExceptionLimitExceeded);
    // Limits do not change modified-date membership in the master's EXDATEs.
    assert_eq!(
        event(&lines(&c), 0)
            .iter()
            .filter(|s| s.starts_with("EXDATE"))
            .count(),
        1
    );
}

#[test]
fn many_exceptions_and_description_amplification_are_bounded() {
    let c = convert(&Fixture::recurring(recurrence::many(2000)));
    assert_eq!(c.losses.len(), 1);
    assert_eq!(c.losses[0].kind, Loss::CalendarExceptionLimitExceeded);
    let c = c.calendar.unwrap();
    assert_eq!(c.event_count(), 1025);
    assert!(field(&lines(&c), "EXDATE").is_none());
    let mut f = Fixture::recurring(recurrence::many(200));
    f.properties
        .tag(Tag::BODY_W, Value::Text("x".repeat(16 * 1024)));
    let budget = 256 * 1024;
    let result = build_calendar(
        &f.load(),
        &CalendarOptions {
            max_exceptions_size: budget,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(result.losses.len(), 1);
    let c = result.calendar.unwrap();
    assert!(c.event_count() > 1 && c.event_count() < 201);
    assert!(c.exceptions_size() <= budget);
    assert!(raw(&c).len() < budget as usize + 2 * (16 * 1024 + 1024));
}

#[test]
fn calendar_control_characters_cannot_inject_content_lines() {
    let payload = "Sync\r\nEND:VEVENT\r\nBEGIN:VEVENT\rUID:evil\nX-EVIL:1\0\x07\x1b";
    let mut f = Fixture::appointment("IPM.Schedule.Meeting.Request");
    f.properties
        .tag(Tag::NORMALIZED_SUBJECT_W, Value::Text(payload.into()));
    f.properties
        .named(Name::LOCATION, Value::Text(payload.into()));
    f.recipients.push(recipient(
        1,
        "Bob\r\nX-EVIL:1;\"x\":",
        "bob@example.com",
        None,
    ));
    let l = lines(&clean(&f));
    assert_eq!(l.iter().filter(|s| *s == "BEGIN:VEVENT").count(), 1);
    assert_eq!(l.iter().filter(|s| *s == "END:VEVENT").count(), 1);
    assert_eq!(l.iter().filter(|s| s.starts_with("UID:")).count(), 1);
    assert!(!l.iter().any(|s| s.starts_with("X-EVIL")));
    assert!(!l.iter().any(|s| s.chars().any(|c| c < ' ' && c != '\t')));
}

#[test]
fn malformed_global_ids_and_extreme_dates_recover_without_panics() {
    let valid = global_id(b"vCal-Uid\x01\0\0\0abc\0", 2024, 7, 10);
    let mut ids = vec![
        vec![],
        vec![0; 39],
        valid.clone(),
        valid[..48].to_vec(),
        global_id(&[], 65535, 255, 255),
    ];
    for size in [-1i32, i32::MIN, i32::MAX, 0x10000, 12, 13, 8] {
        let mut id = valid.clone();
        id[36..40].copy_from_slice(&size.to_le_bytes());
        ids.push(id);
    }
    for id in ids {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties
            .named(Name::GLOBAL_OBJECT_ID, Value::Binary(id));
        assert!(field(&lines(&clean(&f)), "UID:").unwrap().len() > 4);
    }
    for offset in [42, 54, 58, 62, 74] {
        let mut bytes = recurrence::weekly();
        bytes[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let result = convert(&Fixture::recurring(bytes));
        if let Some(c) = result.calendar {
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
    let mut f = Fixture::empty("IPM.Appointment");
    f.properties.named(
        Name::APPOINTMENT_START_WHOLE,
        Value::DateTime(date(9999, 12, 31, 23, 59)),
    );
    f.properties
        .named(Name::APPOINTMENT_DURATION, Value::I32(1));
    let result = convert(&f);
    assert!(result.calendar.is_none());
    assert!(!result.losses.is_empty());
}

#[test]
fn truncation_mutation_and_timezone_mutation_end_to_end() {
    let bytes = recurrence::weekly();
    for len in 0..bytes.len() {
        let result = convert(&Fixture::recurring(bytes[..len].to_vec()));
        if let Some(c) = result.calendar {
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
    let mut rng = 0x31415926u32;
    for _ in 0..1000 {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut mutated = bytes.clone();
        let index = rng as usize % mutated.len();
        mutated[index] ^= (rng >> 24) as u8;
        let result = build_calendar(
            &Fixture::recurring(mutated).load(),
            &CalendarOptions {
                max_exceptions: 4,
                max_exceptions_size: 8192,
                ..options()
            },
        )
        .unwrap();
        if let Some(c) = result.calendar {
            assert!(c.event_count() <= 5);
            assert!(c.exceptions_size() <= 8192);
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
    let zone = eastern("Eastern", 300);
    for len in 0..zone.len() {
        let mut f = Fixture::appointment("IPM.Appointment");
        f.properties.named(
            Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
            Value::Binary(zone[..len].to_vec()),
        );
        let result = convert(&f);
        if let Some(c) = result.calendar {
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
    for _ in 0..200 {
        let mut mutated = zone.clone();
        for _ in 0..4 {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let index = rng as usize % mutated.len();
            mutated[index] ^= (rng >> 24) as u8;
        }
        let mut f = Fixture::recurring(recurrence::weekly());
        f.properties.items.remove(5); // Replace the fixture's valid recurrence zone.
        f.properties.named(
            Name::APPOINTMENT_TIME_ZONE_DEFINITION_RECUR,
            Value::Binary(mutated),
        );
        let result = convert(&f);
        if let Some(c) = result.calendar {
            assert!(c.event_count() <= 2);
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
}

#[test]
fn body_charsets_replacements_and_empty_exception_body() {
    fn body(f: &mut Fixture, tag: Tag, bytes: &[u8]) {
        f.properties
            .items
            .retain(|p| p[2..4] != tag.0.to_le_bytes()[2..4]);
        f.properties.tag(tag, Value::Binary(bytes.to_vec()));
        f.properties.items.last_mut().unwrap()[..4].copy_from_slice(&tag.0.to_le_bytes());
    }
    let mut f = Fixture::appointment("IPM.Appointment");
    body(&mut f, Tag::BODY_A, b"Caf\xe9\r\n \t\0");
    has(&lines(&clean(&f)), "DESCRIPTION:Caf\u{e9}");
    body(&mut f, Tag::BODY_W, &[0xff, 0xfe, b'x', 0, 0, 0]);
    has(&lines(&clean(&f)), "DESCRIPTION:x");
    body(&mut f, Tag::BODY_A, b"\xef\xbb\xbfx");
    has(&lines(&clean(&f)), "DESCRIPTION:\u{ef}\u{bb}\u{bf}x");
    body(&mut f, Tag::BODY_W, &[b'x', 0, 0, 0xd8]);
    let result = convert(&f);
    assert!(
        result
            .losses
            .iter()
            .any(|l| l.description.contains("replacement"))
    );
    has(&lines(&result.calendar.unwrap()), "DESCRIPTION:x\u{fffd}");
    // The captured body charset takes precedence over the export fallback.
    body(&mut f, Tag::BODY_A, b"Caf\xe9");
    let result = build_calendar(
        &f.load(),
        &CalendarOptions {
            fallback_charset: mimecraft::charsets::Charset::UTF8,
            ..options()
        },
    )
    .unwrap();
    assert!(result.losses.is_empty());
    has(&lines(&result.calendar.unwrap()), "DESCRIPTION:Caf\u{e9}");

    let mut f = Fixture::recurring(recurrence::many(1));
    f.properties
        .tag(Tag::BODY_W, Value::Text("Master description".into()));
    let mut embedded = Fixture::empty("IPM.Appointment");
    embedded.properties.named(
        Name::EXCEPTION_REPLACE_TIME,
        Value::DateTime(date(2024, 7, 1, 14, 0)),
    );
    embedded
        .properties
        .tag(Tag::BODY_W, Value::Text(" \r\n\t\0".into()));
    f.attach(2, embedded.bytes(), None);
    let l = lines(&clean(&f));
    has(event(&l, 1), "DESCRIPTION:Master description");
}

#[test]
fn named_property_identity_and_invalid_optional_values() {
    let mut f = Fixture::appointment("IPM.Appointment");
    let mut wrong = Name::LOCATION;
    wrong.guid[0] ^= 1;
    f.properties
        .named(wrong, Value::Text("Wrong property set".into()));
    let mut keywords = Name::keywords();
    keywords.name = mimecraft::tnef::TnefName::String("keywords".into());
    f.properties
        .named(keywords, Value::Text("Wrong case".into()));
    let l = lines(&clean(&f));
    assert!(field(&l, "LOCATION").is_none());
    assert!(field(&l, "CATEGORIES").is_none());
    f.properties.named(
        Name::APPOINTMENT_SEQUENCE,
        Value::Text("not a number".into()),
    );
    f.properties
        .named(Name::REMINDER_SET, Value::Text("not a boolean".into()));
    f.properties.items.remove(3); // Replace the fixture's valid end time.
    f.properties
        .named(Name::APPOINTMENT_END_WHOLE, Value::SysTime(i64::MAX));
    let result = convert(&f);
    assert_eq!(result.losses.len(), 3);
    assert!(
        result
            .losses
            .iter()
            .all(|l| l.kind == Loss::InvalidCalendarData)
    );
    let l = lines(&result.calendar.unwrap());
    has(&l, "SEQUENCE:0");
    assert!(field(&l, "DTEND").is_none());
    assert!(field(&l, "TRIGGER").is_none());
}

#[test]
fn stamp_fallback_precedence_and_clock() {
    for reply in [false, true] {
        let mut f = Fixture::empty(if reply {
            "IPM.Schedule.Meeting.Resp.Pos"
        } else {
            "IPM.Appointment"
        });
        f.properties.named(
            Name::APPOINTMENT_START_WHOLE,
            Value::DateTime(date(2024, 3, 1, 15, 0)),
        );
        has(&lines(&clean(&f)), "DTSTAMP:20240102T030405Z");
        let clock = build_calendar(&f.load(), &CalendarOptions::default()).unwrap();
        let l = lines(&clock.calendar.unwrap());
        let clock = field(&l, "DTSTAMP:").unwrap();
        assert!(clock.ends_with('Z'));
        assert_eq!(clock.len(), 24);
        assert!(!clock.contains("00010101"));
        f.properties.tag(
            Tag::LAST_MODIFICATION_TIME,
            Value::DateTime(date(2024, 1, 3, 0, 0)),
        );
        has(&lines(&clean(&f)), "DTSTAMP:20240103T000000Z");
        f.properties.tag(
            Tag::CLIENT_SUBMIT_TIME,
            Value::DateTime(date(2024, 1, 4, 0, 0)),
        );
        has(&lines(&clean(&f)), "DTSTAMP:20240104T000000Z");
        f.properties.named(
            if reply {
                Name::OWNER_CRITICAL_CHANGE
            } else {
                Name::ATTENDEE_CRITICAL_CHANGE
            },
            Value::DateTime(date(2024, 1, 5, 0, 0)),
        );
        has(&lines(&clean(&f)), "DTSTAMP:20240104T000000Z");
        f.properties.named(
            if reply {
                Name::ATTENDEE_CRITICAL_CHANGE
            } else {
                Name::OWNER_CRITICAL_CHANGE
            },
            Value::DateTime(date(2024, 1, 6, 0, 0)),
        );
        has(&lines(&clean(&f)), "DTSTAMP:20240106T000000Z");
    }
}

#[test]
fn invalid_zone_definitions_fall_back_without_discarding_event() {
    let mut f = Fixture::appointment("IPM.Appointment");
    f.properties.named(
        Name::APPOINTMENT_TIME_ZONE_DEFINITION_START_DISPLAY,
        Value::Binary(eastern("Eastern", 300)),
    );
    f.properties.named(
        Name::APPOINTMENT_TIME_ZONE_DEFINITION_END_DISPLAY,
        Value::Binary(vec![0xff]),
    );
    let result = convert(&f);
    assert_eq!(result.losses.len(), 1);
    has(
        &lines(&result.calendar.unwrap()),
        "DTEND;TZID=Eastern:20240301T110000",
    );
    let mut f = Fixture::recurring(recurrence::weekly());
    f.properties.items.remove(5); // Replace the fixture's valid recurrence zone.
    f.properties.named(
        Name::APPOINTMENT_TIME_ZONE_DEFINITION_RECUR,
        Value::Binary(vec![0xff]),
    );
    f.properties
        .named(Name::TIME_ZONE_STRUCT, Value::Binary(structure()));
    f.properties
        .named(Name::TIME_ZONE_DESCRIPTION, Value::Text("Legacy".into()));
    let result = convert(&f);
    assert_eq!(result.losses.len(), 1);
    let c = result.calendar.unwrap();
    assert_eq!(c.event_count(), 2);
    has(&lines(&c), "DTSTART;TZID=Legacy:20240701T100000");
}

#[test]
fn all_day_exception_subtype_preserves_series_recurrence_id() {
    for master_all_day in [false, true] {
        for explicit_dates in [false, true] {
            let mut f = Fixture::recurring(recurrence::many(1));
            f.properties
                .named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(master_all_day));
            let mut embedded = Fixture::empty("IPM.Appointment");
            embedded.properties.named(
                Name::EXCEPTION_REPLACE_TIME,
                Value::DateTime(date(2024, 7, 1, 14, 0)),
            );
            embedded
                .properties
                .named(Name::APPOINTMENT_SUB_TYPE, Value::Bool(!master_all_day));
            if explicit_dates {
                embedded.properties.named(
                    Name::APPOINTMENT_START_WHOLE,
                    Value::DateTime(date(2024, 7, 2, 14, 0)),
                );
                embedded.properties.named(
                    Name::APPOINTMENT_END_WHOLE,
                    Value::DateTime(date(2024, 7, 2, 15, 0)),
                );
            }
            f.attach(2, embedded.bytes(), None);
            let c = clean(&f);
            assert_eq!(c.exception_attachment_indices(), &[0]);
            let l = lines(&c);
            let e = event(&l, 1);
            has(
                e,
                if master_all_day {
                    "RECURRENCE-ID;VALUE=DATE:20240701"
                } else {
                    "RECURRENCE-ID;TZID=Eastern Standard Time:20240701T100000"
                },
            );
            has(
                e,
                match (master_all_day, explicit_dates) {
                    (false, false) => "DTSTART;TZID=Eastern Standard Time:20240701T130000",
                    (false, true) => "DTSTART;VALUE=DATE:20240702",
                    (true, false) => "DTSTART;VALUE=DATE:20240701",
                    (true, true) => "DTSTART;TZID=Eastern Standard Time:20240702T100000",
                },
            );
        }
    }
}

#[test]
fn zero_exception_limit_avoids_embedded_loading_and_huge_counts_recover() {
    let defaults = CalendarOptions::default().clone();
    assert!(defaults.generate_calendar);
    assert_eq!(defaults.max_exceptions, 1024);
    assert_eq!(defaults.max_exceptions_size, 16 * 1024 * 1024);
    assert!(defaults.fallback_stamp.is_none());
    let mut f = Fixture::recurring(recurrence::many(1));
    f.attach(2, vec![0xff; 20], None);
    let result = build_calendar(
        &f.load(),
        &CalendarOptions {
            max_exceptions: 0,
            ..options()
        },
    )
    .unwrap();
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::CalendarExceptionLimitExceeded);
    assert_eq!(result.calendar.unwrap().event_count(), 1);
    let result = convert(&Fixture::recurring(recurrence::many(0)));
    assert!(result.losses.is_empty());
    assert_eq!(result.calendar.unwrap().event_count(), 1);
    for count in [u32::MAX, 0x7fffffff, 0x10000000] {
        let mut bytes = recurrence::many(2);
        bytes[38..42].copy_from_slice(&count.to_le_bytes());
        let result = convert(&Fixture::recurring(bytes));
        assert!(!result.losses.is_empty());
        assert_eq!(result.calendar.unwrap().event_count(), 1);
    }
    // Claims 65535 exceptions, but carries data for only one.
    let mut bytes = recurrence::many(1);
    bytes[78..80].copy_from_slice(&u16::MAX.to_le_bytes());
    let result = convert(&Fixture::recurring(bytes));
    assert_eq!(result.losses.len(), 1);
    assert_eq!(result.losses[0].kind, Loss::InvalidCalendarData);
    let c = result.calendar.unwrap();
    assert_eq!(c.event_count(), 1);
    assert!(field(&lines(&c), "RRULE:").is_some());
    for offset in [u32::MAX, 0x7fff_ffff, 0x8000_0000] {
        let mut bytes = recurrence::many(2);
        bytes[78..82].copy_from_slice(&offset.to_le_bytes());
        let result = convert(&Fixture::recurring(bytes));
        if let Some(c) = result.calendar {
            lines(&c);
        } else {
            assert!(!result.losses.is_empty());
        }
    }
}

#[test]
fn cancellation_and_writer_failures_are_propagated() {
    let f = Fixture::recurring(recurrence::many(4)).load();
    for stop in 0..5 {
        let mut calls = 0;
        let error = build_calendar_with_check(&f, &options(), || {
            if calls == stop {
                return Err(io::Error::other("cancelled"));
            }
            calls += 1;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "cancelled");
    }
    struct Broken {
        flush: bool,
    }
    impl Write for Broken {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.flush {
                Ok(bytes.len())
            } else {
                Err(io::Error::other("write failed"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush failed"))
        }
    }
    let c = clean(&Fixture::appointment("IPM.Appointment"));
    assert_eq!(
        c.write_to(Broken { flush: false }).unwrap_err().to_string(),
        "write failed"
    );
    assert_eq!(
        c.write_to(Broken { flush: true }).unwrap_err().to_string(),
        "flush failed"
    );
    assert_eq!(raw(&c), raw(&c.clone()));
}

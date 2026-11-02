// Adapted from MimeKit's TnefTimeZoneTests, TnefCalendarTests, and
// TnefCalendarRobustnessTests; additional transition and boundary regressions.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefDateTime as Date, TnefDateTimeError as DateError, TnefDateTimeKind as Kind,
    TnefSystemTime as SystemTime, TnefTimeZone as Zone, TnefTimeZoneError as Error,
    calendar::CalendarWriter,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{self, Write},
};

// The first Sunday of November and second Sunday of March, both at 2:00.
const NOVEMBER: SystemTime = SystemTime {
    year: 0,
    month: 11,
    day_of_week: 0,
    day: 1,
    hour: 2,
    minute: 0,
    second: 0,
    milliseconds: 0,
};
const MARCH: SystemTime = SystemTime {
    month: 3,
    day: 2,
    ..NOVEMBER
};
const EASTERN_CALENDAR: &str = concat!(
    "BEGIN:VTIMEZONE\r\nTZID:Eastern Standard Time\r\n",
    "BEGIN:STANDARD\r\nDTSTART:16011104T020000\r\n",
    "RRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\r\n",
    "TZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nEND:STANDARD\r\n",
    "BEGIN:DAYLIGHT\r\nDTSTART:16010311T020000\r\n",
    "RRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\n",
    "TZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nEND:DAYLIGHT\r\n",
    "END:VTIMEZONE\r\n",
);

fn eastern() -> Zone {
    Zone::new("Eastern Standard Time", 300, 0, -60, NOVEMBER, MARCH).unwrap()
}

fn date(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Date {
    Date::new(year, month, day, hour, minute, 0, Kind::Utc).unwrap()
}

fn calendar(zone: &Zone) -> String {
    let mut writer = CalendarWriter::new(Vec::new());
    zone.write_to(&mut writer).unwrap();
    writer.finish().unwrap();
    String::from_utf8(writer.into_inner()).unwrap()
}

fn system_bytes(date: SystemTime) -> Vec<u8> {
    [
        date.year,
        date.month,
        date.day_of_week,
        date.day,
        date.hour,
        date.minute,
        date.second,
        date.milliseconds,
    ]
    .into_iter()
    .flat_map(u16::to_le_bytes)
    .collect()
}

fn put_u16(bytes: &mut [u8], index: usize, value: u16) {
    bytes[index..index + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], index: usize, value: i32) {
    bytes[index..index + 4].copy_from_slice(&value.to_le_bytes());
}

fn structure() -> Vec<u8> {
    let mut bytes = vec![0; 48];
    put_i32(&mut bytes, 0, 300);
    put_i32(&mut bytes, 8, -60);
    bytes[14..30].copy_from_slice(&system_bytes(NOVEMBER));
    bytes[32..48].copy_from_slice(&system_bytes(MARCH));
    bytes
}

fn rule(bias: i32, effective: bool) -> Vec<u8> {
    let mut bytes = vec![0; 66];
    bytes[..4].copy_from_slice(&[2, 1, 0x3e, 0]);
    put_u16(&mut bytes, 4, if effective { 2 } else { 0 });
    put_u16(&mut bytes, 6, 2007);
    put_i32(&mut bytes, 22, bias);
    put_i32(&mut bytes, 30, -60);
    bytes[34..50].copy_from_slice(&system_bytes(NOVEMBER));
    bytes[50..66].copy_from_slice(&system_bytes(MARCH));
    bytes
}

fn definition(key: &str, rules: &[Vec<u8>]) -> Vec<u8> {
    let key: Vec<u16> = key.encode_utf16().collect();
    let mut bytes = vec![2, 1];
    bytes.extend_from_slice(&(6 + 2 * key.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
    bytes.extend(key.into_iter().flat_map(u16::to_le_bytes));
    bytes.extend_from_slice(&(rules.len() as u16).to_le_bytes());
    for rule in rules {
        bytes.extend_from_slice(rule);
    }
    bytes
}

#[test]
fn system_time_equality_and_raw_reading() {
    let hash = |value: SystemTime| {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    let bytes = system_bytes(NOVEMBER);
    let copy = SystemTime::read(&bytes).unwrap();
    assert_eq!(copy, NOVEMBER);
    assert_eq!(hash(copy), hash(NOVEMBER));
    for field in 0..8 {
        let mut other = bytes.clone();
        other[field * 2] = other[field * 2].wrapping_add(1);
        assert_ne!(SystemTime::read(&other).unwrap(), NOVEMBER, "{field}");
    }
    for len in 0..16 {
        assert_eq!(SystemTime::read(&bytes[..len]), Err(Error::IncompleteValue));
    }
    let raw = SystemTime::read(&[0xff; 17]).unwrap();
    assert_eq!(raw.year, u16::MAX);
    assert_eq!(raw.milliseconds, u16::MAX);
    assert!(!raw.is_valid_transition());
}

#[test]
fn same_rules_ignore_names_but_compare_every_used_field() {
    let eastern = eastern();
    assert!(eastern.has_same_rules(&eastern));
    let mut other = eastern.clone();
    other.set_id("Other");
    assert_ne!(eastern, other);
    assert_eq!(other.id(), "Other");
    assert!(eastern.has_same_rules(&other));
    for (bias, standard_bias, daylight_bias, standard, daylight) in [
        (360, 0, -60, NOVEMBER, MARCH),
        (300, 1, -60, NOVEMBER, MARCH),
        (300, 0, -30, NOVEMBER, MARCH),
        (
            300,
            0,
            -60,
            SystemTime {
                month: 10,
                day: 5,
                ..NOVEMBER
            },
            MARCH,
        ),
        (
            300,
            0,
            -60,
            NOVEMBER,
            SystemTime {
                month: 4,
                day: 1,
                ..MARCH
            },
        ),
        (300, 0, -60, NOVEMBER, SystemTime::default()),
        (
            300,
            0,
            -60,
            SystemTime {
                milliseconds: 1,
                ..NOVEMBER
            },
            MARCH,
        ),
    ] {
        let other = Zone::new(
            "Eastern",
            bias,
            standard_bias,
            daylight_bias,
            standard,
            daylight,
        )
        .unwrap();
        assert!(!eastern.has_same_rules(&other));
    }
    assert_eq!(
        (
            eastern.bias(),
            eastern.standard_bias(),
            eastern.daylight_bias()
        ),
        (300, 0, -60)
    );
    assert_eq!(eastern.standard_date(), NOVEMBER);
    assert_eq!(eastern.daylight_date(), MARCH);
}

#[test]
fn same_rules_without_daylight_time_ignore_unused_fields() {
    let standard = Zone::new(
        "Standard",
        300,
        0,
        0,
        SystemTime::default(),
        SystemTime::default(),
    )
    .unwrap();
    assert!(!standard.has_daylight_time());
    assert_eq!(standard.standard_offset(), standard.daylight_offset());
    // Without daylight time, the daylight bias and transition dates are ignored.
    for (first, second) in [
        (NOVEMBER, SystemTime::default()),
        (SystemTime::default(), MARCH),
    ] {
        let other = Zone::new("Other", 300, 0, -60, first, second).unwrap();
        assert!(standard.has_same_rules(&other));
        assert_eq!(other.standard_date(), SystemTime::default());
        assert_eq!(other.daylight_date(), SystemTime::default());
    }
    assert!(
        !standard.has_same_rules(
            &Zone::new(
                "Other",
                360,
                0,
                0,
                SystemTime::default(),
                SystemTime::default()
            )
            .unwrap()
        )
    );
    assert!(!standard.has_same_rules(&eastern()));
}

#[test]
fn parses_both_upstream_eastern_fixtures_and_writes_expected_vtimezone() {
    let expected = eastern();
    let legacy = structure();
    let modern = definition("Eastern Standard Time", &[rule(300, true)]);
    for actual in [
        Zone::from_time_zone_struct(&legacy, expected.id()).unwrap(),
        Zone::from_time_zone_definition(&modern, None).unwrap(),
    ] {
        assert_eq!(actual, expected);
        assert_eq!(calendar(&actual), EASTERN_CALENDAR);
        // July is in daylight saving time; December is not.
        assert_eq!(
            actual.to_local_time(date(2024, 7, 1, 14, 0)).unwrap(),
            date(2024, 7, 1, 10, 0).with_kind(Kind::Unspecified)
        );
        assert_eq!(
            actual.to_local_time(date(2024, 12, 2, 15, 0)).unwrap(),
            date(2024, 12, 2, 10, 0).with_kind(Kind::Unspecified)
        );
    }
    for len in 0..48 {
        assert_eq!(
            Zone::from_time_zone_struct(&legacy[..len], ""),
            Err(Error::IncompleteValue)
        );
    }
    let mut extended = legacy.clone();
    extended.extend_from_slice(&[0xff; 20]);
    assert_eq!(
        Zone::from_time_zone_struct(&extended, expected.id()).unwrap(),
        expected
    );
}

#[test]
fn definition_selects_effective_or_last_complete_declared_rule() {
    for (rules, expected_bias) in [
        (
            vec![rule(300, false), rule(360, true), rule(420, false)],
            360,
        ),
        (vec![rule(300, true), rule(360, true)], 300),
        (vec![rule(300, false), rule(360, false)], 360),
    ] {
        let bytes = definition("Zone", &rules);
        assert_eq!(
            Zone::from_time_zone_definition(&bytes, None)
                .unwrap()
                .bias(),
            expected_bias
        );
    }
    let mut bytes = definition("Zone", &[rule(300, false), rule(360, false)]);
    bytes.pop();
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, None)
            .unwrap()
            .bias(),
        300
    );
    // Undeclared trailing rules must not be read.
    let mut bytes = definition("Zone", &[rule(300, false)]);
    bytes.extend_from_slice(&rule(360, true));
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, None)
            .unwrap()
            .bias(),
        300
    );
    let mut bytes = definition("Zone", &[rule(300, true), rule(i32::MAX, true)]);
    assert!(Zone::from_time_zone_definition(&bytes, None).is_ok());
    put_u16(&mut bytes, 18 + 4, 0); // Unmark the first rule (header is 18 bytes).
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, None),
        Err(Error::InvalidBias)
    );
}

#[test]
fn definition_header_errors_and_id_recovery_match_upstream() {
    let valid = definition("Zone", &[rule(300, true)]);
    for len in 0..valid.len() {
        assert!(
            Zone::from_time_zone_definition(&valid[..len], None).is_err(),
            "{len}"
        );
    }
    let mut bytes = valid.clone();
    bytes[0] = 3;
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, None),
        Err(Error::InvalidVersion)
    );
    for header in [0, 5, u16::MAX] {
        let mut bytes = valid.clone();
        put_u16(&mut bytes, 2, header);
        assert_eq!(
            Zone::from_time_zone_definition(&bytes, None),
            Err(Error::InvalidHeader)
        );
    }
    assert_eq!(
        Zone::from_time_zone_definition(&definition("", &[]), None),
        Err(Error::NoRule)
    );
    for (key, fallback, expected) in [
        ("  Eastern \0\0", Some("Fallback"), "Eastern"),
        ("", Some(" Fallback "), "Fallback"),
        (" \t", Some(" "), "UTC-05:00"),
        ("", None, "UTC-05:00"),
        (" \u{20ac}\u{1f600} ", None, "\u{20ac}\u{1f600}"),
    ] {
        assert_eq!(
            Zone::from_time_zone_definition(&definition(key, &[rule(300, true)]), fallback)
                .unwrap()
                .id(),
            expected
        );
    }
    let mut bytes = valid.clone();
    put_u16(&mut bytes, 6, u16::MAX);
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, Some("Fallback"))
            .unwrap()
            .id(),
        "Fallback"
    );
    let mut bytes = definition("A", &[rule(-330, true)]);
    put_u16(&mut bytes, 8, 0xd800);
    assert_eq!(
        Zone::from_time_zone_definition(&bytes, None).unwrap().id(),
        "\u{fffd}"
    );
    for (bias, expected) in [(-330, "UTC+05:30"), (0, "UTC+00:00"), (1440, "UTC-24:00")] {
        assert_eq!(
            Zone::from_time_zone_definition(&definition("", &[rule(bias, true)]), None)
                .unwrap()
                .id(),
            expected
        );
    }
    assert_eq!(
        Zone::from_time_zone_struct(&structure(), " Eastern ")
            .unwrap()
            .id(),
        " Eastern "
    );
}

#[test]
fn biases_and_transition_fields_are_checked_without_losing_wire_quirks() {
    for index in [0, 4, 8] {
        for invalid in [i32::MIN, -1441, 1441, i32::MAX] {
            let mut bytes = structure();
            put_i32(&mut bytes, index, invalid);
            assert_eq!(
                Zone::from_time_zone_struct(&bytes, ""),
                Err(Error::InvalidBias)
            );
        }
        for boundary in [-1440, 1440] {
            let mut bytes = structure();
            put_i32(&mut bytes, index, boundary);
            assert!(Zone::from_time_zone_struct(&bytes, "").is_ok());
        }
    }
    for (index, value) in [(2, 13), (4, 7), (6, 0), (6, 6), (8, 24), (10, 60), (12, 60)] {
        let mut bytes = system_bytes(NOVEMBER);
        put_u16(&mut bytes, index, value);
        let invalid = SystemTime::read(&bytes).unwrap();
        assert!(!invalid.is_valid_transition());
        assert_eq!(
            invalid.get_transition(2024),
            Err(DateError::InvalidComponents)
        );
        assert_eq!(
            Zone::new("", 0, 0, 0, invalid, MARCH),
            Err(Error::InvalidTransition)
        );
        // An absent paired transition makes this a fixed-offset zone.
        assert!(Zone::new("", 0, 0, 0, invalid, SystemTime::default()).is_ok());
    }
    let absolute = SystemTime {
        year: u16::MAX,
        month: 2,
        day: 31,
        day_of_week: u16::MAX,
        milliseconds: u16::MAX,
        ..NOVEMBER
    };
    assert!(absolute.is_valid_transition());
    assert_eq!(
        absolute.get_transition(2024).unwrap(),
        date(2024, 2, 29, 2, 0).with_kind(Kind::Unspecified)
    );
    assert_eq!(absolute.get_transition(2023).unwrap().day(), 28);
    assert_eq!(
        absolute.get_transition(0),
        Err(DateError::InvalidComponents)
    );
    assert_eq!(
        absolute.get_transition(10000),
        Err(DateError::InvalidComponents)
    );
}

#[test]
fn relative_transitions_cover_an_entire_gregorian_cycle() {
    // Independent forward weekday counting from Monday, 1601-01-01.
    let mut weekday = 1u16;
    for year in 1601..=2000 {
        for month in 1..=12 {
            let days = match month {
                4 | 6 | 9 | 11 => 30,
                2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
                2 => 28,
                _ => 31,
            };
            let mut occurrences = [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ];
            for day in 1..=days {
                occurrences[usize::from(weekday)].push(day);
                weekday = (weekday + 1) % 7;
            }
            for day_of_week in 0..7 {
                let matching = &occurrences[usize::from(day_of_week)];
                for week in 1..=5 {
                    let transition = SystemTime {
                        month,
                        day_of_week,
                        day: week,
                        ..NOVEMBER
                    };
                    let expected = matching[(usize::from(week) - 1).min(matching.len() - 1)];
                    assert_eq!(
                        transition.get_transition(year).unwrap(),
                        date(year, month as u8, expected, 2, 0).with_kind(Kind::Unspecified)
                    );
                }
            }
        }
    }
}

#[test]
fn utc_transition_boundaries_preserve_individual_ticks_and_kinds() {
    let zone = eastern();
    for (transition, before_offset, after_offset) in [
        (date(2024, 3, 10, 7, 0), -300, -240),
        (date(2024, 11, 3, 6, 0), -240, -300),
    ] {
        for delta in [-1, 0, 1] {
            for kind in [Kind::Utc, Kind::Unspecified] {
                let utc =
                    Date::from_ticks(transition.ticks().checked_add_signed(delta).unwrap(), kind)
                        .unwrap();
                let offset = if delta < 0 {
                    before_offset
                } else {
                    after_offset
                };
                assert_eq!(zone.get_offset(utc).unwrap(), offset);
                let local = zone.to_local_time(utc).unwrap();
                assert_eq!(
                    local.ticks(),
                    utc.ticks()
                        .checked_add_signed(i64::from(offset) * 600_000_000)
                        .unwrap()
                );
                assert_eq!(local.kind(), Kind::Unspecified);
                assert_eq!(local.nanosecond(), utc.nanosecond());
            }
        }
    }
}

#[test]
fn repeated_times_prefer_daylight_and_skipped_times_use_standard() {
    let zone = eastern();
    for (local, expected) in [
        (date(2024, 11, 3, 1, 30), date(2024, 11, 3, 5, 30)),
        (date(2024, 3, 10, 2, 30), date(2024, 3, 10, 7, 30)),
        (date(2024, 3, 10, 3, 0), date(2024, 3, 10, 7, 0)),
        (date(2024, 11, 3, 2, 0), date(2024, 11, 3, 7, 0)),
    ] {
        for kind in [Kind::Utc, Kind::Unspecified] {
            let local = Date::from_ticks(local.ticks() + 1234567, kind).unwrap();
            let utc = zone.to_universal_time(local).unwrap();
            assert_eq!(
                utc,
                Date::from_ticks(expected.ticks() + 1234567, Kind::Utc).unwrap()
            );
        }
    }
}

#[test]
fn southern_hemisphere_fixed_offsets_and_boundary_years() {
    let south = Zone::new(
        "South",
        -600,
        0,
        -60,
        SystemTime {
            month: 4,
            day: 1,
            hour: 3,
            ..NOVEMBER
        },
        SystemTime {
            month: 10,
            day: 1,
            ..NOVEMBER
        },
    )
    .unwrap();
    for (utc, offset) in [
        (date(2024, 1, 1, 0, 0), 660),
        (date(2024, 7, 1, 0, 0), 600),
        (date(2024, 4, 6, 15, 59), 660),
        (date(2024, 4, 6, 16, 0), 600),
        (date(2024, 10, 5, 15, 59), 600),
        (date(2024, 10, 5, 16, 0), 660),
    ] {
        assert_eq!(south.get_offset(utc).unwrap(), offset);
    }
    // MimeKit clamps the transition year, not the input instant.
    let north = eastern();
    assert_eq!(north.get_offset(date(1, 7, 1, 12, 0)).unwrap(), -300);
    assert_eq!(north.get_offset(date(9999, 7, 1, 12, 0)).unwrap(), -300);
    assert_eq!(south.get_offset(date(1, 7, 1, 12, 0)).unwrap(), 660);
    assert_eq!(south.get_offset(date(9999, 7, 1, 12, 0)).unwrap(), 660);

    let minimum = Date::from_ticks(0, Kind::Utc).unwrap();
    let maximum = Date::from_ticks(3155378975999999999, Kind::Utc).unwrap();
    assert_eq!(north.is_daylight_time(minimum), Err(DateError::OutOfRange));
    assert_eq!(south.is_daylight_time(maximum), Err(DateError::OutOfRange));
    for bias in [-1440, -330, 0, 300, 1440] {
        let zone = Zone::new(
            "Fixed",
            bias,
            bias,
            -60,
            SystemTime::default(),
            SystemTime::default(),
        )
        .unwrap();
        assert!(!zone.is_daylight_time(minimum).unwrap());
        assert_eq!(zone.get_offset(maximum).unwrap(), -2 * bias);
        let utc = date(2024, 7, 1, 12, 0);
        assert_eq!(
            zone.to_universal_time(zone.to_local_time(utc).unwrap())
                .unwrap(),
            utc
        );
        if bias > 0 {
            assert_eq!(zone.to_local_time(minimum), Err(DateError::OutOfRange));
            assert_eq!(zone.to_universal_time(maximum), Err(DateError::OutOfRange));
        } else if bias < 0 {
            assert_eq!(zone.to_local_time(maximum), Err(DateError::OutOfRange));
            assert_eq!(zone.to_universal_time(minimum), Err(DateError::OutOfRange));
        }
    }
}

#[test]
fn checked_minute_arithmetic_preserves_precision_and_rejects_overflow() {
    for kind in [Kind::Unspecified, Kind::Utc] {
        let input = Date::from_ticks(date(2024, 2, 29, 23, 59).ticks() + 9999999, kind).unwrap();
        let output = input.checked_add_minutes(1).unwrap();
        assert_eq!(
            (output.month(), output.day(), output.hour(), output.minute()),
            (3, 1, 0, 0)
        );
        assert_eq!(output.nanosecond(), 999999900);
        assert_eq!(output.kind(), kind);
        assert_eq!(output.checked_add_minutes(-1).unwrap(), input);
        for minutes in [i32::MIN, i32::MAX] {
            let result = date(5000, 1, 1, 0, 0)
                .with_kind(kind)
                .checked_add_minutes(minutes)
                .unwrap();
            assert_eq!(
                result.ticks(),
                date(5000, 1, 1, 0, 0)
                    .ticks()
                    .checked_add_signed(i64::from(minutes) * 600_000_000)
                    .unwrap()
            );
        }
    }
    let min = Date::from_ticks(0, Kind::Utc).unwrap();
    let max = Date::from_ticks(3155378975999999999, Kind::Utc).unwrap();
    assert_eq!(min.checked_add_minutes(-1), Err(DateError::OutOfRange));
    assert_eq!(max.checked_add_minutes(1), Err(DateError::OutOfRange));
    assert_eq!(
        min.checked_add_minutes(i32::MIN),
        Err(DateError::OutOfRange)
    );
    assert_eq!(
        max.checked_add_minutes(i32::MAX),
        Err(DateError::OutOfRange)
    );
}

#[test]
fn non_hour_negative_and_zero_daylight_shifts_follow_upstream_selection() {
    for (daylight_bias, daylight_offset) in [(-30, -270), (60, -360), (0, -300)] {
        let zone = Zone::new("Shift", 300, 0, daylight_bias, NOVEMBER, MARCH).unwrap();
        assert_eq!(zone.get_offset(date(2024, 1, 1, 12, 0)).unwrap(), -300);
        assert_eq!(
            zone.get_offset(date(2024, 7, 1, 12, 0)).unwrap(),
            daylight_offset
        );
        let utc = date(2024, 7, 1, 12, 0);
        assert_eq!(
            zone.to_universal_time(zone.to_local_time(utc).unwrap())
                .unwrap(),
            utc
        );
    }
    let negative = Zone::new("Negative", 300, 0, 60, NOVEMBER, MARCH).unwrap();
    assert_eq!(
        negative
            .to_universal_time(date(2024, 3, 10, 1, 30))
            .unwrap(),
        date(2024, 3, 10, 7, 30)
    );
    assert_eq!(
        negative
            .to_universal_time(date(2024, 11, 3, 2, 30))
            .unwrap(),
        date(2024, 11, 3, 7, 30)
    );
    // Equal UTC transition instants use the inclusive year-spanning branch.
    let equal = Zone::new("Equal", 0, 0, 0, MARCH, MARCH).unwrap();
    for month in [1, 3, 7, 12] {
        assert!(equal.is_daylight_time(date(2024, month, 1, 0, 0)).unwrap());
    }
}

#[test]
fn time_zone_errors_have_explicit_messages_and_no_hidden_source() {
    for error in [
        Error::IncompleteValue,
        Error::InvalidVersion,
        Error::InvalidHeader,
        Error::NoRule,
        Error::InvalidBias,
        Error::InvalidTransition,
    ] {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn vtimezone_fixed_absolute_last_week_and_escaped_ids() {
    let fixed = Zone::new(
        "Fixed, zone",
        -330,
        0,
        0,
        SystemTime::default(),
        SystemTime::default(),
    )
    .unwrap();
    assert_eq!(
        calendar(&fixed),
        concat!(
            "BEGIN:VTIMEZONE\r\nTZID:Fixed\\, zone\r\nBEGIN:STANDARD\r\n",
            "DTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\n",
            "TZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
        )
    );
    let zone = Zone::new(
        "Rules",
        0,
        0,
        -60,
        SystemTime {
            month: 10,
            day: 5,
            ..NOVEMBER
        },
        SystemTime {
            year: 2024,
            month: 2,
            day: 31,
            ..MARCH
        },
    )
    .unwrap();
    let text = calendar(&zone);
    assert!(
        text.contains("DTSTART:16011028T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\n")
    );
    assert!(
        text.contains("DTSTART:16010228T020000\r\nRRULE:FREQ=YEARLY;BYMONTHDAY=31;BYMONTH=2\r\n")
    );
    let mut zone = eastern();
    zone.set_id("\u{1f600}".repeat(80));
    for line in calendar(&zone).split_terminator("\r\n") {
        assert!(line.len() <= 75);
    }
}

fn assert_usable(zone: &Zone) {
    for year in (1990..=2040).step_by(5) {
        let utc = date(year, 3, 15, 12, 0);
        zone.get_offset(utc).unwrap();
        zone.to_universal_time(zone.to_local_time(utc).unwrap())
            .unwrap();
    }
    let text = calendar(zone);
    assert!(text.starts_with("BEGIN:VTIMEZONE\r\n"));
    assert!(text.ends_with("END:VTIMEZONE\r\n"));
    for line in text.split_terminator("\r\n") {
        assert!(line.len() <= 75);
    }
}

fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    *state >> 16
}

#[test]
fn definition_truncation_and_mutation_fuzz() {
    let valid = definition("Eastern Standard Time", &[rule(300, true)]);
    for len in 0..valid.len() {
        assert!(Zone::from_time_zone_definition(&valid[..len], Some("Fallback")).is_err());
    }
    let mut state = 2007;
    for iteration in 0..2000 {
        let mut bytes = valid.clone();
        for _ in 0..1 + iteration % 8 {
            let index = random(&mut state) as usize % bytes.len();
            bytes[index] = random(&mut state) as u8;
        }
        if let Ok(zone) = Zone::from_time_zone_definition(&bytes, Some("Fallback")) {
            assert_usable(&zone);
        }
    }
}

#[test]
fn legacy_structure_fuzz_with_plausible_transitions() {
    let mut state = 48;
    for iteration in 0..2000 {
        let mut bytes: Vec<u8> = (0..random(&mut state) % 64)
            .map(|_| random(&mut state) as u8)
            .collect();
        if bytes.len() >= 48 && iteration % 2 == 0 {
            put_i32(&mut bytes, 0, (random(&mut state) % 2881) as i32 - 1440);
            put_i32(&mut bytes, 4, 0);
            put_i32(&mut bytes, 8, -60);
            // Use the actual legacy offsets (14 and 32) so valid paired
            // transitions exercise conversion rather than only rejection.
            for index in [14, 32] {
                let transition = SystemTime {
                    month: (random(&mut state) % 13) as u16,
                    day_of_week: (random(&mut state) % 7) as u16,
                    day: (random(&mut state) % 5 + 1) as u16,
                    hour: (random(&mut state) % 24) as u16,
                    ..SystemTime::default()
                };
                bytes[index..index + 16].copy_from_slice(&system_bytes(transition));
            }
        }
        if let Ok(zone) = Zone::from_time_zone_struct(&bytes, "Fuzz") {
            assert_usable(&zone);
        }
    }
}

struct FailingSink {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for FailingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.limit - self.bytes.len()).min(7);
        if count == 0 {
            return Err(io::Error::other("injected"));
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        panic!("write_to must not flush")
    }
}

#[test]
fn vtimezone_propagates_every_partial_sink_failure_and_writer_state() {
    let zone = eastern();
    for limit in 0..EASTERN_CALENDAR.len() {
        let mut writer = CalendarWriter::new(FailingSink {
            bytes: Vec::new(),
            limit,
        });
        assert_eq!(
            zone.write_to(&mut writer).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(
            writer.get_ref().bytes,
            &EASTERN_CALENDAR.as_bytes()[..limit]
        );
        assert!(writer.is_faulted());
        assert_eq!(
            zone.write_to(&mut writer).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
    let mut writer = CalendarWriter::new(FailingSink {
        bytes: Vec::new(),
        limit: usize::MAX,
    });
    zone.write_to(&mut writer).unwrap();
    assert_eq!(writer.into_inner().bytes, EASTERN_CALENDAR.as_bytes());
    let mut writer = CalendarWriter::new(Vec::new());
    writer.begin_property("SUMMARY").unwrap();
    assert_eq!(
        zone.write_to(&mut writer).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(writer.get_ref().is_empty());
    writer.write_value("Pending").unwrap();
    zone.write_to(&mut writer).unwrap();
    writer.finish().unwrap();
    assert_eq!(
        zone.write_to(&mut writer).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

// Adapted from MimeKit's TnefCalendarTests and TnefCalendarRobustnessTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    charsets::Charset,
    tnef::{
        TnefDateTime as Date, TnefDateTimeError as DateError, TnefDateTimeKind as Kind,
        TnefRecurrence as Recurrence, TnefSystemTime as SystemTime, TnefTimeZone as Zone,
        calendar::{
            CalendarWriter, RecurrenceRuleError as Error, format_date, format_date_time,
            recurrence_exdates, recurrence_rule,
        },
    },
};

#[path = "support/tnef_recurrence.rs"]
mod fixtures;

fn parse(bytes: &[u8]) -> Recurrence {
    Recurrence::parse(bytes, Charset::ASCII).unwrap()
}
fn set32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn set16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn eastern() -> Zone {
    let standard = SystemTime {
        month: 11,
        day: 1,
        hour: 2,
        ..SystemTime::default()
    };
    let daylight = SystemTime {
        month: 3,
        day: 2,
        ..standard
    };
    Zone::new("Eastern", 300, 0, -60, standard, daylight).unwrap()
}
fn date(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> Date {
    Date::new(year, month, day, hour, minute, 0, Kind::Unspecified).unwrap()
}
fn minutes(date: Date) -> u32 {
    u32::try_from(date.to_file_time().unwrap() / 600000000).unwrap()
}

#[test]
fn upstream_recurrence_rules() {
    let zone = eastern();
    for (pattern, period, specific, first_day, expected) in [
        (0, 2880, vec![], 0, "FREQ=DAILY;INTERVAL=2;COUNT=10"),
        (
            1,
            2,
            vec![0x02],
            1,
            "FREQ=WEEKLY;BYDAY=MO;INTERVAL=2;COUNT=10;WKST=MO",
        ),
        (
            1,
            2,
            vec![0x02],
            7,
            "FREQ=WEEKLY;BYDAY=MO;INTERVAL=2;COUNT=10",
        ),
        (
            2,
            12,
            vec![10],
            0,
            "FREQ=YEARLY;BYMONTHDAY=10;BYMONTH=7;COUNT=10",
        ),
        (
            2,
            24,
            vec![10],
            0,
            "FREQ=YEARLY;BYMONTHDAY=10;BYMONTH=7;INTERVAL=2;COUNT=10",
        ),
        (2, 1, vec![31], 0, "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=10"),
        (4, 1, vec![0], 0, "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=10"),
        (
            3,
            1,
            vec![0x02, 5],
            0,
            "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=-1;COUNT=10",
        ),
        (
            3,
            2,
            vec![0x14, 2],
            0,
            "FREQ=MONTHLY;BYDAY=TU,TH;BYSETPOS=2;INTERVAL=2;COUNT=10",
        ),
        (
            3,
            12,
            vec![0x02, 1],
            0,
            "FREQ=YEARLY;BYDAY=MO;BYMONTH=7;BYSETPOS=1;COUNT=10",
        ),
    ] {
        let mut bytes = fixtures::pattern(pattern, period, &specific);
        set32(&mut bytes, 30 + specific.len() * 4, first_day);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &zone, false).unwrap(),
            expected
        );
    }
}

#[test]
fn upstream_hijri_patterns_with_explicit_gregorian_calendars() {
    for (pattern, calendar, specific, expected) in [
        (0x0a, 1, vec![10], "FREQ=MONTHLY;BYMONTHDAY=10;COUNT=10"),
        (
            0x0b,
            9,
            vec![2, 5],
            "FREQ=MONTHLY;BYDAY=MO;BYSETPOS=-1;COUNT=10",
        ),
        (0x0c, 12, vec![0], "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=10"),
    ] {
        // MS-OXCICAL 2.1.3.2.1: Hijri patterns use the same mapping when the
        // calendar is Gregorian.
        let mut bytes = fixtures::pattern(pattern, 1, &specific);
        set16(&mut bytes, 8, calendar);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
            expected
        );
    }
}

#[test]
fn calendar_classification_and_error_precedence() {
    for (pattern, period, specific) in [
        (0, 1440, vec![]),
        (1, 1, vec![2]),
        (2, 1, vec![10]),
        (3, 1, vec![2, 1]),
        (4, 1, vec![0]),
        (10, 1, vec![10]),
        (11, 1, vec![2, 1]),
        (12, 1, vec![0]),
    ] {
        for calendar in (0..=0x10).chain([u16::MAX]) {
            let mut bytes = fixtures::pattern(pattern, period, &specific);
            set16(&mut bytes, 8, calendar);
            let expected = [0, 1, 2, 3, 4, 5, 7, 9, 10, 11, 12].contains(&calendar)
                && !(pattern >= 10 && calendar == 0);
            let result = recurrence_rule(&parse(&bytes), &eastern(), false);
            if expected {
                assert!(result.is_ok(), "{pattern}/{calendar}: {result:?}");
            } else {
                let error = Error::UnsupportedCalendar {
                    pattern_type: pattern,
                    calendar_type: calendar,
                };
                assert_eq!(result, Err(error));
                set32(&mut bytes, 14, 0); // Calendar rejection precedes pattern validation.
                assert_eq!(
                    recurrence_rule(&parse(&bytes), &eastern(), false),
                    Err(error)
                );
            }
        }
    }
}

#[test]
fn upstream_invalid_patterns_and_unsigned_day_boundaries() {
    for (pattern, period, specific, error) in [
        (0, 0, vec![], Error::InvalidDailyPeriod),
        (0, 1000, vec![], Error::InvalidDailyPeriod),
        (1, 1, vec![0x80], Error::InvalidWeeklyPattern),
        (1, 0, vec![2], Error::InvalidWeeklyPattern),
        (2, 0, vec![10], Error::InvalidMonthlyPeriod),
        (2, 1, vec![0], Error::InvalidMonthDay),
        (2, 1, vec![32], Error::InvalidMonthDay),
        (3, 1, vec![0, 1], Error::InvalidMonthDay),
        (3, 1, vec![2, 0], Error::InvalidMonthDay),
        (3, 1, vec![2, 6], Error::InvalidMonthDay),
        (2, 1, vec![u32::MAX], Error::InvalidMonthDay),
        (3, 1, vec![2, u32::MAX], Error::InvalidMonthDay),
        (3, 0, vec![0, 0], Error::InvalidMonthlyPeriod),
    ] {
        assert_eq!(
            recurrence_rule(
                &parse(&fixtures::pattern(pattern, period, &specific)),
                &eastern(),
                false
            ),
            Err(error)
        );
    }
    for day in [0, 1, 30, 31, 32, u32::MAX] {
        let recurrence = parse(&fixtures::pattern(4, 1, &[day]));
        assert_eq!(
            recurrence_rule(&recurrence, &eastern(), false).unwrap(),
            "FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=10"
        );
    }
}

#[test]
fn weekday_order_high_bits_and_week_start_policy() {
    let days = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
    for mask in 1..128 {
        for first in 0..=8 {
            let mut bytes = fixtures::pattern(1, 2, &[mask | 0xffffff80]);
            set32(&mut bytes, 34, first);
            let selected = days
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, day)| *day)
                .collect::<Vec<_>>()
                .join(",");
            let mut expected = format!("FREQ=WEEKLY;BYDAY={selected};INTERVAL=2;COUNT=10");
            if first < 7 {
                expected.push_str(&format!(";WKST={}", days[first as usize]));
            }
            assert_eq!(
                recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
                expected
            );
            set32(&mut bytes, 14, 1);
            assert_eq!(
                recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
                format!("FREQ=WEEKLY;BYDAY={selected};COUNT=10")
            );
        }
    }
}

#[test]
fn count_unknown_end_types_and_maximum_intervals() {
    let mut bytes = fixtures::pattern(1, u32::MAX, &[2]);
    set32(&mut bytes, 34, u32::MAX);
    set32(&mut bytes, 50, u32::MAX); // EndDate is irrelevant for COUNT.
    set32(&mut bytes, 62, u32::MAX); // StartTimeOffset is irrelevant for COUNT.
    for count in [0, u32::MAX] {
        set32(&mut bytes, 30, count);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
            format!("FREQ=WEEKLY;BYDAY=MO;INTERVAL=4294967295;COUNT={count}")
        );
    }
    for end in [0, 0x2023, u32::MAX] {
        set32(&mut bytes, 26, end);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
            "FREQ=WEEKLY;BYDAY=MO;INTERVAL=4294967295"
        );
    }
    let max_daily = u32::MAX / 1440 * 1440;
    let recurrence = parse(&fixtures::pattern(0, max_daily, &[]));
    assert_eq!(
        recurrence_rule(&recurrence, &eastern(), false).unwrap(),
        format!("FREQ=DAILY;INTERVAL={};COUNT=10", max_daily / 1440)
    );
}

#[test]
fn yearly_rules_use_start_date_month_and_retain_clause_order() {
    for (start, month) in [(0, 1), (222737760, 7), (u32::MAX, 2)] {
        let mut bytes = fixtures::pattern(3, 24, &[0x41, 5]);
        set32(&mut bytes, 50, start);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
            format!("FREQ=YEARLY;BYDAY=SU,SA;BYMONTH={month};BYSETPOS=-1;INTERVAL=2;COUNT=10")
        );
    }
}

#[test]
fn upstream_until_all_day_and_never_ending_rules() {
    let mut bytes = fixtures::pattern(0, 1440, &[]);
    set32(&mut bytes, 22, 0x2021);
    let recurrence = parse(&bytes);
    // UNTIL is the last instance's start (10:00 EDT), in UTC.
    assert_eq!(
        recurrence_rule(&recurrence, &eastern(), false).unwrap(),
        "FREQ=DAILY;UNTIL=20240731T140000Z"
    );
    assert_eq!(
        recurrence_rule(&recurrence, &eastern(), true).unwrap(),
        "FREQ=DAILY;UNTIL=20240731"
    );
    set32(&mut bytes, 22, 0x2023);
    assert_eq!(
        recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
        "FREQ=DAILY"
    );
}

#[test]
fn until_uses_local_time_zone_gap_fold_and_fixed_offset_policies() {
    for (end, offset, expected) in [
        (date(2024, 3, 10, 0, 0), 150, "20240310T073000Z"),
        (date(2024, 11, 3, 0, 0), 90, "20241103T053000Z"),
        (date(2024, 1, 1, 0, 0), 600, "20240101T150000Z"),
        (date(2024, 7, 31, 1, 15), 600, "20240731T151500Z"),
    ] {
        let mut bytes = fixtures::pattern(0, 1440, &[]);
        set32(&mut bytes, 22, 0x2021);
        set32(&mut bytes, 46, minutes(end));
        set32(&mut bytes, 58, offset);
        assert_eq!(
            recurrence_rule(&parse(&bytes), &eastern(), false).unwrap(),
            format!("FREQ=DAILY;UNTIL={expected}")
        );
    }
    let zone = Zone::new(
        "India",
        -330,
        0,
        0,
        SystemTime::default(),
        SystemTime::default(),
    )
    .unwrap();
    let mut bytes = fixtures::pattern(0, 1440, &[]);
    set32(&mut bytes, 22, 0x2021);
    assert_eq!(
        recurrence_rule(&parse(&bytes), &zone, false).unwrap(),
        "FREQ=DAILY;UNTIL=20240731T043000Z"
    );
}

#[test]
fn unsigned_offsets_preserve_precision_and_report_range_errors() {
    for kind in [Kind::Utc, Kind::Unspecified] {
        let epoch = Date::from_ticks(date(1601, 1, 1, 0, 0).ticks() + 123, kind).unwrap();
        for offset in [0, 1, i32::MAX as u32, 0x80000000, u32::MAX] {
            let value = epoch.checked_add_unsigned_minutes(offset).unwrap();
            assert_eq!(value.ticks(), epoch.ticks() + u64::from(offset) * 600000000);
            assert_eq!(value.kind(), kind);
            assert_eq!(value.nanosecond(), 12300);
        }
        let last = Date::from_ticks(3155378975999999999, kind).unwrap();
        assert_eq!(last.checked_add_unsigned_minutes(0), Ok(last));
        assert_eq!(
            last.checked_add_unsigned_minutes(1),
            Err(DateError::OutOfRange)
        );
    }
    let mut bytes = fixtures::pattern(0, 1440, &[]);
    set32(&mut bytes, 22, 0x2021);
    set32(&mut bytes, 46, 0);
    set32(&mut bytes, 58, u32::MAX);
    let utc = Zone::new("UTC", 0, 0, 0, SystemTime::default(), SystemTime::default()).unwrap();
    assert_eq!(
        recurrence_rule(&parse(&bytes), &utc, false).unwrap(),
        "FREQ=DAILY;UNTIL=97670216T041500Z"
    );
    set32(&mut bytes, 46, u32::MAX);
    assert_eq!(
        recurrence_rule(&parse(&bytes), &utc, false),
        Err(Error::InvalidDate(DateError::OutOfRange))
    );
    assert_eq!(
        recurrence_rule(&parse(&bytes), &utc, true).unwrap(),
        "FREQ=DAILY;UNTIL=97670216"
    );
    // The offset addition can fit while the subsequent UTC conversion fails.
    let remaining = (date(9999, 12, 31, 23, 59).ticks()
        - Date::from_recurrence_minutes(u32::MAX).ticks())
        / 600000000;
    set32(&mut bytes, 58, u32::try_from(remaining).unwrap());
    assert_eq!(
        recurrence_rule(&parse(&bytes), &utc, false).unwrap(),
        "FREQ=DAILY;UNTIL=99991231T235900Z"
    );
    assert_eq!(
        recurrence_rule(&parse(&bytes), &eastern(), false),
        Err(Error::InvalidDate(DateError::OutOfRange))
    );
}

#[test]
fn exdates_use_exception_original_days_not_the_modified_array() {
    let mut bytes = fixtures::weekly();
    assert_eq!(
        recurrence_exdates(&parse(&bytes)).unwrap(),
        [date(2024, 7, 8, 10, 0)]
    );
    set32(&mut bytes, 54, 0); // ModifiedInstanceDates does not determine exclusions.
    assert_eq!(
        recurrence_exdates(&parse(&bytes)).unwrap(),
        [date(2024, 7, 8, 10, 0)]
    );
    set32(&mut bytes, 92, minutes(date(2024, 7, 8, 23, 59)));
    assert_eq!(
        recurrence_exdates(&parse(&bytes)).unwrap(),
        [date(2024, 7, 10, 10, 0)]
    );
    // Modified occurrences are not EXDATEs, irrespective of later export caps.
    assert!(
        recurrence_exdates(&parse(&fixtures::many(2000)))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exdates_preserve_order_duplicates_and_offset_day_rollover() {
    let mut bytes = fixtures::weekly();
    set32(&mut bytes, 42, minutes(date(2024, 7, 9, 15, 30)));
    set32(&mut bytes, 46, minutes(date(2024, 7, 8, 23, 59)));
    assert_eq!(
        recurrence_exdates(&parse(&bytes)).unwrap(),
        [date(2024, 7, 9, 10, 0), date(2024, 7, 8, 10, 0)]
    );
    set32(&mut bytes, 42, minutes(date(2024, 7, 8, 0, 1)));
    set32(&mut bytes, 74, 1500);
    let dates = recurrence_exdates(&parse(&bytes)).unwrap();
    assert_eq!(dates, [date(2024, 7, 9, 1, 0), date(2024, 7, 9, 1, 0)]);
    // All-day output strips the time only after adding the full offset.
    assert_eq!(format_date(dates[0]), "20240709");
    set32(&mut bytes, 42, 0);
    set32(&mut bytes, 46, u32::MAX);
    set32(&mut bytes, 74, u32::MAX);
    assert_eq!(
        recurrence_exdates(&parse(&bytes)),
        Err(DateError::OutOfRange)
    );
}

#[test]
fn recovered_exceptions_leave_master_rules_and_unsuppressed_deleted_days() {
    let bytes = fixtures::weekly();
    let recurrence = parse(&bytes[..112]);
    let saved = recurrence.clone();
    assert!(recurrence.exception_error().is_some());
    assert_eq!(
        recurrence_rule(&recurrence, &eastern(), false).unwrap(),
        "FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10"
    );
    assert_eq!(
        recurrence_exdates(&recurrence).unwrap(),
        [date(2024, 7, 8, 10, 0), date(2024, 7, 10, 10, 0)]
    );
    assert_eq!(recurrence, saved);
}

#[test]
fn generated_values_compose_with_the_calendar_writer() {
    let recurrence = parse(&fixtures::weekly());
    for all_day in [false, true] {
        let rule = recurrence_rule(&recurrence, &eastern(), all_day).unwrap();
        let dates = recurrence_exdates(&recurrence).unwrap();
        let mut writer = CalendarWriter::new(Vec::new());
        writer.write_property("RRULE", &rule).unwrap();
        for date in dates {
            writer.begin_property("EXDATE").unwrap();
            if all_day {
                writer.write_parameter("VALUE", "DATE").unwrap();
                writer.write_value(&format_date(date)).unwrap();
            } else {
                writer.write_parameter("TZID", "Eastern").unwrap();
                writer.write_value(&format_date_time(date, false)).unwrap();
            }
        }
        writer.finish().unwrap();
        let exdate = if all_day {
            "EXDATE;VALUE=DATE:20240708\r\n"
        } else {
            "EXDATE;TZID=Eastern:20240708T100000\r\n"
        };
        assert_eq!(
            writer.into_inner(),
            format!("RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10\r\n{exdate}").as_bytes()
        );
    }
}

#[test]
fn typed_rule_errors_have_explicit_messages_and_date_sources() {
    for error in [
        Error::UnsupportedCalendar {
            pattern_type: 10,
            calendar_type: 0,
        },
        Error::InvalidDailyPeriod,
        Error::InvalidWeeklyPattern,
        Error::InvalidMonthlyPeriod,
        Error::InvalidMonthDay,
        Error::UnknownPattern(0xffff),
        Error::InvalidDate(DateError::OutOfRange),
    ] {
        assert!(!error.to_string().is_empty());
        assert_eq!(
            std::error::Error::source(&error).is_some(),
            matches!(error, Error::InvalidDate(_))
        );
    }
    assert_eq!(
        Error::UnknownPattern(0xffff).to_string(),
        "the pattern type 0xFFFF is not known"
    );
    assert_eq!(
        Error::from(DateError::OutOfRange),
        Error::InvalidDate(DateError::OutOfRange)
    );
}

#[test]
fn recurrence_mutations_generate_values_or_explicit_errors_without_panicking() {
    let source = fixtures::weekly();
    let zone = eastern();
    let mut seed = 20240701u32;
    fn next(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed
    }
    for _ in 0..5000 {
        let mut bytes = source.clone();
        for _ in 0..1 + next(&mut seed) % 4 {
            let index = next(&mut seed) as usize % bytes.len();
            bytes[index] = next(&mut seed) as u8;
        }
        if let Ok(recurrence) = Recurrence::parse(&bytes, Charset::ASCII) {
            for all_day in [false, true] {
                if let Ok(rule) = recurrence_rule(&recurrence, &zone, all_day) {
                    assert!(rule.starts_with("FREQ="));
                    assert!(rule.is_ascii());
                    assert!(!rule.contains(['\r', '\n']));
                }
            }
            match recurrence_exdates(&recurrence) {
                Ok(dates) => assert!(dates.iter().all(|date| date.kind() == Kind::Unspecified)),
                Err(error) => assert_eq!(error, DateError::OutOfRange),
            }
        }
    }
}

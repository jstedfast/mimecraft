// Compatibility vectors adapted from MimeKit's UnitTests/Utils/DateParserTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{DateTimeOffset, DateTimeOffsetError};

#[test]
fn source_date_vectors() {
    let cases = [
        (
            "Sun, 08 Dec 91 09:11:00 +0000",
            "Sun, 08 Dec 1991 09:11:00 +0000",
        ),
        (
            "8 Dec 1991 09:11 (Sunday)",
            "Sun, 08 Dec 1991 09:11:00 +0000",
        ),
        (
            "26 Dec 1991 20:45 (Thursday)",
            "Thu, 26 Dec 1991 20:45:00 +0000",
        ),
        (
            "Tue, 9 Jun 92 03:45:24 JST",
            "Tue, 09 Jun 1992 03:45:24 +0900",
        ),
        (
            "Mon, 17 Jan 1994 11:14:55 -0500",
            "Mon, 17 Jan 1994 11:14:55 -0500",
        ),
        (
            "Mon, 17 Jan 01 11:14:55 -0500",
            "Wed, 17 Jan 2001 11:14:55 -0500",
        ),
        (
            "Tue, 30 Mar 2004 13:01:38 +0000",
            "Tue, 30 Mar 2004 13:01:38 +0000",
        ),
        (
            "Sat Mar 24 21:23:03 EDT 2007",
            "Sat, 24 Mar 2007 21:23:03 -0400",
        ),
        (
            "Sat, 24 Mar 2007 21:23:03 EDT",
            "Sat, 24 Mar 2007 21:23:03 -0400",
        ),
        (
            "Sat, 24 Mar 2007 21:23:03 GMT",
            "Sat, 24 Mar 2007 21:23:03 +0000",
        ),
        ("17-6-2008 17:10:08", "Tue, 17 Jun 2008 17:10:08 +0000"),
        (
            "FRI, 30 NOV 2012 02:09:10 +0100",
            "Fri, 30 Nov 2012 02:09:10 +0100",
        ),
        (
            "Tue, 11 Feb 2014 22:27:10 +0100 (CET)",
            "Tue, 11 Feb 2014 22:27:10 +0100",
        ),
        (
            "Wed, 6 Aug 2014 01:53:48 -2200",
            "Wed, 06 Aug 2014 01:53:48 +0000",
        ),
        (
            "Tue, 21 Apr 15 14:44:51 GMT",
            "Tue, 21 Apr 2015 14:44:51 +0000",
        ),
        (
            "Tue, 21 April 15 14:44:51 GMT",
            "Tue, 21 Apr 2015 14:44:51 +0000",
        ),
        (
            "Thu, 1 Oct 2015 14:40:57 +0200 (Mitteleuropäische Sommerzeit)",
            "Thu, 01 Oct 2015 14:40:57 +0200",
        ),
        (
            "Tue, 12 Jun 2012 19:22:28 0200",
            "Tue, 12 Jun 2012 19:22:28 +0200",
        ),
        ("Fri, 8 May 2015", "Fri, 08 May 2015 00:00:00 +0000"),
        ("Fri, 8 May 2015 12", "Fri, 08 May 2015 00:00:00 +0000"),
        ("Fri, 8 May 2015 12:05", "Fri, 08 May 2015 12:05:00 +0000"),
        (
            "Fri, 8 May 2015 12:05:01",
            "Fri, 08 May 2015 12:05:01 +0000",
        ),
        (
            "Fri, 8 May 2015 12:05:01 400",
            "Fri, 08 May 2015 12:05:01 +0400",
        ),
        (
            "Sat, 9 May 2015 24:00:00 -0400",
            "Sat, 09 May 2015 00:00:00 -0400",
        ),
        (
            "Sat, 9 May 2015 25:00:00 -0400",
            "Sat, 09 May 2015 00:00:00 -0400",
        ),
        (
            "May 9 2015 25:00:00 -0400",
            "Sat, 09 May 2015 00:00:00 -0400",
        ),
        (
            "2015 May 9 25:00:00 -0400",
            "Sat, 09 May 2015 00:00:00 -0400",
        ),
        (
            "2015 May 9 25:99:78 -0400",
            "Sat, 09 May 2015 00:00:00 -0400",
        ),
        (
            "25 Sep 81 06:03:27 -0400",
            "Fri, 25 Sep 1981 06:03:27 -0400",
        ),
        (
            "Sat, 10 Sep 2022 12:59:19 -1234567890123456789",
            "Sat, 10 Sep 2022 12:59:19 +0000",
        ),
        (
            "Sat, 10 Sep 2022 12:59:19 1234567890123456789",
            "Sat, 10 Sep 2022 12:59:19 +0000",
        ),
        (
            "Sat, 10 Sep 2022 12:59:19 04+00",
            "Sat, 10 Sep 2022 12:59:19 +0000",
        ),
        (
            "Sat, 10 Sep 2022 12:59:19 ECST",
            "Sat, 10 Sep 2022 12:59:19 +0000",
        ),
        (
            "Sat, Sep 10 2022 12:59:19 0400",
            "Sat, 10 Sep 2022 12:59:19 +0400",
        ),
        (
            "Sat, Sep 10 77 12:59:19 0400",
            "Sat, 10 Sep 1977 12:59:19 +0400",
        ),
        (
            "Sat, 01 Mar 2025 09:00:00 XYZ",
            "Sat, 01 Mar 2025 09:00:00 +0000",
        ),
        (
            "Mon, 28 Feb 2025 09:00:00 -0500",
            "Fri, 28 Feb 2025 09:00:00 -0500",
        ),
        (
            "Fri, 28 Feb 2025 12:15:00 AM -0500",
            "Fri, 28 Feb 2025 00:15:00 -0500",
        ),
        (
            "Fri, 28 Feb 2025 11:30:00 PM -0500",
            "Fri, 28 Feb 2025 23:30:00 -0500",
        ),
        (
            "Feb 28 2025 12:15:00 AM -0500",
            "Fri, 28 Feb 2025 00:15:00 -0500",
        ),
        (
            "Feb 28 2025 11:30:00 PM -0500",
            "Fri, 28 Feb 2025 23:30:00 -0500",
        ),
        (
            "Fri, 28 Feb 2025 23:59:60 -0500",
            "Fri, 28 Feb 2025 23:59:59 -0500",
        ),
        (
            "Fri, 28 Feb 2025 23:59:61 -0500",
            "Fri, 28 Feb 2025 00:00:00 -0500",
        ),
        (
            "Fri, 28 Feb 2025 22:59:60 -0500",
            "Fri, 28 Feb 2025 00:00:00 -0500",
        ),
        (
            "Fri, 28 Feb 2025 23:60:59 -0500",
            "Fri, 28 Feb 2025 00:00:00 -0500",
        ),
        ("31 Dec 2024 10:15:00 AM", "Tue, 31 Dec 2024 10:15:00 +0000"),
        ("31 Dec 2024 10:15:00 PM", "Tue, 31 Dec 2024 22:15:00 +0000"),
    ];
    for (input, expected) in cases {
        let date = DateTimeOffset::parse(input.as_bytes()).expect(input);
        assert_eq!(date.to_string(), expected, "{input}");
        assert_eq!(input.parse::<DateTimeOffset>().unwrap(), date);
        assert_eq!(date.to_string().parse::<DateTimeOffset>().unwrap(), date);
        let wrapped = format!("prefix{input}suffix");
        assert_eq!(
            DateTimeOffset::parse(&wrapped.as_bytes()[6..6 + input.len()]).unwrap(),
            date
        );
    }
}

#[test]
fn source_invalid_dates_and_tokens() {
    for input in [
        "this is pure junk",
        "Sunday is the day of our Lord",
        "Sun is so bright, I gotta wear shades",
        "Sat, 8 dogs did while 8 cats hid",
        "Sat, 9 May flies bit my arms",
        "999999999999 Jan 2025 09:00:00 -0500",
        "Fri, 999999999999 Jan 2025 09:00:00 -0500",
        "Fri, 31 Feb 2025 09:00:00 -0500",
        "Feb 31 2025 09:00:00 -0500",
        "",
        " \t\r\n",
        "28 Feb",
        "29 Feb 2025",
        "31 Apr 2025",
        "32 Jan 2025",
        "0 Jan 2025",
    ] {
        assert_eq!(
            input.parse::<DateTimeOffset>(),
            Err(DateTimeOffsetError::InvalidSyntax),
            "{input}"
        );
    }
    assert_eq!(
        "123 Feb 28 2025 09:00:00 -0500"
            .parse::<DateTimeOffset>()
            .unwrap(),
        DateTimeOffset::new(2025, 2, 28, 9, 0, 0, -300).unwrap(),
    );
}

#[test]
fn named_timezones() {
    let zones = [
        ("UT", 0),
        ("UTC", 0),
        ("GMT", 0),
        ("EDT", -240),
        ("EST", -300),
        ("CDT", -300),
        ("CST", -360),
        ("MDT", -360),
        ("MST", -420),
        ("PDT", -420),
        ("PST", -480),
        ("A", 60),
        ("B", 120),
        ("C", 180),
        ("D", 240),
        ("E", 300),
        ("F", 360),
        ("G", 420),
        ("H", 480),
        ("I", 540),
        ("K", 600),
        ("L", 660),
        ("M", 720),
        ("N", -60),
        ("O", -120),
        ("P", -180),
        ("Q", -240),
        ("R", -300),
        ("S", -360),
        ("T", -420),
        ("U", -480),
        ("V", -540),
        ("W", -600),
        ("X", -660),
        ("Y", -720),
        ("Z", 0),
        ("JST", 540),
        ("KST", 540),
    ];
    for (zone, minutes) in zones {
        for input in [
            format!("Fri, 28 Feb 2025 09:00:00 {zone}"),
            format!("Feb 28 09:00:00 {zone} 2025"),
        ] {
            assert_eq!(
                input.parse::<DateTimeOffset>().unwrap().offset_minutes(),
                minutes,
                "{input}"
            );
        }
    }
    for zone in ["J", "CET", "XYZ", "ECST", "jst", "est", "a", "GmT"] {
        let input = format!("28 Feb 2025 09:00:00 {zone}");
        assert_eq!(
            input.parse::<DateTimeOffset>().unwrap().offset_minutes(),
            0,
            "{input}"
        );
    }
}

#[test]
fn numeric_zone_limits_and_source_offset_formatting() {
    for (zone, expected) in [
        ("-1192", 0),
        ("+0060", 0),
        ("+1399", 0),
        ("+0059", 59),
        ("-1159", -719),
        ("-1200", -720),
        ("-1201", 0),
        ("+1400", 840),
        ("+1401", 0),
        ("+0000", 0),
        ("-0000", 0),
        ("+5", 5),
        ("400", 240),
        ("+2147483647", 0),
        ("-2147483648", 0),
        ("+-0500", 0),
        ("+", 0),
    ] {
        let input = format!("Wed, 25 Mar 1992 00:00:00 {zone}");
        let date = input.parse::<DateTimeOffset>().unwrap();
        assert_eq!(date.offset_minutes(), expected, "{input}");
        assert_eq!(date.to_string().parse::<DateTimeOffset>().unwrap(), date);
    }
    for (minutes, expected) in [
        (-210, "Thu, 09 Jan 1992 00:00:00 -0330"),
        (-5, "Thu, 09 Jan 1992 00:00:00 -0005"),
        (345, "Thu, 09 Jan 1992 00:00:00 +0545"),
        (-300, "Thu, 09 Jan 1992 00:00:00 -0500"),
        (0, "Thu, 09 Jan 1992 00:00:00 +0000"),
    ] {
        let date = DateTimeOffset::new(1992, 1, 9, 0, 0, 0, minutes).unwrap();
        assert_eq!(date.to_string(), expected);
    }
}

#[test]
fn month_names_and_numeric_date_orders() {
    let names = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    for (index, name) in names.into_iter().enumerate() {
        for spelling in [
            name.to_owned(),
            name.to_ascii_uppercase(),
            name.to_ascii_lowercase(),
        ] {
            let date = format!("1 {spelling} 2025 09:00:00 +0000")
                .parse::<DateTimeOffset>()
                .unwrap();
            assert_eq!(usize::from(date.month()), index + 1);
        }
    }
    for (input, month, day) in [
        ("2025-03-04", 3, 4),
        ("2025/03/04", 3, 4),
        ("03/04/2025", 3, 4),
        ("17-6-2025", 6, 17),
        ("4 March 2025", 3, 4),
        ("2025 4 March", 3, 4),
        ("2001 8 9 May", 5, 8),
    ] {
        let date = input.parse::<DateTimeOffset>().unwrap();
        assert_eq!((date.month(), date.day()), (month, day), "{input}");
    }
}

#[test]
fn year_pivot_depends_on_standard_or_recovery_order() {
    for (year, expected) in [
        ("0", 2000),
        ("01", 2001),
        ("68", 2068),
        ("69", 2069),
        ("70", 1970),
        ("99", 1999),
        ("0000", 2000),
        ("0069", 2069),
        ("1969", 1969),
        ("9999", 9999),
    ] {
        let input = format!("1 Jan {year} 00:00:00 +0000");
        assert_eq!(
            input.parse::<DateTimeOffset>().unwrap().year(),
            expected,
            "{input}"
        );
    }
    assert_eq!("Jan 1 69".parse::<DateTimeOffset>().unwrap().year(), 1969);
    assert!("Jan 1 68".parse::<DateTimeOffset>().is_err());
    for year in ["100", "1968", "10000", "2147483647", "2147483648"] {
        assert!(
            format!("1 Jan {year} 00:00:00 +0000")
                .parse::<DateTimeOffset>()
                .is_err(),
            "{year}"
        );
    }
}

#[test]
fn comments_folding_and_opaque_bytes() {
    let expected = DateTimeOffset::new(2025, 2, 28, 9, 0, 0, -300).unwrap();
    for input in [
        &b"(nested (escaped \\) comment)) Fri,\r\n\t28 Feb 2025 09:00:00 -0500"[..],
        &b"28 (opaque \xff) Feb 2025 09:00:00 -0500"[..],
        &b"28 Feb 2025 09:00:00 -0500 (unterminated"[..],
        &b"28 Feb 2025 09:00:00 -0500 ignored\xff"[..],
    ] {
        assert_eq!(DateTimeOffset::parse(input).unwrap(), expected);
    }
    assert!(DateTimeOffset::parse(b"(unterminated 28 Feb 2025").is_err());
    assert!(DateTimeOffset::parse(b"28 Feb (unterminated 2025").is_err());
    assert!(DateTimeOffset::parse(b"28 Feb(comment) 2025").is_err());
    let midnight = DateTimeOffset::new(2025, 2, 28, 0, 0, 0, 0).unwrap();
    assert_eq!(
        DateTimeOffset::parse(b"28 Feb 2025 (unterminated").unwrap(),
        midnight
    );
}

#[test]
fn am_pm_and_time_recovery_order() {
    for (input, hour, minute, offset) in [
        ("28 Feb 2025 12:15 AM -0500", 0, 15, -300),
        ("28 Feb 2025 11:15 PM -0500", 23, 15, -300),
        ("28 Feb 2025 12:15 PM -0500", 12, 15, -300),
        ("28 Feb 2025 13:15 AM -0500", 13, 15, -300),
        ("28 Feb 2025 12:15 am -0500", 12, 15, 0),
        ("Feb 28 2025 12:15 am -0500", 12, 15, -300),
        ("Feb 28 2025 -0500 11:15 PM", 11, 15, -300),
        ("28 Feb 2025 1:2 +0000", 1, 2, 0),
        ("28 Feb 2025 1:2: +0000", 0, 0, 0),
        ("28 Feb 2025 1:2:3:4 +0000", 0, 0, 0),
        ("Feb 28 2025 25:00 07:08 -0500", 7, 8, -300),
    ] {
        let date = input.parse::<DateTimeOffset>().unwrap();
        assert_eq!(
            (date.hour(), date.minute(), date.offset_minutes()),
            (hour, minute, offset),
            "{input}"
        );
    }
}

#[test]
fn checked_constructor_rejects_invalid_components() {
    for (year, month, day) in [
        (0, 1, 1),
        (10000, 1, 1),
        (u16::MAX, 1, 1),
        (2025, 0, 1),
        (2025, 13, 1),
        (2025, 255, 1),
        (2025, 1, 0),
        (2025, 1, 32),
        (2025, 1, 255),
        (2025, 2, 29),
        (1900, 2, 29),
        (2100, 2, 29),
        (2025, 4, 31),
    ] {
        assert_eq!(
            DateTimeOffset::new(year, month, day, 0, 0, 0, 0),
            Err(DateTimeOffsetError::InvalidDate)
        );
    }
    for year in [4, 1600, 2000, 2024, 2400] {
        assert!(DateTimeOffset::new(year, 2, 29, 0, 0, 0, 0).is_ok());
    }
    for (hour, minute, second) in [(24, 0, 0), (0, 60, 0), (23, 59, 60), (255, 255, 255)] {
        assert_eq!(
            DateTimeOffset::new(2025, 1, 1, hour, minute, second, 0),
            Err(DateTimeOffsetError::InvalidTime)
        );
    }
    for offset in [i16::MIN, -841, 841, i16::MAX] {
        assert_eq!(
            DateTimeOffset::new(2025, 1, 1, 0, 0, 0, offset),
            Err(DateTimeOffsetError::InvalidOffset)
        );
    }
}

#[test]
fn utc_calendar_bounds_and_timestamp() {
    assert_eq!(
        DateTimeOffset::new(1, 1, 1, 0, 0, 0, 1),
        Err(DateTimeOffsetError::UtcOutOfRange)
    );
    assert_eq!(
        DateTimeOffset::new(9999, 12, 31, 23, 59, 59, -1),
        Err(DateTimeOffsetError::UtcOutOfRange)
    );
    let first = DateTimeOffset::new(1, 1, 1, 0, 0, 0, 0).unwrap();
    let last = DateTimeOffset::new(9999, 12, 31, 23, 59, 59, 0).unwrap();
    assert_eq!(first.to_string(), "Mon, 01 Jan 0001 00:00:00 +0000");
    assert_eq!(last.to_string(), "Fri, 31 Dec 9999 23:59:59 +0000");
    assert_eq!(first.unix_timestamp(), -62_135_596_800);
    assert_eq!(last.unix_timestamp(), 253_402_300_799);
    assert_eq!(
        DateTimeOffset::new(1, 1, 1, 14, 0, 0, 840)
            .unwrap()
            .unix_timestamp(),
        first.unix_timestamp()
    );
    assert_eq!(
        DateTimeOffset::new(9999, 12, 31, 9, 59, 59, -840)
            .unwrap()
            .unix_timestamp(),
        last.unix_timestamp()
    );
    assert_eq!(
        DateTimeOffset::new(1969, 12, 31, 23, 59, 59, 0)
            .unwrap()
            .unix_timestamp(),
        -1
    );
    let utc = DateTimeOffset::new(1970, 1, 1, 0, 0, 0, 0).unwrap();
    let east = DateTimeOffset::new(1970, 1, 1, 1, 0, 0, 60).unwrap();
    assert_eq!(utc.unix_timestamp(), 0);
    assert_eq!(east.unix_timestamp(), 0);
    assert_ne!(utc, east);
}

#[test]
fn every_minute_offset_formats_and_preserves_instant() {
    for offset in -840..=840 {
        let date = DateTimeOffset::new(2025, 2, 28, 9, 17, 42, offset).unwrap();
        let zero = DateTimeOffset::new(2025, 2, 28, 9, 17, 42, 0).unwrap();
        assert_eq!(
            date.unix_timestamp(),
            zero.unix_timestamp() - i64::from(offset) * 60
        );
        let parsed = date.to_string().parse::<DateTimeOffset>().unwrap();
        if offset >= -720 {
            assert_eq!(parsed, date);
        } else {
            assert_eq!(parsed, zero);
        }
    }
}

#[test]
fn gregorian_cycle_has_contiguous_days_and_weekdays() {
    let weekdays = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let mut elapsed = 0usize;
    for year in 1970u16..2370 {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let lengths = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        for (index, length) in lengths.into_iter().enumerate() {
            for day in 1..=length {
                let date = DateTimeOffset::new(year, index as u8 + 1, day, 0, 0, 0, 0).unwrap();
                assert_eq!(date.unix_timestamp(), elapsed as i64 * 86400);
                let formatted = date.to_string();
                assert!(formatted.starts_with(weekdays[elapsed % 7]), "{formatted}");
                assert_eq!(formatted.parse::<DateTimeOffset>().unwrap(), date);
                elapsed += 1;
            }
        }
    }
    assert_eq!(elapsed, 146_097);
}

#[test]
fn raw_header_integration() {
    let mut parser = mimecraft::HeaderParser::new();
    let mut headers = Vec::new();
    parser
        .parse(
            b"Date: Fri, 28 Feb 2025\r\n\t09:00:00 -0500\r\n\r\nbody",
            &mut headers,
        )
        .unwrap();
    let header = &headers[0];
    assert_eq!(header.id(), mimecraft::HeaderId::Date);
    assert_eq!(
        DateTimeOffset::parse(header.raw_value()).unwrap(),
        DateTimeOffset::new(2025, 2, 28, 9, 0, 0, -300).unwrap()
    );
}

#[test]
fn long_tokens_and_deep_comments_are_bounded_and_iterative() {
    let huge = "9".repeat(100_000);
    assert!(
        format!("{huge} Jan 2025 00:00:00 +0000")
            .parse::<DateTimeOffset>()
            .is_err()
    );
    let date = format!("28 Feb 2025 00:00:00 +{huge}")
        .parse::<DateTimeOffset>()
        .unwrap();
    assert_eq!(date.offset_minutes(), 0);
    let nested = format!(
        "{}opaque{} 28 Feb 2025",
        "(".repeat(10_000),
        ")".repeat(10_000)
    );
    assert_eq!(nested.parse::<DateTimeOffset>().unwrap().day(), 28);
}

#[test]
fn generated_arbitrary_bytes_never_create_unchecked_dates() {
    let mut state = 0x5eeda11du32;
    for length in (0..128).cycle().take(10_000) {
        let input: Vec<u8> = (0..length)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            })
            .collect();
        for bytes in [input.clone(), [b"28 Feb 2025 ".as_slice(), &input].concat()] {
            if let Ok(date) = DateTimeOffset::parse(&bytes) {
                assert_eq!(
                    DateTimeOffset::new(
                        date.year(),
                        date.month(),
                        date.day(),
                        date.hour(),
                        date.minute(),
                        date.second(),
                        date.offset_minutes()
                    )
                    .unwrap(),
                    date
                );
                assert_eq!(date.to_string().parse::<DateTimeOffset>().unwrap(), date);
            }
        }
    }
}

#[test]
fn error_values_are_standard_errors() {
    for error in [
        DateTimeOffsetError::InvalidDate,
        DateTimeOffsetError::InvalidTime,
        DateTimeOffsetError::InvalidOffset,
        DateTimeOffsetError::UtcOutOfRange,
        DateTimeOffsetError::InvalidSyntax,
    ] {
        let error: &dyn std::error::Error = &error;
        assert!(!error.to_string().is_empty());
        assert!(error.source().is_none());
    }
}

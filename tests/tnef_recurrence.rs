// Adapted from MimeKit's TnefCalendarTests and TnefCalendarRobustnessTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    charsets::Charset,
    tnef::{
        TnefDateTime as Date, TnefDateTimeKind as Kind, TnefRecurrence as Recurrence,
        TnefRecurrenceError as Error, TnefRecurrenceException as Exception,
    },
};

#[path = "support/tnef_recurrence.rs"]
mod fixtures;
use fixtures::{master, put16, put32};

fn parse(bytes: &[u8]) -> Recurrence {
    Recurrence::parse(bytes, Charset::ASCII).unwrap()
}
fn set16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn set32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn weekly_dates_arrays_and_unicode_overrides() {
    let bytes = fixtures::weekly();
    assert_eq!(bytes.len(), 194);
    let value = parse(&bytes);
    assert_eq!(value.pattern_type(), Recurrence::PATTERN_WEEK);
    assert_eq!(value.calendar_type(), 0);
    assert_eq!(value.first_date_time(), 0);
    assert_eq!(value.period(), 1);
    assert_eq!(value.day_mask(), 0x0a);
    assert_eq!((value.day_of_month(), value.week_of_month()), (0, 0));
    assert_eq!(value.end_type(), Recurrence::END_AFTER_OCCURRENCES);
    assert_eq!(value.occurrence_count(), 10);
    assert_eq!(value.first_day_of_week(), 0);
    assert_eq!(value.deleted_instance_dates(), [222747840, 222750720]);
    assert_eq!(value.modified_instance_dates(), [222750720]);
    assert_eq!(
        (value.start_date(), value.end_date()),
        (222737760, 222780960)
    );
    assert_eq!(
        (value.start_time_offset(), value.end_time_offset()),
        (600, 660)
    );
    assert_eq!(value.exception_error(), None);
    assert!(!value.had_decode_errors());
    assert_eq!(value.exceptions().len(), 1);
    let ex = &value.exceptions()[0];
    assert_eq!(
        (
            ex.start_date_time(),
            ex.end_date_time(),
            ex.original_start_date()
        ),
        (222751500, 222751560, 222751320)
    );
    assert_eq!(ex.override_flags(), 0x11);
    assert!(ex.has_override(Exception::SUBJECT_FLAG | Exception::REMINDER_FLAG));
    assert!(!ex.has_override(0));
    assert!(!ex.has_override(Exception::REMINDER_FLAG));
    assert_eq!(ex.subject(), Some("Moved s\u{fd}nc"));
    assert_eq!(ex.location(), Some("Room 2"));
    assert_eq!(ex.reminder_delta(), 0);
    assert!(!ex.reminder_set());
    assert_eq!(ex.busy_status(), 0);
    assert!(!ex.is_all_day());
    assert_eq!(value, value.clone());
}

#[test]
fn subject_and_location_extensions_are_independently_optional() {
    for flags in [0, Exception::SUBJECT_FLAG, Exception::LOCATION_FLAG, 0x11] {
        let mut bytes = fixtures::weekly();
        set16(&mut bytes, 96, flags);
        if flags == 0 {
            // Without either text flag there are no extended dates or EE2.
            bytes.drain(138..190);
        } else {
            if flags & Exception::LOCATION_FLAG == 0 {
                bytes.drain(172..186);
            }
            if flags & Exception::SUBJECT_FLAG == 0 {
                bytes.drain(150..172);
            }
        }
        if flags & Exception::LOCATION_FLAG == 0 {
            bytes.drain(112..122);
        }
        if flags & Exception::SUBJECT_FLAG == 0 {
            bytes.drain(98..112);
        }
        let value = parse(&bytes);
        assert_eq!(value.exception_error(), None, "{flags}");
        let ex = &value.exceptions()[0];
        assert_eq!(ex.subject(), (flags & 1 != 0).then_some("Moved s\u{fd}nc"));
        assert_eq!(ex.location(), (flags & 0x10 != 0).then_some("Room 2"));
    }
}

#[test]
fn all_pattern_layouts_preserve_raw_fields_without_calendar_validation() {
    for (pattern, specific, expected) in [
        (Recurrence::PATTERN_DAY, vec![], (0, 0, 0)),
        (
            Recurrence::PATTERN_WEEK,
            vec![0x80000002],
            (0x80000002, 0, 0),
        ),
        (Recurrence::PATTERN_MONTH, vec![32], (0, 32, 0)),
        (Recurrence::PATTERN_MONTH_END, vec![0], (0, 0, 0)),
        (Recurrence::PATTERN_HJ_MONTH, vec![10], (0, 10, 0)),
        (Recurrence::PATTERN_HJ_MONTH_END, vec![31], (0, 31, 0)),
        (Recurrence::PATTERN_MONTH_NTH, vec![0x14, 6], (0x14, 0, 6)),
        (
            Recurrence::PATTERN_HJ_MONTH_NTH,
            vec![0x02, 5],
            (0x02, 0, 5),
        ),
    ] {
        for end in [
            Recurrence::END_AFTER_DATE,
            Recurrence::END_AFTER_OCCURRENCES,
            0x2023,
            u32::MAX,
        ] {
            let mut bytes = fixtures::pattern(pattern, 0, &specific);
            set16(&mut bytes, 8, 0xffff);
            set32(&mut bytes, 10, u32::MAX);
            let end_offset = 22 + specific.len() * 4;
            set32(&mut bytes, end_offset, end);
            set32(&mut bytes, end_offset + 4, 0);
            set32(&mut bytes, end_offset + 8, 7);
            let value = parse(&bytes);
            assert_eq!(value.pattern_type(), pattern);
            assert_eq!(value.calendar_type(), 0xffff);
            assert_eq!(value.first_date_time(), u32::MAX);
            assert_eq!(value.period(), 0);
            assert_eq!(
                (
                    value.day_mask(),
                    value.day_of_month(),
                    value.week_of_month()
                ),
                expected
            );
            assert_eq!(value.end_type(), end);
            assert_eq!(value.occurrence_count(), 0);
            assert_eq!(value.first_day_of_week(), 7);
            assert_eq!(value.exception_error(), None);
        }
    }
}

#[test]
fn unsupported_versions_patterns_and_error_diagnostics() {
    let mut bytes = fixtures::pattern(1, 1, &[2]);
    set16(&mut bytes, 0, 0x3005);
    assert_eq!(
        Recurrence::parse(&bytes, Charset::ASCII),
        Err(Error::UnsupportedVersion(0x3005))
    );
    set16(&mut bytes, 0, 0x3004);
    for pattern in [5, 9, 13, u16::MAX] {
        set16(&mut bytes, 6, pattern);
        assert_eq!(
            Recurrence::parse(&bytes, Charset::ASCII),
            Err(Error::UnknownPattern(pattern))
        );
    }
    for (error, message) in [
        (
            Error::UnexpectedEnd { offset: 82 },
            "unexpected end of recurrence data at byte 82",
        ),
        (
            Error::UnsupportedVersion(0x3005),
            "unsupported recurrence pattern version 0x3005",
        ),
        (
            Error::UnknownPattern(0xffff),
            "unknown recurrence pattern type 0xFFFF",
        ),
    ] {
        assert_eq!(error.to_string(), message);
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn every_weekly_prefix_respects_the_exact_master_exception_boundary() {
    let bytes = fixtures::weekly();
    for length in 0..=bytes.len() {
        let result = Recurrence::parse(&bytes[..length], Charset::ASCII);
        if length < 82 {
            assert!(
                matches!(result, Err(Error::UnexpectedEnd { .. })),
                "{length}: {result:?}"
            );
        } else {
            let value = result.unwrap();
            assert_eq!(value.day_mask(), 0x0a);
            assert_eq!(value.modified_instance_dates(), [222750720]);
            if length < 190 {
                assert!(value.exceptions().is_empty(), "{length}");
                assert!(
                    matches!(value.exception_error(), Some(Error::UnexpectedEnd { offset }) if *offset <= length),
                    "{length}"
                );
            } else {
                assert_eq!(value.exceptions().len(), 1, "{length}");
                assert_eq!(value.exception_error(), None);
            }
        }
    }
}

#[test]
fn partial_exception_info_and_extended_exceptions_discard_every_exception() {
    let bytes = fixtures::many(3);
    // Weekly master plus both three-date arrays; three 14-byte ExceptionInfo
    // records, ReservedBlock1, and three 12-byte ExtendedException records.
    let master_end = 70 + 24;
    for length in master_end..bytes.len() - 4 {
        let value = parse(&bytes[..length]);
        assert_eq!(value.deleted_instance_dates().len(), 3);
        assert_eq!(value.occurrence_count(), 13);
        assert!(value.exceptions().is_empty(), "{length}");
        assert!(value.exception_error().is_some(), "{length}");
    }
    let value = parse(&bytes);
    assert_eq!(value.exceptions().len(), 3);
    assert_eq!(value.exception_error(), None);
}

#[test]
fn malformed_arrays_are_fatal_before_allocating_or_wrapping_their_size() {
    for offset in [38, 42] {
        for count in [1, u32::MAX, u32::MAX / 4 + 1] {
            let mut bytes = master(1, 1, &[2]);
            bytes.truncate(offset + 4);
            set32(&mut bytes, offset, count);
            assert_eq!(
                Recurrence::parse(&bytes, Charset::ASCII),
                Err(Error::UnexpectedEnd { offset: offset + 4 })
            );
        }
    }
    let mut bytes = master(1, 1, &[2]);
    put16(&mut bytes, u16::MAX);
    assert_eq!(
        parse(&bytes).exception_error(),
        Some(&Error::UnexpectedEnd { offset: 72 })
    );
}

#[test]
fn ignored_versions_fields_and_trailing_blocks_do_not_change_the_model() {
    let bytes = fixtures::weekly();
    let expected = parse(&bytes);
    let mut modified = bytes.clone();
    set16(&mut modified, 2, 0xffff); // WriterVersion
    set16(&mut modified, 4, 0); // RecurFrequency
    set32(&mut modified, 18, u32::MAX); // SlidingFlag
    set32(&mut modified, 66, 0); // ReaderVersion2
    set32(&mut modified, 70, u32::MAX); // WriterVersion2 still >= 0x3009
    modified[138..150].fill(0xff); // Extended dates do not replace ExceptionInfo.
    modified.truncate(190);
    modified.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]);
    assert_eq!(parse(&modified), expected);
}

#[test]
fn reserved_blocks_and_old_extended_versions() {
    for version in [0, 0x3008, 0x3009, u32::MAX] {
        let mut bytes = fixtures::weekly();
        set32(&mut bytes, 70, version);
        if version < 0x3009 {
            bytes.drain(126..134);
        }
        let expected = parse(&bytes);
        assert_eq!(expected.exception_error(), None);
        // Insert nonempty blocks in reverse order to keep original offsets.
        let adjustment = if version < 0x3009 { 8 } else { 0 };
        for offset in [186 - adjustment, 134 - adjustment, 122] {
            set32(&mut bytes, offset, 3);
            bytes.splice(offset + 4..offset + 4, [0xf1, 0xf2, 0xf3]);
        }
        assert_eq!(parse(&bytes), expected);
    }
}

#[test]
fn oversized_reserved_blocks_and_string_lengths_are_recoverable() {
    for offset in [122, 126, 134, 186] {
        let mut bytes = fixtures::weekly();
        set32(&mut bytes, offset, u32::MAX);
        assert_eq!(
            parse(&bytes).exception_error(),
            Some(&Error::UnexpectedEnd { offset: offset + 4 })
        );
    }
    for offset in [100, 114, 150, 172] {
        let mut bytes = fixtures::weekly();
        set16(&mut bytes, offset, u16::MAX);
        let value = parse(&bytes);
        assert!(value.exceptions().is_empty());
        assert_eq!(
            value.exception_error(),
            Some(&Error::UnexpectedEnd { offset: offset + 2 })
        );
    }
}

#[test]
fn every_flag_payload_signed_values_and_unknown_bits_are_preserved() {
    let mut bytes = master(1, 1, &[0x0a]);
    put16(&mut bytes, 2);
    for flags in [0xffff, Exception::EXCEPTIONAL_BODY_FLAG | 0x8000] {
        for value in [u32::MAX, 0, 1] {
            put32(&mut bytes, value);
        }
        put16(&mut bytes, flags);
        if flags == 0xffff {
            for v in [0, 1] {
                put16(&mut bytes, v);
            } // Ignored SubjectLength.
            bytes.push(b'A');
            for v in [0x12345678, u32::MAX, 2] {
                put32(&mut bytes, v);
            }
            for v in [0, 1] {
                put16(&mut bytes, v);
            }
            bytes.push(b'B');
            for v in [0x80000000, 0x11111111, 0x80000000, 0x22222222] {
                put32(&mut bytes, v);
            }
        }
    }
    put32(&mut bytes, 0);
    for _ in 0..2 {
        put32(&mut bytes, 0);
    } // ChangeHighlight / EE1.
    for _ in 0..3 {
        put32(&mut bytes, 0);
    } // Repeated dates.
    put16(&mut bytes, 0); // Empty Unicode overrides both 8-bit values.
    put16(&mut bytes, 0);
    put32(&mut bytes, 0);
    for _ in 0..2 {
        put32(&mut bytes, 0);
    } // No strings or EE2 for the second exception.
    let value = parse(&bytes);
    assert_eq!(value.exception_error(), None);
    assert_eq!(value.exceptions().len(), 2);
    let ex = &value.exceptions()[0];
    assert_eq!(ex.override_flags(), u16::MAX);
    assert_eq!(
        (
            ex.start_date_time(),
            ex.end_date_time(),
            ex.original_start_date()
        ),
        (u32::MAX, 0, 1)
    );
    assert_eq!(ex.subject(), Some(""));
    assert_eq!(ex.location(), Some(""));
    assert_eq!(ex.reminder_delta(), -1);
    assert!(ex.reminder_set());
    assert_eq!(ex.busy_status(), i32::MIN);
    assert!(ex.is_all_day());
    assert_eq!(value.exceptions()[1].override_flags(), 0x8200);
    assert_eq!(value.exceptions()[1].subject(), None);
    assert_eq!(value.exceptions()[1].location(), None);
}

#[test]
fn upstream_reminder_and_busy_status_overrides() {
    let mut bytes = master(1, 1, &[0x0a]);
    put16(&mut bytes, 3);
    for (day, flags, values) in [
        (222741240, 0x002c, vec![5, 1, 0]),
        (222748440, 0x0008, vec![0]),
        (222751320, 0x0008, vec![1]),
    ] {
        for v in [day, day + 60, day] {
            put32(&mut bytes, v);
        }
        put16(&mut bytes, flags);
        for v in values {
            put32(&mut bytes, v);
        }
    }
    for _ in 0..7 {
        put32(&mut bytes, 0);
    }
    let value = parse(&bytes);
    assert_eq!(value.exception_error(), None);
    assert_eq!(value.exceptions().len(), 3);
    assert_eq!(value.exceptions()[0].reminder_delta(), 5);
    assert!(value.exceptions()[0].reminder_set());
    assert_eq!(value.exceptions()[0].busy_status(), 0);
    assert!(!value.exceptions()[1].reminder_set());
    assert!(value.exceptions()[2].reminder_set());
}

#[test]
fn text_replacement_is_reported_even_for_overwritten_or_discarded_values() {
    let mut bytes = fixtures::weekly();
    bytes[102] = 0xe9;
    let ascii = parse(&bytes);
    assert!(ascii.had_decode_errors());
    assert_eq!(ascii.exceptions()[0].subject(), Some("Moved s\u{fd}nc"));
    for charset in [
        Charset::LATIN1,
        Charset::from_label("windows-1252").unwrap(),
    ] {
        let value = Recurrence::parse(&bytes, charset).unwrap();
        assert!(!value.had_decode_errors());
        assert_eq!(value.exceptions(), ascii.exceptions());
    }
    assert!(
        Recurrence::parse(&bytes, Charset::UTF8)
            .unwrap()
            .had_decode_errors()
    );
    bytes.truncate(112); // Upstream's truncated-exceptions fixture.
    let value = parse(&bytes);
    assert!(value.had_decode_errors());
    assert!(value.exceptions().is_empty());
    assert!(value.exception_error().is_some());
}

#[test]
fn unicode_preserves_boms_spaces_embedded_nuls_and_surrogate_pairs() {
    let mut bytes = fixtures::weekly();
    // Ten UTF-16 units: BOM, space, a surrogate pair, NUL, A, space, NULs.
    let units = [0xfeff, 0x20, 0xd83d, 0xde00, 0, 0x41, 0x20, 0, 0, 0];
    for (i, unit) in units.into_iter().enumerate() {
        set16(&mut bytes, 152 + i * 2, unit);
    }
    let value = parse(&bytes);
    assert_eq!(
        value.exceptions()[0].subject(),
        Some("\u{feff} \u{1f600}\0A ")
    );
    assert!(!value.had_decode_errors());
    set16(&mut bytes, 156, 0xd800);
    set16(&mut bytes, 158, 0x41);
    let value = parse(&bytes);
    assert!(value.had_decode_errors());
    assert_eq!(
        value.exceptions()[0].subject(),
        Some("\u{feff} \u{fffd}A\0A ")
    );
    // The redundant legacy lengths are ignored, unlike the byte counts.
    set16(&mut bytes, 98, u16::MAX);
    set16(&mut bytes, 112, 0);
    assert_eq!(parse(&bytes), value);
}

#[test]
fn local_minutes_cover_the_entire_unsigned_range_without_wrapping() {
    for (minutes, parts) in [
        (0, (1601, 1, 1, 0, 0)),
        (222737760, (2024, 7, 1, 0, 0)),
        (222751500, (2024, 7, 10, 13, 0)),
        (u32::MAX, (9767, 2, 16, 4, 15)),
    ] {
        let value = Date::from_recurrence_minutes(minutes);
        assert_eq!(
            value,
            Date::new(
                parts.0,
                parts.1,
                parts.2,
                parts.3,
                parts.4,
                0,
                Kind::Unspecified
            )
            .unwrap()
        );
        assert_eq!(
            value.ticks(),
            504911232000000000 + u64::from(minutes) * 600000000
        );
        assert_eq!(value.nanosecond(), 0);
    }
    for minutes in [i32::MAX as u32, i32::MAX as u32 + 1, u32::MAX - 1] {
        assert_eq!(
            Date::from_recurrence_minutes(minutes + 1).ticks()
                - Date::from_recurrence_minutes(minutes).ticks(),
            600000000
        );
    }
}

#[test]
fn parser_mutation_fuzz_and_maximum_exception_count() {
    let source = fixtures::weekly();
    let mut seed = 20240701u32;
    fn next(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *seed
    }
    for _ in 0..5000 {
        let mut bytes = source.clone();
        for _ in 0..1 + next(&mut seed) % 4 {
            let index = next(&mut seed) as usize % bytes.len();
            bytes[index] = match next(&mut seed) % 3 {
                0 => next(&mut seed) as u8,
                1 => 0xff,
                _ => bytes[index] ^ (1 << (next(&mut seed) % 8)),
            };
        }
        match Recurrence::parse(&bytes, Charset::ASCII) {
            Ok(value) => {
                assert!(value.exceptions().len() <= usize::from(u16::MAX));
                if value.exception_error().is_some() {
                    assert!(value.exceptions().is_empty());
                }
            }
            Err(Error::UnexpectedEnd { offset }) => assert!(offset <= bytes.len()),
            Err(Error::UnsupportedVersion(_) | Error::UnknownPattern(_)) => {}
        }
    }
    let value = parse(&fixtures::many(u16::MAX));
    assert_eq!(value.exceptions().len(), usize::from(u16::MAX));
    assert_eq!(value.deleted_instance_dates().len(), usize::from(u16::MAX));
    assert_eq!(value.exception_error(), None);
}

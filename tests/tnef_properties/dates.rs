// Adapted from MimeKit's TnefPropertyReaderCoercionTests,
// TnefPropertyReaderRobustnessTests, TnefPropertySetTests, and TnefMessageTests.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use super::*;
use mimecraft::tnef::{
    TnefAttributeType, TnefDateTime as Date, TnefDateTimeError as DateError,
    TnefDateTimeKind as Kind,
};

fn legacy(parts: [u16; 7]) -> Vec<u8> {
    parts.into_iter().flat_map(u16::to_le_bytes).collect()
}

const FILE_TIME_EPOCH: u64 = 504911232000000000;
const MAX_FILE_TIME: i64 = 2650467743999999999;

#[test]
fn filetime_keeps_utc_kind_and_every_hundred_nanoseconds() {
    for value in [
        0,
        1,
        9_999_999,
        10_000_000,
        133551423120000001,
        MAX_FILE_TIME,
    ] {
        let date = Date::from_file_time(value).unwrap();
        assert_eq!(date.ticks(), FILE_TIME_EPOCH + value as u64);
        assert_eq!(date.kind(), Kind::Utc);
        assert_eq!(date.nanosecond(), (value % 10_000_000) as u32 * 100);
    }
    let date = Date::from_file_time(133551423120000001).unwrap();
    assert_eq!((date.year(), date.month(), date.day()), (2024, 3, 17));
    assert_eq!(
        (date.hour(), date.minute(), date.second(), date.nanosecond()),
        (9, 45, 12, 100)
    );
    let max = Date::from_file_time(MAX_FILE_TIME).unwrap();
    assert_eq!((max.year(), max.month(), max.day()), (9999, 12, 31));
    assert_eq!(
        (max.hour(), max.minute(), max.second(), max.nanosecond()),
        (23, 59, 59, 999999900)
    );
    for value in [-1, i64::MIN, MAX_FILE_TIME + 1, i64::MAX] {
        assert_eq!(Date::from_file_time(value), Err(DateError::OutOfRange));
    }
}

#[test]
fn oa_dates_match_dotnet_rounding_and_boundary_vectors() {
    // Captured from DateTime.FromOADate on .NET 10. Inputs use their exact
    // IEEE bits so decimal formatting cannot move a rounding boundary.
    let cases = [
        (0x0000000000000000, Some(599264352000000000)),
        (0x8000000000000000, Some(599264352000000000)),
        (0x3fd0000000000000, Some(599264568000000000)),
        (0xbfd0000000000000, Some(599264568000000000)),
        (0x3ff4000000000000, Some(599265432000000000)),
        (0xbff4000000000000, Some(599263704000000000)),
        (0xbffc000000000000, Some(599264136000000000)),
        (0x4000000000000000, Some(599266080000000000)),
        (0x40d8f84000000000, Some(621355968000000000)),
        (0x40e6270d01234568, Some(638462655120000000)),
        (0xc124103600000000, None),
        // Rounding just inside the lower input boundary reaches year 99.
        (0xc1241035ffffffff, Some(31240512000000000)),
        (0xc124103400000000, Some(31241376000000000)),
        (0x4146924100000000, None),
        (0x41469240ffffffff, None),
        (0x41469240ffffac1d, Some(3155378975991360000)),
        (0x46293e5939a08cea, None),
        (0xfff8000000000000, None),
        (0x7ff0000000000000, None),
        (0xfff0000000000000, None),
        (0x3e38daea1d7f4cf7, Some(599264352000010000)),
        (0xbe38daea1d7f4cf7, Some(599264352000010000)),
        (0x3e52a42f961f79b9, Some(599264352000020000)),
        (0xbe52a42f961f79b9, Some(599264352000020000)),
        (0x0000000000000001, Some(599264352000000000)),
    ];
    for (bits, ticks) in cases {
        let result = Date::from_oa_date(f64::from_bits(bits));
        if let Some(ticks) = ticks {
            let date = result.unwrap();
            assert_eq!(date.ticks(), ticks, "{bits:x}");
            assert_eq!(date.kind(), Kind::Unspecified);
            assert_eq!(date.nanosecond() % 1_000_000, 0);
        } else {
            assert_eq!(result, Err(DateError::OutOfRange), "{bits:x}");
        }
    }
}

#[test]
fn calendar_components_cover_centuries_leap_days_and_tick_limits() {
    for year in 1..=9999 {
        for month in 1..=12 {
            for day in [1, 28, 29, 30, 31] {
                let Ok(date) = Date::new(year, month, day, 23, 59, 59, Kind::Unspecified) else {
                    continue;
                };
                let date = Date::from_ticks(date.ticks() + 9_999_999, Kind::Unspecified).unwrap();
                assert_eq!((date.year(), date.month(), date.day()), (year, month, day));
                assert_eq!((date.hour(), date.minute(), date.second()), (23, 59, 59));
                assert_eq!(date.nanosecond(), 999999900);
            }
        }
    }
    let min = Date::from_ticks(0, Kind::Unspecified).unwrap();
    assert_eq!((min.year(), min.month(), min.day()), (1, 1, 1));
    let max = Date::from_file_time(MAX_FILE_TIME).unwrap();
    assert_eq!(
        Date::from_ticks(max.ticks() + 1, Kind::Utc),
        Err(DateError::OutOfRange)
    );
    assert_eq!(
        Date::from_ticks(u64::MAX, Kind::Utc),
        Err(DateError::OutOfRange)
    );
    assert_ne!(min, Date::from_ticks(0, Kind::Utc).unwrap());
    for parts in [
        (0, 1, 1, 0, 0, 0),
        (10000, 1, 1, 0, 0, 0),
        (1900, 2, 29, 0, 0, 0),
        (2024, 0, 1, 0, 0, 0),
        (2024, 13, 1, 0, 0, 0),
        (2024, 1, 0, 0, 0, 0),
        (2024, 4, 31, 0, 0, 0),
        (2024, 1, 1, 24, 0, 0),
        (2024, 1, 1, 0, 60, 0),
        (2024, 1, 1, 0, 0, 60),
    ] {
        assert_eq!(
            Date::new(
                parts.0,
                parts.1,
                parts.2,
                parts.3,
                parts.4,
                parts.5,
                Kind::Utc
            ),
            Err(DateError::InvalidComponents)
        );
    }
}

#[test]
fn legacy_dates_are_cached_ignore_weekdays_and_do_not_inherit_prior_dates() {
    let mut first = legacy([2024, 3, 15, 10, 30, 0, u16::MAX]);
    first.extend_from_slice(&[1, 2, 3]);
    let mut data = document(Attribute::DATE_SENT, &first);
    attribute(&mut data, Attribute::DATE_RECEIVED, &[0xe9, 7, 1, 0]);
    attribute(
        &mut data,
        Attribute::DATE_MODIFIED,
        &legacy([2024, 3, 16, 0, 0, 0, 0]),
    );
    let options = TnefReaderOptions {
        max_value_length: 0,
        max_total_data_bytes: 0,
        ..Default::default()
    };
    for chunk in [1, 7, 4096] {
        let mut reader = TnefReader::with_options(
            Chunked {
                input: &data,
                chunk,
            },
            &options,
        )
        .unwrap();
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert!(matches!(
            reader.read_value_datetime(),
            Err(Error::NoAttribute)
        ));
        assert!(reader.read_next().unwrap());
        let date = reader.read_value_datetime_with_logger(&mut logger).unwrap();
        assert_eq!(
            date,
            Date::new(2024, 3, 15, 10, 30, 0, Kind::Unspecified).unwrap()
        );
        let position = reader.position();
        assert_eq!(
            reader.read_value_datetime_with_logger(&mut logger).unwrap(),
            date
        );
        assert_eq!(reader.position(), position);
        assert_eq!(reader.value_remaining(), 3);
        assert!(matches!(reader.open_value(), Err(Error::ValueAlreadyRead)));
        assert!(matches!(
            reader.read_value_i32(),
            Err(Error::InvalidValueType(_))
        ));
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        for _ in 0..2 {
            assert!(matches!(
                reader.read_value_datetime_with_logger(&mut logger),
                Err(Error::InvalidDate(DateError::IncompleteValue))
            ));
        }
        assert!(!reader.is_faulted());
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(reader.read_value_datetime().unwrap().day(), 16);
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::InvalidAttributeValue);
    }
}

#[test]
fn invalid_legacy_fields_return_errors_without_faulting_or_relogging() {
    for (field, bad) in [
        (0, 0),
        (0, 10000),
        (0, u16::MAX),
        (1, 0),
        (1, 256),
        (2, 32),
        (3, 24),
        (4, 60),
        (5, 60),
        (5, u16::MAX),
    ] {
        let mut parts = [2024, 1, 2, 3, 4, 5, 2];
        parts[field] = bad;
        let mut data = document(Attribute::DATE_SENT, &legacy(parts));
        attribute(&mut data, Attribute::BODY, b"next");
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        for _ in 0..2 {
            assert!(matches!(
                reader.read_value_datetime_with_logger(&mut logger),
                Err(Error::InvalidDate(DateError::InvalidComponents))
            ));
        }
        assert!(!reader.is_faulted());
        assert!(reader.read_next_with_logger(&mut logger).unwrap());
        assert!(matches!(
            reader.read_value_datetime(),
            Err(Error::InvalidValueType(_))
        ));
        assert_eq!(reader.read_value_bytes().unwrap(), b"next");
        assert_eq!(
            issues,
            [Issue {
                violation: Violation::InvalidDate,
                depth: 0,
                stream_offset: 6,
                attribute_tag: Some(Attribute::DATE_SENT),
                property_tag: None,
            }]
        );
    }
    assert_eq!(Violation::InvalidDate as u8, 12);
}

#[test]
fn physical_date_truncations_and_raw_claims_are_distinct() {
    let data = document(Attribute::DATE_SENT, &legacy([2024, 1, 2, 3, 4, 5, 2]));
    for count in 0..14 {
        let mut reader = TnefReader::new(Chunked {
            input: &data[..15 + count],
            chunk: 1,
        });
        let mut issues = Vec::new();
        let mut logger = |issue| {
            issues.push(issue);
            Ok(())
        };
        assert!(reader.read_next().unwrap());
        assert!(matches!(
            reader.read_value_datetime_with_logger(&mut logger),
            Err(Error::InvalidDate(DateError::IncompleteValue))
        ));
        assert!(matches!(
            reader.read_value_datetime(),
            Err(Error::InvalidDate(DateError::IncompleteValue))
        ));
        assert!(reader.is_truncated());
        assert!(!reader.is_faulted());
        assert!(!reader.read_next_with_logger(&mut logger).unwrap());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].violation, Violation::TruncatedStream);
        assert_eq!(issues[0].stream_offset, 15 + count as u64);
    }
    for stream in [false, true] {
        let mut reader = TnefReader::new(data.as_slice());
        assert!(reader.read_next().unwrap());
        if stream {
            let _value = reader.open_value().unwrap();
        } else {
            reader.read_value_bytes().unwrap();
        }
        assert!(matches!(
            reader.read_value_datetime(),
            Err(Error::ValueAlreadyRead)
        ));
    }
}

#[test]
fn mapi_dates_preserve_native_values_and_owned_snapshots() {
    let mut multi = fixed(0x0042, Type(Type::SYS_TIME.0 | 0x1000), &3i32.to_le_bytes());
    for value in [0i64, 1, MAX_FILE_TIME] {
        multi.extend_from_slice(&value.to_le_bytes());
    }
    let data = document(
        Attribute::MAPI_PROPERTIES,
        &properties(&[
            fixed(0x0040, Type::SYS_TIME, &133551423120000001i64.to_le_bytes()),
            fixed(0x0041, Type::APP_TIME, &2f64.to_le_bytes()),
            multi,
            fixed(0x0043, Type::I8, &0i64.to_le_bytes()),
        ]),
    );
    let mut reader = TnefReader::new(data.as_slice());
    assert!(reader.read_next().unwrap());
    let mut props = reader.open_properties().unwrap();
    assert!(matches!(
        props.read_value_datetime(),
        Err(Error::NoPropertyValue)
    ));
    for expected in [Value::SysTime(133551423120000001), Value::AppTime(2.0)] {
        assert!(props.read_next_property().unwrap());
        let date = props.read_value_datetime().unwrap();
        assert_eq!(date, expected.to_datetime().unwrap());
        assert_eq!(props.read_value_datetime().unwrap(), date);
        assert_eq!(props.read_value().unwrap(), expected);
        let snapshot = props.read_property().unwrap();
        assert_eq!(snapshot.value().unwrap().to_datetime().unwrap(), date);
    }
    assert!(props.read_next_property().unwrap());
    let snapshot = props.read_property().unwrap();
    assert_eq!(snapshot.value_count(), 3);
    for (value, filetime) in snapshot.values().iter().zip([0, 1, MAX_FILE_TIME]) {
        assert_eq!(
            value.to_datetime().unwrap(),
            Date::from_file_time(filetime).unwrap()
        );
    }
    assert!(props.read_next_property().unwrap());
    assert!(matches!(
        props.read_value_datetime(),
        Err(Error::InvalidPropertyType(_))
    ));
    assert_eq!(
        props.read_value().unwrap().to_datetime(),
        Err(DateError::NotDate)
    );
    drop(props);
    assert!(!reader.read_next().unwrap());
    assert_eq!(
        snapshot.values()[1].to_datetime().unwrap().nanosecond(),
        100
    );
}

#[test]
fn invalid_mapi_dates_are_diagnosed_on_selection_but_raw_bits_survive() {
    let cases = [
        (Type::APP_TIME, (1e30f64).to_bits()),
        (Type::APP_TIME, f64::NAN.to_bits()),
        (Type::APP_TIME, f64::INFINITY.to_bits()),
        (Type::APP_TIME, f64::NEG_INFINITY.to_bits()),
        (Type::SYS_TIME, i64::MAX as u64),
        (Type::SYS_TIME, u64::MAX),
        (Type::SYS_TIME, (MAX_FILE_TIME + 1) as u64),
    ];
    for (kind, bits) in cases {
        for skip in [false, true] {
            let data = document(
                Attribute::MAPI_PROPERTIES,
                &properties(&[
                    fixed(0x0042, kind, &bits.to_le_bytes()),
                    fixed(0x0043, Type::LONG, &7i32.to_le_bytes()),
                ]),
            );
            let mut reader = TnefReader::new(data.as_slice());
            let mut issues = Vec::new();
            let mut logger = |issue| {
                issues.push(issue);
                Ok(())
            };
            assert!(reader.read_next().unwrap());
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(props.read_next_property().unwrap());
            if !skip {
                for _ in 0..2 {
                    assert!(matches!(
                        props.read_value_datetime(),
                        Err(Error::InvalidDate(DateError::OutOfRange))
                    ));
                }
                let snapshot = props.read_property().unwrap();
                let raw = match snapshot.value().unwrap() {
                    Value::AppTime(value) => value.to_bits(),
                    Value::SysTime(value) => *value as u64,
                    _ => panic!("raw date was lost"),
                };
                assert_eq!(raw, bits);
            }
            assert!(props.read_next_property().unwrap());
            assert_eq!(props.read_value().unwrap(), Value::I32(7));
            drop(props);
            assert!(!reader.read_next_with_logger(&mut logger).unwrap());
            assert_eq!(
                issues,
                [Issue {
                    violation: Violation::InvalidDate,
                    depth: 0,
                    stream_offset: 23,
                    attribute_tag: Some(Attribute::MAPI_PROPERTIES),
                    property_tag: Some(Tag::new(0x0042, kind)),
                }]
            );
        }
    }
}

#[test]
fn date_report_caps_are_shared_and_checksum_context_is_cleared() {
    let mut multi = fixed(0x0042, Type(Type::SYS_TIME.0 | 0x1000), &3i32.to_le_bytes());
    for value in [-1i64, -2, -3] {
        multi.extend_from_slice(&value.to_le_bytes());
    }
    let mut data = document(Attribute::MAPI_PROPERTIES, &properties(&[multi]));
    let last = data.len() - 1;
    data[last] ^= 1;
    attribute(&mut data, Attribute::DATE_SENT, &legacy([0; 7]));
    let options = TnefReaderOptions {
        max_issues_per_violation: 2,
        max_total_data_bytes: 0,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(data.as_slice(), &options).unwrap();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    assert!(reader.read_next().unwrap());
    let set = reader
        .open_properties_with_logger(&mut logger)
        .unwrap()
        .read_property_set()
        .unwrap();
    assert_eq!(set[0].value_count(), 3);
    assert!(reader.read_next_with_logger(&mut logger).unwrap());
    assert!(matches!(
        reader.read_value_datetime_with_logger(&mut logger),
        Err(Error::InvalidDate(_))
    ));
    assert!(!reader.read_next_with_logger(&mut logger).unwrap());
    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0].stream_offset, 27);
    assert_eq!(issues[1].stream_offset, 35);
    assert_eq!(issues[2].violation, Violation::AttributeChecksumMismatch);
    assert_eq!(issues[2].property_tag, None);
}

#[test]
fn date_logger_and_source_failures_are_terminal() {
    struct Source<'a> {
        bytes: &'a [u8],
        interrupt: bool,
    }
    impl Read for Source<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if std::mem::take(&mut self.interrupt) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.bytes.is_empty() {
                return Err(io::Error::other("date source failed"));
            }
            self.bytes.read(output)
        }
    }
    let data = document(Attribute::DATE_SENT, &legacy([2024, 1, 2, 3, 4, 5, 0]));
    let mut reader = TnefReader::new(Source {
        bytes: &data[..20],
        interrupt: true,
    });
    assert!(reader.read_next().unwrap());
    assert!(matches!(reader.read_value_datetime(), Err(Error::Io(_))));
    assert!(matches!(reader.read_value_datetime(), Err(Error::Faulted)));
    assert!(matches!(reader.read_next(), Err(Error::Faulted)));

    for mapi in [false, true] {
        let data = if mapi {
            document(
                Attribute::MAPI_PROPERTIES,
                &properties(&[fixed(0x0042, Type::SYS_TIME, &(-1i64).to_le_bytes())]),
            )
        } else {
            document(Attribute::DATE_SENT, &legacy([0; 7]))
        };
        let mut reader = TnefReader::new(data.as_slice());
        let mut logger = |_: Issue| Err(io::Error::other("date logger failed"));
        assert!(reader.read_next().unwrap());
        if mapi {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            assert!(matches!(props.read_next_property(), Err(Error::Io(_))));
            assert!(matches!(props.read_value_datetime(), Err(Error::Faulted)));
        } else {
            assert!(matches!(
                reader.read_value_datetime_with_logger(&mut logger),
                Err(Error::Io(_))
            ));
            assert!(matches!(reader.read_value_datetime(), Err(Error::Faulted)));
        }
        assert!(matches!(reader.read_next(), Err(Error::Faulted)));
    }
}

type DateResults = Vec<(u64, Result<Date, DateError>)>;

fn corpus_dates(input: &[u8], chunk: usize) -> (DateResults, Vec<Issue>) {
    let options = TnefReaderOptions {
        max_value_length: usize::MAX,
        max_total_data_bytes: u64::MAX,
        ..Default::default()
    };
    let mut reader = TnefReader::with_options(Chunked { input, chunk }, &options).unwrap();
    let mut dates = Vec::new();
    let mut issues = Vec::new();
    let mut logger = |issue| {
        issues.push(issue);
        Ok(())
    };
    while reader.read_next_with_logger(&mut logger).unwrap() {
        let info = *reader.attribute().unwrap();
        if info.tag.attribute_type() == TnefAttributeType::Date {
            let date = match reader.read_value_datetime_with_logger(&mut logger) {
                Ok(date) => Ok(date),
                Err(Error::InvalidDate(error)) => Err(error),
                other => panic!("unexpected date result: {other:?}"),
            };
            dates.push((info.offset, date));
        } else if matches!(
            info.tag,
            Attribute::MAPI_PROPERTIES | Attribute::ATTACHMENT | Attribute::RECIPIENT_TABLE
        ) {
            let mut props = reader.open_properties_with_logger(&mut logger).unwrap();
            loop {
                if props.is_table() && !props.read_next_row().unwrap() {
                    break;
                }
                while props.read_next_property().unwrap() {
                    if !matches!(
                        props.property().unwrap().tag.value_type(),
                        Type::APP_TIME | Type::SYS_TIME
                    ) {
                        continue;
                    }
                    while props.has_value() {
                        let offset = props.position();
                        dates.push((offset, props.read_value().unwrap().to_datetime()));
                        if !props.read_next_value().unwrap() {
                            break;
                        }
                    }
                }
                if !props.is_table() {
                    break;
                }
            }
        }
    }
    (dates, issues)
}

#[test]
fn corpus_date_interpretations_are_chunk_independent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("tnef");
    let mut count = 0;
    let mut dates = 0;
    for file in std::fs::read_dir(root).unwrap() {
        let path = file.unwrap().path();
        if path.extension().and_then(|value| value.to_str()) != Some("tnef") {
            continue;
        }
        count += 1;
        let data = std::fs::read(&path).unwrap();
        let expected = corpus_dates(&data, 4096);
        dates += expected.0.len();
        for chunk in [1, 7] {
            assert_eq!(
                corpus_dates(&data, chunk),
                expected,
                "{} chunk={chunk}",
                path.display()
            );
        }
    }
    assert_eq!(count, 19);
    assert!(dates > 0);
}

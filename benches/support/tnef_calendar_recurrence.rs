// Original workloads based on MimeKit's TnefCalendarBuilder and calendar tests.
// MimeKit has no corresponding calendar-recurrence benchmark.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    charsets::Charset,
    tnef::{
        TnefDateTime as Date, TnefDateTimeKind as Kind, TnefRecurrence,
        TnefSystemTime as SystemTime, TnefTimeZone as Zone,
        calendar::{RecurrenceRuleError, recurrence_exdates, recurrence_rule},
    },
};
use std::hint::black_box;

use super::recurrence_fixtures as fixtures;

pub const CASES: [&str; 8] = [
    "Daily",
    "Weekly",
    "YearlyNth",
    "UntilUtc",
    "UntilDate",
    "ExDates",
    "ManyExDates",
    "UnsupportedCalendar",
];

enum Expected {
    Rule(&'static str),
    Dates(Vec<Date>),
    Error(RecurrenceRuleError),
}

pub struct Workload {
    pub name: &'static str,
    /// Recurrences processed per batch; no fictitious byte throughput.
    pub operations: usize,
    recurrence: TnefRecurrence,
    zone: Zone,
    expected: Expected,
    expected_summary: Summary,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub values: usize,
    pub checksum: u64,
}

impl Summary {
    fn add(&mut self, value: u64) {
        self.values += 1;
        self.checksum = self.checksum.wrapping_mul(31).wrapping_add(value);
    }
    fn rule(&mut self, rule: &str) {
        self.add(rule.len() as u64);
        for byte in rule.bytes() {
            self.add(u64::from(byte));
        }
    }
    fn dates(&mut self, dates: &[Date]) {
        self.add(dates.len() as u64);
        for date in dates {
            self.add(date.ticks());
            self.add(u64::from(date.kind() == Kind::Utc));
        }
    }
    fn error(&mut self, error: RecurrenceRuleError) {
        match error {
            RecurrenceRuleError::UnsupportedCalendar {
                pattern_type,
                calendar_type,
            } => {
                self.add(u64::from(pattern_type));
                self.add(u64::from(calendar_type));
            }
            other => panic!("unexpected workload error: {other}"),
        }
    }
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let (mut bytes, expected) = match name {
        "Daily" => (
            fixtures::pattern(0, 2880, &[]),
            Expected::Rule("FREQ=DAILY;INTERVAL=2;COUNT=10"),
        ),
        "Weekly" => (
            fixtures::pattern(1, 2, &[0x7f]),
            Expected::Rule("FREQ=WEEKLY;BYDAY=SU,MO,TU,WE,TH,FR,SA;INTERVAL=2;COUNT=10;WKST=SU"),
        ),
        "YearlyNth" => (
            fixtures::pattern(3, 24, &[0x14, 5]),
            Expected::Rule("FREQ=YEARLY;BYDAY=TU,TH;BYMONTH=7;BYSETPOS=-1;INTERVAL=2;COUNT=10"),
        ),
        "UntilUtc" => (
            fixtures::pattern(0, 1440, &[]),
            Expected::Rule("FREQ=DAILY;UNTIL=20240731T140000Z"),
        ),
        "UntilDate" => (
            fixtures::pattern(0, 1440, &[]),
            Expected::Rule("FREQ=DAILY;UNTIL=20240731"),
        ),
        "ExDates" => (
            fixtures::weekly(),
            Expected::Dates(vec![
                Date::new(2024, 7, 8, 10, 0, 0, Kind::Unspecified).unwrap(),
            ]),
        ),
        "ManyExDates" => {
            let mut bytes = fixtures::many(128);
            // Move the original days of the first 64 exceptions outside the
            // deleted-date range, leaving exactly those 64 days as EXDATEs.
            for index in 0..64usize {
                let offset = 70 + 128 * 8 + 2 + index * 14 + 8;
                let original = 222737760 + (128 + index as u32) * 1440 + 600;
                bytes[offset..offset + 4].copy_from_slice(&original.to_le_bytes());
            }
            let dates = (0..64u64)
                .map(|day| {
                    // Independent date coordinates, not output from the generator.
                    Date::from_ticks(638554248000000000 + day * 864000000000, Kind::Unspecified)
                        .unwrap()
                })
                .collect();
            (bytes, Expected::Dates(dates))
        }
        "UnsupportedCalendar" => (
            fixtures::pattern(10, 1, &[10]),
            Expected::Error(RecurrenceRuleError::UnsupportedCalendar {
                pattern_type: 10,
                calendar_type: 0,
            }),
        ),
        _ => unreachable!(),
    };
    if matches!(name, "UntilUtc" | "UntilDate") {
        bytes[22..26].copy_from_slice(&0x2021u32.to_le_bytes());
    }
    let recurrence = TnefRecurrence::parse(&bytes, Charset::ASCII).unwrap();
    assert!(recurrence.exception_error().is_none());
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
    let zone = Zone::new("Eastern", 300, 0, -60, standard, daylight).unwrap();
    let operations = 64;
    let mut expected_summary = Summary::default();
    for _ in 0..operations {
        match &expected {
            Expected::Rule(rule) => expected_summary.rule(rule),
            Expected::Dates(dates) => expected_summary.dates(dates),
            Expected::Error(error) => expected_summary.error(*error),
        }
    }
    Workload {
        name,
        operations,
        recurrence,
        zone,
        expected,
        expected_summary,
    }
}

pub fn run(case: &Workload) -> Summary {
    let mut summary = Summary::default();
    for _ in 0..case.operations {
        let recurrence = black_box(&case.recurrence);
        match &case.expected {
            Expected::Rule(_) => {
                let rule =
                    recurrence_rule(recurrence, &case.zone, case.name == "UntilDate").unwrap();
                summary.rule(&rule);
            }
            Expected::Dates(_) => summary.dates(&recurrence_exdates(recurrence).unwrap()),
            Expected::Error(_) => {
                summary.error(recurrence_rule(recurrence, &case.zone, false).unwrap_err())
            }
        }
    }
    black_box(summary)
}

impl Workload {
    pub fn check(&self) {
        match &self.expected {
            Expected::Rule(rule) => assert_eq!(
                recurrence_rule(&self.recurrence, &self.zone, self.name == "UntilDate").unwrap(),
                *rule
            ),
            Expected::Dates(dates) => {
                assert_eq!(recurrence_exdates(&self.recurrence).unwrap(), *dates)
            }
            Expected::Error(error) => assert_eq!(
                recurrence_rule(&self.recurrence, &self.zone, false),
                Err(*error)
            ),
        }
        assert_eq!(run(self), self.expected_summary, "{}", self.name);
    }
}

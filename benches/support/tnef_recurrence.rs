// Original workloads based on MimeKit's recurrence parser and calendar tests.
// MimeKit has no corresponding recurrence benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::{
    charsets::Charset,
    tnef::{TnefRecurrence, TnefRecurrenceError},
};
use std::hint::black_box;

#[path = "../../tests/support/tnef_recurrence.rs"]
mod fixtures;

pub const CASES: [&str; 7] = [
    "Daily",
    "Weekly",
    "MonthNth",
    "Exceptions",
    "ManyExceptions",
    "LegacyCharset",
    "TruncatedExceptions",
];
const OPERATIONS: usize = 64;
const MANY: u16 = 128;

pub struct Workload {
    pub name: &'static str,
    /// Bytes supplied across all 64 parses in one measured batch.
    pub bytes: usize,
    input: Vec<u8>,
    charset: Charset,
    expected: Summary,
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
    fn array(&mut self, values: &[u32]) {
        self.add(values.len() as u64);
        for &value in values {
            self.add(u64::from(value));
        }
    }
    fn text(&mut self, text: Option<&str>) {
        self.add(u64::from(text.is_some()));
        if let Some(text) = text {
            self.add(text.len() as u64);
            for byte in text.bytes() {
                self.add(u64::from(byte));
            }
        }
    }
    fn exception(&mut self, fields: [u32; 8], subject: Option<&str>, location: Option<&str>) {
        self.array(&fields);
        self.text(subject);
        self.text(location);
    }
    fn recurrence(&mut self, value: &TnefRecurrence) {
        self.array(&[
            u32::from(value.pattern_type()),
            u32::from(value.calendar_type()),
            value.first_date_time(),
            value.period(),
            value.day_mask(),
            value.day_of_month(),
            value.week_of_month(),
            value.end_type(),
            value.occurrence_count(),
            value.first_day_of_week(),
            value.start_date(),
            value.end_date(),
            value.start_time_offset(),
            value.end_time_offset(),
        ]);
        self.array(value.deleted_instance_dates());
        self.array(value.modified_instance_dates());
        self.add(value.exceptions().len() as u64);
        for ex in value.exceptions() {
            self.exception(
                [
                    ex.start_date_time(),
                    ex.end_date_time(),
                    ex.original_start_date(),
                    u32::from(ex.override_flags()),
                    ex.reminder_delta() as u32,
                    u32::from(ex.reminder_set()),
                    ex.busy_status() as u32,
                    u32::from(ex.is_all_day()),
                ],
                ex.subject(),
                ex.location(),
            );
        }
        match value.exception_error() {
            None => self.add(0),
            Some(TnefRecurrenceError::UnexpectedEnd { offset }) => {
                self.add(1);
                self.add(*offset as u64);
            }
            Some(TnefRecurrenceError::UnsupportedVersion(version)) => {
                self.add(2);
                self.add(u64::from(*version));
            }
            Some(TnefRecurrenceError::UnknownPattern(pattern)) => {
                self.add(3);
                self.add(u64::from(*pattern));
            }
        }
        self.add(u64::from(value.had_decode_errors()));
    }
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let mut charset = Charset::ASCII;
    let input = match name {
        "Daily" => fixtures::pattern(0, 2880, &[]),
        "Weekly" => fixtures::pattern(1, 2, &[0x0a]),
        "MonthNth" => fixtures::pattern(3, 2, &[0x14, 2]),
        "Exceptions" => fixtures::weekly(),
        "ManyExceptions" => fixtures::many(MANY),
        "LegacyCharset" => {
            charset = Charset::from_label("windows-1251").unwrap();
            let mut bytes = fixtures::weekly();
            bytes[102..112].fill(0xc0); // Valid 8-bit Cyrillic, overwritten by Unicode.
            bytes[70..74].copy_from_slice(&0x3008u32.to_le_bytes());
            bytes.drain(126..134); // Older writers omit ChangeHighlight.
            bytes
        }
        "TruncatedExceptions" => fixtures::weekly()[..112].to_vec(),
        _ => unreachable!(),
    };
    // Independent model expectations, never obtained by parsing the fixture.
    let (pattern, period, mask, nth) = match name {
        "Daily" => (0, 2880, 0, 0),
        "Weekly" => (1, 2, 0x0a, 0),
        "MonthNth" => (3, 2, 0x14, 2),
        "ManyExceptions" => (1, 1, 0x7f, 0),
        _ => (1, 1, 0x0a, 0),
    };
    let many = name == "ManyExceptions";
    let moved = matches!(name, "Exceptions" | "LegacyCharset" | "TruncatedExceptions");
    let truncated = name == "TruncatedExceptions";
    let dates: Vec<u32> = (0..u32::from(MANY)).map(|i| 222737760 + i * 1440).collect();
    let mut expected = Summary::default();
    for _ in 0..OPERATIONS {
        expected.array(&[
            pattern,
            0,
            0,
            period,
            mask,
            0,
            nth,
            0x2022,
            if many { u32::from(MANY) + 10 } else { 10 },
            0,
            222737760,
            222780960,
            600,
            660,
        ]);
        expected.array(if many {
            &dates
        } else if moved {
            &[222747840, 222750720]
        } else {
            &[]
        });
        expected.array(if many {
            &dates
        } else if moved {
            &[222750720]
        } else {
            &[]
        });
        expected.add(if many {
            u64::from(MANY)
        } else {
            u64::from(moved && !truncated)
        });
        if many {
            for &day in &dates {
                expected.exception([day + 780, day + 840, day + 600, 0, 0, 0, 0, 0], None, None);
            }
        } else if moved && !truncated {
            expected.exception(
                [222751500, 222751560, 222751320, 0x11, 0, 0, 0, 0],
                Some("Moved s\u{fd}nc"),
                Some("Room 2"),
            );
        }
        expected.add(u64::from(truncated));
        if truncated {
            expected.add(112);
        }
        expected.add(0); // No decoding replacements in these fixtures.
    }
    Workload {
        name,
        bytes: input.len() * OPERATIONS,
        input,
        charset,
        expected,
    }
}

pub fn run(case: &Workload) -> Summary {
    let mut summary = Summary::default();
    for _ in 0..OPERATIONS {
        let value = TnefRecurrence::parse(black_box(&case.input), case.charset).unwrap();
        summary.recurrence(&value);
    }
    black_box(summary)
}

impl Workload {
    pub fn check(&self) {
        assert_eq!(self.bytes, self.input.len() * OPERATIONS);
        assert_eq!(run(self), self.expected, "{}", self.name);
    }
}

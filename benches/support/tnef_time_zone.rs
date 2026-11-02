// Original workloads based on MimeKit's TnefTimeZone and calendar tests.
// MimeKit has no corresponding time-zone benchmark class.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use mimecraft::tnef::{
    TnefDateTime as Date, TnefDateTimeKind as Kind, TnefSystemTime as SystemTime,
    TnefTimeZone as Zone, calendar::CalendarWriter,
};
use std::{
    hint::black_box,
    io::{self, Write},
};

pub const CASES: [&str; 7] = [
    "ParseStruct",
    "ParseDefinition",
    "UtcToLocal",
    "LocalToUtc",
    "WriteFixed",
    "WriteDaylight",
    "WriteShort",
];

const STANDARD: SystemTime = SystemTime {
    year: 0,
    month: 11,
    day_of_week: 0,
    day: 1,
    hour: 2,
    minute: 0,
    second: 0,
    milliseconds: 0,
};
const DAYLIGHT: SystemTime = SystemTime {
    month: 3,
    day: 2,
    ..STANDARD
};
const EASTERN: &str = concat!(
    "BEGIN:VTIMEZONE\r\nTZID:Eastern\r\nBEGIN:STANDARD\r\n",
    "DTSTART:16011104T020000\r\nRRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11\r\n",
    "TZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nEND:STANDARD\r\n",
    "BEGIN:DAYLIGHT\r\nDTSTART:16010311T020000\r\n",
    "RRULE:FREQ=YEARLY;BYDAY=2SU;BYMONTH=3\r\n",
    "TZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n",
);
const FIXED: &str = concat!(
    "BEGIN:VTIMEZONE\r\nTZID:India\r\nBEGIN:STANDARD\r\n",
    "DTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\n",
    "TZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
);

pub struct Workload {
    pub name: &'static str,
    /// Total parsed or written bytes; zero for date-conversion workloads.
    pub bytes: usize,
    /// Number of dates, structures, or components per measured operation.
    pub operations: usize,
    input: Vec<u8>,
    zone: Zone,
    dates: Vec<Date>,
    expected_dates: Vec<Date>,
    expected_output: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub values: usize,
    pub checksum: u64,
}

impl Summary {
    fn add(&mut self, value: u64) {
        self.checksum = self.checksum.wrapping_mul(31).wrapping_add(value);
    }
    fn date(&mut self, date: Date) {
        self.values += 1;
        self.add(date.ticks());
        self.add(u64::from(date.kind() == Kind::Utc));
    }
    fn zone(&mut self, zone: &Zone) {
        self.values += 1;
        for byte in zone.id().bytes() {
            self.add(u64::from(byte));
        }
        for bias in [zone.bias(), zone.standard_bias(), zone.daylight_bias()] {
            self.add(bias as u64);
        }
        for transition in [zone.standard_date(), zone.daylight_date()] {
            for field in fields(transition) {
                self.add(u64::from(field));
            }
        }
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.values += bytes.len();
        for &byte in bytes {
            self.add(u64::from(byte));
        }
    }
}

fn fields(time: SystemTime) -> [u16; 8] {
    [
        time.year,
        time.month,
        time.day_of_week,
        time.day,
        time.hour,
        time.minute,
        time.second,
        time.milliseconds,
    ]
}

fn date(parts: (u16, u8, u8, u8, u8), kind: Kind) -> Date {
    Date::new(parts.0, parts.1, parts.2, parts.3, parts.4, 0, kind).unwrap()
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let fixed = name == "WriteFixed";
    let zone = if fixed {
        Zone::new(
            "India",
            -330,
            0,
            0,
            SystemTime::default(),
            SystemTime::default(),
        )
    } else {
        Zone::new("Eastern", 300, 0, -60, STANDARD, DAYLIGHT)
    }
    .unwrap();
    let mut input = Vec::new();
    if name == "ParseStruct" {
        input.extend([300i32, 0, -60].into_iter().flat_map(i32::to_le_bytes));
        input.extend_from_slice(&[0; 2]);
        input.extend(fields(STANDARD).into_iter().flat_map(u16::to_le_bytes));
        input.extend_from_slice(&[0; 2]);
        input.extend(fields(DAYLIGHT).into_iter().flat_map(u16::to_le_bytes));
    } else if name == "ParseDefinition" {
        // Independent MS-OXOCAL fixture: one effective rule and a UTF-16 key.
        input.extend_from_slice(&[2, 1, 20, 0, 2, 0, 7, 0]);
        input.extend("Eastern".encode_utf16().flat_map(u16::to_le_bytes));
        input.extend_from_slice(&1u16.to_le_bytes());
        input.extend_from_slice(&[2, 1, 0x3e, 0, 2, 0, 0xd7, 7]);
        input.extend_from_slice(&[0; 14]);
        input.extend([300i32, 0, -60].into_iter().flat_map(i32::to_le_bytes));
        input.extend(fields(STANDARD).into_iter().flat_map(u16::to_le_bytes));
        input.extend(fields(DAYLIGHT).into_iter().flat_map(u16::to_le_bytes));
    }
    let mut dates = Vec::new();
    let mut expected_dates = Vec::new();
    if matches!(name, "UtcToLocal" | "LocalToUtc") {
        let pairs = if name == "UtcToLocal" {
            [
                ((2024, 3, 10, 6, 59), (2024, 3, 10, 1, 59)),
                ((2024, 3, 10, 7, 0), (2024, 3, 10, 3, 0)),
                ((2024, 11, 3, 5, 30), (2024, 11, 3, 1, 30)),
                ((2024, 11, 3, 6, 30), (2024, 11, 3, 1, 30)),
                ((2024, 1, 1, 1, 0), (2023, 12, 31, 20, 0)),
                ((2024, 7, 1, 0, 0), (2024, 6, 30, 20, 0)),
                ((2024, 7, 1, 14, 0), (2024, 7, 1, 10, 0)),
                ((2024, 12, 2, 15, 0), (2024, 12, 2, 10, 0)),
            ]
        } else {
            [
                ((2024, 3, 10, 1, 59), (2024, 3, 10, 6, 59)),
                ((2024, 3, 10, 2, 30), (2024, 3, 10, 7, 30)),
                ((2024, 3, 10, 3, 0), (2024, 3, 10, 7, 0)),
                ((2024, 11, 3, 1, 30), (2024, 11, 3, 5, 30)),
                ((2024, 11, 3, 2, 0), (2024, 11, 3, 7, 0)),
                ((2023, 12, 31, 20, 0), (2024, 1, 1, 1, 0)),
                ((2024, 7, 1, 10, 0), (2024, 7, 1, 14, 0)),
                ((2024, 12, 2, 10, 0), (2024, 12, 2, 15, 0)),
            ]
        };
        let (input_kind, output_kind) = if name == "UtcToLocal" {
            (Kind::Utc, Kind::Unspecified)
        } else {
            (Kind::Unspecified, Kind::Utc)
        };
        for fraction in 0..64 {
            for (input, output) in pairs {
                dates.push(
                    Date::from_ticks(date(input, input_kind).ticks() + fraction, input_kind)
                        .unwrap(),
                );
                expected_dates.push(
                    Date::from_ticks(date(output, output_kind).ticks() + fraction, output_kind)
                        .unwrap(),
                );
            }
        }
    }
    let writes = name.starts_with("Write");
    let expected_output = if writes {
        if fixed { FIXED } else { EASTERN }.repeat(128).into_bytes()
    } else {
        Vec::new()
    };
    let bytes = if writes {
        expected_output.len()
    } else {
        input.len() * 256
    };
    let operations = if writes {
        128
    } else if input.is_empty() {
        dates.len()
    } else {
        256
    };
    Workload {
        name,
        bytes,
        operations,
        input,
        zone,
        dates,
        expected_dates,
        expected_output,
    }
}

struct Sink {
    bytes: Vec<u8>,
    chunk: usize,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(self.chunk);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(case: &Workload) -> Vec<u8> {
    let mut writer = CalendarWriter::new(Sink {
        bytes: Vec::new(),
        chunk: if case.name == "WriteShort" {
            7
        } else {
            usize::MAX
        },
    });
    for _ in 0..case.operations {
        black_box(&case.zone).write_to(&mut writer).unwrap();
    }
    writer.finish().unwrap();
    writer.into_inner().bytes
}

fn parse(case: &Workload) -> Zone {
    if case.name == "ParseStruct" {
        Zone::from_time_zone_struct(black_box(&case.input), "Eastern").unwrap()
    } else {
        Zone::from_time_zone_definition(black_box(&case.input), None).unwrap()
    }
}

fn convert(case: &Workload, date: Date) -> Date {
    if case.name == "UtcToLocal" {
        black_box(&case.zone)
            .to_local_time(black_box(date))
            .unwrap()
    } else {
        black_box(&case.zone)
            .to_universal_time(black_box(date))
            .unwrap()
    }
}

pub fn run(case: &Workload) -> Summary {
    let case = black_box(case);
    let mut summary = Summary::default();
    match case.name {
        "ParseStruct" | "ParseDefinition" => {
            for _ in 0..case.operations {
                summary.zone(black_box(&parse(case)));
            }
        }
        "UtcToLocal" | "LocalToUtc" => {
            for &date in &case.dates {
                summary.date(black_box(convert(case, date)));
            }
        }
        "WriteFixed" | "WriteDaylight" | "WriteShort" => summary.bytes(black_box(&encode(case))),
        _ => unreachable!(),
    }
    summary
}

impl Workload {
    pub fn check(&self) {
        let mut expected = Summary::default();
        match self.name {
            "ParseStruct" | "ParseDefinition" => {
                assert_eq!(parse(self), self.zone);
                for _ in 0..self.operations {
                    expected.zone(&self.zone);
                }
                assert_eq!(self.bytes, self.input.len() * self.operations);
            }
            "UtcToLocal" | "LocalToUtc" => {
                assert_eq!(self.dates.len(), self.expected_dates.len());
                for (&input, &output) in self.dates.iter().zip(&self.expected_dates) {
                    assert_eq!(convert(self, input), output);
                    expected.date(output);
                }
                assert_eq!(self.operations, self.dates.len());
                assert_eq!(self.bytes, 0);
            }
            "WriteFixed" | "WriteDaylight" | "WriteShort" => {
                assert_eq!(encode(self), self.expected_output);
                expected.bytes(&self.expected_output);
                assert_eq!(self.bytes, self.expected_output.len());
            }
            _ => unreachable!(),
        }
        assert_eq!(run(self), expected);
    }
}

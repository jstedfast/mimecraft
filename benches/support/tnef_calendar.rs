// Original workloads based on MimeKit's CalendarWriterTests.
// There is no upstream CalendarWriter benchmark class to port.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    hint::black_box,
    io::{self, Write},
};

use mimecraft::tnef::{
    TnefDateTime, TnefDateTimeKind,
    calendar::{CalendarWriter, format_date, format_date_time, format_utc_offset},
};

pub const CASES: [&str; 7] = [
    "Components",
    "EscapedText",
    "Parameters",
    "FoldAscii",
    "FoldUtf8",
    "ShortWrites",
    "Formatting",
];

pub struct Workload {
    pub name: &'static str,
    pub output_length: usize,
    input: String,
    date: TnefDateTime,
    expected: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Summary {
    pub bytes: usize,
    pub checksum: u64,
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

fn expected_line(output: &mut Vec<u8>, line: &str) {
    // Independent forward character packing, not the writer's backward
    // byte-boundary search. Only fixture preparation uses this implementation.
    let mut column = 0;
    for c in line.chars() {
        if column + c.len_utf8() > 75 {
            output.extend_from_slice(b"\r\n ");
            column = 1;
        }
        let mut bytes = [0; 4];
        output.extend_from_slice(c.encode_utf8(&mut bytes).as_bytes());
        column += c.len_utf8();
    }
    output.extend_from_slice(b"\r\n");
}

pub fn workload(name: &'static str) -> Workload {
    assert!(CASES.contains(&name));
    let input = match name {
        "EscapedText" => "a\\b;c,d\r\ne\nf\rg\u{1}h".into(),
        "FoldAscii" => "abcdef".repeat(16384),
        "FoldUtf8" | "ShortWrites" => "\u{20ac}\u{1f600}\u{e9}".repeat(8192),
        _ => String::new(),
    };
    let mut expected = Vec::new();
    if matches!(name, "FoldAscii" | "FoldUtf8" | "ShortWrites") {
        expected_line(&mut expected, &format!("DESCRIPTION:{input}"));
    } else {
        for _ in 0..256 {
            match name {
                "Components" => {
                    for line in ["BEGIN:VEVENT", "DTSTART:20240101T000000Z", "END:VEVENT"] {
                        expected_line(&mut expected, line);
                    }
                }
                "EscapedText" => {
                    expected_line(&mut expected, "SUMMARY:a\\\\b\\;c\\,d\\ne\\nf\\ngh")
                }
                "Parameters" => expected_line(
                    &mut expected,
                    "ATTENDEE;CN=\"Doe, John\";ROLE=REQ-PARTICIPANT;X-NAME=say 'hi':mailto:j@x.org",
                ),
                "Formatting" => {
                    for line in [
                        "DTSTART:20240704T130509Z",
                        "DTEND:20240704T130509",
                        "X-DATE:20240704",
                        "TZOFFSETTO:+0530",
                    ] {
                        expected_line(&mut expected, line);
                    }
                }
                _ => unreachable!(),
            }
        }
    }
    Workload {
        name,
        output_length: expected.len(),
        input,
        date: TnefDateTime::new(2024, 7, 4, 13, 5, 9, TnefDateTimeKind::Unspecified).unwrap(),
        expected,
    }
}

fn encode(case: &Workload) -> Vec<u8> {
    let mut writer = CalendarWriter::new(Sink {
        bytes: Vec::new(),
        chunk: if case.name == "ShortWrites" {
            7
        } else {
            usize::MAX
        },
    });
    if matches!(case.name, "FoldAscii" | "FoldUtf8" | "ShortWrites") {
        writer
            .write_text_property("DESCRIPTION", black_box(&case.input))
            .unwrap();
    } else {
        for _ in 0..256 {
            match case.name {
                "Components" => {
                    writer.begin_component("VEVENT").unwrap();
                    writer
                        .write_property("DTSTART", black_box("20240101T000000Z"))
                        .unwrap();
                    writer.end_component("VEVENT").unwrap();
                }
                "EscapedText" => writer
                    .write_text_property("SUMMARY", black_box(&case.input))
                    .unwrap(),
                "Parameters" => {
                    writer.begin_property("ATTENDEE").unwrap();
                    writer
                        .write_parameter("CN", black_box("Doe, John"))
                        .unwrap();
                    writer
                        .write_parameter("ROLE", black_box("REQ-PARTICIPANT"))
                        .unwrap();
                    writer
                        .write_parameter("X-NAME", black_box("say \"hi\""))
                        .unwrap();
                    writer.write_value(black_box("mailto:j@x.org")).unwrap();
                }
                "Formatting" => {
                    writer
                        .write_property("DTSTART", &format_date_time(black_box(case.date), true))
                        .unwrap();
                    writer
                        .write_property("DTEND", &format_date_time(black_box(case.date), false))
                        .unwrap();
                    writer
                        .write_property("X-DATE", &format_date(black_box(case.date)))
                        .unwrap();
                    writer
                        .write_property("TZOFFSETTO", &format_utc_offset(black_box(330)).unwrap())
                        .unwrap();
                }
                _ => unreachable!(),
            }
        }
    }
    writer.finish().unwrap();
    writer.into_inner().bytes
}

fn summarize(bytes: &[u8]) -> Summary {
    Summary {
        bytes: bytes.len(),
        checksum: bytes.iter().fold(0u64, |sum, &byte| {
            sum.wrapping_mul(31).wrapping_add(u64::from(byte))
        }),
    }
}

pub fn write(case: &Workload) -> Summary {
    summarize(black_box(&encode(black_box(case))))
}

impl Workload {
    pub fn check(&self) {
        let actual = encode(self);
        assert_eq!(actual, self.expected);
        assert_eq!(actual.len(), self.output_length);
        let text = std::str::from_utf8(&actual).unwrap();
        assert!(text.ends_with("\r\n"));
        for line in text.split_terminator("\r\n") {
            assert!(line.len() <= 75);
            assert!(!line.contains(['\r', '\n']));
        }
        assert_eq!(write(self), summarize(&self.expected));
    }
}

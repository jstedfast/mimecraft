// Adapted from MimeKit's UnitTests/Tnef/CalendarWriterTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, Write};

use mimecraft::tnef::{
    TnefDateTime as Date, TnefDateTimeKind as Kind,
    calendar::{
        CalendarWriter, append_text, escape_text, format_date, format_date_time, format_utc_offset,
    },
};

fn write(action: impl FnOnce(&mut CalendarWriter<Vec<u8>>)) -> String {
    let mut writer = CalendarWriter::new(Vec::new());
    action(&mut writer);
    writer.finish().unwrap();
    String::from_utf8(writer.into_inner()).unwrap()
}

fn check_lines(text: &str, logical: &str) {
    assert!(text.ends_with("\r\n"));
    for (i, line) in text.split_terminator("\r\n").enumerate() {
        assert!(line.len() <= 75, "line {i}: {} octets", line.len());
        if i > 0 {
            assert!(line.starts_with(' '), "line {i}");
        }
        // The first octet after a fold must not be a UTF-8 continuation octet.
        assert_ne!(line.as_bytes()[usize::from(i > 0)] & 0xc0, 0x80);
    }
    assert_eq!(text.replace("\r\n ", ""), logical);
}

#[test]
fn write_property() {
    let text = write(|writer| {
        writer.begin_component("VEVENT").unwrap();
        writer
            .write_property("DTSTART", "20240101T000000Z")
            .unwrap();
        writer.end_component("VEVENT").unwrap();
    });
    assert_eq!(
        text,
        "BEGIN:VEVENT\r\nDTSTART:20240101T000000Z\r\nEND:VEVENT\r\n"
    );
}

#[test]
fn text_escaping() {
    let input = "a\\b;c,d\r\ne\nf\rg\u{0001}h";
    let expected = "a\\\\b\\;c\\,d\\ne\\nf\\ngh";
    let text = write(|writer| writer.write_text_property("SUMMARY", input).unwrap());
    assert_eq!(text, format!("SUMMARY:{expected}\r\n"));
    assert_eq!(escape_text(input), expected);
    let mut builder = "prefix,".to_owned();
    append_text(&mut builder, input);
    assert_eq!(builder, format!("prefix,{expected}"));
    assert_eq!(escape_text("\r\r\n\n"), "\\n\\n\\n");
    assert_eq!(escape_text(""), "");
}

#[test]
fn parameter_quoting() {
    let text = write(|writer| {
        writer.begin_property("ATTENDEE").unwrap();
        writer.write_parameter("CN", "Doe, John").unwrap();
        writer.write_parameter("ROLE", "REQ-PARTICIPANT").unwrap();
        writer.write_parameter("X-NAME", "say \"hi\"").unwrap();
        writer.write_value("mailto:j@x.org").unwrap();
    });
    assert_eq!(
        text,
        "ATTENDEE;CN=\"Doe, John\";ROLE=REQ-PARTICIPANT;X-NAME=say 'hi':mailto:j@x.org\r\n"
    );
    for (value, expected) in [
        ("", ""),
        ("x;y", "\"x;y\""),
        ("x:y", "\"x:y\""),
        ("x,y", "\"x,y\""),
        ("\"x\"", "'x'"),
        ("\"x,y\"", "\"'x,y'\""),
        ("a\r\n\u{1}b\tc\u{7f}", "ab\tc"),
        ("^n^'^^\\", "^n^'^^\\"),
        ("\u{e9}\u{85}", "\u{e9}\u{85}"),
    ] {
        assert_eq!(
            write(|writer| {
                writer.begin_property("X").unwrap();
                writer.write_parameter("P", value).unwrap();
                writer.write_value("").unwrap();
            }),
            format!("X;P={expected}:\r\n")
        );
    }
}

#[test]
fn folding() {
    let value = "x".repeat(200);
    let text = write(|writer| writer.write_text_property("SUMMARY", &value).unwrap());
    check_lines(&text, &format!("SUMMARY:{value}\r\n"));
    assert_eq!(
        text,
        format!(
            "SUMMARY:{}\r\n {}\r\n {}\r\n",
            "x".repeat(67),
            "x".repeat(74),
            "x".repeat(59)
        )
    );
}

#[test]
fn folding_does_not_split_multibyte_sequences() {
    // Each character is 3 octets in UTF-8, so folds do not happen to line
    // up with character boundaries. Also cover two- and four-octet sequences.
    for unit in [
        "x",
        "\u{e9}",
        "\u{20ac}",
        "\u{1f600}",
        "\u{e9}\u{20ac}\u{1f600}",
    ] {
        for prefix in 0..76 {
            for repeats in [0, 1, 2, 50, 100] {
                let value = "x".repeat(prefix) + &unit.repeat(repeats);
                let text = write(|writer| writer.write_text_property("SUMMARY", &value).unwrap());
                check_lines(&text, &format!("SUMMARY:{value}\r\n"));
            }
        }
    }
}

#[test]
fn formatting() {
    for kind in [Kind::Unspecified, Kind::Utc] {
        let date = Date::new(2024, 7, 4, 13, 5, 9, kind).unwrap();
        assert_eq!(format_date_time(date, true), "20240704T130509Z");
        assert_eq!(format_date_time(date, false), "20240704T130509");
        assert_eq!(format_date(date), "20240704");
        let min = Date::from_ticks(0, kind).unwrap();
        assert_eq!(format_date(min), "00010101");
        assert_eq!(format_date_time(min, true), "00010101T000000Z");
        let max = Date::from_ticks(3155378975999999999, kind).unwrap();
        assert_eq!(format_date(max), "99991231");
        assert_eq!(format_date_time(max, false), "99991231T235959");
    }
    for (minutes, expected) in [
        (-300, "-0500"),
        (330, "+0530"),
        (0, "+0000"),
        (1, "+0001"),
        (-1, "-0001"),
        (59, "+0059"),
        (60, "+0100"),
        (-1439, "-2359"),
        (1440, "+2400"),
        (i32::MAX, "+3579139407"),
        (-i32::MAX, "-3579139407"),
    ] {
        assert_eq!(format_utc_offset(minutes).unwrap(), expected);
    }
    assert_eq!(
        format_utc_offset(i32::MIN).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn property_state_errors() {
    let mut writer = CalendarWriter::new(Vec::new());
    for result in [
        writer.write_value("x"),
        writer.write_text_value("x"),
        writer.write_parameter("A", "B"),
    ] {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    writer.begin_property("X").unwrap();
    for result in [
        writer.begin_property("Y"),
        writer.begin_component("VEVENT"),
        writer.end_component("VEVENT"),
        writer.write_property("Y", "lost"),
        writer.write_text_property("Y", "lost"),
        writer.finish(),
    ] {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    writer.flush().unwrap();
    assert!(writer.get_ref().is_empty());
    assert!(!writer.is_faulted());
    writer.write_parameter("A", "B").unwrap();
    writer.write_value("x").unwrap();
    writer.finish().unwrap();
    assert!(writer.is_finished());
    let before = writer.get_ref().clone();
    writer.finish().unwrap();
    writer.flush().unwrap();
    assert_eq!(
        writer.begin_property("Y").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(writer.get_ref(), &before);
    assert_eq!(writer.into_inner(), b"X;A=B:x\r\n");
}

#[test]
fn controls_are_filtered_without_dropping_tab_or_unicode_controls() {
    for byte in 0..=127u8 {
        let value = format!("a{}b", char::from(byte));
        let kept = byte == b'\t' || (32..127).contains(&byte);
        let expected = if kept { value.clone() } else { "ab".into() };
        assert_eq!(
            write(|w| w.write_property("X", &value).unwrap()),
            format!("X:{expected}\r\n")
        );
    }
    let value = "\t\u{80}\u{85}\u{9f}\u{2028}\u{2029}\u{1f600}";
    assert_eq!(
        write(|w| w.write_property("X", value).unwrap()),
        format!("X:{value}\r\n")
    );
    assert_eq!(escape_text(value), value);
    assert_eq!(
        write(|w| w.write_property("X", "a\\b;c,d").unwrap()),
        "X:a\\b;c,d\r\n"
    );
}

#[test]
fn exact_octet_boundaries_and_long_names_fold_without_empty_continuations() {
    for name_length in [1, 8, 74, 75, 76, 200] {
        for value_length in 0..225 {
            let name = "X".repeat(name_length);
            let value = "y".repeat(value_length);
            let text = write(|w| w.write_property(&name, &value).unwrap());
            let logical = format!("{name}:{value}\r\n");
            check_lines(&text, &logical);
            let mut remaining = logical.len() - 2;
            for (i, line) in text.split_terminator("\r\n").enumerate() {
                let prefix = usize::from(i > 0);
                let content = remaining.min(75 - prefix);
                assert_eq!(line.len(), content + prefix);
                remaining -= content;
            }
            assert_eq!(remaining, 0);
        }
    }
}

#[test]
fn invalid_names_are_rejected_before_mutating_the_pending_line() {
    let names = [
        "", "A:B", "x;y", "x,y", "x=y", "A B", "x\r\nY", "a\0", "\u{e9}", "_", "A\tB",
    ];
    let mut writer = CalendarWriter::new(Vec::new());
    for name in names {
        for result in [
            writer.begin_property(name),
            writer.begin_component(name),
            writer.end_component(name),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }
    writer.begin_property("x-CUSTOM9").unwrap();
    for name in names {
        assert_eq!(
            writer.write_parameter(name, "bad").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    writer.write_parameter("x-Name", "good").unwrap();
    writer.write_value("").unwrap();
    writer.finish().unwrap();
    assert_eq!(writer.into_inner(), b"x-CUSTOM9;x-Name=good:\r\n");
}

struct Sink {
    bytes: Vec<u8>,
    chunk: usize,
    limit: usize,
    zero: bool,
    writes: usize,
    flushes: usize,
    fail_flush: bool,
}

impl Sink {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            chunk: usize::MAX,
            limit: usize::MAX,
            zero: false,
            writes: 0,
            flushes: 0,
            fail_flush: false,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes % 3 == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let count = bytes
            .len()
            .min(self.chunk)
            .min(self.limit - self.bytes.len());
        if count == 0 && !self.zero {
            return Err(io::Error::other("injected write failure"));
        }
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.flushes == 1 {
            return Err(io::ErrorKind::Interrupted.into());
        }
        if self.fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}

fn assert_faulted(writer: &mut CalendarWriter<Sink>) {
    assert!(writer.is_faulted());
    assert!(!writer.is_finished());
    let counts = (writer.get_ref().writes, writer.get_ref().flushes);
    for result in [
        writer.begin_property("X"),
        writer.write_parameter("A", "B"),
        writer.write_value("x"),
        writer.write_text_value("x"),
        writer.write_property("X", "x"),
        writer.write_text_property("X", "x"),
        writer.begin_component("VEVENT"),
        writer.end_component("VEVENT"),
        writer.finish(),
        writer.flush(),
    ] {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
    assert_eq!((writer.get_ref().writes, writer.get_ref().flushes), counts);
}

#[test]
fn short_and_interrupted_writes_and_flushes_preserve_utf8_output() {
    let value = "\u{e9}\u{20ac}\u{1f600}".repeat(100);
    for chunk in [1, 2, 7, usize::MAX] {
        let mut sink = Sink::new();
        sink.chunk = chunk;
        let mut writer = CalendarWriter::new(sink);
        writer.write_text_property("SUMMARY", &value).unwrap();
        writer.finish().unwrap();
        assert_eq!(writer.get_ref().flushes, 2);
        writer.finish().unwrap();
        assert_eq!(writer.get_ref().flushes, 2);
        let text = String::from_utf8(writer.into_inner().bytes).unwrap();
        check_lines(&text, &format!("SUMMARY:{value}\r\n"));
    }
}

#[test]
fn every_partial_write_failure_is_terminal_and_retains_the_exact_prefix() {
    let expected = format!(
        "DESCRIPTION:{}\r\n {}\r\n {}\r\n",
        "x".repeat(63),
        "x".repeat(74),
        "x".repeat(63)
    );
    for limit in 0..expected.len() {
        for zero in [false, true] {
            let mut sink = Sink::new();
            sink.limit = limit;
            sink.chunk = 7;
            sink.zero = zero;
            let mut writer = CalendarWriter::new(sink);
            let error = writer
                .write_text_property("DESCRIPTION", &"x".repeat(200))
                .unwrap_err();
            assert_eq!(
                error.kind(),
                if zero {
                    io::ErrorKind::WriteZero
                } else {
                    io::ErrorKind::Other
                }
            );
            assert_eq!(writer.get_ref().bytes, expected.as_bytes()[..limit]);
            assert_faulted(&mut writer);
            assert_eq!(writer.into_inner().bytes, expected.as_bytes()[..limit]);
        }
    }
}

#[test]
fn flush_failures_are_terminal_even_with_a_pending_property() {
    for pending in [false, true] {
        let mut sink = Sink::new();
        sink.fail_flush = true;
        let mut writer = CalendarWriter::new(sink);
        writer.write_property("X", "complete").unwrap();
        if pending {
            writer.begin_property("Y").unwrap();
        }
        let error = if pending {
            writer.flush()
        } else {
            writer.finish()
        }
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(writer.get_ref().bytes, b"X:complete\r\n");
        assert_faulted(&mut writer);
    }
}

#[test]
fn drop_and_storage_recovery_do_not_write_or_flush() {
    let mut sink = Sink::new();
    {
        let mut writer = CalendarWriter::new(&mut sink);
        writer.begin_property("X").unwrap();
        writer.write_parameter("A", "B").unwrap();
    }
    assert_eq!((sink.writes, sink.flushes), (0, 0));
    let mut writer = CalendarWriter::new(sink);
    writer.begin_property("X").unwrap();
    let sink = writer.into_inner();
    assert_eq!((sink.writes, sink.flushes), (0, 0));
}

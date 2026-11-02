// Ported from MimeKit's Tnef/CalendarWriter.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

//! Low-level iCalendar content-line writing for TNEF calendar conversion.
//!
//! This is not a calendar model or a TNEF-to-MIME converter. Component nesting,
//! property types, time-zone selection, and already formatted values remain the
//! caller's responsibility.

use std::io::{self, Write};

use super::TnefDateTime;

const MAX_LINE_LENGTH: usize = 75;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Idle,
    Property,
    Finished,
    Faulted,
}

/// Writes UTF-8 iCalendar content lines with CRLF endings and RFC 5545 folding.
///
/// Begin a property, append parameters, then terminate it with `write_value`
/// or `write_text_value`. The logical line is buffered; physical lines never
/// exceed 75 octets or split a UTF-8 sequence. The buffer is reused.
///
/// Property, parameter, and component names must be nonempty ASCII letters,
/// digits, or hyphens. Unlike MimeKit's internal helper, this public API checks
/// names before modifying the line. Values retain MimeKit's escaping and
/// control-character filtering, without RFC 6868 parameter encoding.
///
/// Call `finish` to check completeness and flush the sink. Invalid operations
/// leave the writer usable; sink failures are terminal. Drop performs no I/O.
///
/// ```
/// use mimecraft::tnef::calendar::CalendarWriter;
/// let mut writer = CalendarWriter::new(Vec::new());
/// writer.begin_component("VEVENT")?;
/// writer.write_text_property("SUMMARY", "Meet, discuss")?;
/// writer.end_component("VEVENT")?;
/// writer.finish()?;
/// assert_eq!(writer.into_inner(),
///     b"BEGIN:VEVENT\r\nSUMMARY:Meet\\, discuss\r\nEND:VEVENT\r\n");
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
#[must_use = "call finish() to check completeness and flush the sink"]
pub struct CalendarWriter<W> {
    inner: W,
    line: String,
    state: State,
}

impl<W: Write> CalendarWriter<W> {
    /// Creates a writer without writing or flushing anything.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            line: String::new(),
            state: State::Idle,
        }
    }

    /// Borrows the sink. An unfinished logical line has not been emitted.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Recovers the sink without I/O, discarding any unfinished logical line.
    pub fn into_inner(self) -> W {
        self.inner
    }

    /// Returns whether a sink error has permanently faulted the writer.
    pub fn is_faulted(&self) -> bool {
        self.state == State::Faulted
    }

    /// Returns whether `finish` completed successfully.
    pub fn is_finished(&self) -> bool {
        self.state == State::Finished
    }

    /// Writes a BEGIN content line. Component nesting is not tracked.
    ///
    /// # Errors
    ///
    /// Shares `write_property` errors, including invalid component names.
    pub fn begin_component(&mut self, name: &str) -> io::Result<()> {
        self.check(State::Idle)?;
        validate_name(name)?;
        self.write_property("BEGIN", name)
    }

    /// Writes an END content line, without checking a matching BEGIN.
    ///
    /// # Errors
    ///
    /// Shares `write_property` errors, including invalid component names.
    pub fn end_component(&mut self, name: &str) -> io::Result<()> {
        self.check(State::Idle)?;
        validate_name(name)?;
        self.write_property("END", name)
    }

    /// Begins a logical content line, retaining the name's case.
    ///
    /// # Errors
    ///
    /// Rejects invalid names or an unfinished property with `InvalidInput`.
    /// Finished/faulted writers return `BrokenPipe`.
    pub fn begin_property(&mut self, name: &str) -> io::Result<()> {
        self.check(State::Idle)?;
        validate_name(name)?;
        self.line.clear();
        self.line.push_str(name);
        self.state = State::Property;
        Ok(())
    }

    /// Appends a parameter, quoting punctuation and replacing `"` with `'`.
    ///
    /// RFC 5545 controls other than TAB are dropped. RFC 6868 encoding is not used.
    ///
    /// # Errors
    ///
    /// Rejects invalid names, missing properties, or finished/faulted writers.
    pub fn write_parameter(&mut self, name: &str, value: &str) -> io::Result<()> {
        self.check(State::Property)?;
        validate_name(name)?;
        self.line.push(';');
        self.line.push_str(name);
        self.line.push('=');
        append_parameter_value(&mut self.line, value);
        Ok(())
    }

    /// Terminates the content line with an already formatted value.
    ///
    /// For DATE-TIME, INTEGER, RECUR, URI, and other preformatted values.
    /// RFC 5545 controls (including CR/LF, but not TAB) are dropped; punctuation
    /// and backslashes are written as-is.
    ///
    /// # Errors
    ///
    /// Rejects missing properties or finished/faulted writers. Sink errors
    /// preserve the emitted prefix and permanently fault the writer.
    pub fn write_value(&mut self, value: &str) -> io::Result<()> {
        self.check(State::Property)?;
        self.line.push(':');
        self.line.extend(value.chars().filter(|&c| !is_control(c)));
        self.emit()
    }

    /// Terminates the content line with an escaped RFC 5545 TEXT value.
    ///
    /// See [`append_text`] for escaping and control filtering.
    ///
    /// # Errors
    ///
    /// Shares `write_value` state and I/O errors.
    pub fn write_text_value(&mut self, text: &str) -> io::Result<()> {
        self.check(State::Property)?;
        self.line.push(':');
        append_text(&mut self.line, text);
        self.emit()
    }

    /// Writes a complete property with an already formatted value.
    ///
    /// # Errors
    ///
    /// Shares `begin_property` and `write_value` errors.
    pub fn write_property(&mut self, name: &str, value: &str) -> io::Result<()> {
        self.begin_property(name)?;
        self.write_value(value)
    }

    /// Writes a complete property with an escaped TEXT value.
    ///
    /// # Errors
    ///
    /// Shares `begin_property` and `write_text_value` errors.
    pub fn write_text_property(&mut self, name: &str, text: &str) -> io::Result<()> {
        self.begin_property(name)?;
        self.write_text_value(text)
    }

    /// Flushes the sink, preserving any unfinished logical line.
    ///
    /// Also allowed after successful `finish`. Interrupted flushes are retried.
    ///
    /// # Errors
    ///
    /// Sink errors permanently fault the writer; later calls return `BrokenPipe`.
    pub fn flush(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        let previous = self.state;
        self.state = State::Faulted;
        loop {
            match self.inner.flush() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => {
                    result?;
                    self.state = previous;
                    return Ok(());
                }
            }
        }
    }

    /// Checks completeness, flushes the sink, and ends input.
    /// Successful repeats are no-ops; no component-end lines are synthesized.
    ///
    /// # Errors
    ///
    /// An unfinished property returns `InvalidInput` and may still be completed.
    /// Shares `flush` errors.
    pub fn finish(&mut self) -> io::Result<()> {
        self.check_faulted()?;
        if self.state == State::Property {
            return Err(invalid("the calendar property has no value"));
        }
        if !self.is_finished() {
            self.flush()?;
            self.state = State::Finished;
        }
        Ok(())
    }

    fn check_faulted(&self) -> io::Result<()> {
        if self.is_faulted() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the calendar writer is faulted",
            ))
        } else {
            Ok(())
        }
    }

    fn check(&self, expected: State) -> io::Result<()> {
        self.check_faulted()?;
        if self.is_finished() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "calendar input is finished",
            ));
        }
        if self.state != expected {
            return Err(invalid(if expected == State::Property {
                "begin a calendar property first"
            } else {
                "the previous calendar property has not been terminated"
            }));
        }
        Ok(())
    }

    fn emit(&mut self) -> io::Result<()> {
        self.state = State::Faulted;
        let bytes = self.line.as_bytes();
        let mut limit = MAX_LINE_LENGTH;
        let mut index = 0;

        // RFC 5545 3.1: fold lines longer than 75 octets using CRLF and a space.
        // The space counts towards the length of the continuation line.
        while bytes.len() - index > limit {
            let mut end = index + limit;
            // Never split a UTF-8 sequence: back up to the start of the
            // sequence that straddles the limit.
            while !self.line.is_char_boundary(end) {
                end -= 1;
            }
            self.inner.write_all(&bytes[index..end])?;
            self.inner.write_all(b"\r\n ")?;
            limit = MAX_LINE_LENGTH - 1;
            index = end;
        }
        self.inner.write_all(&bytes[index..])?;
        self.inner.write_all(b"\r\n")?;
        self.state = State::Idle;
        self.line.clear();
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        Err(invalid(
            "calendar names must be nonempty ASCII letters, digits, or hyphens",
        ))
    } else {
        Ok(())
    }
}

fn is_control(c: char) -> bool {
    // RFC 5545 3.1: CONTROL is %x00-08 / %x0A-1F / %x7F,
    // which is not allowed in any value.
    (c < '\x20' && c != '\t') || c == '\x7f'
}

/// Appends escaped RFC 5545 TEXT, retaining any existing builder contents.
///
/// Backslash, semicolon, comma, and line breaks are escaped. CRLF is one line
/// break; bare CR/LF are also written as `\n`. Other RFC 5545 controls are
/// dropped; TAB and non-ASCII characters are preserved.
pub fn append_text(builder: &mut String, text: &str) {
    // RFC 5545 3.3.11: backslash, semicolon, comma and line breaks are escaped.
    // Other control characters cannot be represented and are dropped.
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => builder.push_str("\\\\"),
            ';' => builder.push_str("\\;"),
            ',' => builder.push_str("\\,"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                builder.push_str("\\n");
            }
            '\n' => builder.push_str("\\n"),
            c if !is_control(c) => builder.push(c),
            _ => {}
        }
    }
}

/// Escapes RFC 5545 TEXT into a new string; see [`append_text`].
pub fn escape_text(text: &str) -> String {
    let mut builder = String::with_capacity(text.len());
    append_text(&mut builder, text);
    builder
}

fn append_parameter_value(builder: &mut String, value: &str) {
    // RFC 5545 3.2: paramtext excludes DQUOTE, ';', ':', ',' and CONTROL;
    // quoted-string excludes DQUOTE and CONTROL. Many readers do not implement
    // RFC 6868, so replace double quotes with single quotes instead.
    let quote = value.bytes().any(|c| matches!(c, b';' | b':' | b','));
    if quote {
        builder.push('"');
    }
    for c in value.chars() {
        if c == '"' {
            builder.push('\'');
        } else if !is_control(c) {
            builder.push(c);
        }
    }
    if quote {
        builder.push('"');
    }
}

/// Formats an RFC 5545 DATE-TIME at whole-second precision.
///
/// `utc` selects the `Z` suffix only: it does not convert the date or inspect
/// its kind. A non-UTC value needs a TZID parameter unless it is floating time.
pub fn format_date_time(value: TnefDateTime, utc: bool) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}{}",
        value.year(),
        value.month(),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        if utc { "Z" } else { "" }
    )
}

/// Formats an RFC 5545 DATE, without converting the time zone.
pub fn format_date(value: TnefDateTime) -> String {
    format!("{:04}{:02}{:02}", value.year(), value.month(), value.day())
}

/// Formats signed offset minutes with a sign and at least two hour/minute digits.
///
/// Matches MimeKit's UTC-OFFSET formatting; this helper does not validate a
/// time zone's range, and large inputs can produce more than two hour digits.
///
/// # Errors
///
/// Rejects `i32::MIN`, whose absolute value cannot be represented, as MimeKit does.
pub fn format_utc_offset(minutes: i32) -> io::Result<String> {
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes
        .checked_abs()
        .ok_or_else(|| invalid("calendar offset magnitude overflows i32"))?;
    Ok(format!("{sign}{:02}{:02}", minutes / 60, minutes % 60))
}

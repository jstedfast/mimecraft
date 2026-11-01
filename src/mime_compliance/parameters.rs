// Adapted from MimeKit's ParameterValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    io,
};

use super::{
    MimeComplianceContext, MimeComplianceIssue, MimeComplianceLogger, MimeCompliancePositionKind,
    MimeComplianceViolation as Violation,
};
use crate::{
    HeaderId, MimePosition,
    parse_utils::{
        is_attribute, is_whitespace, skip_comment, skip_comments_and_whitespace, skip_quoted,
        skip_whitespace,
    },
};

#[derive(Default)]
struct ParameterState {
    // A balanced tree guarantees bounded insertion costs without relying on
    // integer hashing, including for adversarially chosen segment numbers.
    segments: Option<BTreeSet<u32>>,
    missing_segment_issue: Option<MimeComplianceIssue>,
    extended: bool,
    plain: bool,
}

impl ParameterState {
    fn has_missing_segments(&self) -> bool {
        // Distinct nonnegative indices cover 0..n-1 iff their maximum is n-1.
        self.segments.as_ref().is_some_and(|segments| {
            segments
                .last()
                .is_some_and(|&highest| highest as usize != segments.len() - 1)
        })
    }

    fn add(&mut self, segment: Option<u32>, encoded: bool, allow_fallback: bool) -> bool {
        if let Some(segment) = segment {
            let repeated =
                self.segments.is_none() && (self.extended || (self.plain && !allow_fallback));
            return !self
                .segments
                .get_or_insert_with(BTreeSet::new)
                .insert(segment)
                || repeated;
        }
        if encoded {
            let repeated =
                self.extended || self.segments.is_some() || (self.plain && !allow_fallback);
            self.extended = true;
            repeated
        } else {
            let repeated =
                self.plain || (!allow_fallback && (self.extended || self.segments.is_some()));
            self.plain = true;
            repeated
        }
    }
}

struct Declaration<'a> {
    name: &'a str,
    segment: Option<u32>,
    encoded: bool,
    start: usize,
}

// Inspect raw declarations independently of parameter decoding and precedence.
// Malformed syntax ends or resynchronizes this check, not the MIME parser.
struct Scanner<'a> {
    text: &'a [u8],
    index: usize,
}

impl<'a> Scanner<'a> {
    fn cfws(&mut self) -> bool {
        skip_comments_and_whitespace(self.text, &mut self.index).is_ok()
    }

    fn next_parameter(&mut self) -> bool {
        while let Some(&byte) = self.text.get(self.index) {
            match byte {
                b';' => {
                    self.index += 1;
                    return true;
                }
                b'"' => {
                    if skip_quoted(self.text, &mut self.index).is_err() {
                        return false;
                    }
                }
                b'(' => {
                    if skip_comment(self.text, &mut self.index).is_err() {
                        return false;
                    }
                }
                _ => self.index += 1,
            }
        }
        false
    }

    fn parameter(&mut self) -> Option<Declaration<'a>> {
        let start = self.index;
        while self.text.get(self.index).is_some_and(|&b| is_attribute(b)) {
            self.index += 1;
        }
        let end = self.index;
        if start == end || !self.cfws() {
            return None;
        }
        let mut segment = None;
        let mut encoded = false;
        if self.text.get(self.index) == Some(&b'*') {
            self.index += 1;
            if !self.cfws() {
                return None;
            }
            if self.text.get(self.index).is_some_and(u8::is_ascii_digit) {
                let mut value = 0u32;
                while let Some(&byte) = self.text.get(self.index).filter(|b| b.is_ascii_digit()) {
                    let digit = u32::from(byte - b'0');
                    if value > (i32::MAX as u32 - digit) / 10 {
                        return None;
                    }
                    value = value * 10 + digit;
                    self.index += 1;
                }
                segment = Some(value);
                if !self.cfws() {
                    return None;
                }
                if self.text.get(self.index) == Some(&b'*') {
                    self.index += 1;
                    encoded = true;
                }
            } else {
                encoded = true;
            }
            if !self.cfws() {
                return None;
            }
        }
        if self.text.get(self.index) != Some(&b'=') {
            return None;
        }
        self.index += 1;
        Some(Declaration {
            name: std::str::from_utf8(&self.text[start..end]).expect("attribute names are ASCII"),
            segment,
            encoded,
            start,
        })
    }

    fn skip_value(&mut self, allow_empty: bool) -> (bool, Option<&'a [u8]>) {
        if !self.cfws() {
            return (false, None);
        }
        let start = self.index;
        if self.index == self.text.len() {
            return (false, allow_empty.then_some(&self.text[start..start]));
        }
        let end = if self.text[self.index] == b'"' {
            if skip_quoted(self.text, &mut self.index).is_err() {
                return (false, None);
            }
            self.index
        } else {
            while self.index < self.text.len() && !matches!(self.text[self.index], b';' | b'(') {
                self.index += 1;
            }
            let mut end = self.index;
            while end > start && is_whitespace(self.text[end - 1]) {
                end -= 1;
            }
            end
        };
        let value = (end != start || allow_empty).then_some(&self.text[start..end]);
        if !self.cfws() {
            return (false, None);
        }
        if self.index == self.text.len() {
            return (false, value);
        }
        // Loose parsing also accepts a missing separator after a quote or comment.
        if self.text[self.index] == b';' {
            self.index += 1;
        }
        (self.index < self.text.len(), value)
    }
}

fn validate_encoded_value(mut value: &[u8], initial: bool) -> bool {
    // Match the decoder's tolerance for quoted extended values.
    if value.len() >= 2 && value[0] == b'"' && value[value.len() - 1] == b'"' {
        value = &value[1..value.len() - 1];
    }
    let mut index = 0;
    if initial {
        for _ in 0..2 {
            while index < value.len() && value[index] != b'\'' {
                if value[index] <= 32 || value[index] >= 127 {
                    return false;
                }
                index += 1;
            }
            if index == value.len() {
                return false;
            }
            index += 1;
        }
    }
    // RFC 2231 section 7 requires complete ext-octets in each segment,
    // even though a multibyte character may span several segments.
    while index < value.len() {
        if value[index] == b'%' {
            if value.len() - index < 3
                || !value[index + 1].is_ascii_hexdigit()
                || !value[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            if !is_attribute(value[index]) {
                return false;
            }
            index += 1;
        }
    }
    true
}

fn contains_encoded_word(mut value: &[u8]) -> bool {
    use super::encoded_words::is_encoded_word_token;
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum State {
        Text,
        Equals,
        CharsetStart,
        Charset,
        Encoding,
        Separator,
        PayloadStart,
        Payload,
        Question,
    }

    let quoted = value.len() >= 2 && value[0] == b'"' && value[value.len() - 1] == b'"';
    if quoted {
        value = &value[1..value.len() - 1];
    }
    let mut state = State::Text;
    let mut payload_ends_with_equals = false;
    let mut bytes = value.iter().copied();
    while let Some(mut byte) = bytes.next() {
        if quoted
            && byte == b'\\'
            && let Some(escaped) = bytes.next()
        {
            byte = escaped;
        }
        state = match state {
            State::Equals if byte == b'?' => State::CharsetStart,
            State::Charset if byte == b'?' => State::Encoding,
            State::CharsetStart | State::Charset
                if is_encoded_word_token(byte) && (state == State::Charset || byte != b'*') =>
            {
                State::Charset
            }
            State::Encoding if matches!(byte, b'B' | b'b' | b'Q' | b'q') => State::Separator,
            State::Separator if byte == b'?' => State::PayloadStart,
            State::Payload if byte == b'?' => State::Question,
            State::PayloadStart | State::Payload if byte > 32 && byte < 127 && byte != b'?' => {
                payload_ends_with_equals = byte == b'=';
                State::Payload
            }
            State::Question if byte == b'=' => return true,
            // An unterminated word may contain the start of another word.
            // Resume at its charset without rescanning the preceding payload.
            State::Question
                if payload_ends_with_equals && byte != b'*' && is_encoded_word_token(byte) =>
            {
                State::Charset
            }
            _ if byte == b'=' => State::Equals,
            _ => State::Text,
        };
    }
    false
}

pub(super) fn validate(
    text: &[u8],
    header: HeaderId,
    mut position: MimePosition,
    mut column: u64,
    context: MimeComplianceContext,
    logger: &mut dyn MimeComplianceLogger,
) -> io::Result<()> {
    let mut scanner = Scanner { text, index: 0 };
    if !scanner.next_parameter() {
        return Ok(());
    }
    // The reader's max_header_length bounds the number and size of declarations.
    // Keep the first name outside the dictionary for single-parameter headers.
    let mut first: Option<(&str, ParameterState)> = None;
    let mut parameters: Option<HashMap<Cow<'_, str>, usize>> = None;
    // Preserve declaration order when reporting gaps, independently of hash order.
    let mut states: Vec<ParameterState> = Vec::new();
    let mut scanned = 0;
    let mut create_issue = |violation, at, position_kind| {
        // Advance monotonically so many reports do not rescan the prefix.
        let prefix = &text[scanned..at];
        column = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(column + prefix.len() as u64, |lf| {
                (prefix.len() - lf) as u64
            });
        position = position.advance(prefix);
        scanned = at;
        MimeComplianceIssue {
            context,
            violation,
            stream_offset: position.offset,
            line_number: position.line,
            column_number: column,
            position_kind,
        }
    };
    while scanner.index < text.len() {
        skip_whitespace(text, &mut scanner.index);
        let start = scanner.index;
        if !scanner.cfws() {
            logger.log(create_issue(
                Violation::InvalidParameter,
                start,
                MimeCompliancePositionKind::ElementStart,
            ))?;
            break;
        }
        if scanner.index == text.len() {
            break;
        }
        // Empty parameter slots are also ignored by ParameterList.
        if text[scanner.index] == b';' {
            scanner.index += 1;
            continue;
        }
        let start = scanner.index;
        let Some(declaration) = scanner.parameter() else {
            logger.log(create_issue(
                Violation::InvalidParameter,
                start,
                MimeCompliancePositionKind::ElementStart,
            ))?;
            if !scanner.next_parameter() {
                break;
            }
            continue;
        };
        let name = declaration.name;
        let filename = name.eq_ignore_ascii_case(if header == HeaderId::ContentType {
            "name"
        } else {
            "filename"
        });
        let violation = if header == HeaderId::ContentType && name.eq_ignore_ascii_case("boundary")
        {
            Violation::RepeatedBoundaryParameter
        } else if filename {
            Violation::RepeatedFilenameParameter
        } else {
            Violation::RepeatedParameter
        };
        let (first_name, first_state) =
            first.get_or_insert_with(|| (name, ParameterState::default()));
        let state = if name.eq_ignore_ascii_case(first_name) {
            first_state
        } else {
            let key = if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
                Cow::Owned(name.to_ascii_lowercase())
            } else {
                Cow::Borrowed(name)
            };
            let index = *parameters
                .get_or_insert_with(HashMap::new)
                .entry(key)
                .or_insert_with(|| {
                    states.push(ParameterState::default());
                    states.len() - 1
                });
            &mut states[index]
        };
        // Capture the first segment's position before later issues advance the cursor.
        if declaration.segment.is_some() && state.segments.is_none() {
            state.missing_segment_issue = Some(create_issue(
                Violation::MissingParameterSegment,
                declaration.start,
                MimeCompliancePositionKind::ElementStart,
            ));
        }
        if state.add(declaration.segment, declaration.encoded, filename) {
            logger.log(create_issue(
                violation,
                declaration.start,
                MimeCompliancePositionKind::Exact,
            ))?;
        }
        let initial = declaration.segment.is_none_or(|segment| segment == 0);
        let (more, value) = scanner.skip_value(declaration.encoded && !initial);
        let violation = match value {
            None => Some(Violation::InvalidParameter),
            Some(value) if declaration.encoded && !validate_encoded_value(value, initial) => {
                Some(Violation::InvalidRfc2231Parameter)
            }
            _ => None,
        };
        if let Some(violation) = violation {
            logger.log(create_issue(
                violation,
                declaration.start,
                MimeCompliancePositionKind::ElementStart,
            ))?;
        }
        if value.is_some_and(contains_encoded_word) {
            logger.log(create_issue(
                Violation::EncodedWordInParameter,
                declaration.start,
                MimeCompliancePositionKind::ElementStart,
            ))?;
        }
        if !more {
            break;
        }
    }
    for state in first.into_iter().map(|(_, state)| state).chain(states) {
        if state.has_missing_segments() {
            logger.log(
                state
                    .missing_segment_issue
                    .expect("continuations have a first declaration"),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::allocation_tracking;
    use super::*;
    use std::time::Duration;

    fn measure_validation(text: &[u8]) -> (Vec<MimeComplianceIssue>, usize, Duration) {
        let mut issues = Vec::new();
        let (result, allocated, elapsed) = allocation_tracking::measure(|| {
            validate(
                text,
                HeaderId::ContentType,
                MimePosition { offset: 0, line: 1 },
                1,
                MimeComplianceContext::Transport,
                &mut |issue| {
                    issues.push(issue);
                    Ok(())
                },
            )
        });
        result.unwrap();
        (issues, allocated, elapsed)
    }

    #[test]
    fn single_parameter_name_does_not_allocate_a_dictionary() {
        for (header, text, expected) in [
            (HeaderId::ContentType, "text/plain", 0),
            (HeaderId::ContentType, "text/plain; charset=utf-8", 0),
            (
                HeaderId::ContentType,
                "text/plain; CHARSET=utf-8; charset=us-ascii",
                1,
            ),
            (
                HeaderId::ContentDisposition,
                "attachment; filename=\"report.pdf\"",
                0,
            ),
            (
                HeaderId::ContentDisposition,
                "attachment; filename=a; FILENAME*=utf-8''b",
                0,
            ),
        ] {
            let mut count = 0;
            let (result, allocated, _) = allocation_tracking::measure(|| {
                validate(
                    text.as_bytes(),
                    header,
                    MimePosition { offset: 0, line: 1 },
                    1,
                    MimeComplianceContext::Transport,
                    &mut |_| {
                        count += 1;
                        Ok(())
                    },
                )
            });
            result.unwrap();
            assert_eq!(count, expected, "{text}");
            assert_eq!(allocated, 0, "{text}");
        }
    }

    #[test]
    fn lowercase_dictionary_keys_borrow_the_input() {
        let mut short = String::from("text/plain; first=value");
        let mut long = short.clone();
        let suffix = "x".repeat(128);
        for index in 0..128 {
            short.push_str(&format!("; x{index}=value"));
            long.push_str(&format!("; x{index}{suffix}=value"));
        }
        let (short_issues, short_allocated, _) = measure_validation(short.as_bytes());
        let (long_issues, long_allocated, _) = measure_validation(long.as_bytes());
        assert!(short_issues.is_empty());
        assert!(long_issues.is_empty());
        assert_eq!(short_allocated, long_allocated);
    }

    #[test]
    fn multiple_continuation_families_preserve_order_and_reuse() {
        for value in [
            "text/plain; first*2=a;\r\n second*1=b; third*3=c;\r\n first*2=d; second*1=e",
            "text/plain; FIRST*2=a;\r\n SECOND*1=b; third*3=c;\r\n first*2=d; second*1=e",
            "text/plain; first*2=a;\r\n second*1=b; third*3=c;\r\n FIRST*2=d; SECOND*1=e",
        ] {
            let bytes = format!("xx{value}yy");
            for _ in 0..2 {
                let mut issues = Vec::new();
                validate(
                    &bytes.as_bytes()[2..2 + value.len()],
                    HeaderId::ContentType,
                    MimePosition {
                        offset: 100,
                        line: 5,
                    },
                    7,
                    MimeComplianceContext::Storage,
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                )
                .unwrap();
                let names = [
                    "first*2=d",
                    "second*1=e",
                    "first*2=a",
                    "second*1=b",
                    "third*3=c",
                ];
                assert_eq!(issues.len(), names.len());
                for (i, (issue, name)) in issues.iter().zip(names).enumerate() {
                    let offset = value.to_ascii_lowercase().find(name).unwrap();
                    let prefix = &value.as_bytes()[..offset];
                    assert_eq!(
                        issue.violation,
                        if i < 2 {
                            Violation::RepeatedParameter
                        } else {
                            Violation::MissingParameterSegment
                        }
                    );
                    assert_eq!(issue.stream_offset, 100 + offset as u64);
                    assert_eq!(
                        issue.line_number,
                        5 + prefix.iter().filter(|&&b| b == b'\n').count() as u64
                    );
                    assert_eq!(
                        issue.column_number,
                        prefix
                            .iter()
                            .rposition(|&b| b == b'\n')
                            .map_or(7 + offset, |lf| offset - lf) as u64
                    );
                }
                for clean in [
                    "text/plain",
                    "text/plain; first*0=a; second*0=b; first*1=c; second*1=d",
                ] {
                    validate(
                        clean.as_bytes(),
                        HeaderId::ContentType,
                        MimePosition {
                            offset: 100,
                            line: 5,
                        },
                        7,
                        MimeComplianceContext::Storage,
                        &mut |_| panic!("complete family"),
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn encoded_word_calls_preserve_slice_offsets_and_clear_state() {
        let value = "text/plain;\r\n name=\"=\\?utf-8?Q?file.txt?=\"";
        let bytes = format!("xx{value}yy");
        let incomplete = "text/plain; name==?utf-8?Q?file?";
        let completed = format!("{incomplete}=");
        for _ in 0..2 {
            for (text, count) in [
                (&bytes.as_bytes()[2..2 + value.len()], 1),
                (&completed.as_bytes()[..incomplete.len()], 0),
                (completed.as_bytes(), 1),
            ] {
                let mut issues = Vec::new();
                validate(
                    text,
                    HeaderId::ContentType,
                    MimePosition {
                        offset: 100,
                        line: 5,
                    },
                    7,
                    MimeComplianceContext::Storage,
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(issues.len(), count);
                if count != 0 {
                    let issue = issues[0];
                    let offset = std::str::from_utf8(text).unwrap().find("name=").unwrap() as u64;
                    assert_eq!(issue.violation, Violation::EncodedWordInParameter);
                    assert_eq!(issue.stream_offset, 100 + offset);
                    assert_eq!(
                        issue.line_number,
                        if text == value.as_bytes() { 6 } else { 5 }
                    );
                    assert_eq!(
                        issue.column_number,
                        if text == value.as_bytes() {
                            2
                        } else {
                            7 + offset
                        }
                    );
                    assert_eq!(issue.context, MimeComplianceContext::Storage);
                    assert_eq!(
                        issue.position_kind,
                        MimeCompliancePositionKind::ElementStart
                    );
                }
            }
        }
    }

    #[test]
    fn many_encoded_word_parameters_are_bounded() {
        const COUNT: usize = 16_384;
        let parameter = "x=\"=?utf-8?Q?file?=\"";
        let text = format!("text/plain{}", format!(";\r\n {parameter}").repeat(COUNT));
        let (issues, allocated, elapsed) = measure_validation(text.as_bytes());
        assert_eq!(issues.len(), COUNT * 2 - 1);
        let words: Vec<_> = issues
            .iter()
            .filter(|i| i.violation == Violation::EncodedWordInParameter)
            .collect();
        assert_eq!(words.len(), COUNT);
        assert!(
            words
                .iter()
                .all(|i| i.position_kind == MimeCompliancePositionKind::ElementStart)
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::RepeatedParameter)
                .count(),
            COUNT - 1
        );
        let last = issues.last().unwrap();
        assert_eq!(last.line_number, COUNT as u64 + 1);
        assert_eq!(last.column_number, 2);
        assert_eq!(last.stream_offset, (text.len() - parameter.len()) as u64);
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        assert!(allocated < COUNT * 256 + 65536, "{allocated} bytes");
    }

    #[test]
    fn encoded_word_scan_is_bounded() {
        for fragment in ["plain", "=?", "=?utf-8?Q?payload"] {
            for complete in [false, true] {
                let mut text = String::from("text/plain; name=\"");
                while text.len() < 1024 * 1024 {
                    text.push_str(fragment);
                }
                if complete {
                    text.push_str("=?utf-8?Q?end?=");
                }
                text.push('"');
                let (issues, allocated, elapsed) = measure_validation(text.as_bytes());
                assert_eq!(issues.len(), usize::from(complete));
                if complete {
                    assert_eq!(issues[0].violation, Violation::EncodedWordInParameter);
                    assert_eq!(
                        issues[0].position_kind,
                        MimeCompliancePositionKind::ElementStart
                    );
                }
                assert!(
                    elapsed < Duration::from_secs(5),
                    "fragment={fragment}, complete={complete}: {elapsed:?}"
                );
                assert!(
                    allocated < 65536,
                    "fragment={fragment}, complete={complete}: {allocated} bytes"
                );
            }
        }
    }

    #[test]
    fn encoded_calls_preserve_slice_offsets_and_clear_state() {
        let value = "text/plain;\r\n name*=utf-8''file%2";
        let bytes = format!("xx{value}0yy");
        for _ in 0..2 {
            for text in [
                &bytes.as_bytes()[2..2 + value.len()],
                b"text/plain; name*0*=utf-8''%C3; name*1*=%A9",
            ] {
                let mut issues = Vec::new();
                validate(
                    text,
                    HeaderId::ContentType,
                    MimePosition {
                        offset: 100,
                        line: 5,
                    },
                    7,
                    MimeComplianceContext::Storage,
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                )
                .unwrap();
                if text == value.as_bytes() {
                    assert_eq!(issues.len(), 1);
                    let issue = issues[0];
                    assert_eq!(issue.violation, Violation::InvalidRfc2231Parameter);
                    assert_eq!(
                        issue.stream_offset,
                        100 + value.find("name*=").unwrap() as u64
                    );
                    assert_eq!(issue.line_number, 6);
                    assert_eq!(issue.column_number, 2);
                    assert_eq!(issue.context, MimeComplianceContext::Storage);
                    assert_eq!(
                        issue.position_kind,
                        MimeCompliancePositionKind::ElementStart
                    );
                } else {
                    assert!(issues.is_empty());
                }
            }
        }
    }

    #[test]
    fn many_malformed_encoded_values_are_bounded() {
        const COUNT: usize = 16_384;
        let parameter = "x*=utf-8''%GG";
        let text = format!("text/plain{}", format!(";\r\n {parameter}").repeat(COUNT));
        let (issues, allocated, elapsed) = measure_validation(text.as_bytes());
        assert_eq!(issues.len(), COUNT * 2 - 1);
        let invalid: Vec<_> = issues
            .iter()
            .filter(|i| i.violation == Violation::InvalidRfc2231Parameter)
            .collect();
        assert_eq!(invalid.len(), COUNT);
        assert!(
            invalid
                .iter()
                .all(|i| i.position_kind == MimeCompliancePositionKind::ElementStart)
        );
        assert_eq!(
            issues
                .iter()
                .filter(|i| i.violation == Violation::RepeatedParameter)
                .count(),
            COUNT - 1
        );
        let last = issues.last().unwrap();
        assert_eq!(last.line_number, COUNT as u64 + 1);
        assert_eq!(last.column_number, 2);
        assert_eq!(last.stream_offset, (text.len() - parameter.len()) as u64);
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        assert!(allocated < COUNT * 256 + 65536, "{allocated} bytes");
    }

    #[test]
    fn encoded_value_scan_is_bounded() {
        for prefix in [false, true] {
            let text = format!(
                "text/plain; name*={}{}%2",
                if prefix { "utf-8''" } else { "" },
                "a".repeat(1024 * 1024)
            );
            let (issues, allocated, elapsed) = measure_validation(text.as_bytes());
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].violation, Violation::InvalidRfc2231Parameter);
            assert_eq!(
                issues[0].position_kind,
                MimeCompliancePositionKind::ElementStart
            );
            assert!(
                elapsed < Duration::from_secs(5),
                "prefix={prefix}: {elapsed:?}"
            );
            assert!(allocated < 65536, "prefix={prefix}: {allocated} bytes");
        }
    }

    #[test]
    fn encoded_value_validation_checks_raw_bytes_without_decoding() {
        for value in [
            b"utf-\x7f''file".as_slice(),
            b"utf-\xff''file",
            b"utf-8'e\t n'file",
            b"utf-8''file\x00",
            b"utf-8''file\x7f",
            b"utf-8''file\xff",
            b"\"utf-8''file\\\"name\"",
            b"\"utf-8''file;name\"",
        ] {
            assert!(!validate_encoded_value(value, true), "{value:?}");
        }
        for value in [
            b"utf-8''%00%7F%FF".as_slice(),
            b"x-unknown'not-registered'%ff",
        ] {
            assert!(validate_encoded_value(value, true), "{value:?}");
        }
    }

    #[test]
    fn repeated_calls_preserve_slice_offsets_and_initial_column() {
        let value = "text/plain; x=1;\r\n x=2; x=3";
        let bytes = format!("xx{value}yy");
        for _ in 0..2 {
            let mut issues = Vec::new();
            validate(
                &bytes.as_bytes()[2..bytes.len() - 2],
                HeaderId::ContentType,
                MimePosition {
                    offset: 100,
                    line: 5,
                },
                7,
                MimeComplianceContext::Storage,
                &mut |issue| {
                    issues.push(issue);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(issues.len(), 2);
            for (index, marker) in ["x=2", "x=3"].iter().enumerate() {
                assert_eq!(
                    issues[index].stream_offset,
                    100 + value.find(marker).unwrap() as u64
                );
                assert_eq!(issues[index].line_number, 6);
                assert_eq!(issues[index].column_number, 2 + 5 * index as u64);
                assert_eq!(issues[index].context, MimeComplianceContext::Storage);
                assert_eq!(
                    issues[index].position_kind,
                    MimeCompliancePositionKind::Exact
                );
            }
        }
        let mut issues = Vec::new();
        validate(
            b"text/plain; x=1; x=2",
            HeaderId::ContentType,
            MimePosition {
                offset: 100,
                line: 5,
            },
            7,
            MimeComplianceContext::Storage,
            &mut |issue| {
                issues.push(issue);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(issues[0].column_number, 24);
        assert_eq!(issues[0].stream_offset, 117);
    }

    #[test]
    fn malformed_calls_preserve_slice_offsets_and_clear_state() {
        let value = "text/plain; broken;\r\n x=";
        let bytes = format!("xx{value}yy");
        for _ in 0..2 {
            for text in [&bytes.as_bytes()[2..bytes.len() - 2], b"text/plain; x=\"\""] {
                let mut issues = Vec::new();
                validate(
                    text,
                    HeaderId::ContentType,
                    MimePosition {
                        offset: 100,
                        line: 5,
                    },
                    7,
                    MimeComplianceContext::Storage,
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                )
                .unwrap();
                if text == value.as_bytes() {
                    assert_eq!(issues.len(), 2);
                    for (index, marker) in ["broken", "x="].iter().enumerate() {
                        let issue = issues[index];
                        let offset = value.find(marker).unwrap() as u64;
                        assert_eq!(issue.violation, Violation::InvalidParameter);
                        assert_eq!(issue.stream_offset, 100 + offset);
                        assert_eq!(issue.line_number, 5 + index as u64);
                        assert_eq!(issue.column_number, if index == 0 { 7 + offset } else { 2 });
                        assert_eq!(issue.context, MimeComplianceContext::Storage);
                        assert_eq!(
                            issue.position_kind,
                            MimeCompliancePositionKind::ElementStart
                        );
                    }
                } else {
                    assert!(issues.is_empty());
                }
            }
        }
    }

    #[test]
    fn many_repeated_parameters_are_bounded() {
        for name in ["x", "x*0"] {
            let count = 16_384;
            let text = format!(
                "text/plain; {name}=0{}",
                format!(";\r\n {name}=1").repeat(count)
            );
            let start = std::time::Instant::now();
            let mut issues = Vec::new();
            validate(
                text.as_bytes(),
                HeaderId::ContentType,
                MimePosition { offset: 0, line: 1 },
                1,
                MimeComplianceContext::Transport,
                &mut |issue| {
                    issues.push(issue);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(issues.len(), count);
            assert_eq!(issues[count - 1].line_number, count as u64 + 1);
            assert_eq!(issues[count - 1].column_number, 2);
            assert_eq!(
                issues[count - 1].stream_offset,
                (text.len() - name.len() - 2) as u64
            );
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
        }
    }

    #[test]
    fn continuation_calls_preserve_slice_offsets_and_clear_state() {
        let value = "text/plain; name*2=x;\r\n name*2=y";
        let bytes = format!("xx{value}yy");
        for _ in 0..2 {
            for text in [
                &bytes.as_bytes()[2..bytes.len() - 2],
                b"text/plain; name*0=a; name*1=b",
            ] {
                let mut issues = Vec::new();
                validate(
                    text,
                    HeaderId::ContentType,
                    MimePosition {
                        offset: 100,
                        line: 5,
                    },
                    7,
                    MimeComplianceContext::Storage,
                    &mut |issue| {
                        issues.push(issue);
                        Ok(())
                    },
                )
                .unwrap();
                if text == value.as_bytes() {
                    assert_eq!(issues.len(), 2);
                    assert_eq!(issues[0].violation, Violation::RepeatedFilenameParameter);
                    let missing = issues[1];
                    assert_eq!(missing.violation, Violation::MissingParameterSegment);
                    assert_eq!(
                        missing.stream_offset,
                        100 + value.find("name*2").unwrap() as u64
                    );
                    assert_eq!(missing.line_number, 5);
                    assert_eq!(
                        missing.column_number,
                        7 + value.find("name*2").unwrap() as u64
                    );
                    assert_eq!(missing.context, MimeComplianceContext::Storage);
                    assert_eq!(
                        missing.position_kind,
                        MimeCompliancePositionKind::ElementStart
                    );
                } else {
                    assert!(issues.is_empty());
                }
            }
        }
    }
}

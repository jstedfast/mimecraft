// Adapted from MimeKit's ParameterValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    collections::{BTreeSet, HashMap},
    io,
};

use super::{
    MimeComplianceContext, MimeComplianceIssue, MimeComplianceLogger, MimeCompliancePositionKind,
    MimeComplianceViolation as Violation,
};
use crate::{
    HeaderId, MimePosition,
    parse_utils::{is_attribute, skip_comment, skip_comments_and_whitespace, skip_quoted},
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
// Malformed syntax ends or resynchronizes this check, not the MIME parser;
// this validator reports repetitions and missing segments, not syntax errors.
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
        if !self.cfws() {
            return None;
        }
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

    fn skip_value(&mut self) -> bool {
        if !self.cfws() || self.index == self.text.len() {
            return false;
        }
        if self.text[self.index] == b'"' {
            if skip_quoted(self.text, &mut self.index).is_err() {
                return false;
            }
        } else {
            while self.index < self.text.len() && !matches!(self.text[self.index], b';' | b'(') {
                self.index += 1;
            }
        }
        if !self.cfws() || self.index == self.text.len() {
            return false;
        }
        // Loose parsing also accepts a missing separator after a quote or comment.
        if self.text[self.index] == b';' {
            self.index += 1;
        }
        self.index < self.text.len()
    }
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
    let mut parameters: HashMap<String, usize> = HashMap::new();
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
        let Some(declaration) = scanner.parameter() else {
            if !scanner.next_parameter() {
                break;
            }
            continue;
        };
        let name = declaration.name.to_ascii_lowercase();
        let filename = name
            == if header == HeaderId::ContentType {
                "name"
            } else {
                "filename"
            };
        let violation = if header == HeaderId::ContentType && name == "boundary" {
            Violation::RepeatedBoundaryParameter
        } else if filename {
            Violation::RepeatedFilenameParameter
        } else {
            Violation::RepeatedParameter
        };
        let index = *parameters.entry(name).or_insert_with(|| {
            states.push(ParameterState::default());
            states.len() - 1
        });
        let state = &mut states[index];
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
        if !scanner.skip_value() {
            break;
        }
    }
    for state in states {
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
    use super::*;

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

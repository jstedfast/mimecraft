// Adapted from MimeKit's EncodedWordValidator.cs and AddressValidator.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io;

use super::{
    MimeComplianceContext, MimeComplianceIssue, MimeComplianceLogger, MimeCompliancePositionKind,
    MimeComplianceViolation as Violation,
};
use crate::{
    MimePosition,
    parse_utils::{is_atom, is_token, is_whitespace, skip_comments_and_whitespace, skip_quoted},
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Location {
    Text,
    Phrase,
    Comment,
    Forbidden,
}

pub(super) fn is_encoded_word_token(byte: u8) -> bool {
    is_token(byte) && byte != b'.'
}

// A forward-only scanner shared by unstructured headers and finalized address ranges.
// The RFC's 75-byte word limit bounds lookahead even for overlapping damaged candidates.
struct Scanner<'a> {
    text: &'a [u8],
    index: usize,
    position: MimePosition,
    column: u64,
    context: MimeComplianceContext,
    comment_depth: usize,
    quoted: bool,
    literal: bool,
}

impl Scanner<'_> {
    fn read_word(&self, location: Location) -> (bool, Option<usize>) {
        let limit = self.index + (self.text.len() - self.index).min(75);
        let mut at = self.index + 2;
        let charset = at;
        while at < limit && is_encoded_word_token(self.text[at]) {
            at += 1;
        }
        if at == charset || self.text[charset] == b'*' || at >= limit || self.text[at] != b'?' {
            return (false, None);
        }
        at += 1;
        let encoding = at;
        while at < limit && is_encoded_word_token(self.text[at]) {
            at += 1;
        }
        if at == encoding || at >= limit || self.text[at] != b'?' {
            return (false, None);
        }
        at += 1;
        let kind = self.text[encoding];
        let base64 = matches!(kind, b'B' | b'b');
        let mut valid = at - encoding == 2 && (base64 || matches!(kind, b'Q' | b'q'));
        let payload = at;
        let mut padding = 0;
        let mut hex = 0;
        while at < limit {
            let byte = self.text[at];
            if byte == b'?' && at + 1 < limit && self.text[at + 1] == b'=' {
                valid &= at != payload
                    && hex == 0
                    && (!base64 || ((at - payload).is_multiple_of(4) && padding <= 2));
                return (valid, Some(at + 2));
            }
            // Leave a nested candidate to the next iteration instead of hiding it in this one.
            if byte == b'='
                && at + 2 < limit
                && self.text[at + 1] == b'?'
                && self.text[at + 2] != b'='
            {
                return (false, None);
            }
            if byte <= 32 || byte >= 127 || byte == b'?' {
                valid = false;
            }
            if base64 {
                if byte == b'=' {
                    padding += 1;
                } else if padding != 0
                    || !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
                {
                    valid = false;
                }
            } else if hex > 0 {
                valid &= byte.is_ascii_hexdigit();
                hex -= 1;
            } else if byte == b'=' {
                hex = 2;
            } else if location == Location::Phrase {
                valid &= byte.is_ascii_alphanumeric() || b"!*+-/_".contains(&byte);
            } else if location == Location::Comment && matches!(byte, b'(' | b')' | b'\\') {
                valid = false;
            }
            at += 1;
        }
        (false, None)
    }

    fn boundary(&self, at: Option<usize>, location: Location) -> bool {
        at.and_then(|at| self.text.get(at)).is_none_or(|&byte| {
            is_whitespace(byte) || (location == Location::Comment && matches!(byte, b'(' | b')'))
        })
    }

    fn log(&self, violation: Violation, logger: &mut dyn MimeComplianceLogger) -> io::Result<()> {
        logger.log(MimeComplianceIssue {
            context: self.context,
            violation,
            stream_offset: self.position.offset,
            line_number: self.position.line,
            column_number: self.column,
            position_kind: MimeCompliancePositionKind::ElementStart,
        })
    }

    fn advance(&mut self, end: usize) {
        for &byte in &self.text[self.index..end] {
            if byte == b'\n' {
                self.position.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
        self.position.offset += (end - self.index) as u64;
        self.index = end;
    }

    fn validate(
        &mut self,
        end: usize,
        location: Location,
        logger: &mut dyn MimeComplianceLogger,
    ) -> io::Result<()> {
        while self.index < end {
            let byte = self.text[self.index];
            let mut word_context = location;
            if location != Location::Text {
                if self.comment_depth > 0 {
                    word_context = Location::Comment;
                } else if self.quoted || self.literal {
                    word_context = Location::Forbidden;
                }
                if byte == b'\\' && (self.comment_depth > 0 || self.quoted || self.literal) {
                    self.advance(self.index + (self.text.len() - self.index).min(2));
                    continue;
                }
                if byte == b'(' && !self.quoted && !self.literal {
                    self.comment_depth += 1;
                } else if byte == b')' && self.comment_depth > 0 {
                    self.comment_depth -= 1;
                } else if self.comment_depth == 0 && !self.literal && byte == b'"' {
                    self.quoted = !self.quoted;
                } else if self.comment_depth == 0 && !self.quoted && byte == b'[' {
                    self.literal = true;
                } else if self.literal && byte == b']' {
                    self.literal = false;
                }
            }
            if byte == b'=' && self.text.get(self.index + 1) == Some(&b'?') {
                let (mut valid, word_end) = self.read_word(word_context);
                // In an addr-spec, '=' and '?' can be ordinary local-part characters.
                // Only a complete envelope is evidence of misplaced encoded-word syntax.
                if word_context != Location::Forbidden || word_end.is_some() {
                    if word_context == Location::Forbidden {
                        self.log(Violation::InvalidEncodedWordPlacement, logger)?;
                    } else if !self.boundary(self.index.checked_sub(1), word_context)
                        || !self.boundary(Some(word_end.unwrap_or(self.index + 2)), word_context)
                    {
                        valid = false;
                    }
                    if !valid {
                        self.log(Violation::InvalidEncodedWord, logger)?;
                    }
                }
                if let Some(word_end) = word_end {
                    self.advance(word_end);
                    continue;
                }
            }
            self.advance(self.index + 1);
        }
        Ok(())
    }
}

fn skip_literal(text: &[u8], index: &mut usize) {
    // A domain-literal is one lexical token: its IPv6 colons are not group delimiters.
    *index += 1;
    while *index < text.len() && text[*index] != b']' {
        *index += 1;
    }
    if *index < text.len() {
        *index += 1;
    }
}

fn atom_or_control(byte: u8) -> bool {
    is_atom(byte) || (byte.is_ascii_control() && !is_whitespace(byte))
}

fn skip_addrspec(
    text: &[u8],
    index: &mut usize,
    confirmed: bool,
    in_angle: bool,
    in_group: bool,
) -> bool {
    loop {
        if text.get(*index) == Some(&b'"') {
            if skip_quoted(text, index).is_err() {
                return false;
            }
        } else {
            while text
                .get(*index)
                .is_some_and(|&b| atom_or_control(b) || b == b'.')
            {
                *index += 1;
            }
        }
        let before = *index;
        if skip_comments_and_whitespace(text, index).is_err() {
            return false;
        }
        // Continue a confirmed addr-spec across embedded line breaks, including
        // bare CR, instead of interpreting its remainder as another address.
        if !confirmed
            || !text.get(*index).is_some_and(|&b| atom_or_control(b))
            || !text[before..*index]
                .iter()
                .any(|&b| matches!(b, b'\r' | b'\n'))
        {
            break;
        }
    }
    if text.get(*index) != Some(&b'@') {
        return text
            .get(*index)
            .is_some_and(|&b| b != b',' && !(in_group && b == b';') && !(in_angle && b == b'>'));
    } else {
        *index += 1;
        if skip_comments_and_whitespace(text, index).is_err() {
            return false;
        }
        if text.get(*index) == Some(&b'[') {
            skip_literal(text, index);
        } else {
            loop {
                while text.get(*index).is_some_and(|&b| atom_or_control(b)) {
                    *index += 1;
                }
                let before = *index;
                if skip_comments_and_whitespace(text, index).is_err() {
                    return false;
                }
                if text.get(*index) == Some(&b'.') {
                    *index += 1;
                    if skip_comments_and_whitespace(text, index).is_err() {
                        return false;
                    }
                } else if !text.get(*index).is_some_and(|&b| atom_or_control(b))
                    || !text[before..*index]
                        .iter()
                        .any(|&b| matches!(b, b'\r' | b'\n'))
                {
                    break;
                }
            }
        }
    }
    false
}

fn skip_angle(text: &[u8], index: &mut usize, in_group: bool) -> bool {
    *index += 1;
    if skip_comments_and_whitespace(text, index).is_err() {
        return false;
    }
    while text.get(*index) == Some(&b'<') {
        *index += 1;
    }
    if skip_comments_and_whitespace(text, index).is_err() {
        return false;
    }
    if text.get(*index) == Some(&b'@') {
        // Obsolete source routes end at ':'; a missing route terminator stops at '>'.
        while *index < text.len() && !matches!(text[*index], b':' | b'>') {
            *index += 1;
        }
        if text.get(*index) == Some(&b':') {
            *index += 1;
        }
        if skip_comments_and_whitespace(text, index).is_err() {
            return false;
        }
    }
    let damaged = skip_addrspec(text, index, true, true, in_group);
    if skip_comments_and_whitespace(text, index).is_err() {
        return damaged;
    }
    while text.get(*index) == Some(&b'>') {
        *index += 1;
    }
    damaged
}

#[derive(Clone, Copy, Default)]
struct RecoveryEntry {
    phrase_end: usize,
    unquoted_at: usize,
    quote_end: usize,
    comment_end: usize,
}

#[derive(Default)]
struct Lookahead {
    start: usize,
    end: usize,
    simple: bool,
    unquoted_at: Option<usize>,
    name_addr: Option<(usize, bool)>,
    recovery: Vec<RecoveryEntry>,
}

impl Lookahead {
    fn index(&mut self, text: &[u8]) {
        // Recovery can resume inside a previously scanned token. Index its suffixes
        // backwards so even overlapping quotes, comments and literals cost O(length),
        // rather than rescanning the rest of a malformed list once per mailbox.
        let missing = text.len() + 1;
        let mut quote = missing;
        let mut comment = missing;
        let mut bracket = text.len();
        self.recovery.resize(text.len(), RecoveryEntry::default());
        for i in (0..text.len()).rev() {
            let next_quote = quote;
            let next_comment = comment;
            let byte = text[i];
            if byte == b'\\' {
                quote = self.recovery.get(i + 2).map_or(missing, |e| e.quote_end);
                comment = self.recovery.get(i + 2).map_or(missing, |e| e.comment_end);
            } else {
                if byte == b'"' {
                    quote = i + 1;
                }
                if byte == b')' {
                    comment = i + 1;
                } else if byte == b'(' {
                    comment = self
                        .recovery
                        .get(next_comment)
                        .map_or(missing, |e| e.comment_end);
                }
            }
            let next = match byte {
                b'"' => next_quote,
                b'(' => next_comment,
                b'[' => bracket,
                _ => i + 1,
            };
            let phrase_end = if matches!(byte, b'<' | b':' | b',' | b';' | b'>') {
                i
            } else {
                self.recovery.get(next).map_or(next, |e| e.phrase_end)
            };
            let next = if byte == b'"' { next_quote } else { i + 1 };
            let unquoted_at = if byte == b'@' {
                i
            } else {
                self.recovery.get(next).map_or(missing, |e| e.unquoted_at)
            };
            self.recovery[i] = RecoveryEntry {
                phrase_end,
                unquoted_at,
                quote_end: quote,
                comment_end: comment,
            };
            if byte == b']' {
                bracket = i + 1;
            }
        }
    }

    fn scan(&mut self, text: &[u8], start: usize) -> Option<usize> {
        if self.recovery.is_empty() && start >= self.start && start < self.end {
            if self.simple {
                return Some(self.end);
            }
            self.index(text);
        }
        if let Some(entry) = self.recovery.get(start) {
            return (entry.phrase_end <= text.len()).then_some(entry.phrase_end);
        }
        let mut index = start;
        let mut simple = true;
        while index < text.len() {
            match text[index] {
                b'(' => {
                    simple = false;
                    skip_comments_and_whitespace(text, &mut index).ok()?;
                }
                b'"' => {
                    simple = false;
                    skip_quoted(text, &mut index).ok()?;
                }
                b'[' => {
                    simple = false;
                    skip_literal(text, &mut index);
                }
                b'<' | b':' | b',' | b';' | b'>' => break,
                _ => index += 1,
            }
        }
        if !matches!(text.get(index), Some(b'<' | b':')) {
            self.start = start;
            self.end = index;
            self.simple = simple;
            self.unquoted_at = None;
        }
        Some(index)
    }

    fn contains_at(&mut self, text: &[u8], start: usize, end: usize) -> bool {
        if let Some(entry) = self.recovery.get(start) {
            return entry.unquoted_at < end;
        }
        if self.simple && start >= self.start && end == self.end {
            let at = self
                .unquoted_at
                .filter(|&at| at >= start)
                .unwrap_or_else(|| {
                    text[start..end]
                        .iter()
                        .position(|&b| b == b'@')
                        .map_or(end, |at| start + at)
                });
            self.unquoted_at = Some(at);
            return at < end;
        }
        let mut index = start;
        while index < end {
            if text[index] == b'"' {
                if skip_quoted(&text[..end], &mut index).is_err() {
                    return false;
                }
            } else if text[index] == b'@' {
                return true;
            } else {
                index += 1;
            }
        }
        false
    }

    fn followed_by_name_addr(&mut self, text: &[u8], start: usize) -> bool {
        if let Some((from, follows)) = self.name_addr
            && from == start
        {
            return follows;
        }
        let mut index = start;
        let mut follows = false;
        while index < text.len() {
            match text[index] {
                b'"' => {
                    if skip_quoted(text, &mut index).is_err() {
                        break;
                    }
                }
                b'<' => {
                    follows = true;
                    break;
                }
                b',' | b';' | b':' => break,
                _ => index += 1,
            }
        }
        self.name_addr = Some((start, follows));
        follows
    }
}

pub(super) fn validate(
    text: &[u8],
    address: bool,
    position: MimePosition,
    column: u64,
    context: MimeComplianceContext,
    logger: &mut dyn MimeComplianceLogger,
) -> io::Result<()> {
    let mut scanner = Scanner {
        text,
        index: 0,
        position,
        column,
        context,
        comment_depth: 0,
        quoted: false,
        literal: false,
    };
    if !address {
        return scanner.validate(text.len(), Location::Text, logger);
    }
    // Classify finalized ranges, not speculative phrase scans that may be reread as addr-specs.
    // No decoded address objects are needed, and the normal address parser is untouched.
    // Only malformed lists that re-enter complex lookahead regions need a suffix index.
    // The header's trailing line terminator is not folding inside an address token.
    let text = &text[..text
        .iter()
        .rposition(|&b| !is_whitespace(b))
        .map_or(0, |i| i + 1)];
    let mut lookahead = Lookahead::default();
    let mut index = 0;
    let mut group_depth = 0;
    while index < text.len() {
        if skip_comments_and_whitespace(text, &mut index).is_err() {
            break;
        }
        if index == text.len() {
            break;
        }
        let damaged;
        if matches!(text[index], b',' | b';') {
            let closed_group = text[index] == b';' && group_depth > 0;
            index += 1;
            if !closed_group {
                continue;
            }
            group_depth -= 1;
            damaged = false;
        } else {
            let start = index;
            scanner.validate(start, Location::Forbidden, logger)?;
            let Some(end) = lookahead.scan(text, start) else {
                break;
            };
            match text.get(end) {
                Some(b'<' | b':') => {
                    scanner.validate(end, Location::Phrase, logger)?;
                    index = end;
                    if text[index] == b':' {
                        index += 1;
                        group_depth += 1;
                        continue;
                    }
                    damaged = skip_angle(text, &mut index, group_depth > 0);
                    scanner.validate(index, Location::Forbidden, logger)?;
                }
                Some(b'>') => {
                    index = end + 1;
                    damaged = false;
                }
                _ => {
                    let confirmed = lookahead.contains_at(text, start, end);
                    if text.get(end) == Some(&b',')
                        && !confirmed
                        && lookahead.followed_by_name_addr(text, end + 1)
                    {
                        // An unquoted comma split a display-name. Leave it for the
                        // list loop rather than rereading the name as a local-part.
                        index = end;
                        damaged = false;
                    } else {
                        damaged =
                            skip_addrspec(text, &mut index, confirmed, false, group_depth > 0);
                        scanner.validate(index, Location::Forbidden, logger)?;
                    }
                }
            }
            if index == start {
                // Never spin on a character no production consumed.
                index += 1;
                continue;
            }
        }
        if skip_comments_and_whitespace(text, &mut index).is_err() || index == text.len() {
            break;
        }
        if text[index] == b',' {
            index += 1;
            continue;
        }
        if text[index] == b';' {
            if group_depth > 0 || !damaged {
                continue;
            }
            index += 1;
        }
        // Resynchronize at a byte that can begin an address, not inside a run of
        // invalid punctuation. A list separator belongs to the list loop itself.
        while index < text.len()
            && !atom_or_control(text[index])
            && !matches!(text[index], b'"' | b'<' | b'.' | b'@' | b':' | b',' | b';')
        {
            index += 1;
        }
    }
    scanner.validate(text.len(), Location::Forbidden, logger)
}

#[cfg(test)]
mod tests {
    use super::super::allocation_tracking;
    use super::*;
    use std::time::Duration;

    #[test]
    fn offsets_and_reuse() {
        let value = "=?utf-8?Q?bad=GG?= <a@example.com>,\r\n \"=?utf-8?Q?other?=\" <b@example.com>";
        let bytes = format!("xx{value}yy");
        let truncated = "=?utf-8?Q?hello?";
        let partial = format!("xx{truncated}=yy");
        for _ in 0..2 {
            for (text, address) in [
                (&bytes.as_bytes()[2..2 + value.len()], true),
                (&partial.as_bytes()[2..2 + truncated.len()], false),
            ] {
                let mut issues = Vec::new();
                validate(
                    text,
                    address,
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
                assert_eq!(issues.len(), if address { 2 } else { 1 });
                let invalid = issues[0];
                assert_eq!(invalid.violation, Violation::InvalidEncodedWord);
                assert_eq!(
                    (
                        invalid.stream_offset,
                        invalid.line_number,
                        invalid.column_number
                    ),
                    (100, 5, 7)
                );
                if address {
                    let placement = issues[1];
                    assert_eq!(placement.violation, Violation::InvalidEncodedWordPlacement);
                    assert_eq!(
                        placement.stream_offset,
                        100 + value.find("=?utf-8?Q?other").unwrap() as u64
                    );
                    assert_eq!((placement.line_number, placement.column_number), (6, 3));
                }
                assert!(
                    issues
                        .iter()
                        .all(|issue| issue.context == MimeComplianceContext::Storage
                            && issue.position_kind == MimeCompliancePositionKind::ElementStart)
                );
            }
        }
    }

    #[test]
    fn bounded_scan() {
        for fragment in ["plain", "=?", "=?utf-8?Q?broken", "=?x-unknown?Q?bad=GG?= "] {
            for address in [false, true] {
                let mut text = fragment.repeat((1024usize * 1024).div_ceil(fragment.len()));
                if address {
                    text.push_str(" <a@example.com>");
                }
                let mut count = 0;
                let (result, allocated, elapsed) = allocation_tracking::measure(|| {
                    validate(
                        text.as_bytes(),
                        address,
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
                assert_eq!(count == 0, fragment == "plain");
                assert!(
                    elapsed < Duration::from_secs(5),
                    "{fragment:?}, address={address}: {elapsed:?}"
                );
                assert!(
                    allocated < 65536,
                    "{fragment:?}, address={address}: {allocated} bytes"
                );
            }
        }
    }

    #[test]
    fn address_structure_is_bounded_without_recursion_or_recovery_indexes() {
        for text in [
            format!(
                "{}=?utf-8?Q?bad=GG?={} a@example.com",
                "(".repeat(65536),
                ")".repeat(65536)
            ),
            format!(
                "{} =?utf-8?Q?bad=GG?= <a@example.com>",
                "nested : ".repeat(65536)
            ),
            format!("{} =?utf-8?Q?bad=GG?=", "a@example.com ".repeat(65536)),
            format!(
                "{} =?utf-8?Q?bad=GG?= <b@example.com>",
                "Name <local ".repeat(65536)
            ),
        ] {
            let mut count = 0;
            let (result, allocated, elapsed) = allocation_tracking::measure(|| {
                validate(
                    text.as_bytes(),
                    true,
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
            assert!(count > 0);
            assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
            assert!(allocated < 65536, "{allocated} bytes");
        }
    }

    #[test]
    fn complex_address_recovery_has_linear_time_and_storage() {
        for fragment in ["a@[literal] ", "a@example.com (comment) ", "[\"],"] {
            let text = format!(
                "{} =?utf-8?Q?bad=GG?=",
                fragment.repeat((1024usize * 1024).div_ceil(fragment.len()))
            );
            let mut count = 0;
            let (result, allocated, elapsed) = allocation_tracking::measure(|| {
                validate(
                    text.as_bytes(),
                    true,
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
            assert!(count > 0, "{fragment:?}");
            assert!(
                elapsed < Duration::from_secs(5),
                "{fragment:?}: {elapsed:?}"
            );
            assert!(
                allocated <= text.len() * size_of::<RecoveryEntry>() + 65536,
                "{fragment:?}: {allocated} bytes"
            );
        }
    }

    #[test]
    fn indexed_recovery_matches_direct_suffix_scans() {
        for text in [
            &b"a@[literal] (outer (nested\\) comment)) \"q\\\"uote\" , next@example.com"[..],
            &b"[\"],=?< ['( ] =?utf-8?Q?bad=GG?= <next@example.com>"[..],
            &b"\"(\\\"[(x)) \\)]\r\n @outside <end>"[..],
            &b"a@[literal] \"unterminated (comment"[..],
        ] {
            let mut indexed = Lookahead::default();
            indexed.index(text);
            for start in 0..=text.len() {
                let mut direct = Lookahead::default();
                let end = direct.scan(text, start);
                assert_eq!(indexed.scan(text, start), end, "{text:?}, {start}");
                if let Some(end) = end {
                    assert_eq!(
                        indexed.contains_at(text, start, end),
                        direct.contains_at(text, start, end),
                        "{text:?}, {start}..{end}"
                    );
                }
            }
        }
    }
}

// Adapted from MimeKit's Text/RtfTokenizer.cs.
// Reporting limits adapted from Text/RtfComplianceOptions.cs and RtfCappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};

use super::RtfKeyword;
use super::rtf_reporting::{RtfReport, RtfReportingLimits};

const MAX_KEYWORD_LENGTH: usize = 32;
const MAX_PARAMETER_DIGITS: u8 = 10;
const MAX_TEXT_LENGTH: usize = 4096;

/// An owned RTF control word with its original bounded name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtfControlWord {
    name: String,
    id: RtfKeyword,
    parameter: Option<i32>,
    name_truncated: bool,
    parameter_invalid: bool,
}

impl RtfControlWord {
    /// The first at most 32 ASCII letters, without the backslash or parameter.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The case-sensitive classification; overlong names are always unknown.
    pub fn id(&self) -> RtfKeyword {
        self.id
    }

    /// The optional signed parameter, clamped to the `i32` range.
    ///
    /// Only the first ten digits contribute; further digits are consumed and
    /// ignored, even when they follow leading zeros.
    pub fn parameter(&self) -> Option<i32> {
        self.parameter
    }

    /// Whether excess letters were consumed without storing them.
    pub fn is_name_truncated(&self) -> bool {
        self.name_truncated
    }

    /// Whether the parameter exceeded ten digits or the `i32` range, or a
    /// minus sign was followed by a non-digit.
    ///
    /// As in MimeKit, a lone minus at EOF finalizes the pending word without
    /// setting this flag. This is lexical recovery, not semantic validation.
    pub fn is_parameter_invalid(&self) -> bool {
        self.parameter_invalid
    }
}

/// The data carried by an owned RTF token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RtfTokenKind {
    /// Opening brace.
    GroupStart,
    /// Closing brace, including unmatched closing braces.
    GroupEnd,
    /// ASCII control word and optional numeric parameter.
    ControlWord(RtfControlWord),
    /// Escaped non-letter byte. Escaped CR and LF are both normalized to LF.
    ControlSymbol(u8),
    /// A byte from a hexadecimal escape, not yet decoded through a code page.
    HexChar(u8),
    /// Raw bytes, at most 4096 per token, without RTF delimiters or CR/LF.
    ///
    /// Token boundaries may depend on the reader's buffering. Bytes are not
    /// necessarily UTF-8 and remain unchanged, including NUL and high bytes.
    Text(Vec<u8>),
}

/// A byte position in the original RTF stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RtfSourcePosition {
    /// Zero-based byte offset, including any skipped binary data.
    pub offset: u64,
    /// One-based line number; only LF starts a new line. Saturates at `u64::MAX`.
    pub line_number: u64,
    /// One-based byte position within the line, saturating at `u64::MAX`.
    pub line_position: u64,
    /// Enclosing group depth, saturating at `usize::MAX`.
    /// Both braces use the depth outside their group.
    pub depth: usize,
}

/// An owned token and the position of its first source byte.
///
/// Unlike MimeKit's reusable token buffers, this remains valid after subsequent
/// reads. Positions for control words and escapes point to their backslash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtfToken {
    /// Token data.
    pub kind: RtfTokenKind,
    /// Original source position.
    pub position: RtfSourcePosition,
}

/// A lexical recovery or report-truncation marker from an [`RtfTokenizer`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RtfLexicalViolation {
    /// A control word had more than 32 letters.
    KeywordTooLong,
    /// A parameter had excess digits, overflowed, or had a malformed sign.
    InvalidParameter,
    /// A hexadecimal escape had fewer than two valid digits.
    InvalidHexEscape,
    /// EOF occurred inside binary data or with an unclosed group.
    TruncatedStream,
    /// The first issue suppressed by a per-violation reporting limit.
    ///
    /// Emitted once per tokenizer at that issue's source position. This marks
    /// an incomplete diagnostic report, not truncated input.
    TooManyComplianceIssues,
}

/// A lexical issue and its original source position.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RtfLexicalIssue {
    /// The recovery condition.
    pub violation: RtfLexicalViolation,
    /// The affected token's position, or EOF for truncated input.
    pub position: RtfSourcePosition,
}

/// Options for a streaming RTF tokenizer's lexical diagnostic reporting.
///
/// Options are copied on construction; there is no mutable global default.
/// They affect reporting only, never tokenization or recovery.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RtfTokenizerOptions {
    /// Maximum reports per violation for one tokenizer; zero means unlimited.
    ///
    /// Budgets persist across reads, callback changes, and retryable I/O errors.
    /// Calls to [`RtfTokenizer::read_token`] discard issues without using any
    /// budget. A new tokenizer starts with fresh counters.
    ///
    /// The first suppressed issue produces one
    /// [`RtfLexicalViolation::TooManyComplianceIssues`] marker at its position.
    /// Reaching the limit alone does not produce a marker. Other kinds of
    /// violations retain their independent budgets.
    ///
    /// For a positive limit, at most four times the limit plus one issues are
    /// delivered. The tokenizer uses fixed counters, not an issue collection.
    pub max_issues_per_violation: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanState {
    Default,
    Backslash,
    Keyword,
    ParameterSign,
    ParameterDigits,
    Hex1,
    Hex2,
}

/// A bounded, synchronous RTF lexer over a buffered byte reader.
///
/// Pass original RTF bytes, not charset-decoded Unicode. Real-world RTF often
/// contains raw high bytes, and binary lengths count bytes. Charset/font
/// interpretation, Unicode fallback handling, destination filtering, rendering,
/// full compliance checks, and async I/O are not provided.
///
/// Control names retain at most 32 letters; parameters retain the first ten
/// digits and clamp to `i32`. Positive `\binN` payloads are skipped directly in
/// the reader, without allocation proportional to N or interpreting braces.
/// Text tokens own at most 4096 bytes. The reader's own buffer is separate.
///
/// Raw CR/LF are ignored; escaped CR/LF become LF control symbols. Malformed
/// zero-digit hex escapes and trailing backslashes are dropped; one-digit hex
/// escapes yield that digit's value. Recovery follows MimeKit.
///
/// [`Self::read_token_with_issues`] reports lexical recoveries without storing
/// an unbounded issue collection. Ordinary [`Self::read_token`] does not report
/// issues. Neither mode validates the RTF header or rejects unmatched end groups.
/// [`Self::with_options`] can cap reports per violation for the entire stream,
/// emitting one marker at the first suppressed issue. The default is unlimited.
///
/// Source errors, including `Interrupted` and `WouldBlock`, are propagated
/// without resetting pending lexical state. Calls may be retried. An empty
/// source buffer permanently latches EOF; pending words or hex bytes are still
/// emitted before `None`. Repeated reads after `None` perform no I/O.
///
/// ```
/// use mimecraft::text::{RtfKeyword, RtfTokenKind, RtfTokenizer};
///
/// let mut tokenizer = RtfTokenizer::new(&b"{\\rtf1 hello}"[..]);
/// assert_eq!(tokenizer.read_token()?.unwrap().kind, RtfTokenKind::GroupStart);
/// let token = tokenizer.read_token()?.unwrap();
/// let RtfTokenKind::ControlWord(word) = token.kind else { panic!("expected word") };
/// assert_eq!(word.id(), RtfKeyword::Rtf);
/// assert_eq!(word.parameter(), Some(1));
/// assert_eq!(tokenizer.read_token()?.unwrap().kind, RtfTokenKind::Text(b"hello".to_vec()));
/// assert_eq!(tokenizer.read_token()?.unwrap().kind, RtfTokenKind::GroupEnd);
/// assert!(tokenizer.read_token()?.is_none());
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct RtfTokenizer<R> {
    inner: R,
    keyword: String,
    keyword_truncated: bool,
    parameter_invalid: bool,
    parameter_digits: u8,
    negative: bool,
    parameter: u64,
    hex_value: u8,
    binary_remaining: usize,
    state: ScanState,
    offset: u64,
    line_start: u64,
    line_number: u64,
    depth: usize,
    token_position: RtfSourcePosition,
    eof: bool,
    finished: bool,
    reported_truncation: bool,
    reporting: RtfReportingLimits<{ RtfLexicalViolation::TooManyComplianceIssues as usize }>,
}

impl<R: BufRead> RtfTokenizer<R> {
    /// Wraps a buffered source without reading. Use `BufReader` for a `Read`
    /// source; byte slices already implement `BufRead`. Reporting is unlimited.
    pub fn new(inner: R) -> Self {
        Self::with_options(inner, &RtfTokenizerOptions::default())
    }

    /// Wraps a source without reading, copying its diagnostic reporting options.
    ///
    /// Limits apply independently to each tokenizer, not each read or callback.
    ///
    /// ```
    /// use mimecraft::text::{RtfLexicalViolation, RtfTokenizer, RtfTokenizerOptions};
    ///
    /// let options = RtfTokenizerOptions { max_issues_per_violation: 1 };
    /// let mut tokenizer = RtfTokenizer::with_options(&b"\\'z\\'z\\'z"[..], &options);
    /// let mut issues = Vec::new();
    /// while tokenizer.read_token_with_issues(|issue| issues.push(issue))?.is_some() {}
    /// assert_eq!(issues.len(), 2);
    /// assert_eq!(issues[0].violation, RtfLexicalViolation::InvalidHexEscape);
    /// assert_eq!(issues[1].violation, RtfLexicalViolation::TooManyComplianceIssues);
    /// assert_eq!(issues[1].position.offset, 3);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn with_options(inner: R, options: &RtfTokenizerOptions) -> Self {
        Self {
            inner,
            keyword: String::with_capacity(MAX_KEYWORD_LENGTH),
            keyword_truncated: false,
            parameter_invalid: false,
            parameter_digits: 0,
            negative: false,
            parameter: 0,
            hex_value: 0,
            binary_remaining: 0,
            state: ScanState::Default,
            offset: 0,
            line_start: 0,
            line_number: 1,
            depth: 0,
            token_position: RtfSourcePosition {
                offset: 0,
                line_number: 1,
                line_position: 1,
                depth: 0,
            },
            eof: false,
            finished: false,
            reported_truncation: false,
            // One counter per defect in the dense enum range. The marker is
            // not a document defect, so it has no counter or reporting budget.
            reporting: RtfReportingLimits::new(options.max_issues_per_violation),
        }
    }

    /// Returns the buffered source without advancing it.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Returns mutable access to the source.
    ///
    /// Reading or seeking it directly invalidates lexical state and positions.
    /// Changing the source does not clear this tokenizer's EOF latch.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Recovers the buffered source, retaining its unconsumed read-ahead bytes.
    ///
    /// Pending lexical state is discarded. Do not use this to resume a token
    /// that was partially consumed before an I/O error.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Reads an owned token without reporting lexical recovery issues.
    ///
    /// Discarded issues do not consume reporting budgets or emit a deferred
    /// marker when reporting is enabled on a later read.
    ///
    /// # Errors
    ///
    /// Propagates source errors and reports an unrepresentable byte offset.
    pub fn read_token(&mut self) -> io::Result<Option<RtfToken>> {
        self.read_token_core(None::<fn(RtfLexicalIssue)>)
    }

    /// Reads a token, delivering lexical issues to a caller-owned callback.
    ///
    /// Issues may precede a later I/O error; they are not replayed on retry.
    /// Issues for discarded escapes can occur without a corresponding token.
    /// Truncation is reported once, even when both binary data and groups are
    /// incomplete. Calls without a callback discard, rather than defer, issues.
    /// The callback is not retained and this tokenizer allocates no issue list.
    /// Configured per-violation budgets persist across calls and I/O retries;
    /// changing the callback does not reset them.
    ///
    /// # Errors
    ///
    /// Propagates source errors without losing partial words, escapes, or
    /// binary-skip progress. An unrepresentable byte offset is also an error.
    pub fn read_token_with_issues(
        &mut self,
        report: impl FnMut(RtfLexicalIssue),
    ) -> io::Result<Option<RtfToken>> {
        self.read_token_core(Some(report))
    }

    fn read_token_core(
        &mut self,
        mut report: Option<impl FnMut(RtfLexicalIssue)>,
    ) -> io::Result<Option<RtfToken>> {
        if self.finished {
            return Ok(None);
        }

        // All scanning state lives in fields so refills and retryable errors
        // can occur at any point without restarting a partially consumed token.
        loop {
            if self.binary_remaining != 0 {
                let available = self.available()?;
                let count = available.min(self.binary_remaining);
                self.consume(count)?;
                self.binary_remaining -= count;
                if self.binary_remaining != 0 {
                    if self.eof {
                        self.binary_remaining = 0;
                        self.mark_token_start();
                        self.report_truncated(&mut report);
                    }
                    continue;
                }
            }

            if self.available()? == 0 {
                // EOF legitimately terminates pending words and one-digit
                // hex escapes. Emit those before returning end-of-stream.
                match self.state {
                    ScanState::Keyword | ScanState::ParameterSign | ScanState::ParameterDigits => {
                        return Ok(Some(self.complete_keyword(&mut report)));
                    }
                    ScanState::Hex1 => {
                        self.report(RtfLexicalViolation::InvalidHexEscape, &mut report)
                    }
                    ScanState::Hex2 => {
                        self.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
                        return Ok(Some(self.complete_hex()));
                    }
                    _ => {}
                }
                self.state = ScanState::Default;
                self.mark_token_start();
                if self.depth != 0 {
                    self.report_truncated(&mut report);
                }
                self.finished = true;
                return Ok(None);
            }

            let byte = self.inner.fill_buf()?[0];
            match self.state {
                ScanState::Default => {
                    self.mark_token_start();
                    match byte {
                        b'{' => {
                            self.consume(1)?;
                            self.depth = self.depth.saturating_add(1);
                            return Ok(Some(self.token(RtfTokenKind::GroupStart)));
                        }
                        b'}' => {
                            self.consume(1)?;
                            self.depth = self.depth.saturating_sub(1);
                            // Both group braces are at the enclosing group's depth.
                            self.token_position.depth = self.depth;
                            return Ok(Some(self.token(RtfTokenKind::GroupEnd)));
                        }
                        b'\\' => {
                            self.consume(1)?;
                            self.state = ScanState::Backslash;
                        }
                        b'\r' | b'\n' => {
                            // RTF readers ignore raw CR/LF. Escaped line endings
                            // are different: see the Backslash state below.
                            self.consume(1)?;
                        }
                        _ => {
                            let buffer = self.inner.fill_buf()?;
                            let length = buffer
                                .iter()
                                .take(MAX_TEXT_LENGTH)
                                .take_while(|&&c| !matches!(c, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                                .count();
                            let text = buffer[..length].to_vec();
                            self.consume(length)?;
                            return Ok(Some(self.token(RtfTokenKind::Text(text))));
                        }
                    }
                }
                ScanState::Backslash => {
                    self.consume(1)?;
                    // Most names are lowercase ASCII, but real control words
                    // such as Office Math's \mmathPr also contain uppercase.
                    if byte.is_ascii_alphabetic() {
                        self.keyword.clear();
                        self.keyword.push(char::from(byte));
                        self.keyword_truncated = false;
                        self.parameter_invalid = false;
                        self.parameter_digits = 0;
                        self.negative = false;
                        self.parameter = 0;
                        self.state = ScanState::Keyword;
                    } else if byte == b'\'' {
                        // \'hh denotes an 8-bit value in the current character set.
                        self.hex_value = 0;
                        self.state = ScanState::Hex1;
                    } else {
                        // A single non-letter after '\' is a control symbol.
                        // Escaped CR and LF are both equivalent to \par.
                        self.state = ScanState::Default;
                        let symbol = if byte == b'\r' { b'\n' } else { byte };
                        return Ok(Some(self.token(RtfTokenKind::ControlSymbol(symbol))));
                    }
                }
                ScanState::Keyword => {
                    if byte.is_ascii_alphabetic() {
                        self.consume(1)?;
                        // Consume excess letters but retain only the bounded
                        // prefix. An overlong word must remain unknown.
                        if self.keyword.len() < MAX_KEYWORD_LENGTH {
                            self.keyword.push(char::from(byte));
                        } else {
                            self.keyword_truncated = true;
                        }
                    } else if byte == b'-' {
                        // A hyphen after a control word introduces its parameter.
                        self.consume(1)?;
                        self.negative = true;
                        self.state = ScanState::ParameterSign;
                    } else if byte.is_ascii_digit() {
                        self.state = ScanState::ParameterDigits;
                    } else {
                        // One delimiting space belongs to the control word.
                        // Every other delimiter belongs to the following token.
                        if byte == b' ' {
                            self.consume(1)?;
                        }
                        return Ok(Some(self.complete_keyword(&mut report)));
                    }
                }
                ScanState::ParameterSign => {
                    if byte.is_ascii_digit() {
                        self.state = ScanState::ParameterDigits;
                    } else {
                        // A '-' without digits is malformed. Leave the next
                        // byte (even a space) for ordinary scanning.
                        self.parameter_invalid = true;
                        self.negative = false;
                        return Ok(Some(self.complete_keyword(&mut report)));
                    }
                }
                ScanState::ParameterDigits => {
                    if byte.is_ascii_digit() {
                        self.consume(1)?;
                        // Consume all digits, but only accumulate the first ten.
                        // Clamp in complete_keyword rather than overflow.
                        if self.parameter_digits < MAX_PARAMETER_DIGITS {
                            self.parameter = self.parameter * 10 + u64::from(byte - b'0');
                            self.parameter_digits += 1;
                        } else {
                            self.parameter_invalid = true;
                        }
                    } else {
                        if byte == b' ' {
                            self.consume(1)?;
                        }
                        return Ok(Some(self.complete_keyword(&mut report)));
                    }
                }
                ScanState::Hex1 => {
                    if let Some(value) = hex_digit(byte) {
                        self.consume(1)?;
                        self.hex_value = value;
                        self.state = ScanState::Hex2;
                    } else {
                        // No hex digits: drop the escape and scan the current
                        // non-hex byte normally without consuming it here.
                        self.state = ScanState::Default;
                        self.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
                    }
                }
                ScanState::Hex2 => {
                    // One hex digit is accepted leniently as its own value.
                    // Leave a non-hex delimiter for the following token.
                    if let Some(value) = hex_digit(byte) {
                        self.consume(1)?;
                        self.hex_value = (self.hex_value << 4) | value;
                    } else {
                        self.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
                    }
                    return Ok(Some(self.complete_hex()));
                }
            }
        }
    }

    fn available(&mut self) -> io::Result<usize> {
        if self.eof {
            return Ok(0);
        }
        let length = self.inner.fill_buf()?.len();
        self.eof = length == 0;
        Ok(length)
    }

    fn consume(&mut self, count: usize) -> io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let end = self
            .offset
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("RTF input byte offset overflow"))?;
        // Binary payloads can contain LF too. Track them without storing the
        // payload, so positions remain correct even if reporting starts later.
        for (index, &byte) in self.inner.fill_buf()?[..count].iter().enumerate() {
            if byte == b'\n' {
                self.line_number = self.line_number.saturating_add(1);
                self.line_start = self.offset + index as u64 + 1;
            }
        }
        self.inner.consume(count);
        self.offset = end;
        Ok(())
    }

    fn mark_token_start(&mut self) {
        self.token_position = self.source_position();
    }

    pub(super) fn source_position(&self) -> RtfSourcePosition {
        RtfSourcePosition {
            offset: self.offset,
            line_number: self.line_number,
            line_position: (self.offset - self.line_start).saturating_add(1),
            depth: self.depth,
        }
    }

    fn token(&self, kind: RtfTokenKind) -> RtfToken {
        RtfToken {
            kind,
            position: self.token_position,
        }
    }

    fn report(
        &mut self,
        violation: RtfLexicalViolation,
        report: &mut Option<impl FnMut(RtfLexicalIssue)>,
    ) {
        let Some(report) = report else {
            return;
        };
        let violation = match self.reporting.check(violation as usize) {
            RtfReport::Issue => violation,
            RtfReport::Limit => RtfLexicalViolation::TooManyComplianceIssues,
            RtfReport::Suppress => return,
        };
        report(RtfLexicalIssue {
            violation,
            position: self.token_position,
        });
    }

    fn report_truncated(&mut self, report: &mut Option<impl FnMut(RtfLexicalIssue)>) {
        if !self.reported_truncation {
            self.reported_truncation = true;
            self.report(RtfLexicalViolation::TruncatedStream, report);
        }
    }

    fn complete_keyword(&mut self, report: &mut Option<impl FnMut(RtfLexicalIssue)>) -> RtfToken {
        self.state = ScanState::Default;
        let signed = if self.negative {
            -(self.parameter as i64)
        } else {
            self.parameter as i64
        };
        let bounded = signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        self.parameter_invalid |= signed != bounded;
        let id = if self.keyword_truncated {
            RtfKeyword::Unknown
        } else {
            RtfKeyword::from_name(&self.keyword)
        };
        let word = RtfControlWord {
            name: self.keyword.clone(),
            id,
            parameter: (self.parameter_digits != 0).then_some(bounded as i32),
            name_truncated: self.keyword_truncated,
            parameter_invalid: self.parameter_invalid,
        };
        if self.keyword_truncated {
            self.report(RtfLexicalViolation::KeywordTooLong, report);
        }
        if self.parameter_invalid {
            self.report(RtfLexicalViolation::InvalidParameter, report);
        }
        // The bytes after \binN are not RTF and may contain braces or '\'.
        // Skip them in the lexer, not the interpreter, without buffering N bytes.
        if id == RtfKeyword::Bin && bounded > 0 {
            self.binary_remaining = bounded as usize;
        }
        self.token(RtfTokenKind::ControlWord(word))
    }

    fn complete_hex(&mut self) -> RtfToken {
        self.state = ScanState::Default;
        self.token(RtfTokenKind::HexChar(self.hex_value))
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_words_and_digit_runs_retain_bounded_state() {
        let input = format!("\\{}{} x", "a".repeat(100_000), "9".repeat(100_000));
        let mut tokenizer = RtfTokenizer::new(input.as_bytes());
        let capacity = tokenizer.keyword.capacity();
        let token = tokenizer.read_token().unwrap().unwrap();
        assert!(matches!(token.kind, RtfTokenKind::ControlWord(_)));
        assert_eq!(tokenizer.keyword.len(), MAX_KEYWORD_LENGTH);
        assert_eq!(tokenizer.keyword.capacity(), capacity);
        assert_eq!(tokenizer.parameter_digits, MAX_PARAMETER_DIGITS);
        assert_eq!(tokenizer.parameter, 9_999_999_999);
        assert_eq!(tokenizer.binary_remaining, 0);
    }

    #[test]
    fn offset_overflow_is_an_error_without_consuming_input() {
        let mut tokenizer = RtfTokenizer::new(b"{".as_slice());
        tokenizer.offset = u64::MAX;
        let error = tokenizer.read_token().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(*tokenizer.get_ref(), b"{");
        assert_eq!(tokenizer.depth, 0);
        assert_eq!(tokenizer.state, ScanState::Default);
    }

    #[test]
    fn issue_counter_does_not_overflow_at_the_largest_limit() {
        let options = RtfTokenizerOptions {
            max_issues_per_violation: usize::MAX,
        };
        let mut tokenizer = RtfTokenizer::with_options(b"".as_slice(), &options);
        let index = RtfLexicalViolation::InvalidHexEscape as usize;
        tokenizer.reporting.counts[index] = usize::MAX - 1;
        let mut issues = Vec::new();
        {
            let mut report = Some(|issue| issues.push(issue));
            for _ in 0..3 {
                tokenizer.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
            }
        }
        assert_eq!(tokenizer.reporting.counts[index], usize::MAX);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].violation, RtfLexicalViolation::InvalidHexEscape);
        assert_eq!(
            issues[1].violation,
            RtfLexicalViolation::TooManyComplianceIssues
        );
    }

    #[test]
    fn each_defect_has_its_own_counter_but_the_marker_has_none() {
        let options = RtfTokenizerOptions {
            max_issues_per_violation: 2,
        };
        let mut tokenizer = RtfTokenizer::with_options(b"".as_slice(), &options);
        let kinds = [
            RtfLexicalViolation::KeywordTooLong,
            RtfLexicalViolation::InvalidParameter,
            RtfLexicalViolation::InvalidHexEscape,
            RtfLexicalViolation::TruncatedStream,
        ];
        let mut issues = Vec::new();
        {
            let mut report = Some(|issue| issues.push(issue));
            for kind in kinds {
                for _ in 0..10 {
                    tokenizer.report(kind, &mut report);
                }
            }
        }
        assert_eq!(tokenizer.reporting.counts.len(), kinds.len());
        assert!(tokenizer.reporting.counts.iter().all(|&count| count == 2));
        assert_eq!(issues.len(), kinds.len() * 2 + 1);
        for kind in kinds {
            assert_eq!(
                issues
                    .iter()
                    .filter(|issue| issue.violation == kind)
                    .count(),
                2
            );
        }
        assert_eq!(
            issues
                .iter()
                .filter(|issue| issue.violation == RtfLexicalViolation::TooManyComplianceIssues)
                .count(),
            1
        );
    }

    #[test]
    fn unlimited_reporting_and_absent_callbacks_do_not_update_counters() {
        for limit in [0, 1, usize::MAX] {
            let options = RtfTokenizerOptions {
                max_issues_per_violation: limit,
            };
            let mut tokenizer = RtfTokenizer::with_options(b"".as_slice(), &options);
            for _ in 0..100 {
                tokenizer.report(
                    RtfLexicalViolation::InvalidHexEscape,
                    &mut None::<fn(RtfLexicalIssue)>,
                );
            }
            if limit == 0 {
                let mut report = Some(|_| {});
                for _ in 0..100 {
                    tokenizer.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
                }
            }
            assert!(tokenizer.reporting.counts.iter().all(|&count| count == 0));
            assert!(!tokenizer.reporting.reported_limit);
        }
    }
}

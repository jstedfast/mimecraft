// Adapted from MimeKit's Text/RtfTokenizer.cs.
// Reporting limits adapted from Text/RtfComplianceOptions.cs and RtfCappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{
    error::Error,
    fmt,
    io::{self, BufRead},
};

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
    /// Token boundaries may depend on input chunking. Bytes are not
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

/// Outcome of advancing an [`RtfTokenizerEngine`].
#[derive(Clone, Debug, Eq, PartialEq)]
// Keep the outcome tag separate from the token payload, avoiding field
// reshuffling when the caller extracts an owned token from the result.
#[repr(u8)]
pub enum RtfTokenizerResult {
    /// A complete owned token; the supplied slice may still contain input.
    Token(RtfToken),
    /// The supplied slice was exhausted, but EOF has not been signaled.
    NeedInput,
    /// Input was explicitly finished and all pending tokens were returned.
    EndOfInput,
}

/// Rejected input or an unrepresentable source position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtfTokenizerError {
    /// Nonempty input was supplied after [`RtfTokenizerEngine::finish`].
    InputFinished,
    /// Consuming the next bytes would overflow the source byte offset.
    PositionOverflow,
}

impl fmt::Display for RtfTokenizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputFinished => "RTF input has already been finished",
            Self::PositionOverflow => "RTF input byte offset overflow",
        })
    }
}

impl Error for RtfTokenizerError {}

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

/// Synchronous `BufRead` adapter for the sans-I/O [`RtfTokenizerEngine`].
///
/// Pass original RTF bytes, not charset-decoded Unicode. Real-world RTF often
/// contains raw high bytes, and binary lengths count bytes. Charset/font
/// interpretation, Unicode fallback handling, destination filtering, rendering,
/// full compliance checks, and async I/O are not provided.
///
/// Control names retain at most 32 letters; parameters retain the first ten
/// digits and clamp to `i32`. Positive `\binN` payloads are skipped directly in
/// the supplied buffer, without allocation proportional to N or interpreting
/// braces.
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
    engine: RtfTokenizerEngine,
}

/// A bounded, byte-fed RTF lexer with no I/O or retained input buffer.
///
/// [`Self::parse`] advances a caller-owned slice and returns one owned token,
/// [`RtfTokenizerResult::NeedInput`], or [`RtfTokenizerResult::EndOfInput`].
/// Keep passing the unconsumed suffix until it is exhausted. Empty input is
/// not EOF: partial words, parameters, hex escapes, and binary-skip progress
/// survive any number of empty polls. Call [`Self::finish`] after consuming
/// the final input, then parse empty slices to drain pending tokens.
///
/// Control names retain at most 32 letters, parameters accumulate at most ten
/// digits, and text tokens own at most 4096 bytes. Binary payloads are skipped
/// directly from the supplied slices, without copying or retaining them.
/// Adjacent text tokens may split differently with different chunking; their
/// bytes and original source positions are preserved. No charset decoding,
/// rendering, header validation, or destination filtering is performed.
///
/// Recovery and reporting options are shared with [`RtfTokenizer`]. Callbacks
/// are not retained, and calls without one discard issues without spending
/// reporting budgets. An async driver can supply bytes without changing the
/// engine; no async runtime is required.
///
/// ```
/// use mimecraft::text::{RtfTokenKind, RtfTokenizerEngine, RtfTokenizerResult};
///
/// let mut engine = RtfTokenizerEngine::new();
/// let mut input = &b"\\fs-"[..];
/// assert_eq!(engine.parse(&mut input)?, RtfTokenizerResult::NeedInput);
/// let mut input = &b"20 "[..];
/// let RtfTokenizerResult::Token(token) = engine.parse(&mut input)? else {
///     panic!("expected word");
/// };
/// let RtfTokenKind::ControlWord(word) = token.kind else { panic!("expected word") };
/// assert_eq!(word.parameter(), Some(-20));
/// engine.finish();
/// assert_eq!(engine.parse(&mut &[][..])?, RtfTokenizerResult::EndOfInput);
/// # Ok::<(), mimecraft::text::RtfTokenizerError>(())
/// ```
#[derive(Debug)]
pub struct RtfTokenizerEngine {
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
            engine: RtfTokenizerEngine::with_options(options),
        }
    }

    /// Wraps a source and existing engine, preserving partial-token state.
    ///
    /// The source must begin at the engine's next unconsumed byte. This also
    /// preserves EOF and reporting budgets; it does not restart the document.
    pub fn from_parts(inner: R, engine: RtfTokenizerEngine) -> Self {
        Self { inner, engine }
    }

    /// Recovers source and engine without losing partial-token state.
    pub fn into_parts(self) -> (R, RtfTokenizerEngine) {
        (self.inner, self.engine)
    }

    /// Borrows the engine and its current source position.
    pub fn engine(&self) -> &RtfTokenizerEngine {
        &self.engine
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
    #[inline]
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

    #[inline(always)]
    fn read_token_core(
        &mut self,
        mut report: Option<impl FnMut(RtfLexicalIssue)>,
    ) -> io::Result<Option<RtfToken>> {
        loop {
            let mut input = if self.engine.eof {
                &[][..]
            } else {
                self.inner.fill_buf()?
            };
            let length = input.len();
            if input.is_empty() {
                self.engine.finish();
            }
            let result = self.engine.parse_core(&mut input, report.as_mut());
            let consumed = length - input.len();
            // Advance even if a later byte overflowed: the engine and source
            // must agree on every byte already consumed before the error.
            if consumed != 0 {
                self.inner.consume(consumed);
            }
            match result.map_err(io::Error::other)? {
                Some(kind) => return Ok(Some(self.engine.token(kind))),
                None if self.engine.finished => return Ok(None),
                None => {}
            }
        }
    }

    pub(super) fn source_position(&self) -> RtfSourcePosition {
        self.engine.source_position()
    }
}

impl Default for RtfTokenizerEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RtfTokenizerEngine {
    /// Creates an engine with unlimited lexical reporting.
    pub fn new() -> Self {
        Self::with_options(&RtfTokenizerOptions::default())
    }

    /// Creates an engine with copied, per-stream reporting options.
    pub fn with_options(options: &RtfTokenizerOptions) -> Self {
        Self {
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

    /// Signals EOF without discarding pending words or escapes. Idempotent.
    ///
    /// Consume all supplied input first. Then parse empty slices until
    /// [`RtfTokenizerResult::EndOfInput`]; nonempty input is rejected.
    pub fn finish(&mut self) {
        self.eof = true;
    }

    /// Parses one token, advancing `input` past consumed bytes.
    ///
    /// Empty input before [`Self::finish`] returns `NeedInput`, never EOF.
    /// Tokens can be returned without consuming a delimiter belonging to the
    /// next token. No input references are retained. Issues are discarded
    /// without spending reporting budgets.
    ///
    /// # Errors
    ///
    /// Rejects nonempty input after `finish`. A position overflow leaves the
    /// offending bytes unconsumed; earlier progress remains reflected in
    /// `input` and the engine's position.
    #[inline(always)]
    pub fn parse(&mut self, input: &mut &[u8]) -> Result<RtfTokenizerResult, RtfTokenizerError> {
        let token = self.parse_core(input, None::<fn(RtfLexicalIssue)>)?;
        Ok(self.parse_result(token))
    }

    /// Parses one token and reports lexical recoveries without retaining them.
    ///
    /// Issues may be delivered without a token, before `NeedInput`, or before
    /// an error. They are never replayed. Budgets persist across chunks and
    /// callback changes. EOF recovery occurs only after [`Self::finish`].
    ///
    /// # Errors
    ///
    /// Returns the same input/position errors as [`Self::parse`].
    #[inline(always)]
    pub fn parse_with_issues(
        &mut self,
        input: &mut &[u8],
        report: impl FnMut(RtfLexicalIssue),
    ) -> Result<RtfTokenizerResult, RtfTokenizerError> {
        let token = self.parse_core(input, Some(report))?;
        Ok(self.parse_result(token))
    }

    #[inline(always)]
    fn parse_result(&self, token: Option<RtfTokenKind>) -> RtfTokenizerResult {
        match token {
            Some(kind) => RtfTokenizerResult::Token(self.token(kind)),
            None if self.finished => RtfTokenizerResult::EndOfInput,
            None => RtfTokenizerResult::NeedInput,
        }
    }

    #[inline(always)]
    fn parse_core(
        &mut self,
        input: &mut &[u8],
        report: Option<impl FnMut(RtfLexicalIssue)>,
    ) -> Result<Option<RtfTokenKind>, RtfTokenizerError> {
        // Attach the source position only at the public API boundary, rather
        // than copying it through each scanner and adapter result.
        if self.eof && !input.is_empty() {
            return Err(RtfTokenizerError::InputFinished);
        }
        if self.finished {
            return Ok(None);
        }
        // Braces dominate deeply nested documents. Avoid the full scanner's
        // stack frame and scratch state for these single-byte tokens.
        if self.state == ScanState::Default && self.binary_remaining == 0 {
            match input.first() {
                Some(b'{') => return self.consume_group(input, true).map(Some),
                Some(b'}') => return self.consume_group(input, false).map(Some),
                _ => {}
            }
        }
        self.parse_slow(input, report)
    }

    fn parse_slow(
        &mut self,
        input: &mut &[u8],
        mut report: Option<impl FnMut(RtfLexicalIssue)>,
    ) -> Result<Option<RtfTokenKind>, RtfTokenizerError> {
        // All scanning state lives in fields so chunk boundaries
        // can occur at any point without restarting a partially consumed token.
        loop {
            if self.binary_remaining != 0 {
                let count = input.len().min(self.binary_remaining);
                self.consume(input, count)?;
                self.binary_remaining -= count;
                if self.binary_remaining != 0 {
                    if self.eof {
                        self.binary_remaining = 0;
                        self.mark_token_start();
                        self.report_truncated(&mut report);
                    } else {
                        return Ok(None);
                    }
                }
            }

            if input.is_empty() {
                if !self.eof {
                    return Ok(None);
                }
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

            let byte = input[0];
            match self.state {
                ScanState::Default => {
                    self.mark_token_start();
                    match byte {
                        b'{' => {
                            return self.consume_group(input, true).map(Some);
                        }
                        b'}' => {
                            return self.consume_group(input, false).map(Some);
                        }
                        b'\\' => {
                            self.consume(input, 1)?;
                            self.state = ScanState::Backslash;
                        }
                        b'\r' | b'\n' => {
                            // RTF readers ignore raw CR/LF. Escaped line endings
                            // are different: see the Backslash state below.
                            self.consume(input, 1)?;
                        }
                        _ => {
                            let length = input
                                .iter()
                                .take(MAX_TEXT_LENGTH)
                                .take_while(|&&c| !matches!(c, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                                .count();
                            let text = input[..length].to_vec();
                            self.consume(input, length)?;
                            return Ok(Some(RtfTokenKind::Text(text)));
                        }
                    }
                }
                ScanState::Backslash => {
                    self.consume(input, 1)?;
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
                        return Ok(Some(RtfTokenKind::ControlSymbol(symbol)));
                    }
                }
                ScanState::Keyword => {
                    if byte.is_ascii_alphabetic() {
                        let length = input.iter().take_while(|c| c.is_ascii_alphabetic()).count();
                        // Consume excess letters but retain only the bounded
                        // prefix. An overlong word must remain unknown.
                        let stored = length.min(MAX_KEYWORD_LENGTH - self.keyword.len());
                        let letters = std::str::from_utf8(&input[..stored])
                            .expect("control names contain only ASCII letters");
                        self.consume(input, length)?;
                        self.keyword.push_str(letters);
                        self.keyword_truncated |= stored != length;
                    } else if byte == b'-' {
                        // A hyphen after a control word introduces its parameter.
                        self.consume(input, 1)?;
                        self.negative = true;
                        self.state = ScanState::ParameterSign;
                    } else if byte.is_ascii_digit() {
                        self.state = ScanState::ParameterDigits;
                    } else {
                        // One delimiting space belongs to the control word.
                        // Every other delimiter belongs to the following token.
                        if byte == b' ' {
                            self.consume(input, 1)?;
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
                        self.consume(input, 1)?;
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
                            self.consume(input, 1)?;
                        }
                        return Ok(Some(self.complete_keyword(&mut report)));
                    }
                }
                ScanState::Hex1 => {
                    if let Some(value) = hex_digit(byte) {
                        self.consume(input, 1)?;
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
                        self.consume(input, 1)?;
                        self.hex_value = (self.hex_value << 4) | value;
                    } else {
                        self.report(RtfLexicalViolation::InvalidHexEscape, &mut report);
                    }
                    return Ok(Some(self.complete_hex()));
                }
            }
        }
    }

    #[inline]
    fn consume_group(
        &mut self,
        input: &mut &[u8],
        opening: bool,
    ) -> Result<RtfTokenKind, RtfTokenizerError> {
        self.mark_token_start();
        self.consume(input, 1)?;
        let kind = if opening {
            self.depth = self.depth.saturating_add(1);
            RtfTokenKind::GroupStart
        } else {
            self.depth = self.depth.saturating_sub(1);
            // Both group braces are at the enclosing group's depth.
            self.token_position.depth = self.depth;
            RtfTokenKind::GroupEnd
        };
        Ok(kind)
    }

    #[inline(always)]
    fn consume(&mut self, input: &mut &[u8], count: usize) -> Result<(), RtfTokenizerError> {
        if count == 0 {
            return Ok(());
        }
        let end = self
            .offset
            .checked_add(count as u64)
            .ok_or(RtfTokenizerError::PositionOverflow)?;
        // Binary payloads can contain LF too. Track them without storing the
        // payload, so positions remain correct even if reporting starts later.
        // Short lexical runs do not need the bulk newline search.
        if count <= 32 {
            for (index, &byte) in input[..count].iter().enumerate() {
                if byte == b'\n' {
                    self.line_number = self.line_number.saturating_add(1);
                    self.line_start = self.offset + index as u64 + 1;
                }
            }
        } else {
            let mut newlines = memchr::memchr_iter(b'\n', &input[..count]);
            if let Some(last) = newlines.next_back() {
                self.line_number = self
                    .line_number
                    .saturating_add(1)
                    .saturating_add(newlines.count() as u64);
                self.line_start = self.offset + last as u64 + 1;
            }
        }
        *input = &input[count..];
        self.offset = end;
        Ok(())
    }

    fn mark_token_start(&mut self) {
        self.token_position = self.source_position();
    }

    /// Returns the position of the next unconsumed source byte.
    pub fn source_position(&self) -> RtfSourcePosition {
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

    fn complete_keyword(
        &mut self,
        report: &mut Option<impl FnMut(RtfLexicalIssue)>,
    ) -> RtfTokenKind {
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
        RtfTokenKind::ControlWord(word)
    }

    fn complete_hex(&mut self) -> RtfTokenKind {
        self.state = ScanState::Default;
        RtfTokenKind::HexChar(self.hex_value)
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
        let capacity = tokenizer.engine.keyword.capacity();
        let token = tokenizer.read_token().unwrap().unwrap();
        assert!(matches!(token.kind, RtfTokenKind::ControlWord(_)));
        assert_eq!(tokenizer.engine.keyword.len(), MAX_KEYWORD_LENGTH);
        assert_eq!(tokenizer.engine.keyword.capacity(), capacity);
        assert_eq!(tokenizer.engine.parameter_digits, MAX_PARAMETER_DIGITS);
        assert_eq!(tokenizer.engine.parameter, 9_999_999_999);
        assert_eq!(tokenizer.engine.binary_remaining, 0);
    }

    #[test]
    fn offset_overflow_is_an_error_without_consuming_input() {
        let mut tokenizer = RtfTokenizer::new(b"{".as_slice());
        tokenizer.engine.offset = u64::MAX;
        let error = tokenizer.read_token().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(*tokenizer.get_ref(), b"{");
        assert_eq!(tokenizer.engine.depth, 0);
        assert_eq!(tokenizer.engine.state, ScanState::Default);
    }

    #[test]
    fn engine_offset_overflow_preserves_prior_progress_and_partial_state() {
        let mut engine = RtfTokenizerEngine::new();
        engine.offset = u64::MAX - 1;
        let mut input = b"\\b".as_slice();
        assert_eq!(
            engine.parse(&mut input),
            Err(RtfTokenizerError::PositionOverflow)
        );
        assert_eq!(input, b"b");
        assert_eq!(engine.offset, u64::MAX);
        assert_eq!(engine.state, ScanState::Backslash);
        assert_eq!(engine.token_position.offset, u64::MAX - 1);
        assert_eq!(
            engine.parse(&mut input),
            Err(RtfTokenizerError::PositionOverflow)
        );
        assert_eq!(input, b"b");
    }

    #[test]
    fn short_and_bulk_consumption_saturate_lines_and_track_the_last_newline() {
        for count in [0, 1, 2, 31, 32, 33, 64, 4096] {
            let mut bytes = vec![b'x'; count + 1];
            for (index, byte) in bytes[..count].iter_mut().enumerate() {
                if index % 7 == 0 {
                    *byte = b'\n';
                }
            }
            let mut engine = RtfTokenizerEngine::new();
            engine.offset = 100;
            engine.line_start = 90;
            engine.line_number = u64::MAX - 1;
            let mut input = bytes.as_slice();
            engine.consume(&mut input, count).unwrap();
            let lines = bytes[..count].iter().filter(|&&b| b == b'\n').count();
            let line_start = bytes[..count]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(90, |index| 101 + index as u64);
            assert_eq!(input, b"x");
            assert_eq!(engine.offset, 100 + count as u64);
            assert_eq!(
                engine.line_number,
                (u64::MAX - 1).saturating_add(lines as u64)
            );
            assert_eq!(engine.line_start, line_start);
            assert_eq!(
                engine.source_position().line_position,
                101 + count as u64 - line_start
            );
        }
    }

    #[test]
    fn issue_counter_does_not_overflow_at_the_largest_limit() {
        let options = RtfTokenizerOptions {
            max_issues_per_violation: usize::MAX,
        };
        let mut tokenizer = RtfTokenizerEngine::with_options(&options);
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
        let mut tokenizer = RtfTokenizerEngine::with_options(&options);
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
            let mut tokenizer = RtfTokenizerEngine::with_options(&options);
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

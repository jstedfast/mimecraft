// Structural checks adapted from MimeKit's Text/RtfInterpreter.cs.
// Reporting options adapted from Text/RtfComplianceOptions.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::io::{self, BufRead};

use super::rtf_reporting::{RtfReport, RtfReportingLimits};
use super::{
    RtfKeyword, RtfLexicalIssue, RtfLexicalViolation, RtfSourcePosition, RtfToken, RtfTokenKind,
    RtfTokenizer,
};

/// An RTF diagnostic reported by [`RtfValidator`], [`super::RtfToText`],
/// or [`super::RtfToHtml`].
///
/// The validator reports only lexical and document-structure conditions.
/// Conversion additionally reports interpretation conditions; color-table and
/// field-instruction checks are performed only by the HTML converter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RtfViolation {
    /// A control word had more than 32 letters.
    KeywordTooLong,
    /// A numeric parameter was lexically malformed or outside its supported range.
    InvalidParameter,
    /// A hexadecimal escape had fewer than two valid digits.
    InvalidHexEscape,
    /// EOF occurred inside binary data or with an unclosed group.
    TruncatedStream,
    /// The first two tokens were not `{` and `\rtf`, or EOF interrupted them.
    ///
    /// The version parameter is not validated. Reported once per stream.
    MissingRtfHeader,
    /// A closing brace had no corresponding opening brace.
    UnbalancedGroupEnd,
    /// Non-padding content followed the first closed top-level group.
    ///
    /// Stray closing braces are reported separately. Reported once per stream.
    ContentAfterDocument,
    /// A group requiring another distinct saved state exceeded the limit.
    GroupDepthLimitExceeded,
    /// A new font number was discarded because the font table was full.
    FontTableLimitExceeded,
    /// A selected font was not present in the recorded font table.
    UndefinedFont,
    /// A font size was outside the supported range of 1 through 3276 half-points.
    InvalidFontSize,
    /// A requested code page is unsupported; Windows-1252 was substituted.
    UnsupportedCodePage,
    /// The code-page cache was full; an uncached page used Windows-1252.
    CodePageLimitExceeded,
    /// Malformed encoded bytes were replaced while decoding a text run.
    InvalidByteSequence,
    /// A raw high byte occurred in RTF text rather than in an escape.
    UnescapedEightBitCharacter,
    /// An unpaired UTF-16 surrogate was replaced by U+FFFD.
    UnpairedSurrogate,
    /// A color-table entry was discarded because the table was full.
    ColorTableLimitExceeded,
    /// A nonzero color index did not name a recorded color.
    UndefinedColor,
    /// A field instruction exceeded the retained UTF-8 byte limit.
    FieldInstructionTooLong,
    /// The first issue suppressed by a per-violation reporting limit.
    ///
    /// This marks an incomplete report, not truncated input. Emitted once per
    /// stream or conversion at the first suppressed issue's source position.
    TooManyComplianceIssues,
}

impl From<RtfLexicalViolation> for RtfViolation {
    fn from(violation: RtfLexicalViolation) -> Self {
        match violation {
            RtfLexicalViolation::KeywordTooLong => Self::KeywordTooLong,
            RtfLexicalViolation::InvalidParameter => Self::InvalidParameter,
            RtfLexicalViolation::InvalidHexEscape => Self::InvalidHexEscape,
            RtfLexicalViolation::TruncatedStream => Self::TruncatedStream,
            RtfLexicalViolation::TooManyComplianceIssues => Self::TooManyComplianceIssues,
        }
    }
}

/// An RTF diagnostic and its original byte position.
///
/// Structural issues use the affected token's start, or EOF when the header
/// is incomplete. Text token boundaries can depend on source buffering.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RtfIssue {
    /// The lexical, structural, or interpretation condition.
    pub violation: RtfViolation,
    /// Original source position, including the enclosing group depth.
    pub position: RtfSourcePosition,
}

impl From<RtfLexicalIssue> for RtfIssue {
    fn from(issue: RtfLexicalIssue) -> Self {
        Self {
            violation: issue.violation.into(),
            position: issue.position,
        }
    }
}

/// Diagnostic reporting options copied when an [`RtfValidator`] is constructed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RtfValidationOptions {
    /// Maximum reports per violation for one stream; zero means unlimited.
    ///
    /// Lexical and structural defects have independent quotas but share one
    /// [`RtfViolation::TooManyComplianceIssues`] marker. Budgets persist across
    /// reads, callback changes, and retryable source errors. Callback-free reads
    /// do not spend them. A new validator gets fresh counters.
    ///
    /// With a positive limit, at most seven times the limit plus one issues
    /// reach callbacks. The validator stores fixed counters, not an issue list.
    pub max_issues_per_violation: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderState {
    ExpectGroupStart,
    ExpectRtf,
    Done,
}

/// Streaming lexical and document-structure validation without changing tokens.
///
/// Wraps an [`RtfTokenizer`] and adds MimeKit's missing-header, unmatched-closing-
/// brace, and content-after-document checks. This is not full RTF validation:
/// destinations, formatting, font/color tables, code pages, Unicode fallback,
/// and field instructions are not interpreted. Nothing is rendered or rejected.
///
/// Tokens, lexical recovery, binary skipping, and source positions are identical
/// to those of a tokenizer using the same buffering. Structural checks need
/// constant additional storage, without recursion or a group stack.
///
/// ```
/// use mimecraft::text::{RtfValidator, RtfViolation};
///
/// let mut validator = RtfValidator::new(&b"{\\rtf1 hello}}junk"[..]);
/// let mut issues = Vec::new();
/// while validator.read_token_with_issues(|issue| issues.push(issue))?.is_some() {}
/// assert_eq!(issues.len(), 2);
/// assert_eq!(issues[0].violation, RtfViolation::UnbalancedGroupEnd);
/// assert_eq!(issues[1].violation, RtfViolation::ContentAfterDocument);
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct RtfValidator<R> {
    tokenizer: RtfTokenizer<R>,
    reporting: RtfReportingLimits<{ RtfViolation::TooManyComplianceIssues as usize }>,
    header: HeaderState,
    depth: usize,
    root_group_closed: bool,
    reported_trailing_content: bool,
    finished: bool,
}

impl<R: BufRead> RtfValidator<R> {
    /// Wraps a buffered source without reading, with unlimited reporting.
    pub fn new(inner: R) -> Self {
        Self::with_options(inner, &RtfValidationOptions::default())
    }

    /// Wraps a source without reading, copying its reporting options.
    ///
    /// Limits affect issue delivery only, never tokens or structural tracking.
    pub fn with_options(inner: R, options: &RtfValidationOptions) -> Self {
        Self {
            // Cap the combined report here, rather than capping the lexer too:
            // otherwise lexical suppression could consume a second marker.
            tokenizer: RtfTokenizer::new(inner),
            reporting: RtfReportingLimits::new(options.max_issues_per_violation),
            header: HeaderState::ExpectGroupStart,
            depth: 0,
            root_group_closed: false,
            reported_trailing_content: false,
            finished: false,
        }
    }

    /// Returns the buffered source without advancing it.
    pub fn get_ref(&self) -> &R {
        self.tokenizer.get_ref()
    }

    /// Returns mutable access to the source.
    ///
    /// Reading or seeking directly invalidates lexical and structural state.
    /// Replacing the source does not clear the EOF latch.
    pub fn get_mut(&mut self) -> &mut R {
        self.tokenizer.get_mut()
    }

    /// Recovers the buffered source with its unconsumed read-ahead bytes.
    ///
    /// Pending lexical and structural state is discarded. This cannot resume
    /// a partially consumed token after an I/O error.
    pub fn into_inner(self) -> R {
        self.tokenizer.into_inner()
    }

    pub(super) fn source_position(&self) -> RtfSourcePosition {
        self.tokenizer.source_position()
    }

    /// Reads an owned token, discarding diagnostics without spending quotas.
    ///
    /// Structural tracking still advances. Discarded issues, including once-
    /// per-stream conditions, are not replayed if reporting is enabled later.
    ///
    /// # Errors
    ///
    /// Propagates source errors and reports an unrepresentable byte offset.
    pub fn read_token(&mut self) -> io::Result<Option<RtfToken>> {
        self.read_token_core(None::<fn(RtfIssue)>)
    }

    /// Reads a token, delivering lexical and structural issues to a callback.
    ///
    /// Lexical issues are delivered before checking the returned token's
    /// structure. At EOF, lexical truncation precedes an incomplete-header
    /// issue. An initially unmatched `}` reports both a missing header and an
    /// unmatched group end, in that order.
    ///
    /// The callback is not retained. Source errors leave partial tokens and
    /// structural state available for retry; issues delivered before an error
    /// are not replayed. After `None`, repeated reads perform no I/O or reporting.
    ///
    /// # Errors
    ///
    /// Propagates source errors, including `Interrupted` and `WouldBlock`,
    /// without resetting state. An unrepresentable byte offset is also an error.
    pub fn read_token_with_issues(
        &mut self,
        report: impl FnMut(RtfIssue),
    ) -> io::Result<Option<RtfToken>> {
        self.read_token_core(Some(report))
    }

    fn read_token_core(
        &mut self,
        mut report: Option<impl FnMut(RtfIssue)>,
    ) -> io::Result<Option<RtfToken>> {
        if self.finished {
            return Ok(None);
        }

        let token = if report.is_some() {
            let reporting = &mut self.reporting;
            self.tokenizer.read_token_with_issues(|issue| {
                Self::report(reporting, issue.into(), &mut report);
            })?
        } else {
            self.tokenizer.read_token()?
        };

        let Some(token) = token else {
            self.finished = true;
            // An empty document or a lone opening brace has no complete
            // {\rtf header either. Finish once, at the actual EOF position.
            if self.header != HeaderState::Done {
                self.header = HeaderState::Done;
                self.report_structure(
                    RtfViolation::MissingRtfHeader,
                    self.tokenizer.source_position(),
                    &mut report,
                );
            }
            return Ok(None);
        };

        // Track structure even without a callback, so enabling reporting later
        // cannot mistake the middle of the document for its header.
        // RTF "RTF Syntax": a file starts with '{', then a header beginning
        // with \rtfN. Check presence, not the version or the rest of the header.
        if self.header != HeaderState::Done {
            match (self.header, &token.kind) {
                (HeaderState::ExpectGroupStart, RtfTokenKind::GroupStart) => {
                    self.header = HeaderState::ExpectRtf;
                }
                (HeaderState::ExpectRtf, RtfTokenKind::ControlWord(word))
                    if word.id() == RtfKeyword::Rtf =>
                {
                    self.header = HeaderState::Done;
                }
                _ => {
                    self.header = HeaderState::Done;
                    self.report_structure(
                        RtfViolation::MissingRtfHeader,
                        token.position,
                        &mut report,
                    );
                }
            }
        } else if self.root_group_closed && self.depth == 0 && !self.reported_trailing_content {
            // Whitespace and NUL padding after the document are common and
            // harmless. Raw CR/LF were already removed by the tokenizer.
            // A stray '}' is reported as an unbalanced group end instead.
            let ignorable = match &token.kind {
                RtfTokenKind::GroupEnd => true,
                RtfTokenKind::Text(bytes) => bytes
                    .iter()
                    .all(|byte| matches!(byte, b' ' | b'\t' | b'\0')),
                _ => false,
            };
            if !ignorable {
                self.reported_trailing_content = true;
                self.report_structure(
                    RtfViolation::ContentAfterDocument,
                    token.position,
                    &mut report,
                );
            }
        }

        match token.kind {
            RtfTokenKind::GroupStart => self.depth = self.depth.saturating_add(1),
            RtfTokenKind::GroupEnd if self.depth == 0 => {
                // Real writers emit unbalanced '}'. Be lenient rather than
                // failing the document: retain the token and report the defect.
                self.report_structure(
                    RtfViolation::UnbalancedGroupEnd,
                    token.position,
                    &mut report,
                );
            }
            RtfTokenKind::GroupEnd => {
                self.depth -= 1;
                // Once the first root closes, later groups are trailing
                // content, not new roots that restart header validation.
                self.root_group_closed |= self.depth == 0;
            }
            _ => {}
        }

        Ok(Some(token))
    }

    fn report_structure(
        &mut self,
        violation: RtfViolation,
        position: RtfSourcePosition,
        report: &mut Option<impl FnMut(RtfIssue)>,
    ) {
        Self::report(
            &mut self.reporting,
            RtfIssue {
                violation,
                position,
            },
            report,
        );
    }

    fn report(
        reporting: &mut RtfReportingLimits<{ RtfViolation::TooManyComplianceIssues as usize }>,
        mut issue: RtfIssue,
        report: &mut Option<impl FnMut(RtfIssue)>,
    ) {
        let Some(report) = report else {
            return;
        };
        match reporting.check(issue.violation as usize) {
            RtfReport::Issue => {}
            RtfReport::Limit => issue.violation = RtfViolation::TooManyComplianceIssues,
            RtfReport::Suppress => return,
        }
        report(issue);
    }
}

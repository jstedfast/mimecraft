// Adapted from MimeKit's MimeCompliance*.cs, IMimeComplianceLogger.cs,
// and CappedComplianceLogger.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::{fmt, io, ops};

mod encoding;
mod reporting;
mod violation;

pub(crate) use reporting::{HeaderEnd, Reporting};
pub use violation::MimeComplianceViolation;

/// How the message is being used; affects severity, not which issues are reported.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MimeComplianceContext {
    /// Network transport, where CRLF and SMTP line limits apply.
    #[default]
    Transport,
    /// Local storage, where bare LF and long lines are commonplace.
    Storage,
}

/// Practical harm rather than a decision to accept or reject a message.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MimeComplianceSeverity {
    /// A routine or well-defined deviation.
    Minor,
    /// Significant ambiguity, corruption, or incomplete inspection.
    Major,
    /// A particularly serious ambiguity or injection primitive.
    Critical,
}

/// What an issue's position identifies, without pretending every defect is one byte.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum MimeCompliancePositionKind {
    /// The offending byte, or EOF where the defect was detected.
    #[default]
    Exact,
    /// Search this physical line to locate the offending bytes.
    LineStart,
    /// The affected header or body part; it may span multiple lines.
    ElementStart,
}

/// Independently selectable kinds of harm, not evidence of an attack.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct MimeComplianceCategories(u8);

impl MimeComplianceCategories {
    /// No categories.
    pub const NONE: Self = Self(0);
    /// Other software may reject, repair, or render the message differently.
    pub const INTEROPERABILITY: Self = Self(1);
    /// Decoding may lose or corrupt content.
    pub const DATA_LOSS: Self = Self(2);
    /// Different consumers can be made to see different messages or content.
    pub const SECURITY: Self = Self(4);

    /// Returns the underlying category bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether all the specified categories are present.
    pub const fn contains(self, categories: Self) -> bool {
        self.0 & categories.0 == categories.0
    }

    /// Returns whether any specified category is present.
    pub const fn intersects(self, categories: Self) -> bool {
        self.0 & categories.0 != 0
    }
}

impl ops::BitOr for MimeComplianceCategories {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl ops::BitAnd for MimeComplianceCategories {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// A diagnostic in the original input, before normalization or transfer decoding.
///
/// Lines and known columns are one-based. A zero column means unknown. Position
/// kind distinguishes exact bytes from line/element starts, avoiding rescanning
/// merely to refine a position that a consumer may never display.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MimeComplianceIssue {
    /// Context used to rate severity.
    pub context: MimeComplianceContext,
    /// The detected violation.
    pub violation: MimeComplianceViolation,
    /// Zero-based original stream offset.
    pub stream_offset: u64,
    /// One-based physical line number.
    pub line_number: u64,
    /// One-based byte column, or zero if unknown.
    pub column_number: u64,
    /// What the position refers to.
    pub position_kind: MimeCompliancePositionKind,
}

impl MimeComplianceIssue {
    /// Returns the severity in this issue's context.
    pub fn severity(&self) -> MimeComplianceSeverity {
        self.violation.severity(self.context)
    }

    /// Returns the independently selectable kinds of harm.
    pub fn categories(&self) -> MimeComplianceCategories {
        self.violation.categories()
    }

    /// Returns a short description.
    pub fn description(&self) -> &'static str {
        self.violation.description()
    }

    /// Returns the detailed explanation.
    pub fn remarks(&self) -> &'static str {
        self.violation.remarks()
    }
}

impl fmt::Display for MimeComplianceIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at line {}", self.violation, self.line_number)?;
        if self.column_number != 0 {
            write!(f, ", column {}", self.column_number)?;
        }
        write!(f, " (offset {})", self.stream_offset)
    }
}

/// Optional synchronous diagnostic sink. Closures returning `io::Result<()>`
/// implement this trait automatically.
///
/// Returning an error aborts parsing immediately, like throwing from MimeKit's
/// logger. Errors propagate unchanged; no partial owned tree is returned.
/// Logging work is part of parsing time. Sinks retaining issues should impose
/// their own storage limit as well as configuring a per-violation report cap.
pub trait MimeComplianceLogger {
    /// Records an issue, or returns the caller's reason to abort.
    fn log(&mut self, issue: MimeComplianceIssue) -> io::Result<()>;
}

impl<F> MimeComplianceLogger for F
where
    F: FnMut(MimeComplianceIssue) -> io::Result<()>,
{
    fn log(&mut self, issue: MimeComplianceIssue) -> io::Result<()> {
        self(issue)
    }
}

/// Deeper content checks, independently selectable without changing parsing.
///
/// Bit values match MimeKit. Address validation is not yet implemented.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MimeComplianceValidators(u8);

impl MimeComplianceValidators {
    /// Disable deeper validators, retaining structural and raw-byte checks.
    pub const NONE: Self = Self(0);
    /// Validate Base64 alphabet, padding, and complete quantums.
    pub const BASE64: Self = Self(1);
    /// Validate quoted-printable escapes and soft breaks.
    pub const QUOTED_PRINTABLE: Self = Self(2);
    /// Validate UU begin/end markers, file mode, and encoded lines.
    pub const UU_ENCODE: Self = Self(4);
    /// All currently implemented validators.
    pub const ALL: Self = Self(7);

    /// Returns the underlying validator bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether all specified validators are enabled.
    pub const fn contains(self, validators: Self) -> bool {
        self.0 & validators.0 == validators.0
    }
}

impl Default for MimeComplianceValidators {
    fn default() -> Self {
        Self::ALL
    }
}

impl ops::BitOr for MimeComplianceValidators {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl ops::BitAnd for MimeComplianceValidators {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Reporting settings copied at the beginning of each logged read/parse.
///
/// These do not change parsing or recovery. No checks run without a logger.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MimeComplianceOptions {
    /// Severity context; defaults to network transport.
    pub context: MimeComplianceContext,
    /// Deeper validators to run; defaults to all supported checks.
    ///
    /// A validator can be disabled if it is too expensive for, or misbehaves on,
    /// a deployment's messages, without disabling other compliance reporting.
    pub enabled_validators: MimeComplianceValidators,
    /// Per-violation cap per operation; zero means unlimited.
    ///
    /// Each violation has its own budget: a flood of cheap minor defects must
    /// not crowd out a later serious defect. The first suppressed issue emits
    /// one `TooManyComplianceIssues` marker, not one marker per exhausted kind.
    pub max_issues_per_violation: usize,
}

/// A reusable per-violation capped logger.
///
/// A zero limit is unlimited. Reader/parser options apply the same mechanism
/// automatically and reset it for each operation, including each mbox message.
pub struct CappedMimeComplianceLogger<L> {
    logger: L,
    budget: Budget,
}

impl<L: MimeComplianceLogger> CappedMimeComplianceLogger<L> {
    /// Wraps a logger with independent budgets for all violation kinds.
    pub fn new(logger: L, limit: usize) -> Self {
        Self {
            logger,
            budget: Budget::new(limit),
        }
    }

    /// Resets quotas and the report-truncation marker for a new operation.
    pub fn reset(&mut self) {
        self.budget.reset();
    }

    /// Returns the per-violation limit; zero means unlimited.
    pub fn limit(&self) -> usize {
        self.budget.limit
    }

    /// Borrows the underlying logger.
    pub fn get_ref(&self) -> &L {
        &self.logger
    }

    /// Returns the underlying logger.
    pub fn into_inner(self) -> L {
        self.logger
    }
}

impl<L: MimeComplianceLogger> MimeComplianceLogger for CappedMimeComplianceLogger<L> {
    fn log(&mut self, issue: MimeComplianceIssue) -> io::Result<()> {
        if let Some(issue) = self.budget.check(issue) {
            self.logger.log(issue)?;
        }
        Ok(())
    }
}

struct Budget {
    limit: usize,
    // One counter per defect: a small dense range needs no dictionary.
    // TooManyComplianceIssues is outside this range, never counted or suppressed.
    counts: [usize; 91],
    reported_truncation: bool,
}

impl Budget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            counts: [0; 91],
            reported_truncation: false,
        }
    }

    fn reset(&mut self) {
        self.counts.fill(0);
        self.reported_truncation = false;
    }

    fn check(&mut self, issue: MimeComplianceIssue) -> Option<MimeComplianceIssue> {
        if self.limit == 0 {
            return Some(issue);
        }
        let Some(count) = self.counts.get_mut(issue.violation as usize) else {
            return Some(issue);
        };
        if *count < self.limit {
            *count += 1;
            return Some(issue);
        }
        // The report becomes incomplete at the first suppressed issue. Repeating
        // the marker for each later exhausted kind would reintroduce flooding.
        if !self.reported_truncation {
            self.reported_truncation = true;
            return Some(MimeComplianceIssue {
                violation: MimeComplianceViolation::TooManyComplianceIssues,
                position_kind: MimeCompliancePositionKind::Exact,
                ..issue
            });
        }
        None
    }
}
